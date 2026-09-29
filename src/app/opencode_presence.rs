use std::{
    env, fs,
    io::Write,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use super::opencode_tabs::{OpenCodeTab, OpenCodeTabs, OpenCodeUpdate};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    version: u32,
    directory: Option<PathBuf>,
    #[serde(rename = "instanceID")]
    instance_id: Option<String>,
    #[serde(rename = "activeSessionID")]
    active_session_id: Option<String>,
    pid: Option<i32>,
    focus_socket: Option<PathBuf>,
    #[serde(default)]
    tabs: Vec<OpenCodeTab>,
}

/// Mirrors an OpenCode CLI's tabs and forwards focus requests. OpenCode owns them.
pub(crate) struct OpenCodePresence {
    path: PathBuf,
    seen: Option<(u64, i64, i64)>,
    snapshot: Option<Snapshot>,
    checked_alive: Instant,
    pub(crate) tabs: OpenCodeTabs,
}

impl OpenCodePresence {
    pub(crate) fn new() -> Self {
        // Tests opt into publications explicitly rather than observing the user's CLI.
        if cfg!(test) {
            return Self::at(PathBuf::new());
        }
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("/tmp/hunkle-{}", unsafe { libc::geteuid() }))
            });
        Self::at(runtime.join("hunkle/opencode-active.json"))
    }

    fn at(path: PathBuf) -> Self {
        let mut presence = Self {
            path,
            seen: None,
            snapshot: None,
            checked_alive: Instant::now(),
            tabs: OpenCodeTabs::default(),
        };
        // Populate cards immediately, but never navigate on the startup baseline.
        presence.poll();
        presence
    }

    pub(crate) fn poll(&mut self) -> OpenCodeUpdate {
        let current = fingerprint(&self.path);
        if current == self.seen {
            if self.checked_alive.elapsed() >= Duration::from_secs(1) {
                self.checked_alive = Instant::now();
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| !is_alive(snapshot))
                {
                    self.snapshot = None;
                    return OpenCodeUpdate {
                        changed: self.tabs.replace(Vec::new()),
                        ..Default::default()
                    };
                }
            }
            return OpenCodeUpdate::default();
        }
        self.seen = current;
        let mut snapshot = current.and_then(|_| self.read_snapshot());
        let active_session = |snapshot: &Snapshot| {
            (
                snapshot.instance_id.clone(),
                snapshot.active_session_id.clone(),
            )
        };
        let active_session_changed = snapshot.as_ref().is_some_and(|next| {
            self.snapshot.as_ref().map(active_session) != Some(active_session(next))
        });
        let selection = |snapshot: &Snapshot| {
            (
                snapshot.instance_id.clone(),
                snapshot.active_session_id.clone(),
                snapshot.directory.clone(),
            )
        };
        let workspace = snapshot.as_ref().and_then(|next| {
            (self.snapshot.as_ref().map(selection) != Some(selection(next)))
                .then(|| next.directory.clone())
                .flatten()
        });
        let items = snapshot
            .as_mut()
            .map_or_else(Vec::new, |snapshot| std::mem::take(&mut snapshot.tabs));
        let publisher =
            |snapshot: &Snapshot| (snapshot.instance_id.clone(), snapshot.focus_socket.clone());
        let publisher_changed =
            self.snapshot.as_ref().map(publisher) != snapshot.as_ref().map(publisher);
        let changed = self.tabs.replace(items) || publisher_changed;
        self.snapshot = snapshot;
        OpenCodeUpdate {
            changed,
            active_session_changed,
            workspace,
        }
    }

    pub(crate) fn instance_id(&self) -> Option<&str> {
        self.snapshot.as_ref()?.focus_socket.as_ref()?;
        self.snapshot.as_ref()?.instance_id.as_deref()
    }

    pub(crate) fn focus_tab(&self, instance_id: &str, session_id: &str) -> std::io::Result<()> {
        self.request_tab(instance_id, session_id, "focus", None)
    }

    pub(crate) fn rename_tab(
        &self,
        instance_id: &str,
        session_id: &str,
        title: &str,
    ) -> std::io::Result<()> {
        // The per-CLI socket accepts at most 16 KiB, including JSON escaping.
        if title.len() > 4096 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "session name is too long",
            ));
        }
        self.request_tab(instance_id, session_id, "rename", Some(title))
    }

    fn request_tab(
        &self,
        instance_id: &str,
        session_id: &str,
        action: &str,
        title: Option<&str>,
    ) -> std::io::Result<()> {
        // Check the live publication, not an index or a potentially stale rendered list.
        let current = self.read_snapshot().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "OpenCode CLI is unavailable",
            )
        })?;
        if current.instance_id.as_deref() != Some(instance_id)
            || !current.tabs.iter().any(|tab| tab.session_id == session_id)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "OpenCode tab changed or closed; try again after the strip updates",
            ));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_nanos();
        let mut request = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "instanceID": instance_id,
            "sessionID": session_id,
            "action": action,
            "title": title,
            "requestID": format!("{}-{nonce}", std::process::id()),
        }))?;
        request.push(b'\n');
        let socket = current
            .focus_socket
            .filter(|path| path.parent() == self.path.parent())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "OpenCode plugin does not provide a local tab socket",
                )
            })?;
        let mut stream = UnixStream::connect(socket)?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        stream.write_all(&request)
    }

    fn read_snapshot(&self) -> Option<Snapshot> {
        let directory = self.path.parent()?;
        let owner = unsafe { libc::geteuid() };
        let dir = fs::symlink_metadata(directory).ok()?;
        if !dir.is_dir() || dir.uid() != owner {
            return None;
        }
        // Hunkle's existing runtime directory can be group-writable. It is still
        // same-user-only when enclosed by a private runtime root (e.g. /run/user/1000).
        let private_parent = fs::symlink_metadata(directory.parent()?).ok()?;
        let enclosed = private_parent.is_dir()
            && private_parent.uid() == owner
            && private_parent.permissions().mode() & 0o077 == 0;
        if dir.permissions().mode() & 0o022 != 0 && !enclosed {
            return None;
        }
        let file = fs::symlink_metadata(&self.path).ok()?;
        if !file.is_file() || file.uid() != owner || file.len() > 1024 * 1024 {
            return None;
        }
        let bytes = fs::read(&self.path).ok()?;
        let mut snapshot: Snapshot = serde_json::from_slice(&bytes).ok()?;
        if snapshot.version != 1
            || !is_alive(&snapshot)
            || snapshot
                .directory
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || snapshot.tabs.len() > 512
            || snapshot.tabs.iter().any(|tab| {
                tab.session_id.is_empty()
                    || tab
                        .directory
                        .as_ref()
                        .is_some_and(|path| !path.is_absolute())
            })
        {
            return None;
        }
        for tab in &mut snapshot.tabs {
            tab.title.retain(|character| !character.is_control());
            if let Some(project) = &mut tab.project {
                project.retain(|character| !character.is_control());
            }
            if let Some(branch) = &mut tab.branch {
                branch.retain(|character| !character.is_control());
            }
        }
        Some(snapshot)
    }

    #[cfg(test)]
    pub(crate) fn at_for_test(path: PathBuf) -> Self {
        Self::at(path)
    }
}

