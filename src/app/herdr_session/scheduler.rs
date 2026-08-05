use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{
    AgentStatus,
    client::{
        SchedulerLaunchRequest, SchedulerLaunchResult, SchedulerObserveResult, scheduler_launch,
        scheduler_observe,
    },
};

const MAX_RUNS: i64 = 50;
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTask {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTaskEdit {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduledRunStatus {
    Launching,
    Working,
    Blocked,
    Unknown,
    Completed,
    Failed,
}

impl ScheduledRunStatus {
    fn text(self) -> &'static str {
        [
            "launching",
            "working",
            "blocked",
            "unknown",
            "completed",
            "failed",
        ][self as usize]
    }

    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "launching" => Ok(Self::Launching),
            "working" => Ok(Self::Working),
            "blocked" => Ok(Self::Blocked),
            "unknown" => Ok(Self::Unknown),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledRun {
    pub(crate) id: i64,
    pub(crate) task_id: i64,
    pub(crate) status: ScheduledRunStatus,
    pub(crate) output: String,
    pub(crate) error: Option<String>,
}

type State = (Vec<ScheduledTask>, Vec<ScheduledRun>);
type Update = Result<State, String>;
type Launcher<'a> = dyn FnMut(SchedulerLaunchRequest) -> SchedulerLaunchResult + 'a;
type Observer<'a> = dyn FnMut(&str) -> SchedulerObserveResult + 'a;

pub(crate) struct SchedulerService {
    pub(crate) tasks: Vec<ScheduledTask>,
    pub(crate) runs: Vec<ScheduledRun>,
    commands: Sender<Command>,
    updates: Receiver<Update>,
    worker: Option<JoinHandle<()>>,
    error: Option<String>,
}

impl SchedulerService {
    pub(crate) fn open(path: Option<PathBuf>) -> Result<Self, String> {
        let enabled = path.is_some();
        let mut db = path
            .map_or_else(Connection::open_in_memory, Connection::open)
            .map_err(|error| format!("Could not open scheduler database: {error}"))?;
        prepare_database(&mut db)?;
        recover_stale_launches(&db, now_ms())?;
        let (tasks, runs) = load_state(&db)?;
        let (commands, command_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("hunkle-scheduler".to_owned())
            .spawn(move || worker(db, command_rx, update_tx, enabled))
            .map_err(|error| format!("Could not start scheduler worker: {error}"))?;
        Ok(Self {
            tasks,
            runs,
            commands,
            updates,
            worker: Some(worker),
            error: None,
        })
    }

    pub(crate) fn save_task(&self, mut task: ScheduledTaskEdit) -> Result<(), String> {
        let interval = interval_ms(task.interval_minutes)?;
        task.destination = validate_destination(&task.destination)?;
        task.title = task.title.trim().to_owned();
        if task.title.is_empty() || task.prompt.trim().is_empty() {
            return Err(format!(
                "scheduled task {} is required",
                if task.title.is_empty() {
                    "title"
                } else {
                    "prompt"
                }
            ));
        }
        let next = now_ms()
            .checked_add(interval)
            .ok_or_else(|| "schedule is too large".to_owned())?;
        self.send(Command::Save(task, next))
    }

    pub(crate) fn toggle_task(&self, id: i64, enabled: bool) -> Result<(), String> {
        self.send(Command::Toggle(id, enabled))
    }
    pub(crate) fn delete_task(&self, id: i64) -> Result<(), String> {
        self.send(Command::Delete(id))
    }
    pub(crate) fn run_now(&self, id: i64) -> Result<(), String> {
        self.send(Command::RunNow(id))
    }
    pub(crate) fn refresh_run(&self, id: i64) -> Result<(), String> {
        self.send(Command::Refresh(id))
    }

    pub(crate) fn poll_completions(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Ok((tasks, runs)) => {
                    self.tasks = tasks;
                    self.runs = runs;
                    changed = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        changed
    }

    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.commands.send(Command::Shutdown);
            let _ = worker.join();
        }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "scheduler worker is not running".to_owned())
    }
}

impl Drop for SchedulerService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum Command {
    Save(ScheduledTaskEdit, i64),
    Toggle(i64, bool),
    Delete(i64),
    RunNow(i64),
    Refresh(i64),
    Shutdown,
}

struct Claim {
    run_id: i64,
    task_id: i64,
    title: String,
    prompt: String,
    destination: PathBuf,
}

fn worker(mut db: Connection, commands: Receiver<Command>, updates: Sender<Update>, enabled: bool) {
    loop {
        match commands.recv_timeout(Duration::from_secs(if enabled { 2 } else { 30 })) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                let result = match command {
                    Command::Save(task, next) => insert_task(&db, task, next),
                    Command::Toggle(id, enabled) => changed(
                        db.execute(
                            "UPDATE scheduled_tasks SET enabled = ?2 WHERE id = ?1",
                            params![id, enabled],
                        ),
                        "scheduled task not found",
                    ),
                    Command::Delete(id) => delete_task(&db, id),
                    Command::RunNow(id) if enabled => claim(&mut db, Some(id), now_ms())
                        .and_then(|run| run.ok_or_else(|| "scheduled task not found".to_owned()))
                        .and_then(|run| execute_claim(&db, run, &mut scheduler_launch)),
                    Command::RunNow(_) => {
                        Err("scheduler execution is disabled for an in-memory service".to_owned())
                    }
                    Command::Refresh(id) => refresh(&db, Some(id), &mut scheduler_observe),
                    Command::Shutdown => unreachable!(),
                };
                publish(&db, &updates, result);
            }
            Err(RecvTimeoutError::Timeout) => {
                let result = if enabled {
                    (|| {
                        while let Some(run) = claim(&mut db, None, now_ms())? {
                            execute_claim(&db, run, &mut scheduler_launch)?;
                        }
                        refresh(&db, None, &mut scheduler_observe)
                    })()
                } else {
                    Ok(())
                };
                publish(&db, &updates, result);
            }
        }
    }
}

