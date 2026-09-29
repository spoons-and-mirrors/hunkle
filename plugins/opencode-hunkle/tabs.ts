import type { Plugin } from "@opencode/plugin/tui"
import type { createEffect } from "solid-js"

type Tab = { sessionID: string; title?: string }
type Catalog = { initialized: boolean; tabs: Tab[] }

// OpenCode's plugin storage owns the external tab list. Sessions and execution
// still belong to the server; closing a card only removes it from this list.
// The CLI supplies Solid to the entrypoint; reuse that same reactive runtime.
export function createTabs(context: Plugin.Context, effect: typeof createEffect) {
  const [catalog, update] = context.storage.store<Catalog>("open-tabs", {
    initial: { initialized: false, tabs: [] },
  })
  let disposed = false
  let followed: string | undefined
  const cancelled = new Set<string>()
  const deleted = new Set<string>()
  const hydrated = new Set<string>()
  const locations = new Set<string>()
  const active = () => {
    const route = context.ui.router.current()
    return route.type === "session" && route.sessionID !== "dummy"
      ? context.data.session.root(route.sessionID)
      : undefined
  }
  const open = () => context.ui.tabs.enabled() ? context.ui.tabs.list() : catalog.tabs
  const persist = (mutation: (draft: Catalog) => void) => {
    return update((draft) => { if (!disposed) mutation(draft) }).catch((error: unknown) => {
      if (!disposed) context.ui.toast.show({ message: `Could not save Hunkle tabs: ${error}`, variant: "error" })
    })
  }

  function close(sessionID = active()) {
    if (!sessionID) return
    if (context.ui.tabs.enabled()) { context.ui.tabs.close(sessionID); return }
    cancelled.add(sessionID)
    for (const id of [sessionID, ...context.data.session.family(sessionID)]) hydrated.delete(id)
    const index = catalog.tabs.findIndex((tab) => tab.sessionID === sessionID)
    const remaining = catalog.tabs.filter((tab) => tab.sessionID !== sessionID)
    void persist((draft) => { draft.tabs = draft.tabs.filter((tab) => tab.sessionID !== sessionID) })
      .then(() => { cancelled.delete(sessionID) })
    if (active() !== sessionID) return
    const next = remaining[Math.min(Math.max(index, 0), remaining.length - 1)]
    context.ui.router.navigate(next ? { type: "session", sessionID: next.sessionID } : { type: "home" })
  }

  function cycle(direction: number) {
    const tabs = open()
    if (!tabs.length) return
    const index = tabs.findIndex((tab) => tab.sessionID === active())
    const next = index < 0 ? (direction > 0 ? 0 : tabs.length - 1) : (index + direction + tabs.length) % tabs.length
    context.ui.router.navigate({ type: "session", sessionID: tabs[next]!.sessionID })
  }

  effect(() => {
    if (disposed) return
    if (context.ui.tabs.enabled()) { followed = undefined; return }
    const sessionID = active()
    // A storage/status update must not re-open a card closed by another CLI.
    const selection = sessionID ?? ""
    if (followed === selection) return
    followed = selection
    if (sessionID && !deleted.has(sessionID)) cancelled.delete(sessionID)
    const seeds = context.ui.tabs.list().map((tab) => ({
      sessionID: context.data.session.root(tab.sessionID), title: tab.title,
    }))
    if (catalog.initialized && (!sessionID || catalog.tabs.some((tab) => tab.sessionID === sessionID))) return
    void persist((draft) => {
      if (!draft.initialized) {
        draft.initialized = true
        draft.tabs = seeds.filter((tab, index) => !cancelled.has(tab.sessionID) && !deleted.has(tab.sessionID)
          && seeds.findIndex((other) => other.sessionID === tab.sessionID) === index)
      }
      if (!sessionID || cancelled.has(sessionID) || deleted.has(sessionID)
        || draft.tabs.some((tab) => tab.sessionID === sessionID)) return
      draft.tabs.push({ sessionID, title: context.data.session.get(sessionID)?.title })
    })
  })

  function hydrate(sessionID: string, children: boolean) {
    if (disposed || hydrated.has(sessionID)) return
    hydrated.add(sessionID)
    void context.data.session.sync(sessionID).then(async () => {
      if (disposed || deleted.has(sessionID)) return
      await Promise.allSettled([
        context.data.session.pending.sync(sessionID),
        context.data.session.permission.sync(sessionID),
        context.data.session.form.sync(sessionID),
      ])
      if (!children || disposed) return
      const response = await context.client.session.list({ parentID: sessionID })
      if (!disposed) for (const child of response.data) hydrate(child.id, false)
    }).catch((error: unknown) => {
      if (disposed) return
      if (typeof error === "object" && error !== null && "_tag" in error && error._tag === "SessionNotFoundError") {
        deleted.add(sessionID)
        close(sessionID)
      }
      // A disconnected server is not evidence that a session was deleted.
      // Reconnect retries hydration; live cache events carry subsequent status.
    })
  }

  effect(() => {
    if (disposed || context.ui.tabs.enabled()) return
    for (const tab of catalog.tabs) {
      hydrate(tab.sessionID, true)
      for (const id of context.data.session.family(tab.sessionID)) hydrate(id, false)
      const location = context.data.session.get(tab.sessionID)?.location
      if (!location) continue
      const key = JSON.stringify(location)
      if (locations.has(key)) continue
      locations.add(key)
      void Promise.allSettled([context.data.location.sync(location), context.data.location.vcs.sync(location)])
    }
  })

  const stopDeleted = context.data.on("session.deleted", (event) => {
    deleted.add(event.data.sessionID)
    if (!context.ui.tabs.enabled()) close(event.data.sessionID)
  })
  const stopConnected = context.data.on("server.connected", () => {
    hydrated.clear()
    locations.clear()
    if (!context.ui.tabs.enabled()) {
      for (const tab of catalog.tabs) {
        hydrate(tab.sessionID, true)
        const location = context.data.session.get(tab.sessionID)?.location
        if (location) void Promise.allSettled([context.data.location.sync(location), context.data.location.vcs.sync(location)])
      }
    }
  })

  context.keymap.layer(() => ({
    mode: "global",
    enabled: () => !context.ui.tabs.enabled(),
    commands: [
      { id: "hunkle.tab.next", title: "Next Hunkle tab", group: "Hunkle", bind: "ctrl+tab,alt+down", palette: true, run: () => cycle(1) },
      { id: "hunkle.tab.previous", title: "Previous Hunkle tab", group: "Hunkle", bind: "ctrl+shift+tab,alt+up", palette: true, run: () => cycle(-1) },
      { id: "hunkle.tab.close", title: "Close Hunkle tab", group: "Hunkle", palette: true, slash: { name: "hunkle-close" }, run: () => close() },
    ],
  }))

  return {
    list() {
      if (context.ui.tabs.enabled()) return context.ui.tabs.list()
      return catalog.tabs.filter((tab) => !cancelled.has(tab.sessionID) && !deleted.has(tab.sessionID)).map((tab) => {
        const session = context.data.session.get(tab.sessionID)
        const family = [tab.sessionID, ...context.data.session.family(tab.sessionID)]
        const unread = session?.time.idle !== undefined
          && (session.time.viewed === undefined || session.time.idle > session.time.viewed)
        return {
          ...tab,
          active: active() === tab.sessionID,
          busy: family.some((id) => context.data.session.status(id) === "running"
            || context.data.session.pending.list(id).some((item) => item.type !== "synthetic")),
          attention: family.some((id) => (context.data.session.permission.list(id)?.length ?? 0) > 0
            || (context.data.session.form.list(id)?.length ?? 0) > 0),
          unread: unread ? session?.outcome === "failed" ? "error" as const : "activity" as const : undefined,
        }
      })
    },
    dispose() { disposed = true; stopDeleted(); stopConnected() },
  }
}
