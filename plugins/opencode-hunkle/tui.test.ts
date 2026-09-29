import { afterAll, expect, mock, test } from "bun:test"
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import { readFileSync } from "node:fs"
import type { Plugin } from "@opencode/plugin/tui"
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

function host(prefix: string, enabled = true) {
  const start = effects.length
  let end: number | undefined
  const update = () => effects.slice(start, end).forEach((effect) => effect())
  const state = {
    active: `${prefix}-a`,
    enabled,
    tabs: [
      { sessionID: `${prefix}-a`, title: "First", active: true, busy: false, attention: false },
      { sessionID: `${prefix}-b`, title: "Second", active: false, busy: true, attention: false },
    ],
    titles: {} as Record<string, string>,
    branches: { [`${prefix}-a`]: "main", [`${prefix}-b`]: "feature/b" } as Record<string, string>,
    renameError: false,
    running: new Set<string>(),
    permissions: new Set<string>(),
    forms: new Set<string>(),
    pending: {} as Record<string, Array<{ type: string }>>,
    parents: {} as Record<string, string>,
    time: {} as Record<string, { idle?: number; viewed?: number }>,
    failed: new Set<string>(),
    missing: new Set<string>(),
    offline: false,
  }
  const select = (id: string) => {
    state.active = id
    state.tabs.forEach((tab) => { tab.active = tab.sessionID === id })
    update()
  }
  const focused: string[] = []
  const renamed: Array<{ sessionID: string; title: string }> = []
  const errors: string[] = []
  const synced: string[] = []
  const commands: Array<{ id: string; run: () => void }> = []
  const events = new Map<string, (event: { data: { sessionID: string } }) => void>()
  const catalogPath = join(runtime, `${prefix}-tabs.json`)
  let writes = Promise.resolve()
  const catalog = (() => {
    try { return JSON.parse(readFileSync(catalogPath, "utf8")) }
    catch { return { initialized: false, tabs: [] } }
  })()
  const cleanup = plugin.setup({
    storage: { store: () => [catalog, (mutation: (draft: typeof catalog) => void) => {
      writes = writes.then(async () => {
        const draft = await readFile(catalogPath, "utf8").then(JSON.parse).catch(() => ({ initialized: false, tabs: [] }))
        mutation(draft)
        await writeFile(catalogPath, JSON.stringify(draft))
        Object.assign(catalog, draft)
        update()
      })
      return writes
    }] },
    keymap: { layer: (get: () => { commands: typeof commands }) => commands.push(...get().commands) },
    ui: {
      slot: ({ render }: { render: () => void }) => { render(); return () => {} },
      toast: { show: ({ message }: { message: string }) => errors.push(message) },
      router: {
        current: () => state.active ? { type: "session", sessionID: state.active } : { type: "home" },
        navigate: (route: { type: string; sessionID?: string }) => {
          if (route.sessionID) focused.push(route.sessionID)
          select(route.sessionID ?? "")
        },
      },
      tabs: {
        enabled: () => state.enabled, list: () => state.tabs,
        focus: () => { throw new Error("Use the public router, not the gated tabs API") },
      },
    },
    client: { session: { list: async () => ({ data: [] }), update: async ({ sessionID, title }: { sessionID: string; title: string }) => {
      if (state.renameError) throw new Error("server rejected rename")
      renamed.push({ sessionID, title })
      state.titles[sessionID] = title
      update()
    } } },
    data: { on: (type: string, fn: (event: { data: { sessionID: string } }) => void) => {
      events.set(type, fn)
      return () => { events.delete(type) }
    }, session: {
      get: (id: string) => state.missing.has(id) ? undefined : ({ projectID: "project", title: state.titles[id],
        location: { directory: `/projects/${id}` }, time: state.time[id] ?? {}, outcome: state.failed.has(id) ? "failed" : "completed" }),
      root: (id: string) => state.parents[id] ?? id,
      family: (id: string) => Object.keys(state.parents).filter((child) => state.parents[child] === id),
      status: (id: string) => state.running.has(id) ? "running" : "idle",
      pending: { list: (id: string) => state.pending[id] ?? [], sync: async () => {} },
      permission: { list: (id: string) => state.permissions.has(id) ? [{}] : [], sync: async () => {} },
      form: { list: (id: string) => state.forms.has(id) ? [{}] : [], sync: async () => {} },
      sync: async (id: string) => {
        synced.push(id)
        if (state.offline) throw new Error("Disconnected")
        if (state.missing.has(id)) throw { _tag: "SessionNotFoundError" }
      },
    }, project: { get: () => ({ name: "Repository", canonical: "/projects/repository" }) },
    location: { sync: async () => {}, vcs: { sync: async () => {}, info: ({ directory }: { directory: string }) => ({ branch: { current: state.branches[directory.split("/").at(-1)!] } }) } } },
  } as unknown as Plugin.Context) as () => void
  end = effects.length
  return { state, update, select, cleanup, focused, renamed, errors, synced, catalogPath, commands,
    emit: (type: string, sessionID = "") => { events.get(type)?.({ data: { sessionID } }); update() },
    command: (id: string) => commands.find((command) => command.id === id)!.run(),
    flush: async () => { await writes },
    reload: async () => { Object.assign(catalog, await readFile(catalogPath, "utf8").then(JSON.parse)); update() },
  }
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
    await until(async () => (await readFile(first.catalogPath, "utf8").then(JSON.parse).catch(() => undefined))?.initialized)
    expect((await snapshot()).tabs.length).toBe(1)

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
    second.update()
    await until(async () => (await readFile(second!.catalogPath, "utf8").then(JSON.parse).catch(() => undefined))?.initialized)
    await publish(request(secondID, "click-a"))
    await until(async () => second!.focused.length === 2)
    expect(second.focused).toEqual(["click-b", "click-a"])

    second.cleanup()
    await expect(publish(request(secondID, "click-a"))).rejects.toThrow()
    expect(second.focused).toEqual(["click-b", "click-a"])
  } finally {
    first.cleanup()
    second?.cleanup()
  }
})