fn publish(db: &Connection, updates: &Sender<Update>, result: Result<(), String>) {
    let _ = result.map_err(|error| updates.send(Err(error)));
    let _ = updates.send(load_state(db));
}

fn insert_task(db: &Connection, task: ScheduledTaskEdit, next: i64) -> Result<(), String> {
    db.execute(
        "INSERT INTO scheduled_tasks (title, description, prompt, destination, repository, branch, enabled, interval_minutes, next_run_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![task.title, task.description, task.prompt, encode_path(&task.destination), task.repository, task.branch, task.enabled, i64::try_from(task.interval_minutes).map_err(|_| "schedule is too large")?, next],
    )
    .map_err(db_error)?;
    Ok(())
}

fn delete_task(db: &Connection, id: i64) -> Result<(), String> {
    changed(
        db.execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id]),
        "scheduled task not found",
    )
}

fn claim(db: &mut Connection, requested: Option<i64>, now: i64) -> Result<Option<Claim>, String> {
    loop {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let task: Option<(i64, String, String, Vec<u8>, i64, i64)> = tx
            .query_row(
                "SELECT id, title, prompt, destination, interval_minutes, next_run_ms FROM scheduled_tasks WHERE (?1 IS NULL AND enabled = 1 AND next_run_ms <= ?2) OR id = ?1 ORDER BY next_run_ms, id LIMIT 1",
                params![requested, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((task_id, title, prompt, path, minutes, next)) = task else {
            tx.commit().map_err(db_error)?;
            return Ok(None);
        };
        let interval = minutes
            .checked_mul(60_000)
            .filter(|value| *value > 0)
            .ok_or_else(|| "stored schedule interval must be positive".to_owned())?;
        let scheduled = if requested.is_some() {
            now
        } else {
            let scheduled = now - now.saturating_sub(next) % interval;
            tx.execute(
                "UPDATE scheduled_tasks SET next_run_ms = ?2 WHERE id = ?1",
                params![
                    task_id,
                    scheduled
                        .checked_add(interval)
                        .ok_or("schedule overflowed")?
                ],
            )
            .map_err(db_error)?;
            scheduled
        };
        let active = tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduled_runs WHERE task_id = ?1 AND status IN ('launching', 'working', 'blocked', 'unknown'))", [task_id], |row| row.get(0)).map_err(db_error)?;
        if active {
            if requested.is_some() {
                return Err("scheduled task already has an active run".to_owned());
            }
            tx.commit().map_err(db_error)?;
            continue;
        }
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms) VALUES (?1, ?2, 'launching', ?3)",
                params![task_id, scheduled, now_ms()],
            )
            .map_err(db_error)?;
        if inserted == 0 {
            if requested.is_some() {
                return Err("scheduled run was already claimed".to_owned());
            }
            tx.commit().map_err(db_error)?;
            continue;
        }
        let run_id = tx.last_insert_rowid();
        retain_history(&tx, task_id)?;
        let destination = decode_path(path).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        return Ok(Some(Claim {
            run_id,
            task_id,
            title,
            prompt,
            destination,
        }));
    }
}

