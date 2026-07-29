use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::filesystem::atomic_write;

use super::{AgentTiming, AgentTimingKey, HerdrAgent};

const INDEX_VERSION: u8 = 2;
const MAX_AGENT_TIMINGS: usize = 512;

#[derive(Default, Deserialize, Serialize)]
struct TimingIndex {
    version: u8,
    #[serde(default)]
    cleared_at_ms: u64,
    timings: Vec<TimingRecord>,
}

#[derive(Default)]
struct LoadedTimings {
    timings: HashMap<AgentTimingKey, AgentTiming>,
    cleared_at_ms: u64,
    migrated: bool,
}

#[derive(Deserialize, Serialize)]
struct TimingRecord {
    key: AgentTimingKey,
    timing: AgentTiming,
}

pub(super) fn sync(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[HerdrAgent],
    now_ms: u64,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = IndexLock::acquire(path)?;
    let loaded = load(path);
    let needs_rewrite = match loaded.as_ref() {
        Ok(loaded) => loaded.migrated,
        Err(_) => true,
    } || !path.exists();
    let loaded = loaded.unwrap_or_default();
    let mut shared = loaded.timings;

    merge_local_timings(&mut shared, local, loaded.cleared_at_ms);
    migrate_session_timings(&mut shared, agents);
    update(&mut shared, agents, now_ms);
    prune(&mut shared, agents.iter().map(|agent| &agent.timing_key));

    if needs_rewrite || &shared != local {
        save(path, &shared, loaded.cleared_at_ms)?;
        *local = shared;
    }
    Ok(())
}

fn migrate_session_timings(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[HerdrAgent],
) {
    for agent in agents {
        if timings.contains_key(&agent.timing_key) {
            continue;
        }
        let Some(session_key) = agent.session_timing_key.as_ref() else {
            continue;
        };
        if let Some(timing) = timings.remove(session_key) {
            timings.insert(agent.timing_key.clone(), timing);
        }
    }
}

pub(super) fn update(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[HerdrAgent],
    now_ms: u64,
) {
    for agent in agents {
        let key = agent.timing_key.clone();
        if let Some(timing) = timings.get_mut(&key) {
            if timing.state_change_seq == 0
                || agent.state_change_seq == 0
                || agent.state_change_seq >= timing.state_change_seq
            {
                timing.observe(agent.status, agent.state_change_seq, now_ms);
            }
        } else if agent.status.should_track_timing() {
            timings.insert(
                key,
                AgentTiming::new(agent.status, agent.state_change_seq, now_ms),
            );
        }
    }
}

pub(super) fn observe_status(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    key: &AgentTimingKey,
    status: super::AgentStatus,
    state_change_seq: u64,
    now_ms: u64,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = IndexLock::acquire(path)?;
    let loaded = load(path).unwrap_or_default();
    let mut shared = loaded.timings;
    merge_local_timings(&mut shared, local, loaded.cleared_at_ms);
    if let Some(timing) = shared.get_mut(key) {
        timing.observe_event(status, now_ms);
    } else if status.should_track_timing() {
        let mut timing = AgentTiming::new(status, state_change_seq, now_ms);
        timing.awaiting_sequence = true;
        shared.insert(key.clone(), timing);
    }
    prune(&mut shared, std::iter::once(key));
    save(path, &shared, loaded.cleared_at_ms)?;
    *local = shared;
    Ok(())
}

pub(super) fn reset(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[HerdrAgent],
    now_ms: u64,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = IndexLock::acquire(path)?;
    let mut reset = HashMap::new();
    update(&mut reset, agents, now_ms);
    save(path, &reset, now_ms)?;
    *local = reset;
    Ok(())
}

