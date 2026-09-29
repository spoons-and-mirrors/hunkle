import { afterAll, expect, mock, test } from "bun:test"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import { randomUUID } from "node:crypto"
import { createConnection } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"

// Drive the host's reactive effect explicitly; all publication I/O is real.
const effects: Array<() => void> = []
mock.module("@opencode/plugin/tui", () => ({ Plugin: { define: <T>(plugin: T) => plugin } }))
mock.module("solid-js", () => ({ createEffect: (effect: () => void) => { effects.push(effect); effect() } }))
const runtime = await mkdtemp(join(tmpdir(), "hunkle-plugin-test-"))
const original = process.env.XDG_RUNTIME_DIR
process.env.XDG_RUNTIME_DIR = runtime
const plugin = (await import("./tui")).default
const target = join(runtime, "hunkle/opencode-active.json")
afterAll(async () => {
  if (original === undefined) delete process.env.XDG_RUNTIME_DIR
  else process.env.XDG_RUNTIME_DIR = original
  await rm(runtime, { recursive: true, force: true })
})

async function until(check: () => Promise<boolean>) {
  const deadline = Date.now() + 2000
  while (Date.now() < deadline) {
    if (await check()) return
    await Bun.sleep(5)
  }
  throw new Error("Timed out waiting for tab publication")
}

const snapshot = () => readFile(target, "utf8").then(JSON.parse).catch(() => undefined)

function host(prefix: string) {
  const state = {
    active: `${prefix}-a`,
    enabled: true,
    tabs: [
      { sessionID: `${prefix}-a`, title: "First", active: true, busy: false, attention: false },
      { sessionID: `${prefix}-b`, title: "Second", active: false, busy: true, attention: false },
    ],
    titles: {} as Record<string, string>,
    branches: { [`${prefix}-a`]: "main", [`${prefix}-b`]: "feature/b" } as Record<string, string>,
    renameError: false,
  }
  const focused: string[] = []
  const renamed: Array<{ sessionID: string; title: string }> = []
  const errors: string[] = []
  const cleanup = plugin.setup({
    ui: {
      slot: ({ render }: { render: () => void }) => { render(); return () => {} },
      toast: { show: ({ message }: { message: string }) => errors.push(message) },
      router: { current: () => state.active ? { type: "session", sessionID: state.active } : { type: "home" } },
      tabs: {
        enabled: () => state.enabled, list: () => state.tabs,
        focus: (id: string) => { focused.push(id); select(id) },
      },
    },
    client: { session: { update: async ({ sessionID, title }: { sessionID: string; title: string }) => {
      if (state.renameError) throw new Error("server rejected rename")
      renamed.push({ sessionID, title })
      state.titles[sessionID] = title
      update()
    } } },
    data: { session: {
      get: (id: string) => ({ projectID: "project", title: state.titles[id], location: { directory: `/projects/${id}` } }),
      sync: () => Promise.reject(new Error("Cached tabs should not trigger API calls")),
    }, project: { get: () => ({ name: "Repository", canonical: "/projects/repository" }) },
    location: { vcs: { info: ({ directory }: { directory: string }) => ({ branch: { current: state.branches[directory.split("/").at(-1)!] } }) } } },
  })
  const update = effects.at(-1)!
  const select = (id: string) => {
    state.active = id
    state.tabs.forEach((tab) => { tab.active = tab.sessionID === id })
    update()
  }
  return { state, update, select, cleanup, focused, renamed, errors }
}

test("mirrors tab changes, coalesces switches, respects the selected CLI, and cleans up", async () => {
  const first = host("one")
  let second: ReturnType<typeof host> | undefined
  try {
    await until(async () => (await snapshot())?.directory === "/projects/one-a")
    expect((await snapshot()).tabs.map((tab: { title: string }) => tab.title)).toEqual(["First", "Second"])
    expect((await snapshot()).tabs.map((tab: { project: string; branch: string }) => [tab.project, tab.branch]))
      .toEqual([["Repository", "main"], ["Repository", "feature/b"]])
    const inode = (await stat(target)).ino
    first.update()
    await Bun.sleep(25)
    expect((await stat(target)).ino).toBe(inode)

    first.state.tabs[1]!.busy = false
    first.state.tabs[1]!.attention = true
    first.state.tabs[1]!.title = "Renamed"
    first.update()
    await until(async () => (await snapshot())?.tabs[1]?.title === "Renamed")
    expect((await snapshot()).directory).toBe("/projects/one-a")
    expect((await snapshot()).tabs[1].attention).toBe(true)
    first.state.branches["one-b"] = "feature/next"
    first.update()
    await until(async () => (await snapshot())?.tabs[1]?.branch === "feature/next")

    first.state.tabs.reverse()
    first.select("one-b")
    first.select("one-a")
    first.select("one-b")
    await until(async () => (await snapshot())?.directory === "/projects/one-b")
    expect((await snapshot()).tabs[0].sessionID).toBe("one-b")

    second = host("two")
    await until(async () => (await snapshot())?.directory === "/projects/two-a")
    first.state.tabs[0]!.title = "Background change"
    first.update()
    await Bun.sleep(25)
    expect((await snapshot()).directory).toBe("/projects/two-a")
    first.select("one-a")
    await until(async () => (await snapshot())?.directory === "/projects/one-a")

    first.select("")
    await until(async () => (await snapshot())?.activeSessionID === undefined)
    expect((await snapshot()).tabs.every((tab: { active: boolean }) => !tab.active)).toBe(true)
    first.state.tabs.pop()
    first.update()
    await until(async () => (await snapshot())?.tabs.length === 1)
    first.state.enabled = false
    first.update()
    await until(async () => (await snapshot())?.tabs.length === 0)

    second.cleanup()
    expect(await snapshot()).toBeDefined()
    first.cleanup()
    expect(await snapshot()).toBeUndefined()
  } finally {
    first.cleanup()
    second?.cleanup()
  }
})

