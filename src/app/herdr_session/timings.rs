use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::filesystem::atomic_write;

use super::{AgentPane, AgentTiming, AgentTimingKey};

const INDEX_VERSION: u8 = 2;
const MAX_AGENT_TIMINGS: usize = 512;
const MAX_PENDING_REQUESTS: usize = 3;

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

pub(super) struct Persistence {
    requests: Arc<RequestMailbox>,
    receiver: Receiver<Completion>,
    worker: Option<thread::JoinHandle<()>>,
    disconnected: bool,
}

#[derive(Default)]
struct RequestMailbox {
    state: Mutex<PendingRequests>,
    ready: Condvar,
}

#[derive(Default)]
struct PendingRequests {
    queue: VecDeque<Request>,
    closed: bool,
}

struct Request {
    submitted: HashMap<AgentTimingKey, AgentTiming>,
    clear_generation: u64,
    operation: Operation,
}

enum Operation {
    Snapshot {
        agents: Vec<AgentPane>,
        now_ms: u64,
    },
    Status {
        key: AgentTimingKey,
        status: super::AgentStatus,
        state_change_seq: u64,
        now_ms: u64,
    },
    Reset {
        agents: Vec<AgentPane>,
        now_ms: u64,
    },
}

struct Completion {
    submitted: HashMap<AgentTimingKey, AgentTiming>,
    clear_generation: u64,
    result: io::Result<PersistedTimings>,
}

struct PersistedTimings {
    timings: HashMap<AgentTimingKey, AgentTiming>,
    cleared_at_ms: u64,
}

impl PendingRequests {
    fn enqueue(&mut self, request: Request) -> Option<Request> {
        if matches!(request.operation, Operation::Reset { .. }) {
            // A clear makes all queued work from older generations irrelevant, but each clear
            // remains an ordered barrier so its watermark cannot be skipped.
            self.queue
                .retain(|pending| matches!(pending.operation, Operation::Reset { .. }));
        } else if let Some(position) = self.queue.iter().rposition(|pending| {
            pending.clear_generation == request.clear_generation
                && matches!(
                    (&pending.operation, &request.operation),
                    (Operation::Snapshot { .. }, Operation::Snapshot { .. })
                        | (Operation::Status { .. }, Operation::Status { .. })
                )
        }) {
            // Submitted state is complete, so the newer operation of the same kind includes the
            // local effects of the one it replaces. Moving it to the back preserves its order
            // relative to the other operation kind.
            self.queue.remove(position);
        }

        if self.queue.len() >= MAX_PENDING_REQUESTS {
            return Some(request);
        }
        self.queue.push_back(request);
        None
    }
}

impl Persistence {
    pub(super) fn new(path: PathBuf) -> Self {
        let requests = Arc::new(RequestMailbox::default());
        let (completions, receiver) = mpsc::channel();
        let worker_requests = Arc::clone(&requests);
        let worker = thread::Builder::new()
            .name("agent-timing-persistence".to_owned())
            .spawn(move || persistence_loop(path, &worker_requests, completions))
            .expect("agent timing persistence worker should start");
        Self {
            requests,
            receiver,
            worker: Some(worker),
            disconnected: false,
        }
    }

    pub(super) fn sync(
        &self,
        local: &HashMap<AgentTimingKey, AgentTiming>,
        agents: &[AgentPane],
        now_ms: u64,
        clear_generation: u64,
    ) -> io::Result<()> {
        self.send(
            local,
            clear_generation,
            Operation::Snapshot {
                agents: agents.to_vec(),
                now_ms,
            },
        )
    }

    pub(super) fn observe_status(
        &self,
        local: &HashMap<AgentTimingKey, AgentTiming>,
        key: AgentTimingKey,
        status: super::AgentStatus,
        state_change_seq: u64,
        now_ms: u64,
        clear_generation: u64,
    ) -> io::Result<()> {
        self.send(
            local,
            clear_generation,
            Operation::Status {
                key,
                status,
                state_change_seq,
                now_ms,
            },
        )
    }

    pub(super) fn reset(
        &self,
        local: &HashMap<AgentTimingKey, AgentTiming>,
        agents: &[AgentPane],
        now_ms: u64,
        clear_generation: u64,
    ) -> io::Result<()> {
        self.send(
            local,
            clear_generation,
            Operation::Reset {
                agents: agents.to_vec(),
                now_ms,
            },
        )
    }