fn retain_history(db: &Connection, task_id: i64) -> Result<(), String> {
    db.execute(
        "DELETE FROM scheduled_runs WHERE task_id = ?1 AND id NOT IN (SELECT id FROM scheduled_runs WHERE task_id = ?1 ORDER BY created_at_ms DESC, id DESC LIMIT ?2)",
        params![task_id, MAX_RUNS],
    )
    .map_err(db_error)?;
    Ok(())
}

fn execute_claim(db: &Connection, claim: Claim, launch: &mut Launcher) -> Result<(), String> {
    if let Err(error) = validate_destination(&claim.destination) {
        return fail_run(db, claim.run_id, &error);
    }
    let result = launch(SchedulerLaunchRequest {
        destination: claim.destination,
        label: format!("Hunkle: {} #{}", claim.title, claim.task_id),
        prompt: claim.prompt,
    });
    let status = if result.error.is_some() && !result.agent_started {
        ScheduledRunStatus::Failed
    } else if result.error.is_some() {
        ScheduledRunStatus::Unknown
    } else {
        agent_status(result.status)
    };
    db.execute(
        "UPDATE scheduled_runs SET status = ?2, pane_id = ?3, error = ?4 WHERE id = ?1",
        params![claim.run_id, status.text(), result.pane_id, result.error],
    )
    .map_err(db_error)?;
    Ok(())
}