test("tab requests target one CLI, rename sessions, and never reopen closed tabs", async () => {
  const first = host("click")
  let second: ReturnType<typeof host> | undefined
  let socketPath = ""
  const publish = (json: string) => new Promise<void>((resolve, reject) => {
    const socket = createConnection(socketPath)
    socket.on("connect", () => socket.end(`${json}\n`))
    socket.on("close", () => resolve())
    socket.on("error", reject)
  })
  const request = (instanceID: string, sessionID: string) => JSON.stringify({
    version: 1, instanceID, sessionID, requestID: randomUUID(),
  })
  try {
    await until(async () => (await snapshot())?.directory === "/projects/click-a")
    const firstID = (await snapshot()).instanceID
    socketPath = (await snapshot()).focusSocket
    const json = request(firstID, "click-b")
    await publish(json)
    await until(async () => (await snapshot())?.activeSessionID === "click-b")
    expect(first.focused).toEqual(["click-b"])
    await publish(JSON.stringify({ version: 1, instanceID: firstID, sessionID: "click-a", action: "rename", title: "Named from Hunkle", requestID: randomUUID() }))
    await until(async () => (await snapshot())?.tabs.find((tab: { sessionID: string }) => tab.sessionID === "click-a")?.title === "Named from Hunkle")
    expect(first.renamed).toEqual([{ sessionID: "click-a", title: "Named from Hunkle" }])
    expect(first.errors).toEqual([])
    first.state.renameError = true
    await publish(JSON.stringify({ version: 1, instanceID: firstID, sessionID: "click-a", action: "rename", title: "Another name", requestID: randomUUID() }))
    await until(async () => first.errors.some((message) => message.includes("server rejected rename")))
    first.state.renameError = false
    expect((await snapshot()).tabs.find((tab: { sessionID: string }) => tab.sessionID === "click-a")?.title).toBe("Named from Hunkle")
    expect(first.focused).toEqual(["click-b"])
    await publish(json)
    await Bun.sleep(25)
    expect(first.focused).toEqual(["click-b"])

    first.state.tabs.pop()
    await publish(request(firstID, "click-b"))
    await publish(JSON.stringify({ version: 1, instanceID: firstID, sessionID: "click-b", action: "rename", title: "Closed", requestID: randomUUID() }))
    await Bun.sleep(25)
    expect(first.focused).toEqual(["click-b"])
    expect(first.renamed).toEqual([{ sessionID: "click-a", title: "Named from Hunkle" }])

    // Both CLIs have the same session open: the instance address must disambiguate.
    second = host("click")
    await until(async () => (await snapshot())?.instanceID !== firstID)
    const secondID = (await snapshot()).instanceID
    socketPath = (await snapshot()).focusSocket
    await publish(request(secondID, "click-b"))
    await until(async () => second!.focused.length === 1)
    expect(first.focused).toEqual(["click-b"])
    expect(second.focused).toEqual(["click-b"])

    await publish(request("old-plugin-instance", "click-a"))
    await publish(JSON.stringify({ version: 1, instanceID: "old-plugin-instance", sessionID: "click-a", action: "rename", title: "Invalid", requestID: randomUUID() }))
    await Bun.sleep(25)
    expect(second.focused).toEqual(["click-b"])
    expect(second.renamed).toEqual([])
    await publish(JSON.stringify({ version: 1, instanceID: secondID, sessionID: "click-a", action: "rename", title: "  ", requestID: randomUUID() }))
    await Bun.sleep(25)
    expect(second.renamed).toEqual([])
    second.state.enabled = false
    await publish(request(secondID, "click-a"))
    await Bun.sleep(25)
    expect(second.focused).toEqual(["click-b"])

    second.cleanup()
    await expect(publish(request(secondID, "click-a"))).rejects.toThrow()
    expect(second.focused).toEqual(["click-b"])
  } finally {
    first.cleanup()
    second?.cleanup()
  }
})
