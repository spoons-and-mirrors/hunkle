import { Plugin } from "@opencode/plugin/tui"
import { createEffect } from "solid-js"
import { randomUUID } from "node:crypto"
import { mkdir, rename, unlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

// The only handoff is an atomic, same-user snapshot of the active tab's workspace.
const uid = process.geteuid?.() ?? process.getuid?.() ?? 0
const base = process.env.XDG_RUNTIME_DIR || join(tmpdir(), `hunkle-${uid}`)
const directory = join(base, "hunkle")
const target = join(directory, "opencode-active.json")

export default Plugin.define({
  id: "hunkle.active-workspace",
  setup(context) {
    let disposed = false
    let desired: { key: string; workspace: string } | undefined
    let writing = false
    let last: string | undefined
    const sync = new Set<string>()

    async function flush() {
      if (writing) return
      writing = true
      try {
        while (desired !== undefined && desired.key !== last && !disposed) {
          const { key, workspace } = desired
          const temporary = join(directory, `.opencode-active-${randomUUID()}`)
          try {
            await mkdir(directory, { recursive: true, mode: 0o700 })
            await writeFile(temporary, JSON.stringify({ version: 1, directory: workspace }), { mode: 0o600 })
            if (!disposed && desired?.key === key) {
              await rename(temporary, target)
              last = key
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

    const remove = context.ui.slot({
      append: "app",
      render: () => {
        createEffect(() => {
          const route = context.ui.router.current()
          if (route.type !== "session") {
            desired = undefined
            last = undefined
            return
          }
          const workspace = context.data.session.get(route.sessionID)?.location.directory
          if (!workspace) {
            if (!sync.has(route.sessionID)) {
              sync.add(route.sessionID)
              void context.data.session.sync(route.sessionID).catch(() => {}).finally(() => sync.delete(route.sessionID))
            }
            return
          }
          const key = `${route.sessionID}\0${workspace}`
          if (key !== desired?.key) {
            desired = { key, workspace }
            void flush()
          }
        })
        return null
      },
    })
    return () => {
      disposed = true
      remove()
    }
  },
})