    pub(super) fn poll(
        &mut self,
        local: &mut HashMap<AgentTimingKey, AgentTiming>,
        clear_generation: u64,
    ) -> Option<io::Result<bool>> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(completion) => Some(completion.result.map(|persisted| {
                if completion.clear_generation == clear_generation {
                    reconcile(local, &completion.submitted, persisted)
                } else {
                    false
                }
            })),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "agent timing persistence worker stopped",
                )))
            }
        }
    }

    pub(super) fn shutdown(&mut self) {
        let mut state = self
            .requests
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.closed = true;
        drop(state);
        self.requests.ready.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn send(
        &self,
        local: &HashMap<AgentTimingKey, AgentTiming>,
        clear_generation: u64,
        operation: Operation,
    ) -> io::Result<()> {
        let mut request = Request {
            submitted: local.clone(),
            clear_generation,
            operation,
        };
        let mut state = self
            .requests
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if state.closed {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "agent timing persistence worker stopped",
                ));
            }
            match state.enqueue(request) {
                None => {
                    drop(state);
                    self.requests.ready.notify_one();
                    return Ok(());
                }
                Some(returned) => {
                    request = returned;
                    state = self
                        .requests
                        .ready
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }
}

impl Drop for Persistence {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn persistence_loop(path: PathBuf, requests: &RequestMailbox, completions: Sender<Completion>) {
    loop {
        let request = {
            let mut state = requests
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                if let Some(request) = state.queue.pop_front() {
                    requests.ready.notify_all();
                    break request;
                }
                if state.closed {
                    return;
                }
                state = requests
                    .ready
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };

        let submitted = request.submitted.clone();
        let clear_generation = request.clear_generation;
        let result = execute(&path, &request);
        if completions
            .send(Completion {
                submitted,
                clear_generation,
                result,
            })
            .is_err()
        {
            return;
        }
    }
}

fn execute(path: &Path, request: &Request) -> io::Result<PersistedTimings> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut delay = Duration::from_millis(10);
    loop {
        let mut timings = request.submitted.clone();
        let result = match &request.operation {
            Operation::Snapshot { agents, now_ms } => sync(path, &mut timings, agents, *now_ms),
            Operation::Status {
                key,
                status,
                state_change_seq,
                now_ms,
            } => observe_status(path, &mut timings, key, *status, *state_change_seq, *now_ms),
            Operation::Reset { agents, now_ms } => reset(path, &mut timings, agents, *now_ms),
        };
        match result {
            Ok(cleared_at_ms) => {
                return Ok(PersistedTimings {
                    timings,
                    cleared_at_ms,
                });
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(delay.min(deadline.saturating_duration_since(Instant::now())));
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            Err(error) => return Err(error),
        }
    }
}

fn reconcile(
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    submitted: &HashMap<AgentTimingKey, AgentTiming>,
    mut persisted: PersistedTimings,
) -> bool {
    for (key, timing) in local.iter() {
        if submitted.get(key) != Some(timing) && timing.last_seen_ms > persisted.cleared_at_ms {
            merge_timing(&mut persisted.timings, key.clone(), timing.clone());
        }
    }
    if *local == persisted.timings {
        false
    } else {
        *local = persisted.timings;
        true
    }
}

pub(super) fn sync(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
    now_ms: u64,
) -> io::Result<u64> {
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
    let cleared_at_ms = loaded.cleared_at_ms;
    let mut shared = loaded.timings;

    merge_local_timings(&mut shared, local, cleared_at_ms);
    migrate_session_timings(&mut shared, agents);
    update(&mut shared, agents, now_ms);
    prune(
        &mut shared,
        agents.iter().map(|agent| &agent.runtime.timing_key),
    );

    if needs_rewrite || &shared != local {
        save(path, &shared, cleared_at_ms)?;
        *local = shared;
    }
    Ok(cleared_at_ms)
}

fn migrate_session_timings(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
) {
    for agent in agents {
        if timings.contains_key(&agent.runtime.timing_key) {
            continue;
        }
        let Some(session_key) = agent.runtime.session_timing_key.as_ref() else {
            continue;
        };
        if let Some(timing) = timings.remove(session_key) {
            timings.insert(agent.runtime.timing_key.clone(), timing);
        }
    }
}

pub(super) fn update(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
    now_ms: u64,
) {
    for agent in agents {
        let key = agent.runtime.timing_key.clone();
        if let Some(timing) = timings.get_mut(&key) {
            if timing.state_change_seq == 0
                || agent.runtime.state_change_seq == 0
                || agent.runtime.state_change_seq >= timing.state_change_seq
            {
                timing.observe(agent.runtime.status, agent.runtime.state_change_seq, now_ms);
            }
        } else if agent.runtime.status.should_track_timing() {
            timings.insert(
                key,
                AgentTiming::new(agent.runtime.status, agent.runtime.state_change_seq, now_ms),
            );
        }
    }
}

pub(super) fn update_snapshot(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
    now_ms: u64,
) {
    migrate_session_timings(timings, agents);
    update(timings, agents, now_ms);
    prune(
        timings,
        agents.iter().map(|agent| &agent.runtime.timing_key),
    );
}

pub(super) fn observe_status_local(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    key: &AgentTimingKey,
    status: super::AgentStatus,
    state_change_seq: u64,
    now_ms: u64,
) {
    if let Some(timing) = timings.get_mut(key) {
        timing.observe_event(status, now_ms);
    } else if status.should_track_timing() {
        let mut timing = AgentTiming::new(status, state_change_seq, now_ms);
        timing.awaiting_sequence = true;
        timings.insert(key.clone(), timing);
    }
}

pub(super) fn reset_local(
    timings: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
    now_ms: u64,
) {
    timings.clear();
    update(timings, agents, now_ms);
}

pub(super) fn observe_status(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    key: &AgentTimingKey,
    status: super::AgentStatus,
    state_change_seq: u64,
    now_ms: u64,
) -> io::Result<u64> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = IndexLock::acquire(path)?;
    let loaded = load(path).unwrap_or_default();
    let cleared_at_ms = loaded.cleared_at_ms;
    let mut shared = loaded.timings;
    merge_local_timings(&mut shared, local, cleared_at_ms);
    if let Some(timing) = shared.get_mut(key) {
        timing.observe_event(status, now_ms);
    } else if status.should_track_timing() {
        let mut timing = AgentTiming::new(status, state_change_seq, now_ms);
        timing.awaiting_sequence = true;
        shared.insert(key.clone(), timing);
    }
    prune(&mut shared, std::iter::once(key));
    save(path, &shared, cleared_at_ms)?;
    *local = shared;
    Ok(cleared_at_ms)
}