fn is_alive(snapshot: &Snapshot) -> bool {
    snapshot.pid.is_none_or(|pid| {
        pid > 0
            && (unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
    })
}

fn fingerprint(path: &std::path::Path) -> Option<(u64, i64, i64)> {
    let meta = fs::symlink_metadata(path).ok()?;
    Some((meta.ino(), meta.mtime(), meta.mtime_nsec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(path: &std::path::Path, value: &serde_json::Value) {
        let temporary = path.with_extension("next");
        fs::write(&temporary, value.to_string()).unwrap();
        fs::rename(temporary, path).unwrap();
    }

    fn snapshot() -> serde_json::Value {
        serde_json::json!({
            "version": 1, "pid": std::process::id(), "instanceID": "cli-a",
            "activeSessionID": "first", "directory": "/first",
            "tabs": [
                {"sessionID":"first", "title":"First", "directory":"/first", "active":true, "busy":false, "attention":false},
                {"sessionID":"second", "title":"Second", "directory":"/second", "active":false, "busy":true, "attention":false}
            ]
        })
    }

    #[test]
    fn opencode_cards_load_at_startup_and_status_changes_do_not_navigate() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        let mut value = snapshot();
        publish(&path, &value);
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        assert_eq!(presence.tabs.items.len(), 2);
        assert!(presence.tabs.items[1].busy);
        assert_eq!(presence.poll().workspace, None);
        presence.tabs.reveal_active = false;
        presence.tabs.scroll = 10;

        value["tabs"][1]["busy"] = false.into();
        value["tabs"][1]["attention"] = true.into();
        value["tabs"][1]["title"] = "Renamed".into();
        publish(&path, &value);
        let update = presence.poll();
        assert!(update.changed);
        assert_eq!(update.workspace, None);
        assert!(presence.tabs.items[1].attention);
        assert_eq!(presence.tabs.items[1].title, "Renamed");
        assert!(!presence.tabs.reveal_active);
        assert_eq!(presence.tabs.scroll, 10);

        value["tabs"][0]["active"] = false.into();
        value["tabs"][1]["active"] = true.into();
        value["activeSessionID"] = "second".into();
        value["directory"] = "/second".into();
        publish(&path, &value);
        let update = presence.poll();
        assert_eq!(update.workspace, Some(PathBuf::from("/second")));
        assert!(presence.tabs.reveal_active);

        value["tabs"].as_array_mut().unwrap().reverse();
        publish(&path, &value);
        assert_eq!(presence.poll().workspace, None);
        assert_eq!(presence.tabs.items[0].session_id, "second");

        value["directory"] = serde_json::Value::Null;
        value["activeSessionID"] = serde_json::Value::Null;
        value["tabs"][0]["active"] = false.into();
        publish(&path, &value);
        assert_eq!(presence.poll().workspace, None);
        assert_eq!(presence.tabs.items.len(), 2);

        // Even identical cards must refresh their click addresses after a CLI reload.
        value["instanceID"] = "cli-b".into();
        publish(&path, &value);
        let update = presence.poll();
        assert!(update.changed);
        assert_eq!(update.workspace, None);

        fs::remove_file(path).unwrap();
        assert!(presence.poll().changed);
        assert!(presence.tabs.items.is_empty());
    }

    #[test]
    fn opencode_stale_process_and_invalid_snapshots_clear_the_strip() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        let mut value = snapshot();
        publish(&path, &value);
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        // Simulate the publishing CLI exiting without changing its file.
        presence.snapshot.as_mut().unwrap().pid = Some(i32::MAX);
        presence.checked_alive = Instant::now() - Duration::from_secs(2);
        assert!(presence.poll().changed);
        assert!(presence.tabs.items.is_empty());
        assert_eq!(presence.poll().workspace, None);

        value["pid"] = i32::MAX.into();
        publish(&path, &value);
        let stale = OpenCodePresence::at_for_test(path.clone());
        assert!(stale.tabs.items.is_empty());

        value["pid"] = std::process::id().into();
        publish(&path, &value);
        presence.poll();
        assert_eq!(presence.tabs.items.len(), 2);
        value["tabs"][0]["directory"] = "relative".into();
        publish(&path, &value);
        let update = presence.poll();
        assert!(update.changed);
        assert_eq!(update.workspace, None);
        assert!(presence.tabs.items.is_empty());
    }

    #[test]
    fn existing_publication_is_baseline_and_replacement_follows() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        fs::write(&path, r#"{"version":1,"directory":"/first"}"#).unwrap();
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        assert_eq!(presence.poll().workspace, None);
        let next = root.path().join("next");
        fs::write(&next, r#"{"version":1,"directory":"/second"}"#).unwrap();
        fs::rename(next, &path).unwrap();
        assert_eq!(presence.poll().workspace, Some(PathBuf::from("/second")));
        assert_eq!(presence.poll().workspace, None);
    }

    #[test]
    fn rejects_invalid_or_untrusted_publications() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        fs::write(&path, r#"{"version":1,"directory":"relative"}"#).unwrap();
        assert_eq!(presence.poll().workspace, None);
        fs::remove_file(&path).unwrap();
        presence.poll();
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert_eq!(presence.poll().workspace, None);
    }

    #[test]
    fn accepts_existing_group_writable_hunkle_directory_under_private_runtime() {
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = runtime.path().join("hunkle");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o775)).unwrap();
        let path = directory.join("opencode-active.json");
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        fs::write(&path, r#"{"version":1,"directory":"/workspace"}"#).unwrap();
        assert_eq!(presence.poll().workspace, Some(PathBuf::from("/workspace")));
    }
}