fn refresh(db: &Connection, requested: Option<i64>, observe: &mut Observer) -> Result<(), String> {
    let mut statement = db
        .prepare("SELECT id, pane_id FROM scheduled_runs WHERE (?1 IS NULL AND status IN ('working', 'blocked', 'unknown') AND pane_id IS NOT NULL) OR id = ?1")
        .map_err(db_error)?;
    let runs = statement
        .query_map([requested], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    if requested.is_some() && runs.is_empty() {
        return Err("scheduled run not found".to_owned());
    }
    for (id, pane) in runs {
        let pane = pane.ok_or_else(|| "scheduled run has no Herdr pane".to_owned())?;
        apply_observation(db, id, observe(&pane))?;
    }
    Ok(())
}

fn apply_observation(
    db: &Connection,
    run_id: i64,
    result: SchedulerObserveResult,
) -> Result<(), String> {
    let (status, output, error) = match result {
        SchedulerObserveResult::Missing(error) => {
            return fail_run(db, run_id, &error);
        }
        SchedulerObserveResult::Unavailable(error) => {
            (ScheduledRunStatus::Unknown, None, Some(error))
        }
        SchedulerObserveResult::Observed(status, result) => match result {
            Ok(output) => (agent_status(status), Some(bounded_output(&output)), None),
            Err(error) => (agent_status(status), None, Some(error)),
        },
    };
    db.execute(
        "UPDATE scheduled_runs SET status = ?2, output = COALESCE(?3, output), error = ?4 WHERE id = ?1",
        params![run_id, status.text(), output, error],
    )
    .map_err(db_error)?;
    Ok(())
}

fn agent_status(status: AgentStatus) -> ScheduledRunStatus {
    match status {
        AgentStatus::Idle | AgentStatus::Done => ScheduledRunStatus::Completed,
        AgentStatus::Working => ScheduledRunStatus::Working,
        AgentStatus::Blocked => ScheduledRunStatus::Blocked,
        AgentStatus::Unknown => ScheduledRunStatus::Unknown,
    }
}

fn fail_run(db: &Connection, id: i64, error: &str) -> Result<(), String> {
    db.execute(
        "UPDATE scheduled_runs SET status = 'failed', error = ?2 WHERE id = ?1",
        params![id, error],
    )
    .map_err(db_error)?;
    Ok(())
}

fn interval_ms(minutes: u64) -> Result<i64, String> {
    i64::try_from(
        minutes
            .checked_mul(60_000)
            .filter(|value| *value > 0)
            .ok_or_else(|| "schedule interval must be positive".to_owned())?,
    )
    .map_err(|_| "schedule interval is too large".to_owned())
}

fn validate_destination(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err("scheduler destination must be an existing absolute directory".to_owned());
    }
    std::fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve scheduler destination: {error}"))
}

fn bounded_output(output: &str) -> String {
    let mut start = output.len().saturating_sub(MAX_OUTPUT);
    while !output.is_char_boundary(start) {
        start += 1;
    }
    output[start..].to_owned()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn db_error(error: rusqlite::Error) -> String {
    error.to_string()
}

fn changed(result: rusqlite::Result<usize>, missing: &str) -> Result<(), String> {
    match result.map_err(db_error)? {
        1 => Ok(()),
        _ => Err(missing.to_owned()),
    }
}

fn prepare_database(db: &mut Connection) -> Result<(), String> {
    db.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(db_error)?;
    let version = db
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(db_error)?;
    let sql = match version {
        0 => "CREATE TABLE scheduled_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, description TEXT NOT NULL, prompt TEXT NOT NULL, destination BLOB NOT NULL, repository TEXT NOT NULL, branch TEXT NOT NULL, enabled INTEGER NOT NULL, interval_minutes INTEGER NOT NULL CHECK (interval_minutes > 0), next_run_ms INTEGER NOT NULL);
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, UNIQUE (task_id, scheduled_for_ms));
              CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id); PRAGMA user_version = 2;",
        1 => "ALTER TABLE scheduled_runs RENAME TO scheduled_runs_v1;
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, UNIQUE (task_id, scheduled_for_ms));
              INSERT INTO scheduled_runs (id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, output, error) SELECT id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, output, error FROM scheduled_runs_v1 WHERE task_id IS NOT NULL;
              DROP TABLE scheduled_runs_v1; CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id); PRAGMA user_version = 2;",
        2 => return Ok(()),
        _ => return Err(format!("scheduler database version {version} is newer than supported")),
    };
    db.execute_batch(sql).map_err(db_error)
}

fn recover_stale_launches(db: &Connection, now: i64) -> Result<(), String> {
    db.execute(
        "UPDATE scheduled_runs SET status = 'failed', error = 'Hunkle stopped while launching this run' WHERE status = 'launching' AND created_at_ms <= ?1",
        [now.saturating_sub(5 * 60 * 1_000)],
    )
    .map_err(db_error)?;
    Ok(())
}

#[cfg(unix)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn decode_path(bytes: Vec<u8>) -> rusqlite::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(windows)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn decode_path(bytes: Vec<u8>) -> rusqlite::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    if !bytes.len().is_multiple_of(2) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )))
}