fn prune<'a>(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    active: impl IntoIterator<Item = &'a AgentTimingKey>,
) {
    if timings.len() <= MAX_AGENT_TIMINGS {
        return;
    }
    let active = active.into_iter().collect::<HashSet<_>>();
    let mut candidates = timings
        .iter()
        .filter(|(key, _)| !active.contains(key))
        .map(|(key, timing)| (timing.last_seen_ms, key.stable_id(), key.clone()))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    for (_, _, key) in candidates {
        if timings.len() <= MAX_AGENT_TIMINGS {
            break;
        }
        timings.remove(&key);
    }
    if timings.len() > MAX_AGENT_TIMINGS {
        let mut candidates = timings
            .iter()
            .map(|(key, timing)| (timing.last_seen_ms, key.stable_id(), key.clone()))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
        for (_, _, key) in candidates
            .into_iter()
            .take(timings.len() - MAX_AGENT_TIMINGS)
        {
            timings.remove(&key);
        }
    }
}

fn merge_timing(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    key: AgentTimingKey,
    incoming: AgentTiming,
) {
    let Some(existing) = timings.get_mut(&key) else {
        timings.insert(key, incoming);
        return;
    };
    if incoming.state_change_seq > existing.state_change_seq
        || (incoming.state_change_seq == existing.state_change_seq
            && incoming.last_seen_ms > existing.last_seen_ms)
    {
        *existing = incoming;
    }
}

fn merge_local_timings(
    shared: &mut HashMap<AgentTimingKey, AgentTiming>,
    local: &HashMap<AgentTimingKey, AgentTiming>,
    cleared_at_ms: u64,
) {
    // A clear watermark prevents another Hunkle process from restoring deleted history.
    for (key, timing) in local {
        if timing.last_seen_ms > cleared_at_ms {
            merge_timing(shared, key.clone(), timing.clone());
        }
    }
}

fn load(path: &Path) -> io::Result<LoadedTimings> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedTimings {
                timings: HashMap::new(),
                cleared_at_ms: 0,
                migrated: false,
            });
        }
        Err(error) => return Err(error),
    };
    let index: TimingIndex = serde_json::from_slice(&content).map_err(io::Error::other)?;
    if !matches!(index.version, 1 | INDEX_VERSION) {
        return Ok(LoadedTimings {
            timings: HashMap::new(),
            cleared_at_ms: 0,
            migrated: true,
        });
    }
    let migrated = index.version != INDEX_VERSION;
    Ok(LoadedTimings {
        timings: index
            .timings
            .into_iter()
            .map(|record| (record.key, record.timing))
            .collect(),
        cleared_at_ms: index.cleared_at_ms,
        migrated,
    })
}