pub(super) fn reset(
    path: &Path,
    local: &mut HashMap<AgentTimingKey, AgentTiming>,
    agents: &[AgentPane],
    now_ms: u64,
) -> io::Result<u64> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = IndexLock::acquire(path)?;
    let mut reset = HashMap::new();
    update(&mut reset, agents, now_ms);
    save(path, &reset, now_ms)?;
    *local = reset;
    Ok(now_ms)
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

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
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
    use crate::app::herdr_session::AgentStatus;

    #[test]
    fn persistence_worker_merges_shared_history_without_losing_newer_local_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let shared_key = AgentTimingKey::Terminal("shared".to_owned());
        let local_key = AgentTimingKey::Terminal("local".to_owned());
        save(
            &path,
            &HashMap::from([(
                shared_key.clone(),
                AgentTiming::new(AgentStatus::Idle, 1, 1_000),
            )]),
            0,
        )
        .unwrap();
        let mut local = HashMap::from([(
            local_key.clone(),
            AgentTiming::new(AgentStatus::Working, 2, 2_000),
        )]);
        let mut persistence = Persistence::new(path);

        persistence.sync(&local, &[], 2_000, 0).unwrap();
        local
            .get_mut(&local_key)
            .unwrap()
            .observe(AgentStatus::Done, 2, 3_000);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(result) = persistence.poll(&mut local, 0) {
                result.unwrap();
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert!(local.contains_key(&shared_key));
        assert_eq!(local[&local_key].status, AgentStatus::Done);
    }

    #[test]
    fn shutdown_flushes_queued_timing_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let key = AgentTimingKey::Terminal("queued".to_owned());
        let local = HashMap::from([(
            key.clone(),
            AgentTiming::new(AgentStatus::Working, 1, 1_000),
        )]);
        let mut persistence = Persistence::new(path.clone());

        persistence.sync(&local, &[], 1_000, 0).unwrap();
        persistence.shutdown();

        assert!(load(&path).unwrap().timings.contains_key(&key));
    }

    fn status_request(index: u64, generation: u64) -> Request {
        let key = AgentTimingKey::Terminal("agent".to_owned());
        Request {
            submitted: HashMap::from([(
                key.clone(),
                AgentTiming::new(AgentStatus::Working, index, index),
            )]),
            clear_generation: generation,
            operation: Operation::Status {
                key,
                status: AgentStatus::Working,
                state_change_seq: index,
                now_ms: index,
            },
        }
    }

    fn snapshot_request(index: u64, generation: u64) -> Request {
        Request {
            submitted: status_request(index, generation).submitted,
            clear_generation: generation,
            operation: Operation::Snapshot {
                agents: Vec::new(),
                now_ms: index,
            },
        }
    }

    fn reset_request(index: u64, generation: u64) -> Request {
        Request {
            submitted: HashMap::new(),
            clear_generation: generation,
            operation: Operation::Reset {
                agents: Vec::new(),
                now_ms: index,
            },
        }
    }

    #[test]
    fn pending_timing_work_coalesces_status_and_snapshot_bursts() {
        let mut pending = PendingRequests::default();

        for index in 1..=100 {
            assert!(pending.enqueue(status_request(index, 0)).is_none());
            assert!(pending.enqueue(snapshot_request(index, 0)).is_none());
            assert!(pending.queue.len() <= 2);
        }

        assert_eq!(pending.queue.len(), 2);
        assert!(matches!(
            &pending.queue[0].operation,
            Operation::Status { now_ms: 100, .. }
        ));
        assert!(matches!(
            &pending.queue[1].operation,
            Operation::Snapshot { now_ms: 100, .. }
        ));
        assert_eq!(
            pending.queue[1]
                .submitted
                .values()
                .next()
                .unwrap()
                .state_change_seq,
            100
        );
    }

    #[test]
    fn pending_timing_resets_remain_ordered_barriers() {
        let mut pending = PendingRequests::default();
        assert!(pending.enqueue(status_request(1, 0)).is_none());
        assert!(pending.enqueue(snapshot_request(2, 0)).is_none());
        assert!(pending.enqueue(reset_request(3, 1)).is_none());
        assert!(pending.enqueue(status_request(4, 1)).is_none());
        assert!(pending.enqueue(snapshot_request(5, 1)).is_none());

        assert!(pending.enqueue(reset_request(6, 2)).is_none());
        assert!(pending.enqueue(status_request(7, 2)).is_none());

        assert_eq!(pending.queue.len(), 3);
        assert!(matches!(
            &pending.queue[0].operation,
            Operation::Reset { now_ms: 3, .. }
        ));
        assert!(matches!(
            &pending.queue[1].operation,
            Operation::Reset { now_ms: 6, .. }
        ));
        assert!(matches!(
            &pending.queue[2].operation,
            Operation::Status { now_ms: 7, .. }
        ));
        assert_eq!(
            pending
                .queue
                .iter()
                .map(|request| request.clear_generation)
                .collect::<Vec<_>>(),
            vec![1, 2, 2]
        );
        assert!(pending.enqueue(reset_request(8, 3)).is_none());
        assert!(pending.enqueue(reset_request(9, 4)).is_some());
        assert_eq!(pending.queue.len(), MAX_PENDING_REQUESTS);
        assert_eq!(
            pending
                .queue
                .iter()
                .map(|request| request.clear_generation)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn shutdown_flushes_reset_barrier_and_final_timing_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let old_key = AgentTimingKey::Terminal("old".to_owned());
        save(
            &path,
            &HashMap::from([(
                old_key.clone(),
                AgentTiming::new(AgentStatus::Idle, 1, 1_000),
            )]),
            0,
        )
        .unwrap();
        let mut persistence = Persistence::new(path.clone());
        persistence.reset(&HashMap::new(), &[], 2_000, 1).unwrap();
        let new_key = AgentTimingKey::Terminal("new".to_owned());
        let local = HashMap::from([(
            new_key.clone(),
            AgentTiming::new(AgentStatus::Working, 2, 3_000),
        )]);
        persistence
            .observe_status(&local, new_key.clone(), AgentStatus::Working, 2, 3_000, 1)
            .unwrap();

        persistence.shutdown();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.cleared_at_ms, 2_000);
        assert!(!loaded.timings.contains_key(&old_key));
        assert!(loaded.timings.contains_key(&new_key));
    }

    #[test]
    fn migrates_the_active_session_timer_to_the_agent() {
        let session_key = AgentTimingKey::Session(super::super::AgentSessionIdentity {
            source: "herdr:opencode".to_owned(),
            agent: "opencode".to_owned(),
            kind: "id".to_owned(),
            value: "ses_old".to_owned(),
        });
        let agent_key = AgentTimingKey::Terminal("opencode@term-1".to_owned());
        let agent = AgentPane {
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            pane_id: "pane".to_owned(),
            cwd: None,
            destination_cwd: None,
            focused: false,
            runtime: super::super::AgentRuntime {
                name: "opencode".to_owned(),
                session_name: None,
                status: AgentStatus::Idle,
                timing_key: agent_key.clone(),
                session_timing_key: Some(session_key.clone()),
                state_change_seq: 7,
            },
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

    #[cfg(unix)]
    #[test]
    fn timing_index_lock_never_waits_for_another_process() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-timings.json");
        let _held = IndexLock::acquire(&path).unwrap();

        let error = match IndexLock::acquire(&path) {
            Ok(_) => panic!("a held timing lock was acquired twice"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    }
}