fn load_state(db: &Connection) -> Result<State, String> {
    let mut statement = db.prepare("SELECT id, title, destination, repository, branch, enabled, interval_minutes FROM scheduled_tasks ORDER BY id").map_err(db_error)?;
    let tasks = statement
        .query_map([], |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                title: row.get(1)?,
                destination: decode_path(row.get(2)?)?,
                repository: row.get(3)?,
                branch: row.get(4)?,
                enabled: row.get(5)?,
                interval_minutes: row.get(6)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    let mut statement = db.prepare("SELECT id, task_id, status, output, error FROM scheduled_runs ORDER BY created_at_ms DESC, id DESC").map_err(db_error)?;
    let runs = statement
        .query_map([], |row| {
            Ok(ScheduledRun {
                id: row.get(0)?,
                task_id: row.get(1)?,
                status: ScheduledRunStatus::parse(&row.get::<_, String>(2)?)?,
                output: bounded_output(&row.get::<_, String>(3)?),
                error: row.get(4)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok((tasks, runs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_behaviors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scheduler.sqlite3");
        let mut first = Connection::open(&path).unwrap();
        first.execute_batch("CREATE TABLE scheduled_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, description TEXT NOT NULL, prompt TEXT NOT NULL, destination BLOB NOT NULL, repository TEXT NOT NULL, branch TEXT NOT NULL, enabled INTEGER NOT NULL, interval_minutes INTEGER NOT NULL, next_run_ms INTEGER NOT NULL); CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER REFERENCES scheduled_tasks(id) ON DELETE SET NULL, task_title TEXT NOT NULL, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, started_at_ms INTEGER, completed_at_ms INTEGER, workspace_id TEXT, tab_id TEXT, pane_id TEXT, terminal_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT); PRAGMA user_version = 1;").unwrap();
        first.execute("INSERT INTO scheduled_tasks (title, description, prompt, destination, repository, branch, enabled, interval_minutes, next_run_ms) VALUES ('Review', '', 'Review it', ?1, 'repo', 'main', 1, 1, 100)", [encode_path(directory.path())]).unwrap();
        first.execute("INSERT INTO scheduled_runs (task_id, task_title, scheduled_for_ms, status, created_at_ms) VALUES (1, 'Review', 1, 'completed', 1)", []).unwrap();
        prepare_database(&mut first).unwrap();
        let mut second = Connection::open(&path).unwrap();
        let run = claim(&mut first, None, 190_000).unwrap().unwrap();
        assert!(claim(&mut second, None, 250_000).unwrap().is_none());
        execute_claim(&first, run, &mut |_| SchedulerLaunchResult {
            pane_id: Some("w1:p1".into()),
            agent_started: true,
            status: AgentStatus::Working,
            error: None,
        })
        .unwrap();
        refresh(&first, None, &mut |_| {
            SchedulerObserveResult::Observed(AgentStatus::Done, Ok("é".repeat(MAX_OUTPUT)))
        })
        .unwrap();
        first.execute("INSERT OR REPLACE INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms) VALUES (1, 99, 'launching', 1)", []).unwrap();
        recover_stale_launches(&first, 500_000).unwrap();
        let state = load_state(&first).unwrap();
        let bounded = state.1.iter().all(|run| run.output.len() <= MAX_OUTPUT);
        let failed = state
            .1
            .iter()
            .any(|run| run.status == ScheduledRunStatus::Failed);
        assert!(bounded && failed);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let destination = directory
                .path()
                .join(std::ffi::OsString::from_vec(b"repo-\xff".to_vec()));
            first
                .execute(
                    "UPDATE scheduled_tasks SET destination = ?1",
                    [encode_path(&destination)],
                )
                .unwrap();
            drop(second);
            drop(first);
            first = Connection::open(&path).unwrap();
            prepare_database(&mut first).unwrap();
            assert_eq!(load_state(&first).unwrap().0[0].destination, destination);
        }
        delete_task(&first, 1).unwrap();
        assert_eq!(load_state(&first).unwrap(), (Vec::new(), Vec::new()));
    }
}