fn save(
    path: &Path,
    timings: &HashMap<AgentTimingKey, AgentTiming>,
    cleared_at_ms: u64,
) -> io::Result<()> {
    let mut records = timings
        .iter()
        .map(|(key, timing)| TimingRecord {
            key: key.clone(),
            timing: timing.clone(),
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.key.stable_id());
    let mut content = serde_json::to_vec_pretty(&TimingIndex {
        version: INDEX_VERSION,
        cleared_at_ms,
        timings: records,
    })
    .map_err(io::Error::other)?;
    content.push(b'\n');
    atomic_write(path, &content)
}

struct IndexLock {
    file: File,
}

impl IndexLock {
    fn acquire(index_path: &Path) -> io::Result<Self> {
        let path = lock_path(index_path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

fn lock_path(index_path: &Path) -> PathBuf {
    let mut path = index_path.as_os_str().to_owned();
    path.push(".lock");
    PathBuf::from(path)
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn lock_exclusive(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn unlock(file: &File) {
    use std::os::fd::AsRawFd;

    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}

#[cfg(not(unix))]
fn unlock(_file: &File) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::workspace_panel::AgentStatus;

    #[test]
    fn migrates_the_active_session_timer_to_the_agent() {
        let session_key = AgentTimingKey::Session(super::super::AgentSessionIdentity {
            source: "herdr:opencode".to_owned(),
            agent: "opencode".to_owned(),
            kind: "id".to_owned(),
            value: "ses_old".to_owned(),
        });
        let agent_key = AgentTimingKey::Terminal("opencode@term-1".to_owned());
        let agent = HerdrAgent {
            name: "opencode".to_owned(),
            session_name: None,
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            pane_id: "pane".to_owned(),
            focused: false,
            status: AgentStatus::Idle,
            timing_key: agent_key.clone(),
            session_timing_key: Some(session_key.clone()),
            state_change_seq: 7,
        };
        let timing = AgentTiming {
            elapsed_ms: 3_000,
            session_elapsed_ms: 12_000,
            running_since_ms: None,
            status: AgentStatus::Idle,
            state_change_seq: 7,
            last_seen_ms: 1_000,
            awaiting_sequence: false,
        };
        let mut timings = HashMap::from([(session_key.clone(), timing)]);

        migrate_session_timings(&mut timings, &[agent]);

        assert!(!timings.contains_key(&session_key));
        assert_eq!(
            timings
                .get(&agent_key)
                .unwrap()
                .elapsed_at(crate::app::settings::AgentTimeDisplay::AgentTotal, 2_000),
            std::time::Duration::from_secs(15)
        );
    }

    #[test]
    fn keeps_only_the_512_most_recent_agents() {
        let mut timings = HashMap::new();
        for index in 0..=MAX_AGENT_TIMINGS {
            let key = AgentTimingKey::Terminal(format!("session-{index:03}"));
            timings.insert(
                key,
                AgentTiming::new(AgentStatus::Working, index as u64, index as u64),
            );
        }

        prune(&mut timings, std::iter::empty::<&AgentTimingKey>());

        assert_eq!(timings.len(), MAX_AGENT_TIMINGS);
        assert!(!timings.contains_key(&AgentTimingKey::Terminal("session-000".to_owned())));
        assert!(timings.contains_key(&AgentTimingKey::Terminal("session-512".to_owned())));
    }

    #[test]
    fn migrates_version_one_timings_without_losing_the_latest_loop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        fs::write(
            &path,
            r#"{"version":1,"timings":[{"key":{"scope":"terminal","identity":"session-a"},"timing":{"elapsed_ms":3000,"running_since_ms":null,"status":"idle","state_change_seq":7}}]}"#,
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        let timing = loaded
            .timings
            .get(&AgentTimingKey::Terminal("session-a".to_owned()))
            .unwrap();
        assert!(loaded.migrated);
        assert_eq!(timing.elapsed_ms, 3_000);
        assert_eq!(timing.session_elapsed_ms, 0);

        let mut local = HashMap::new();
        sync(&path, &mut local, &[], 10_000).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(rewritten["version"], INDEX_VERSION);
    }

    #[test]
    fn reset_removes_persisted_session_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let key = AgentTimingKey::Terminal("old-session".to_owned());
        let mut timings = HashMap::from([(key, AgentTiming::new(AgentStatus::Idle, 3, 1_000))]);
        save(&path, &timings, 0).unwrap();

        reset(&path, &mut timings, &[], 2_000).unwrap();

        assert!(timings.is_empty());
        let loaded = load(&path).unwrap();
        assert!(loaded.timings.is_empty());
        assert_eq!(loaded.cleared_at_ms, 2_000);
    }

    #[test]
    fn stale_process_cannot_restore_cleared_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let key = AgentTimingKey::Terminal("old-session".to_owned());
        let old_timing = AgentTiming::new(AgentStatus::Idle, 3, 1_000);
        let mut clearing_process = HashMap::from([(key.clone(), old_timing.clone())]);
        let mut stale_process = HashMap::from([(key, old_timing)]);
        save(&path, &clearing_process, 0).unwrap();

        reset(&path, &mut clearing_process, &[], 2_000).unwrap();
        sync(&path, &mut stale_process, &[], 3_000).unwrap();

        assert!(stale_process.is_empty());
        assert!(load(&path).unwrap().timings.is_empty());
    }
}
