import { Plugin } from "@opencode/plugin/tui"
import { createEffect } from "solid-js"
import { randomUUID } from "node:crypto"
import { readFileSync, unlinkSync } from "node:fs"
import { mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, join } from "node:path"

// Display-only tab data and the active workspace, published only when they change.
const uid = process.geteuid?.() ?? process.getuid?.() ?? 0
const base = process.env.XDG_RUNTIME_DIR || join(tmpdir(), `hunkle-${uid}`)
const directory = join(base, "hunkle")
const target = join(directory, "opencode-active.json")

export default Plugin.define({
  id: "hunkle.active-workspace",
  setup(context) {
    let disposed = false
    const instanceID = randomUUID()
    const focusSocket = join(directory, `opencode-${instanceID}.sock`)
    let desired: { json: string; selection: string } | undefined
    let writing = false
    let last: string | undefined
    let lastSelection: string | undefined
    const sync = new Set<string>()
    let listener: ReturnType<typeof Bun.listen<string>> | undefined
    let lastFocus: string | undefined
    let lastRename: string | undefined

    function handleTabRequest(json: string) {
      if (disposed || json === lastFocus || json === lastRename) return
      let request: { version?: number; instanceID?: string; sessionID?: string; action?: string; title?: string }
      try { request = JSON.parse(json) } catch { return }
      if (request.version !== 1 || request.instanceID !== instanceID || !context.ui.tabs.enabled()) return
      // Never open or rename a tab that is no longer in this CLI.
      if (typeof request.sessionID !== "string" || !context.ui.tabs.list().some((tab) => tab.sessionID === request.sessionID)) return
      try {
        if (request.action === "rename") {
          lastRename = json
          if (typeof request.title !== "string" || !request.title.trim()) return
          void context.client.session.update({ sessionID: request.sessionID, title: request.title.trim() })
            .catch((error: unknown) => context.ui.toast.show({ message: `Could not rename session: ${error}`, variant: "error" }))
        } else if (request.action === "focus" || request.action === undefined) {
          lastFocus = json
          context.ui.tabs.focus(request.sessionID)
        }
      } catch (error) {
        context.ui.toast.show({ message: `Could not handle Hunkle tab request: ${error}`, variant: "error" })
      }
    }

    async function flush() {
      if (writing) return
      writing = true
      try {
        while (desired !== undefined && desired.json !== last && !disposed) {
          const { json, selection } = desired
          const temporary = join(directory, `.opencode-active-${randomUUID()}`)
          try {
            await mkdir(directory, { recursive: true, mode: 0o700 })
            if (!listener && !disposed) {
              listener = Bun.listen<string>({
                unix: focusSocket,
                data: "",
                socket: {
                  data(socket, bytes) {
                    socket.data += bytes.toString()
                    if (socket.data.length > 16_384) { socket.end(); return }
                    const end = socket.data.indexOf("\n")
                    if (end < 0) return
                    handleTabRequest(socket.data.slice(0, end))
                    socket.end()
                  },
                },
              })
            }
            // Tab switches claim the handoff. Background activity in another CLI
            // must not steal Hunkle's strip or workspace back from that selection.
            if (selection === lastSelection) {
              const owner = await readFile(target, "utf8").then((text) => JSON.parse(text).instanceID).catch(() => undefined)
              if (owner && owner !== instanceID) {
                last = json
                continue
              }
            }
            await writeFile(temporary, json, { mode: 0o600 })
            if (!disposed && desired?.json === json) {
              await rename(temporary, target)
              last = json
              lastSelection = selection
            } else {
              await unlink(temporary)
            }
          } catch {
            await unlink(temporary).catch(() => {})
            // The integration is optional; an unavailable runtime directory must not disrupt OpenCode.
            break
          }
        }
      } finally {
        writing = false
      }
    }

    function workspace(sessionID: string) {
      const path = context.data.session.get(sessionID)?.location.directory
      if (!path && !sync.has(sessionID)) {
        sync.add(sessionID)
        void context.data.session.sync(sessionID).catch(() => {})
      }
      return path
    }

    const remove = context.ui.slot({
      append: "app",
      render: () => {
        createEffect(() => {
          const route = context.ui.router.current()
          const activeSessionID = route.type === "session" ? route.sessionID : undefined
          const path = activeSessionID ? workspace(activeSessionID) : undefined
           const tabs = context.ui.tabs.enabled() ? context.ui.tabs.list().map((tab) => {
             const path = workspace(tab.sessionID)
             const session = context.data.session.get(tab.sessionID)
             const project = session && context.data.project.get(session.projectID)
             return {
               sessionID: tab.sessionID,
               title: session?.title || tab.title || "New session",
               directory: path,
               project: project?.name || (project?.canonical ? basename(project.canonical) : path ? basename(path) : undefined),
               branch: session && context.data.location.vcs.info(session.location)?.branch.current,
               active: Boolean(activeSessionID) && tab.active,
               busy: tab.busy,
               attention: tab.attention,
               unread: tab.unread,
             }
           }) : []
          const json = JSON.stringify({ version: 1, instanceID, pid: process.pid, focusSocket, activeSessionID, directory: path, tabs })
          if (json !== desired?.json) {
            desired = { json, selection: `${activeSessionID ?? ""}\0${path ?? ""}` }
            void flush()
          }
        })
        return null
      },
    })
    return () => {
      disposed = true
      listener?.stop(true)
      try { unlinkSync(focusSocket) } catch {}
      remove()
      // Synchronous cleanup avoids racing this CLI's replacement plugin on hot reload.
      try {
        if (JSON.parse(readFileSync(target, "utf8")).instanceID === instanceID) unlinkSync(target)
      } catch {}
    }
  },
})