test("external tabs follow routes, survive restart, close without deletion, and publish family status", async () => {
  let cli = host("external", false)
  try {
    await until(async () => (await snapshot())?.tabs.length === 2)
    cli.select("external-c")
    await until(async () => (await snapshot())?.tabs.length === 3)
    expect((await snapshot()).tabs.map((tab: { sessionID: string }) => tab.sessionID)).toEqual(["external-a", "external-b", "external-c"])
    expect(cli.state.tabs.length).toBe(2) // Native tabs are disabled and never updated.

    cli.state.parents["child"] = "external-b"
    cli.state.running.add("child")
    cli.state.forms.add("child")
    cli.state.time["external-b"] = { idle: 20, viewed: 10 }
    cli.state.failed.add("external-b")
    cli.update()
    await until(async () => (await snapshot())?.tabs[1]?.unread === "error")
    expect((await snapshot()).tabs[1]).toMatchObject({ busy: true, attention: true, active: false })
    cli.state.running.clear()
    cli.state.forms.clear()
    cli.state.permissions.add("external-b")
    cli.state.pending["external-b"] = [{ type: "synthetic" }]
    cli.state.time["external-b"] = { idle: 20, viewed: 20 }
    cli.update()
    await until(async () => (await snapshot())?.tabs[1]?.busy === false)
    expect((await snapshot()).tabs[1]).toMatchObject({ attention: true })
    expect((await snapshot()).tabs[1].unread).toBeUndefined()
    cli.state.pending["external-b"] = [{ type: "user" }]
    cli.update()
    await until(async () => (await snapshot())?.tabs[1]?.busy === true)

    cli.select("child")
    await until(async () => (await snapshot())?.activeSessionID === "child")
    expect((await snapshot()).tabs.length).toBe(3)
    expect((await snapshot()).tabs[1].active).toBe(true)
    cli.command("hunkle.tab.next")
    await until(async () => (await snapshot())?.activeSessionID === "external-c")
    cli.command("hunkle.tab.previous")
    await until(async () => (await snapshot())?.activeSessionID === "external-b")
    cli.command("hunkle.tab.close")
    await until(async () => (await snapshot())?.tabs.length === 2)
    expect((await snapshot()).activeSessionID).toBe("external-c")
    cli.update()
    await cli.flush()
    cli.cleanup()

    cli = host("external", false)
    await until(async () => (await snapshot())?.directory === "/projects/external-a")
    expect((await snapshot()).tabs.map((tab: { sessionID: string }) => tab.sessionID)).toEqual(["external-a", "external-c"])
    // The old native saved list must not resurrect the closed external-b card.
    cli.select("external-b")
    await until(async () => (await snapshot())?.tabs.length === 3)
    cli.emit("session.deleted", "external-b")
    await until(async () => (await snapshot())?.tabs.length === 2)
    expect((await snapshot()).activeSessionID).toBe("external-c")

    cli.state.missing.add("external-a")
    cli.state.offline = true
    cli.emit("server.connected")
    await Bun.sleep(25)
    expect((await snapshot()).tabs.length).toBe(2)
    cli.state.offline = false
    cli.emit("server.connected")
    await until(async () => (await snapshot())?.tabs.length === 1)
    cli.command("hunkle.tab.close")
    await until(async () => (await snapshot())?.tabs.length === 0)
    expect((await snapshot()).activeSessionID).toBeUndefined()
    expect(cli.errors).toEqual([])
  } finally { await cli.flush(); cli.cleanup() }
})

