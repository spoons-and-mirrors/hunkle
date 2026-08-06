use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{
    filesystem::{
        atomic_write_if_unchanged, atomic_write_workspace, ensure_workspace_directory,
        read_optional_workspace_directory, read_workspace_file, remove_workspace_file,
        remove_workspace_file_if_unchanged,
    },
    repo_path::RepoPath,
};

use super::{
    AgentStatus,
    client::{
        SchedulerLaunchRequest, SchedulerLaunchResult, SchedulerObserveResult, scheduler_launch,
        scheduler_observe,
    },
};

const MAX_RUNS: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTask {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
    pub(crate) next_run_ms: i64,
    pub(crate) source: Option<RepoPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTaskEdit {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
    pub(crate) source: Option<RepoPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTaskSource {
    pub(crate) root: PathBuf,
    pub(crate) path: RepoPath,
    pub(crate) original: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTaskDestination {
    pub(crate) path: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
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
    pub(crate) fn text(self) -> &'static str {
        [
            "launching",
            "working",
            "blocked",
            "unknown",
            "completed",
            "failed",
        ][self as usize]
    }

    pub(crate) fn is_active(self) -> bool {
        matches!(
            self,
            Self::Launching | Self::Working | Self::Blocked | Self::Unknown
        )
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
    pub(crate) created_at_ms: i64,
    pub(crate) status: ScheduledRunStatus,
    pub(crate) pane_id: Option<String>,
    pub(crate) terminal_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) error: Option<String>,
}

type State = (Vec<ScheduledTask>, Vec<ScheduledRun>);
type Update = Result<State, String>;
pub(crate) struct SchedulerService {
    pub(crate) tasks: Vec<ScheduledTask>,
    pub(crate) runs: Vec<ScheduledRun>,
    commands: Sender<Command>,
    updates: Receiver<Update>,
    worker: Option<JoinHandle<()>>,
    files_root: Option<PathBuf>,
}

impl SchedulerService {
    pub(crate) fn open(path: Option<PathBuf>, files_root: Option<PathBuf>) -> Result<Self, String> {
        let enabled = path.is_some();
        let files_root = files_root
            .map(|root| {
                std::fs::create_dir_all(&root)
                    .map_err(|error| format!("Could not create Hunkle data directory: {error}"))?;
                std::fs::canonicalize(&root)
                    .map_err(|error| format!("Could not resolve Hunkle data directory: {error}"))
            })
            .transpose()?;
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
            .spawn({
                let files_root = files_root.clone();
                move || worker(db, command_rx, update_tx, enabled, files_root)
            })
            .map_err(|error| format!("Could not start scheduler worker: {error}"))?;
        Ok(Self {
            tasks,
            runs,
            commands,
            updates,
            worker: Some(worker),
            files_root,
        })
    }

    pub(crate) fn save_task(
        &self,
        id: Option<i64>,
        mut task: ScheduledTaskEdit,
        original: Option<ScheduledTaskSource>,
    ) -> Result<(), String> {
        let interval = interval_ms(task.interval_minutes)?;
        task.destination = validate_destination(&task.destination)?;
        task.title = task.title.trim().to_owned();
        validate_task(&task)?;
        let next = now_ms()
            .checked_add(interval)
            .ok_or_else(|| "schedule is too large".to_owned())?;
        if let Some(source) = original
            && let Some(files_root) = self.files_root.as_deref()
        {
            if task.source.as_ref() != Some(&source.path) {
                return Err("scheduled task source changed while editing".to_owned());
            }
            if source.root == files_root {
                let content = render_task_file(&task);
                atomic_write_if_unchanged(
                    files_root,
                    &source.path,
                    &source.original,
                    content.as_bytes(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                ensure_workspace_directory(files_root, &RepoPath::from("scheduled"))
                    .map_err(|error| error.to_string())?;
                let target = available_task_path(files_root, &task.title, id)?;
                task.source = Some(target.clone());
                atomic_write_workspace(files_root, &target, render_task_file(&task).as_bytes())
                    .map_err(|error| error.to_string())?;
                if let Err(error) =
                    remove_workspace_file_if_unchanged(&source.root, &source.path, &source.original)
                {
                    let rollback = remove_workspace_file(files_root, &target);
                    return Err(match rollback {
                        Ok(()) => error.to_string(),
                        Err(rollback) => format!(
                            "{error}; could not remove the new task file during rollback: {rollback}"
                        ),
                    });
                }
            }
        } else if id.is_none()
            && let Some(files_root) = self.files_root.as_deref()
        {
            ensure_workspace_directory(files_root, &RepoPath::from("scheduled"))
                .map_err(|error| error.to_string())?;
            let source = available_task_path(files_root, &task.title, None)?;
            task.source = Some(source.clone());
            atomic_write_workspace(files_root, &source, render_task_file(&task).as_bytes())
                .map_err(|error| error.to_string())?;
        }
        self.send(Command::Save(id, task, next))
    }

    pub(crate) fn task_source(
        &self,
        task: &ScheduledTask,
    ) -> Result<Option<ScheduledTaskSource>, String> {
        let Some(path) = task.source.clone() else {
            return Ok(None);
        };
        let root = if path.as_path().starts_with("scheduled") {
            self.files_root
                .clone()
                .ok_or_else(|| "Hunkle task storage is unavailable".to_owned())?
        } else {
            task.destination.clone()
        };
        let original = read_workspace_file(&root, &path)
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        Ok(Some(ScheduledTaskSource {
            root,
            path,
            original,
        }))
    }

    pub(crate) fn sync_task_files(
        &self,
        destinations: Vec<ScheduledTaskDestination>,
    ) -> Result<(), String> {
        self.send(Command::Sync(destinations))
    }

    pub(crate) fn toggle_task(&self, id: i64, enabled: bool) -> Result<(), String> {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return Err("scheduled task not found".to_owned());
        };
        if task.source.is_none() {
            return self.send(Command::Toggle(id, enabled));
        }
        let source = self.task_source(task)?.expect("source was checked");
        let mut edit = task.edit();
        edit.enabled = enabled;
        self.save_task(Some(id), edit, Some(source))
    }
    pub(crate) fn delete_task(&self, id: i64) -> Result<(), String> {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return Err("scheduled task not found".to_owned());
        };
        if let Some(source) = task.source.as_ref() {
            let root = if source.as_path().starts_with("scheduled") {
                self.files_root
                    .as_deref()
                    .ok_or_else(|| "Hunkle task storage is unavailable".to_owned())?
            } else {
                &task.destination
            };
            remove_workspace_file(root, source).map_err(|error| error.to_string())?;
        }
        self.send(Command::Delete(id))
    }
    pub(crate) fn run_now(&self, id: i64) -> Result<(), String> {
        if !self.tasks.iter().any(|task| task.id == id) {
            return Err("scheduled task not found".to_owned());
        }
        if self
            .runs
            .iter()
            .any(|run| run.task_id == id && run.status.is_active())
        {
            return Err("scheduled task already has an active run".to_owned());
        }
        self.send(Command::RunNow(id))
    }
    pub(crate) fn refresh_run(&self, id: i64) -> Result<(), String> {
        self.send(Command::Refresh(id))
    }

    pub(crate) fn bind_agent(&mut self, id: i64, pane_id: String, terminal_id: String) {
        let Some(run) = self
            .runs
            .iter_mut()
            .find(|run| run.id == id && run.terminal_id.is_none())
        else {
            return;
        };
        run.pane_id = Some(pane_id.clone());
        run.terminal_id = Some(terminal_id.clone());
        let _ = self.send(Command::BindAgent(id, pane_id, terminal_id));
    }

    pub(crate) fn bind_session(&mut self, id: i64, session_id: String) {
        let Some(run) = self
            .runs
            .iter_mut()
            .find(|run| run.id == id && run.session_id.is_none())
        else {
            return;
        };
        run.session_id = Some(session_id.clone());
        let _ = self.send(Command::BindSession(id, session_id));
    }

    pub(crate) fn poll_completions(&mut self) -> (bool, Option<String>) {
        let mut changed = false;
        let mut error = None;
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Ok((tasks, runs)) => {
                    self.tasks = tasks;
                    self.runs = runs;
                    changed = true;
                }
                Err(update_error) => error = Some(update_error),
            }
        }
        (changed, error)
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
    Save(Option<i64>, ScheduledTaskEdit, i64),
    Sync(Vec<ScheduledTaskDestination>),
    Toggle(i64, bool),
    Delete(i64),
    RunNow(i64),
    Refresh(i64),
    BindAgent(i64, String, String),
    BindSession(i64, String),
    Shutdown,
}

struct Claim {
    run_id: i64,
    task_id: i64,
    title: String,
    prompt: String,
    model: Option<String>,
    destination: PathBuf,
}

fn worker(
    mut db: Connection,
    commands: Receiver<Command>,
    updates: Sender<Update>,
    enabled: bool,
    files_root: Option<PathBuf>,
) {
    loop {
        match commands.recv_timeout(Duration::from_secs(if enabled { 2 } else { 30 })) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                let result = match command {
                    Command::Save(id, task, next) => save_task(&db, id, task, next),
                    Command::Sync(destinations) => files_root
                        .as_deref()
                        .ok_or_else(|| "Hunkle task storage is unavailable".to_owned())
                        .and_then(|files_root| sync_task_files(&db, files_root, &destinations)),
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
                        .and_then(|run| {
                            execute_claim_with_update(
                                &db,
                                run,
                                &mut scheduler_launch,
                                &updates,
                            )
                        }),
                    Command::RunNow(_) => {
                        Err("scheduler execution is disabled for an in-memory service".to_owned())
                    }
                    Command::Refresh(id) => refresh(&db, Some(id), &mut scheduler_observe),
                    Command::BindAgent(id, pane_id, terminal_id) => db
                        .execute(
                            "UPDATE scheduled_runs SET pane_id = ?2, terminal_id = ?3 WHERE id = ?1 AND terminal_id IS NULL",
                            params![id, pane_id, terminal_id],
                        )
                        .map(|_| ())
                        .map_err(db_error),
                    Command::BindSession(id, session_id) => db
                        .execute(
                            "UPDATE scheduled_runs SET session_id = ?2 WHERE id = ?1 AND session_id IS NULL",
                            params![id, session_id],
                        )
                        .map(|_| ())
                        .map_err(db_error),
                    Command::Shutdown => unreachable!(),
                };
                publish(&db, &updates, result);
            }
            Err(RecvTimeoutError::Timeout) => {
                let result = if enabled {
                    (|| {
                        while let Some(run) = claim(&mut db, None, now_ms())? {
                            execute_claim_with_update(&db, run, &mut scheduler_launch, &updates)?;
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

fn save_task(
    db: &Connection,
    id: Option<i64>,
    task: ScheduledTaskEdit,
    next: i64,
) -> Result<(), String> {
    let source = task.source.as_ref().map(|path| encode_path(path.as_path()));
    let minutes = i64::try_from(task.interval_minutes).map_err(|_| "schedule is too large")?;
    if let Some(id) = id {
        return changed(
            db.execute(
                "UPDATE scheduled_tasks SET title = ?2, description = ?3, prompt = ?4, model = ?5, destination = ?6, repository = ?7, branch = ?8, enabled = ?9, interval_minutes = ?10, next_run_ms = ?11, source_path = ?12 WHERE id = ?1",
                params![id, task.title, task.description, task.prompt, task.model, encode_path(&task.destination), task.repository, task.branch, task.enabled, minutes, next, source],
            ),
            "scheduled task not found",
        );
    }
    db.execute(
        "INSERT INTO scheduled_tasks (title, description, prompt, model, destination, repository, branch, enabled, interval_minutes, next_run_ms, source_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![task.title, task.description, task.prompt, task.model, encode_path(&task.destination), task.repository, task.branch, task.enabled, minutes, next, source],
    )
    .map_err(db_error)?;
    Ok(())
}

fn sync_task_files(
    db: &Connection,
    files_root: &Path,
    destinations: &[ScheduledTaskDestination],
) -> Result<(), String> {
    let destinations = destinations
        .iter()
        .map(|destination| {
            Ok(ScheduledTaskDestination {
                path: validate_destination(&destination.path)?,
                repository: destination.repository.clone(),
                branch: destination.branch.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ensure_workspace_directory(files_root, &RepoPath::from("scheduled"))
        .map_err(|error| error.to_string())?;

    for task in load_state(db)?.0 {
        if task
            .source
            .as_ref()
            .is_some_and(|source| source.as_path().starts_with("scheduled"))
        {
            continue;
        }
        let legacy = task.source.as_ref().and_then(|source| {
            read_workspace_file(&task.destination, source)
                .ok()
                .map(|content| (source.clone(), content))
        });
        let mut edit = if let Some((source, content)) = legacy.as_ref() {
            parse_task_file(
                content,
                source.clone(),
                Some(ScheduledTaskDestination {
                    path: task.destination.clone(),
                    repository: task.repository.clone(),
                    branch: task.branch.clone(),
                }),
            )?
        } else {
            task.edit()
        };
        let source = available_task_path(files_root, &edit.title, Some(task.id))?;
        edit.source = Some(source.clone());
        atomic_write_workspace(files_root, &source, render_task_file(&edit).as_bytes())
            .map_err(|error| error.to_string())?;
        if let Some((legacy_source, content)) = legacy
            && let Err(error) =
                remove_workspace_file_if_unchanged(&task.destination, &legacy_source, &content)
        {
            let _ = remove_workspace_file(files_root, &source);
            return Err(error.to_string());
        }
        save_task(db, Some(task.id), edit, task.next_run_ms)?;
    }

    let mut errors = Vec::new();
    for destination in &destinations {
        let entries = read_optional_workspace_directory(
            &destination.path,
            &RepoPath::from(".hunkle/scheduled"),
        )
        .map_err(|error| error.to_string())?;
        for entry in entries {
            if entry.is_directory
                || entry
                    .path
                    .as_path()
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("md")
            {
                continue;
            }
            let content = match read_workspace_file(&destination.path, &entry.path) {
                Ok(content) => content,
                Err(error) => {
                    errors.push(format!("{}: {error}", entry.path.display()));
                    continue;
                }
            };
            let mut edit =
                match parse_task_file(&content, entry.path.clone(), Some(destination.clone())) {
                    Ok(edit) => edit,
                    Err(error) => {
                        errors.push(format!("{}: {error}", entry.path.display()));
                        continue;
                    }
                };
            let source = available_task_path(files_root, &edit.title, None)?;
            edit.source = Some(source.clone());
            atomic_write_workspace(files_root, &source, render_task_file(&edit).as_bytes())
                .map_err(|error| error.to_string())?;
            if let Err(error) =
                remove_workspace_file_if_unchanged(&destination.path, &entry.path, &content)
            {
                let _ = remove_workspace_file(files_root, &source);
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
            let next = now_ms()
                .checked_add(interval_ms(edit.interval_minutes)?)
                .ok_or_else(|| "schedule is too large".to_owned())?;
            save_task(db, None, edit, next)?;
        }
    }

    for task in load_state(db)?.0.into_iter().filter(|task| {
        task.source
            .as_ref()
            .is_some_and(|source| source.as_path().starts_with("scheduled"))
    }) {
        db.execute(
            "UPDATE scheduled_tasks SET enabled = 0 WHERE id = ?1",
            [task.id],
        )
        .map_err(db_error)?;
    }
    let entries = read_optional_workspace_directory(files_root, &RepoPath::from("scheduled"))
        .map_err(|error| error.to_string())?;
    for entry in entries {
        if entry.is_directory
            || entry
                .path
                .as_path()
                .extension()
                .and_then(|value| value.to_str())
                != Some("md")
        {
            continue;
        }
        let content = match read_workspace_file(files_root, &entry.path) {
            Ok(content) => content,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
        };
        let mut edit = match parse_task_file(&content, entry.path.clone(), None) {
            Ok(edit) => edit,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
        };
        let source = encode_path(entry.path.as_path());
        let existing = db
            .query_row(
                "SELECT id, interval_minutes, next_run_ms FROM scheduled_tasks WHERE source_path = ?1",
                [source],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;
        let next = if let Some((_, interval, next)) = existing
            && interval == edit.interval_minutes
        {
            next
        } else {
            now_ms()
                .checked_add(interval_ms(edit.interval_minutes)?)
                .ok_or_else(|| "schedule is too large".to_owned())?
        };
        edit.source = Some(entry.path);
        save_task(db, existing.map(|(id, _, _)| id), edit, next)?;
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Could not load scheduled task files: {}",
            errors.join("; ")
        ))
    }
}

fn available_task_path(root: &Path, title: &str, id: Option<i64>) -> Result<RepoPath, String> {
    let existing = read_optional_workspace_directory(root, &RepoPath::from("scheduled"))
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|entry| entry.path.file_name().map(|name| name.to_owned()))
        .collect::<HashSet<_>>();
    let slug = task_slug(title);
    let preferred = id.map_or_else(|| slug.clone(), |id| format!("{slug}-{id}"));
    for suffix in 1.. {
        let name = if suffix == 1 {
            format!("{preferred}.md")
        } else {
            format!("{preferred}-{suffix}.md")
        };
        if !existing.contains(std::ffi::OsStr::new(&name)) {
            return Ok(RepoPath::from(format!("scheduled/{name}")));
        }
    }
    unreachable!()
}

fn task_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if slug.is_empty() {
        "task".to_owned()
    } else {
        slug
    }
}

fn parse_task_file(
    content: &[u8],
    source: RepoPath,
    default_destination: Option<ScheduledTaskDestination>,
) -> Result<ScheduledTaskEdit, String> {
    let content = std::str::from_utf8(content).map_err(|_| "task file must be UTF-8".to_owned())?;
    let mut offset = 0;
    let mut lines = content.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| "missing YAML frontmatter".to_owned())?;
    offset += first.len();
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Err("task file must start with YAML frontmatter".to_owned());
    }
    let mut fields = std::collections::HashMap::new();
    let mut closed = false;
    for line in lines {
        offset += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        if line == "---" {
            closed = true;
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("invalid frontmatter line `{trimmed}`"))?;
        let key = key.trim();
        if !matches!(
            key,
            "status"
                | "frequency"
                | "title"
                | "description"
                | "model"
                | "destination"
                | "repository"
                | "branch"
        ) {
            return Err(format!("unknown frontmatter field `{key}`"));
        }
        if fields
            .insert(key, parse_yaml_scalar(value.trim())?)
            .is_some()
        {
            return Err(format!("duplicate frontmatter field `{key}`"));
        }
    }
    if !closed {
        return Err("YAML frontmatter is not closed with `---`".to_owned());
    }
    let destination = match fields.remove("destination") {
        Some(destination) => parse_frontmatter_path(&destination)?,
        None => default_destination
            .as_ref()
            .map(|destination| destination.path.clone())
            .ok_or_else(|| "missing frontmatter field `destination`".to_owned())?,
    };
    let repository = fields
        .remove("repository")
        .or_else(|| {
            default_destination
                .as_ref()
                .map(|destination| destination.repository.clone())
        })
        .ok_or_else(|| "missing frontmatter field `repository`".to_owned())?;
    let branch = fields
        .remove("branch")
        .or_else(|| {
            default_destination
                .as_ref()
                .map(|destination| destination.branch.clone())
        })
        .ok_or_else(|| "missing frontmatter field `branch`".to_owned())?;
    let mut required = |key| {
        fields
            .remove(key)
            .ok_or_else(|| format!("missing frontmatter field `{key}`"))
    };
    let enabled = match required("status")?.as_str() {
        "enabled" => true,
        "disabled" => false,
        _ => return Err("status must be `enabled` or `disabled`".to_owned()),
    };
    let interval_minutes = parse_frequency(&required("frequency")?)?;
    let title = required("title")?;
    let description = required("description")?;
    let model = fields.remove("model").unwrap_or_default();
    let prompt = content[offset..].trim_matches(['\r', '\n']).to_owned();
    let edit = ScheduledTaskEdit {
        title,
        description,
        prompt,
        model,
        destination: validate_destination(&destination)?,
        repository,
        branch,
        enabled,
        interval_minutes,
        source: Some(source),
    };
    validate_task(&edit)?;
    Ok(edit)
}

fn parse_frontmatter_path(value: &str) -> Result<PathBuf, String> {
    if let Some(encoded) = value.strip_prefix("base64:") {
        return STANDARD
            .decode(encoded)
            .map_err(|error| format!("invalid encoded destination: {error}"))
            .and_then(|bytes| decode_path(bytes).map_err(|error| error.to_string()));
    }
    Ok(PathBuf::from(value))
}

fn parse_yaml_scalar(value: &str) -> Result<String, String> {
    if value.starts_with('"') {
        return serde_json::from_str(value)
            .map_err(|error| format!("invalid quoted value: {error}"));
    }
    if let Some(value) = value.strip_prefix('\'') {
        let value = value
            .strip_suffix('\'')
            .ok_or_else(|| "unterminated quoted value".to_owned())?;
        return Ok(value.replace("''", "'"));
    }
    Ok(value.trim().to_owned())
}

fn parse_frequency(value: &str) -> Result<u64, String> {
    let (number, multiplier) = match value.chars().last() {
        Some('m') => (&value[..value.len() - 1], 1),
        Some('h') => (&value[..value.len() - 1], 60),
        Some('d') => (&value[..value.len() - 1], 24 * 60),
        _ => (value, 1),
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .filter(|minutes| *minutes > 0)
        .ok_or_else(|| {
            "frequency must be a positive number of minutes, or use m, h, or d".to_owned()
        })
}

fn render_task_file(task: &ScheduledTaskEdit) -> String {
    let title = serde_json::to_string(&task.title).expect("strings serialize as JSON");
    let description = serde_json::to_string(&task.description).expect("strings serialize as JSON");
    let model = serde_json::to_string(&task.model).expect("strings serialize as JSON");
    let destination = task.destination.to_str().map_or_else(
        || format!("base64:{}", STANDARD.encode(encode_path(&task.destination))),
        str::to_owned,
    );
    let destination = serde_json::to_string(&destination).expect("strings serialize as JSON");
    let repository = serde_json::to_string(&task.repository).expect("strings serialize as JSON");
    let branch = serde_json::to_string(&task.branch).expect("strings serialize as JSON");
    format!(
        "---\nstatus: {}\nfrequency: {}m\ntitle: {}\ndescription: {}\nmodel: {}\ndestination: {}\nrepository: {}\nbranch: {}\n---\n\n{}\n",
        if task.enabled { "enabled" } else { "disabled" },
        task.interval_minutes,
        title,
        description,
        model,
        destination,
        repository,
        branch,
        task.prompt.trim_end()
    )
}

fn validate_task(task: &ScheduledTaskEdit) -> Result<(), String> {
    if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
        return Err(format!(
            "scheduled task {} is required",
            if task.title.trim().is_empty() {
                "title"
            } else {
                "prompt"
            }
        ));
    }
    interval_ms(task.interval_minutes).map(|_| ())
}

impl ScheduledTask {
    pub(crate) fn edit(&self) -> ScheduledTaskEdit {
        ScheduledTaskEdit {
            title: self.title.clone(),
            description: self.description.clone(),
            prompt: self.prompt.clone(),
            model: self.model.clone(),
            destination: self.destination.clone(),
            repository: self.repository.clone(),
            branch: self.branch.clone(),
            enabled: self.enabled,
            interval_minutes: self.interval_minutes,
            source: self.source.clone(),
        }
    }
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
        let task: Option<(i64, String, String, String, Vec<u8>, i64, i64)> = tx
            .query_row(
                "SELECT id, title, prompt, model, destination, interval_minutes, next_run_ms FROM scheduled_tasks WHERE (?1 IS NULL AND enabled = 1 AND next_run_ms <= ?2) OR id = ?1 ORDER BY next_run_ms, id LIMIT 1",
                params![requested, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((task_id, title, prompt, model, path, minutes, next)) = task else {
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
            model: (!model.trim().is_empty()).then_some(model),
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

fn execute_claim(
    db: &Connection,
    claim: Claim,
    launch: &mut impl FnMut(SchedulerLaunchRequest) -> SchedulerLaunchResult,
) -> Result<(), String> {
    if let Err(error) = validate_destination(&claim.destination) {
        return fail_run(db, claim.run_id, &error);
    }
    let result = launch(SchedulerLaunchRequest {
        run_id: claim.run_id,
        destination: claim.destination,
        label: format!("Hunkle: {} #{}", claim.title, claim.task_id),
        prompt: claim.prompt,
        model: claim.model,
    });
    let (status, error) = match result.status {
        Ok(status)
            if result.session_id.is_none()
                && matches!(status, AgentStatus::Idle | AgentStatus::Done) =>
        {
            (
                ScheduledRunStatus::Unknown,
                Some("OpenCode did not report a session after accepting the prompt".to_owned()),
            )
        }
        Ok(status) => (agent_status(status), None),
        Err(error) if result.pane_id.is_some() => (ScheduledRunStatus::Unknown, Some(error)),
        Err(error) => (ScheduledRunStatus::Failed, Some(error)),
    };
    db.execute(
        "UPDATE scheduled_runs SET status = ?2, pane_id = ?3, terminal_id = ?4, session_id = ?5, error = ?6 WHERE id = ?1",
        params![
            claim.run_id,
            status.text(),
            result.pane_id,
            result.terminal_id,
            result.session_id,
            error
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

fn execute_claim_with_update(
    db: &Connection,
    claim: Claim,
    launch: &mut impl FnMut(SchedulerLaunchRequest) -> SchedulerLaunchResult,
    updates: &Sender<Update>,
) -> Result<(), String> {
    // Publish the claimed row before the Herdr launch, which can take several seconds.
    let _ = updates.send(load_state(db));
    execute_claim(db, claim, launch)
}

fn refresh(
    db: &Connection,
    requested: Option<i64>,
    observe: &mut impl FnMut(&str, Option<&str>) -> SchedulerObserveResult,
) -> Result<(), String> {
    let mut statement = db
        .prepare("SELECT id, pane_id, terminal_id, session_id, created_at_ms FROM scheduled_runs WHERE (?1 IS NULL AND status IN ('working', 'blocked', 'unknown') AND pane_id IS NOT NULL) OR id = ?1")
        .map_err(db_error)?;
    let runs = statement
        .query_map([requested], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    if requested.is_some() && runs.is_empty() {
        return Err("scheduled run not found".to_owned());
    }
    for (id, pane, terminal, session, created_at_ms) in runs {
        let pane = pane.ok_or_else(|| "scheduled run has no Herdr pane".to_owned())?;
        let observation = observe(&pane, terminal.as_deref());
        if session.is_none()
            && matches!(
                &observation,
                SchedulerObserveResult::Observed(AgentStatus::Idle | AgentStatus::Done)
            )
        {
            if now_ms().saturating_sub(created_at_ms) >= 30_000 {
                fail_run(
                    db,
                    id,
                    "OpenCode did not report a session within 30 seconds of accepting the prompt",
                )?;
            } else {
                apply_observation(
                    db,
                    id,
                    SchedulerObserveResult::Unavailable(
                        "Waiting for OpenCode to report this run's session".to_owned(),
                    ),
                )?;
            }
        } else {
            apply_observation(db, id, observation)?;
        }
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
            (ScheduledRunStatus::Unknown, None::<String>, Some(error))
        }
        SchedulerObserveResult::Observed(status) => (agent_status(status), None, None),
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
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let version = tx
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(db_error)?;
    let sql = match version {
        0 => "CREATE TABLE scheduled_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, description TEXT NOT NULL, prompt TEXT NOT NULL, model TEXT NOT NULL DEFAULT '', destination BLOB NOT NULL, repository TEXT NOT NULL, branch TEXT NOT NULL, enabled INTEGER NOT NULL, interval_minutes INTEGER NOT NULL CHECK (interval_minutes > 0), next_run_ms INTEGER NOT NULL, source_path BLOB);
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, terminal_id TEXT, session_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, UNIQUE (task_id, scheduled_for_ms));
              CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id); CREATE UNIQUE INDEX scheduled_tasks_source ON scheduled_tasks(destination, source_path) WHERE source_path IS NOT NULL;",
        1 => "ALTER TABLE scheduled_runs RENAME TO scheduled_runs_v1;
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, terminal_id TEXT, session_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, UNIQUE (task_id, scheduled_for_ms));
              INSERT INTO scheduled_runs (id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, terminal_id, output, error) SELECT id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, terminal_id, output, error FROM scheduled_runs_v1 WHERE task_id IS NOT NULL;
              DROP TABLE scheduled_runs_v1; CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id);",
        2 => "ALTER TABLE scheduled_runs ADD COLUMN terminal_id TEXT; ALTER TABLE scheduled_runs ADD COLUMN session_id TEXT;",
        3 => "ALTER TABLE scheduled_runs ADD COLUMN session_id TEXT;",
        4 => "ALTER TABLE scheduled_tasks ADD COLUMN source_path BLOB; CREATE UNIQUE INDEX scheduled_tasks_source ON scheduled_tasks(destination, source_path) WHERE source_path IS NOT NULL;",
        5 => "ALTER TABLE scheduled_tasks ADD COLUMN model TEXT NOT NULL DEFAULT '';",
        6 => "",
        _ => return Err(format!("scheduler database version {version} is newer than supported")),
    };
    tx.execute_batch(sql).map_err(db_error)?;
    if version > 0 && version < 4 {
        let has_tasks = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'scheduled_tasks')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(db_error)?;
        if has_tasks {
            tx.execute_batch("ALTER TABLE scheduled_tasks ADD COLUMN source_path BLOB; CREATE UNIQUE INDEX scheduled_tasks_source ON scheduled_tasks(destination, source_path) WHERE source_path IS NOT NULL;")
                .map_err(db_error)?;
        }
    }
    if version > 0 && version < 5 {
        let has_tasks = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'scheduled_tasks')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(db_error)?;
        if has_tasks {
            tx.execute_batch(
                "ALTER TABLE scheduled_tasks ADD COLUMN model TEXT NOT NULL DEFAULT '';",
            )
            .map_err(db_error)?;
        }
    }
    if version < 6 {
        tx.execute_batch("PRAGMA user_version = 6;")
            .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)
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
    let tasks = query_all(
        db,
        "SELECT id, title, description, prompt, model, destination, repository, branch, enabled, interval_minutes, next_run_ms, source_path FROM scheduled_tasks ORDER BY id",
        |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                prompt: row.get(3)?,
                model: row.get(4)?,
                destination: decode_path(row.get(5)?)?,
                repository: row.get(6)?,
                branch: row.get(7)?,
                enabled: row.get(8)?,
                interval_minutes: row.get(9)?,
                next_run_ms: row.get(10)?,
                source: row
                    .get::<_, Option<Vec<u8>>>(11)?
                    .map(decode_path)
                    .transpose()?
                    .map(RepoPath::from),
            })
        },
    )?;
    let runs = query_all(
        db,
        "SELECT id, task_id, created_at_ms, status, pane_id, terminal_id, session_id, error FROM scheduled_runs ORDER BY created_at_ms DESC, id DESC",
        |row| {
            Ok(ScheduledRun {
                id: row.get(0)?,
                task_id: row.get(1)?,
                created_at_ms: row.get(2)?,
                status: ScheduledRunStatus::parse(&row.get::<_, String>(3)?)?,
                pane_id: row.get(4)?,
                terminal_id: row.get(5)?,
                session_id: row.get(6)?,
                error: row.get(7)?,
            })
        },
    )?;
    Ok((tasks, runs))
}

fn query_all<T>(
    db: &Connection,
    sql: &str,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String> {
    let mut statement = db.prepare(sql).map_err(db_error)?;
    statement
        .query_map([], map)
        .map_err(db_error)?
        .collect::<Result<_, _>>()
        .map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(path: &Path) -> ScheduledTaskDestination {
        ScheduledTaskDestination {
            path: path.to_owned(),
            repository: "repo".to_owned(),
            branch: "main".to_owned(),
        }
    }

    #[test]
    fn markdown_task_frontmatter_round_trips_supported_yaml_scalars() {
        let directory = tempfile::tempdir().unwrap();
        let source = RepoPath::from("scheduled/nightly.md");
        let content = br#"---
status: disabled
frequency: 2h
title: 'Nightly: review'
description: "Review \"open\" changes"
---

Inspect the diff.
Summarize risks.
"#;

        let mut task =
            parse_task_file(content, source, Some(destination(directory.path()))).unwrap();

        assert_eq!(task.title, "Nightly: review");
        assert_eq!(task.description, "Review \"open\" changes");
        assert_eq!(task.prompt, "Inspect the diff.\nSummarize risks.");
        assert_eq!(task.interval_minutes, 120);
        assert_eq!(task.model, "");
        assert!(!task.enabled);
        task.model = "opencode-go/deepseek-flash-v4".to_owned();
        let reparsed = parse_task_file(
            render_task_file(&task).as_bytes(),
            task.source.clone().unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(reparsed, task);
    }

    #[test]
    fn migrates_v5_tasks_with_an_empty_model() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE scheduled_tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL, prompt TEXT NOT NULL, destination BLOB NOT NULL, repository TEXT NOT NULL, branch TEXT NOT NULL, enabled INTEGER NOT NULL, interval_minutes INTEGER NOT NULL, next_run_ms INTEGER NOT NULL, source_path BLOB); PRAGMA user_version = 5;",
        )
        .unwrap();

        prepare_database(&mut db).unwrap();

        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert!(db.prepare("SELECT model FROM scheduled_tasks").is_ok());
    }

    #[test]
    fn sync_imports_markdown_without_postponing_unchanged_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let scheduled = directory.path().join(".hunkle/scheduled");
        std::fs::create_dir_all(&scheduled).unwrap();
        std::fs::write(
            scheduled.join("review.md"),
            "---\nstatus: enabled\nfrequency: 1d\ntitle: Review\ndescription: Check changes\n---\n\nReview the repository.\n",
        )
        .unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();

        sync_task_files(&db, files.path(), &[destination(directory.path())]).unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.interval_minutes, 24 * 60);
        assert_eq!(task.source, Some(RepoPath::from("scheduled/review.md")));
        assert!(!scheduled.join("review.md").exists());
        db.execute(
            "UPDATE scheduled_tasks SET next_run_ms = 123456 WHERE id = ?1",
            [task.id],
        )
        .unwrap();

        sync_task_files(&db, files.path(), &[destination(directory.path())]).unwrap();

        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.next_run_ms, 123456);

        std::fs::remove_file(files.path().join("scheduled/review.md")).unwrap();
        sync_task_files(&db, files.path(), &[destination(directory.path())]).unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert!(!task.enabled);
    }

    #[test]
    fn sync_migrates_database_tasks_to_hunkle_data() {
        let directory = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();
        save_task(
            &db,
            None,
            ScheduledTaskEdit {
                title: "Legacy Review".to_owned(),
                description: "Check changes".to_owned(),
                prompt: "Review the repository.".to_owned(),
                model: String::new(),
                destination: directory.path().to_owned(),
                repository: "repo".to_owned(),
                branch: "main".to_owned(),
                enabled: true,
                interval_minutes: 60,
                source: None,
            },
            123456,
        )
        .unwrap();

        sync_task_files(&db, files.path(), &[destination(directory.path())]).unwrap();

        let task = load_state(&db).unwrap().0.pop().unwrap();
        let source = task.source.unwrap();
        assert_eq!(source, RepoPath::from("scheduled/legacy-review-1.md"));
        let content = std::fs::read_to_string(files.path().join(source.as_path())).unwrap();
        assert!(content.contains("status: enabled\nfrequency: 60m"));
        assert!(content.ends_with("Review the repository.\n"));
    }

    #[test]
    fn editing_changes_destination_without_moving_the_hunkle_owned_file() {
        let directory = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let source_root = directory.path().join("source");
        let destination_root = directory.path().join("destination");
        std::fs::create_dir_all(source_root.join(".hunkle/scheduled")).unwrap();
        std::fs::create_dir(&destination_root).unwrap();
        let source_path = RepoPath::from(".hunkle/scheduled/review.md");
        std::fs::write(
            source_root.join(source_path.as_path()),
            "---\nstatus: enabled\nfrequency: 1h\ntitle: Review\ndescription: Check changes\n---\n\nReview the repository.\n",
        )
        .unwrap();
        let mut scheduler = SchedulerService::open(None, Some(files.path().to_owned())).unwrap();
        scheduler
            .sync_task_files(vec![
                ScheduledTaskDestination {
                    path: source_root.clone(),
                    repository: "source".to_owned(),
                    branch: "main".to_owned(),
                },
                ScheduledTaskDestination {
                    path: destination_root.clone(),
                    repository: "destination".to_owned(),
                    branch: "feature".to_owned(),
                },
            ])
            .unwrap();
        for _ in 0..100 {
            scheduler.poll_completions();
            if !scheduler.tasks.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let task = scheduler.tasks.first().unwrap().clone();
        let source = scheduler.task_source(&task).unwrap().unwrap();
        let mut edit = task.edit();
        edit.destination = destination_root.clone();
        edit.repository = "destination".to_owned();
        edit.branch = "feature".to_owned();

        scheduler
            .save_task(Some(task.id), edit, Some(source))
            .unwrap();

        assert!(!source_root.join(source_path.as_path()).exists());
        let owned_path = files.path().join("scheduled/review.md");
        assert!(owned_path.exists());
        assert!(
            !destination_root
                .join(".hunkle/scheduled/review.md")
                .exists()
        );
        for _ in 0..100 {
            scheduler.poll_completions();
            if scheduler
                .tasks
                .iter()
                .any(|moved| moved.id == task.id && moved.destination == destination_root)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let moved = scheduler
            .tasks
            .iter()
            .find(|moved| moved.id == task.id)
            .unwrap();
        assert_eq!(moved.destination, destination_root);
        assert_eq!(moved.source, Some(RepoPath::from("scheduled/review.md")));
        assert_eq!(moved.repository, "destination");
        assert_eq!(moved.branch, "feature");
        let content = std::fs::read_to_string(owned_path).unwrap();
        assert!(content.contains(&format!(
            "destination: {}",
            serde_json::to_string(destination_root.to_str().unwrap()).unwrap()
        )));
    }

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
        let (updates, published) = mpsc::channel();
        execute_claim_with_update(
            &first,
            run,
            &mut |_| SchedulerLaunchResult {
                pane_id: Some("w1:p1".into()),
                terminal_id: Some("term-1".into()),
                session_id: Some("ses-1".into()),
                status: Ok(AgentStatus::Working),
            },
            &updates,
        )
        .unwrap();
        let launching = published.recv().unwrap().unwrap();
        assert_eq!(launching.1[0].status, ScheduledRunStatus::Launching);
        first.execute("INSERT INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms) VALUES (1, 200, 'launching', 200)", []).unwrap();
        let missing_session_run_id = first.last_insert_rowid();
        execute_claim(
            &first,
            Claim {
                run_id: missing_session_run_id,
                task_id: 1,
                title: "Review".to_owned(),
                prompt: "Review it".to_owned(),
                model: None,
                destination: directory.path().to_owned(),
            },
            &mut |_| SchedulerLaunchResult {
                pane_id: Some("w1:p2".into()),
                terminal_id: Some("term-2".into()),
                session_id: None,
                status: Ok(AgentStatus::Done),
            },
        )
        .unwrap();
        let missing_session = load_state(&first)
            .unwrap()
            .1
            .into_iter()
            .find(|run| run.id == missing_session_run_id)
            .unwrap();
        assert_eq!(missing_session.status, ScheduledRunStatus::Unknown);
        assert!(
            missing_session
                .error
                .unwrap()
                .contains("did not report a session")
        );
        refresh(&first, None, &mut |_, _| {
            SchedulerObserveResult::Observed(AgentStatus::Done)
        })
        .unwrap();
        let missing_session = load_state(&first)
            .unwrap()
            .1
            .into_iter()
            .find(|run| run.id == missing_session_run_id)
            .unwrap();
        assert_eq!(missing_session.status, ScheduledRunStatus::Failed);
        assert!(
            missing_session
                .error
                .unwrap()
                .contains("did not report a session within 30 seconds")
        );
        first.execute("INSERT OR REPLACE INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms) VALUES (1, 99, 'launching', 1)", []).unwrap();
        recover_stale_launches(&first, 500_000).unwrap();
        let state = load_state(&first).unwrap();
        assert_eq!(state.0[0].next_run_ms, 300_100);
        assert!(state.1.iter().any(|run| {
            run.terminal_id.as_deref() == Some("term-1")
                && run.session_id.as_deref() == Some("ses-1")
        }));
        let failed = state
            .1
            .iter()
            .any(|run| run.status == ScheduledRunStatus::Failed);
        assert!(failed);
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

    #[test]
    fn migrates_v2_run_identities() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY, pane_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT); PRAGMA user_version = 2;",
        )
        .unwrap();

        prepare_database(&mut db).unwrap();

        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert!(db.prepare("SELECT terminal_id FROM scheduled_runs").is_ok());
        assert!(db.prepare("SELECT session_id FROM scheduled_runs").is_ok());
    }

    #[test]
    fn migrates_v3_session_identity() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY, pane_id TEXT, terminal_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT); PRAGMA user_version = 3;",
        )
        .unwrap();

        prepare_database(&mut db).unwrap();

        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert!(db.prepare("SELECT session_id FROM scheduled_runs").is_ok());
    }
}
