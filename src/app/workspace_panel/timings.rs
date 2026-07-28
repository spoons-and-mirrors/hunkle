use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::filesystem::atomic_write;

use super::{AgentTiming, AgentTimingKey, HerdrAgent};

const INDEX_VERSION: u8 = 1;

#[derive(Default, Deserialize, Serialize)]
struct TimingIndex {
    version: u8,
    timings: Vec<TimingRecord>,
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
    let needs_rewrite = loaded.is_err() || !path.exists();
    let mut shared = loaded.unwrap_or_default();

    // A local record can be newer when an earlier write failed. Sequence ordering
    // prevents a stale Hunkle process from replacing a newer shared transition.
    for (key, timing) in local.iter() {
        merge_timing(&mut shared, key.clone(), timing.clone());
    }
    update(&mut shared, agents, now_ms);

    if needs_rewrite || &shared != local {
        save(path, &shared)?;
        *local = shared;
    }
    Ok(())
}

pub(super) fn update(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[HerdrAgent],
    now_ms: u64,
) {
    let active_keys = agents
        .iter()
        .map(|agent| agent.timing_key.clone())
        .collect::<HashSet<_>>();
    timings.retain(|key, _| active_keys.contains(key));

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

fn merge_timing(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    key: AgentTimingKey,
    incoming: AgentTiming,
) {
    let Some(existing) = timings.get_mut(&key) else {
        timings.insert(key, incoming);
        return;
    };
    if incoming.state_change_seq > existing.state_change_seq {
        *existing = incoming;
    }
}

fn load(path: &Path) -> io::Result<HashMap<AgentTimingKey, AgentTiming>> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };
    let index: TimingIndex = serde_json::from_slice(&content).map_err(io::Error::other)?;
    if index.version != INDEX_VERSION {
        return Ok(HashMap::new());
    }
    Ok(index
        .timings
        .into_iter()
        .map(|record| (record.key, record.timing))
        .collect())
}

fn save(path: &Path, timings: &HashMap<AgentTimingKey, AgentTiming>) -> io::Result<()> {
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