test("external socket focus and rename validate membership and publisher identity", async () => {
  const cli = host("external-socket", false)
  try {
    await until(async () => (await snapshot())?.directory === "/projects/external-socket-a" && (await snapshot())?.tabs.length === 2)
    const publication = await snapshot()
    const request = async (sessionID: string, extra = {}) => {
      await new Promise<void>((resolve, reject) => {
        const socket = createConnection(publication.focusSocket)
        socket.on("connect", () => socket.end(JSON.stringify({ version: 1, instanceID: publication.instanceID, sessionID, requestID: randomUUID(), ...extra }) + "\n"))
        socket.on("close", resolve)
        socket.on("error", reject)
      })
    }
    await request("external-socket-b")
    await until(async () => (await snapshot())?.activeSessionID === "external-socket-b")
    await request("external-socket-b", { action: "rename", title: "External title" })
    await until(async () => (await snapshot())?.tabs[1]?.title === "External title")
    cli.command("hunkle.tab.close")
    await until(async () => (await snapshot())?.tabs.length === 1)
    const count = cli.focused.length
    await request("external-socket-b")
    await request("external-socket-b", { action: "rename", title: "Closed" })
    await request("external-socket-a", { instanceID: "stale" })
    await Bun.sleep(25)
    expect(cli.focused.length).toBe(count)
    expect(cli.renamed).toEqual([{ sessionID: "external-socket-b", title: "External title" }])
  } finally { await cli.flush(); cli.cleanup() }
})

test("shared external storage can reopen cards without route watchers undoing remote closes", async () => {
  const cli = host("shared", false)
  try {
    await until(async () => (await snapshot())?.tabs.length === 2)
    cli.command("hunkle.tab.close")
    await until(async () => (await snapshot())?.tabs.length === 1)
    await cli.flush()
    const saved = await readFile(cli.catalogPath, "utf8").then(JSON.parse)
    saved.tabs.push({ sessionID: "shared-a" })
    await writeFile(cli.catalogPath, JSON.stringify(saved))
    await cli.reload() // OpenCode's storage watcher delivers another CLI's edit.
    await until(async () => (await snapshot())?.tabs.length === 2)
    expect((await snapshot()).tabs.map((tab: { sessionID: string }) => tab.sessionID)).toEqual(["shared-b", "shared-a"])

    saved.tabs = saved.tabs.filter((tab: { sessionID: string }) => tab.sessionID !== "shared-b")
    await writeFile(cli.catalogPath, JSON.stringify(saved))
    await cli.reload()
    await until(async () => (await snapshot())?.tabs.length === 1)
    expect((await snapshot()).activeSessionID).toBe("shared-b")
    cli.state.running.add("shared-b")
    cli.update()
    await cli.flush()
    expect((await readFile(cli.catalogPath, "utf8").then(JSON.parse)).tabs.length).toBe(1)
    cli.select("shared-a")
    cli.select("shared-b")
    await until(async () => (await snapshot())?.tabs.length === 2)
  } finally { await cli.flush(); cli.cleanup() }
})
