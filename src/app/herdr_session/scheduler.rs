use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{
    app::valid_discord_webhook_url,
    filesystem::{read_optional_workspace_directory, read_workspace_file},
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
const DISCORD_DELIVERY_ERROR: &str = "Discord delivery failed: ";
const DISCORD_MESSAGE_BYTES: usize = 1_900;
const RESULT_FETCH_ATTEMPTS: usize = 20;
const RESULT_FETCH_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTask {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) discord_webhook_id: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
    pub(crate) next_run_ms: i64,
    pub(crate) source: Option<RepoPath>,
    pub(crate) project_status: Option<ProjectTaskStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledTaskEdit {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) discord_webhook_id: String,
    pub(crate) destination: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) enabled: bool,
    pub(crate) interval_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectTaskStatus {
    Pending,
    Current,
    Changed,
    Missing,
}

impl ProjectTaskStatus {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Pending => "approval required",
            Self::Current => "current",
            Self::Changed => "changed; approval required",
            Self::Missing => "source missing",
        }
    }
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
    notices: Receiver<String>,
    worker: Option<JoinHandle<()>>,
}

impl SchedulerService {
    pub(crate) fn open(
        path: Option<PathBuf>,
        files_root: Option<PathBuf>,
        discord_webhooks: Vec<crate::app::DiscordWebhookConfig>,
    ) -> Result<Self, String> {
        let enabled = path.is_some();
        let discord_webhooks = discord_webhooks_by_id(discord_webhooks);
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
        let import_notice = files_root
            .as_deref()
            .and_then(|root| import_legacy_task_files(&db, root).err());
        recover_stale_launches(&db, now_ms())?;
        let (tasks, runs) = load_state(&db)?;
        let (commands, command_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let (notice_tx, notices) = mpsc::channel();
        if let Some(notice) = import_notice {
            let _ = notice_tx.send(notice);
        }
        let worker = thread::Builder::new()
            .name("hunkle-scheduler".to_owned())
            .spawn(move || {
                worker(
                    db,
                    command_rx,
                    update_tx,
                    enabled,
                    discord_webhooks,
                    notice_tx,
                )
            })
            .map_err(|error| format!("Could not start scheduler worker: {error}"))?;
        Ok(Self {
            tasks,
            runs,
            commands,
            updates,
            notices,
            worker: Some(worker),
        })
    }

    pub(crate) fn save_task(
        &self,
        id: Option<i64>,
        mut task: ScheduledTaskEdit,
    ) -> Result<(), String> {
        let interval = interval_ms(task.interval_minutes)?;
        task.destination = validate_destination(&task.destination)?;
        task.title = task.title.trim().to_owned();
        validate_task(&task)?;
        let next = now_ms()
            .checked_add(interval)
            .ok_or_else(|| "schedule is too large".to_owned())?;
        self.send(Command::Save(id, task, next))
    }

    pub(crate) fn discover_project_tasks(
        &self,
        destination: ScheduledTaskDestination,
        repository_identity: PathBuf,
    ) -> Result<(), String> {
        self.send(Command::Discover(
            destination,
            encode_path(&repository_identity),
        ))
    }

    pub(crate) fn toggle_task(&self, id: i64, enabled: bool) -> Result<(), String> {
        self.send(Command::Toggle(id, enabled))
    }
    pub(crate) fn delete_task(&self, id: i64) -> Result<(), String> {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return Err("scheduled task not found".to_owned());
        };
        if task.project_status.is_some() {
            return Err("remove the project task's Markdown file from the repository".to_owned());
        }
        self.send(Command::Delete(id))
    }

    pub(crate) fn configure_project_task(
        &self,
        id: i64,
        discord_webhook_id: String,
    ) -> Result<(), String> {
        self.send(Command::ConfigureProject(id, discord_webhook_id))
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

    pub(crate) fn configure_discord_webhooks(
        &self,
        webhooks: Vec<crate::app::DiscordWebhookConfig>,
    ) -> Result<(), String> {
        self.send(Command::ConfigureDiscord(webhooks))
    }

    pub(crate) fn test_discord_webhook(&self, channel: String) -> Result<(), String> {
        self.send(Command::TestDiscord(channel))
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
        while let Ok(notice) = self.notices.try_recv() {
            error = Some(notice);
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
    Discover(ScheduledTaskDestination, Vec<u8>),
    Toggle(i64, bool),
    ConfigureProject(i64, String),
    Delete(i64),
    RunNow(i64),
    Refresh(i64),
    BindAgent(i64, String, String),
    BindSession(i64, String),
    ConfigureDiscord(Vec<crate::app::DiscordWebhookConfig>),
    TestDiscord(String),
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

struct DiscordWebhook {
    url: String,
    agent: ureq::Agent,
}

impl DiscordWebhook {
    fn new(url: String) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build();
        Self {
            url: url.trim().to_owned(),
            agent: config.into(),
        }
    }

    fn publish(&self, title: &str, output: &str) -> Result<(), String> {
        if !valid_discord_webhook_url(&self.url) {
            return Err("Discord webhook URL is invalid".to_owned());
        }
        self.agent
            .post(&self.url)
            .send_json(serde_json::json!({
                "content": discord_message(title, output),
                "allowed_mentions": { "parse": [] },
            }))
            .map(|_| ())
            .map_err(|_| "Discord webhook request failed".to_owned())
    }
}

fn worker(
    mut db: Connection,
    commands: Receiver<Command>,
    updates: Sender<Update>,
    enabled: bool,
    mut discord_webhooks: HashMap<String, DiscordWebhook>,
    notices: Sender<String>,
) {
    loop {
        match commands.recv_timeout(Duration::from_secs(if enabled { 2 } else { 30 })) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                let result = match command {
                    Command::Save(id, task, next) => save_task(&db, id, task, next),
                    Command::Discover(destination, repository_identity) => {
                        match discover_project_tasks(&mut db, &destination, &repository_identity) {
                            Ok(()) => Ok(()),
                            Err(error) => {
                                let _ = notices.send(error);
                                Ok(())
                            }
                        }
                    }
                    Command::Toggle(id, enabled) => toggle_task(&db, id, enabled),
                    Command::ConfigureProject(id, discord_webhook_id) => changed(
                        db.execute(
                            "UPDATE scheduled_tasks SET discord_webhook_id = ?2 WHERE id = ?1 AND source_kind = 'project'",
                            params![id, discord_webhook_id],
                        ),
                        "project task not found",
                    ),
                    Command::Delete(id) => delete_task(&db, id),
                    Command::RunNow(id) if enabled => claim(&mut db, Some(id), now_ms())
                        .and_then(|run| run.ok_or_else(|| "scheduled task not found".to_owned()))
                        .and_then(|run| {
                            execute_claim_with_update(
                                &db,
                                run,
                                &mut scheduler_launch,
                                &mut |db, run_id| {
                                    complete_run(db, run_id, &discord_webhooks)
                                },
                                &updates,
                            )
                        }),
                    Command::RunNow(_) => {
                        Err("scheduler execution is disabled for an in-memory service".to_owned())
                    }
                    Command::Refresh(id) => match retry_delivery(&db, id, &mut |db, run_id| {
                        complete_run(db, run_id, &discord_webhooks)
                    }) {
                        Ok(true) => Ok(()),
                        Ok(false) => refresh(
                            &db,
                            Some(id),
                            &mut scheduler_observe,
                            &mut |db, run_id| {
                                complete_run(db, run_id, &discord_webhooks)
                            },
                        ),
                        Err(error) => Err(error),
                    },
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
                    Command::ConfigureDiscord(webhooks) => {
                        discord_webhooks = discord_webhooks_by_id(webhooks);
                        Ok(())
                    }
                    Command::TestDiscord(channel) => {
                        let notice = match discord_webhooks.get(&channel) {
                            Some(webhook) => match webhook.publish(
                                "Hunkle Discord integration",
                                "Test message delivered successfully.",
                            ) {
                                Ok(()) => "Discord test message sent".to_owned(),
                                Err(error) => format!("Discord test failed: {error}"),
                            },
                            None => "Discord webhook is not configured".to_owned(),
                        };
                        let _ = notices.send(notice);
                        Ok(())
                    }
                    Command::Shutdown => unreachable!(),
                };
                publish(&db, &updates, result);
            }
            Err(RecvTimeoutError::Timeout) => {
                let result = if enabled {
                    (|| {
                        while let Some(run) = claim(&mut db, None, now_ms())? {
                            execute_claim_with_update(
                                &db,
                                run,
                                &mut scheduler_launch,
                                &mut |db, run_id| complete_run(db, run_id, &discord_webhooks),
                                &updates,
                            )?;
                        }
                        refresh(&db, None, &mut scheduler_observe, &mut |db, run_id| {
                            complete_run(db, run_id, &discord_webhooks)
                        })
                    })()
                } else {
                    Ok(())
                };
                publish(&db, &updates, result);
            }
        }
    }
}

fn complete_run(
    db: &Connection,
    run_id: i64,
    discord_webhooks: &HashMap<String, DiscordWebhook>,
) -> Result<(), String> {
    let channel = db
        .query_row(
            "SELECT discord_webhook_id FROM scheduled_runs WHERE id = ?1",
            [run_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(db_error)?;
    if channel.is_empty() {
        return Ok(());
    }
    let webhook = discord_webhooks.get(&channel);
    finalize_completed_run(
        db,
        run_id,
        &mut fetch_scheduled_result,
        &mut |title, output| match webhook {
            Some(webhook) => webhook.publish(title, output),
            None => Err("Selected Discord webhook is not configured".to_owned()),
        },
    )
}

fn discord_webhooks_by_id(
    webhooks: Vec<crate::app::DiscordWebhookConfig>,
) -> HashMap<String, DiscordWebhook> {
    webhooks
        .into_iter()
        .map(|webhook| (webhook.id, DiscordWebhook::new(webhook.url)))
        .collect()
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
    let minutes = i64::try_from(task.interval_minutes).map_err(|_| "schedule is too large")?;
    if let Some(id) = id {
        return changed(
            db.execute(
                "UPDATE scheduled_tasks SET title = ?2, description = ?3, prompt = ?4, model = ?5, discord_webhook_id = ?6, destination = ?7, repository = ?8, branch = ?9, enabled = ?10, interval_minutes = ?11, next_run_ms = ?12 WHERE id = ?1 AND source_kind = 'local'",
                params![id, task.title, task.description, task.prompt, task.model, task.discord_webhook_id, encode_path(&task.destination), task.repository, task.branch, task.enabled, minutes, next],
            ),
            "local scheduled task not found",
        );
    }
    db.execute(
        "INSERT INTO scheduled_tasks (title, description, prompt, model, discord_webhook_id, destination, repository, branch, enabled, interval_minutes, next_run_ms, source_kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'local')",
        params![task.title, task.description, task.prompt, task.model, task.discord_webhook_id, encode_path(&task.destination), task.repository, task.branch, task.enabled, minutes, next],
    )
    .map_err(db_error)?;
    Ok(())
}

struct ProjectTaskDefinition {
    key: String,
    source: RepoPath,
    content: Vec<u8>,
    edit: ScheduledTaskEdit,
}

fn import_legacy_task_files(db: &Connection, files_root: &Path) -> Result<(), String> {
    let imported = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduler_metadata WHERE key = 'legacy_task_files_imported')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(db_error)?;
    if imported {
        return Ok(());
    }
    let mut errors = Vec::new();
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
        let edit = match parse_task_file(&content, None) {
            Ok(edit) => edit,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
        };
        let source = encode_path(entry.path.as_path());
        let destination = encode_path(&edit.destination);
        let existing = db
            .query_row(
                "SELECT id, interval_minutes, next_run_ms FROM scheduled_tasks WHERE source_path = ?1 OR (title = ?2 AND destination = ?3 AND prompt = ?4) LIMIT 1",
                params![source, edit.title, destination, edit.prompt],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        let next = existing
            .filter(|(_, interval, _)| *interval == edit.interval_minutes)
            .map(|(_, _, next)| next)
            .unwrap_or_else(|| {
                now_ms().saturating_add(interval_ms(edit.interval_minutes).unwrap_or(i64::MAX))
            });
        save_task(db, existing.map(|(id, _, _)| id), edit, next)?;
    }
    if !errors.is_empty() {
        return Err(format!(
            "Could not import existing scheduled tasks: {}",
            errors.join("; ")
        ));
    }
    db.execute(
        "UPDATE scheduled_tasks SET source_path = NULL, source_kind = 'local' WHERE source_kind = 'local'",
        [],
    )
    .map_err(db_error)?;
    db.execute(
        "INSERT OR REPLACE INTO scheduler_metadata (key, value) VALUES ('legacy_task_files_imported', '1')",
        [],
    )
    .map_err(db_error)?;
    Ok(())
}

fn discover_project_tasks(
    db: &mut Connection,
    destination: &ScheduledTaskDestination,
    repository_identity: &[u8],
) -> Result<(), String> {
    let destination = ScheduledTaskDestination {
        path: validate_destination(&destination.path)?,
        repository: destination.repository.clone(),
        branch: destination.branch.clone(),
    };
    let mut errors = Vec::new();
    if let Err(error) = import_legacy_repository_task_files(db, &destination, repository_identity) {
        errors.push(error);
    }
    let directory = RepoPath::from(".agents/scheduled");
    let entries = read_optional_workspace_directory(&destination.path, &directory)
        .map_err(|error| error.to_string())?;
    let mut definitions = Vec::new();
    let mut keys = HashSet::new();
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
        match parse_project_task(&content, entry.path.clone(), &destination) {
            Ok(definition) if keys.insert(definition.key.clone()) => definitions.push(definition),
            Ok(definition) => errors.push(format!(
                "{}: duplicate project task id `{}`",
                entry.path.display(),
                definition.key
            )),
            Err(error) => errors.push(format!("{}: {error}", entry.path.display())),
        }
    }

    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let destination_path = encode_path(&destination.path);
    let known = {
        let mut statement = tx
            .prepare(
                "SELECT id, project_key FROM scheduled_tasks WHERE source_kind = 'project' AND project_repository = ?1 AND destination = ?2",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![repository_identity, destination_path], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
    };
    for definition in definitions {
        let existing = tx
            .query_row(
                "SELECT id, interval_minutes, next_run_ms, approved_content, destination FROM scheduled_tasks WHERE source_kind = 'project' AND project_repository = ?1 AND project_key = ?2",
                params![repository_identity, definition.key],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?, row.get::<_, i64>(2)?, row.get::<_, Option<Vec<u8>>>(3)?, row.get::<_, Vec<u8>>(4)?)),
            )
            .optional()
            .map_err(db_error)?;
        let next = existing
            .as_ref()
            .filter(|(_, interval, _, _, path)| {
                *path == destination_path && *interval == definition.edit.interval_minutes
            })
            .map(|(_, _, next, _, _)| *next)
            .unwrap_or_else(|| {
                now_ms().saturating_add(
                    interval_ms(definition.edit.interval_minutes).unwrap_or(i64::MAX),
                )
            });
        if let Some((id, _, _, approved, path)) = existing {
            if path != destination_path {
                continue;
            }
            let enabled = approved.as_deref() == Some(definition.content.as_slice());
            tx.execute(
                "UPDATE scheduled_tasks SET title = ?2, description = ?3, prompt = ?4, model = ?5, destination = ?6, repository = ?7, branch = ?8, enabled = CASE WHEN ?9 THEN enabled ELSE 0 END, interval_minutes = ?10, next_run_ms = ?11, source_path = ?12, source_content = ?13, source_missing = 0 WHERE id = ?1",
                params![id, definition.edit.title, definition.edit.description, definition.edit.prompt, definition.edit.model, destination_path, destination.repository, destination.branch, enabled, definition.edit.interval_minutes, next, encode_path(definition.source.as_path()), definition.content],
            ).map_err(db_error)?;
        } else {
            tx.execute(
                "INSERT INTO scheduled_tasks (title, description, prompt, model, discord_webhook_id, destination, repository, branch, enabled, interval_minutes, next_run_ms, source_path, source_kind, project_key, source_content, source_missing, project_repository) VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, 0, ?8, ?9, ?10, 'project', ?11, ?12, 0, ?13)",
                params![definition.edit.title, definition.edit.description, definition.edit.prompt, definition.edit.model, destination_path, destination.repository, destination.branch, definition.edit.interval_minutes, next, encode_path(definition.source.as_path()), definition.key, definition.content, repository_identity],
            ).map_err(db_error)?;
        }
    }
    for (id, key) in known {
        if !keys.contains(&key) {
            tx.execute(
                "UPDATE scheduled_tasks SET source_missing = 1, enabled = 0 WHERE id = ?1",
                [id],
            )
            .map_err(db_error)?;
        }
    }
    tx.commit().map_err(db_error)?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Could not load project tasks: {}",
            errors.join("; ")
        ))
    }
}

fn import_legacy_repository_task_files(
    db: &Connection,
    destination: &ScheduledTaskDestination,
    repository_identity: &[u8],
) -> Result<(), String> {
    let identity = repository_identity
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let metadata_key = format!("legacy_repository_task_files_imported:{identity}");
    let imported = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduler_metadata WHERE key = ?1)",
            [&metadata_key],
            |row| row.get::<_, bool>(0),
        )
        .map_err(db_error)?;
    if imported {
        return Ok(());
    }
    let entries =
        read_optional_workspace_directory(&destination.path, &RepoPath::from(".hunkle/scheduled"))
            .map_err(|error| error.to_string())?;
    let mut found = false;
    let mut errors = Vec::new();
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
        found = true;
        let content = match read_workspace_file(&destination.path, &entry.path) {
            Ok(content) => content,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
        };
        let edit = match parse_task_file(&content, Some(destination.clone())) {
            Ok(edit) => edit,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
        };
        let destination_path = encode_path(&edit.destination);
        let existing = db
            .query_row(
                "SELECT id, interval_minutes, next_run_ms FROM scheduled_tasks WHERE source_kind = 'local' AND title = ?1 AND destination = ?2 AND prompt = ?3 LIMIT 1",
                params![edit.title, destination_path, edit.prompt],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        let next = existing
            .filter(|(_, interval, _)| *interval == edit.interval_minutes)
            .map(|(_, _, next)| next)
            .unwrap_or_else(|| {
                now_ms().saturating_add(interval_ms(edit.interval_minutes).unwrap_or(i64::MAX))
            });
        save_task(db, existing.map(|(id, _, _)| id), edit, next)?;
    }
    if !errors.is_empty() {
        return Err(format!(
            "Could not import existing repository tasks: {}",
            errors.join("; ")
        ));
    }
    if found {
        db.execute(
            "INSERT INTO scheduler_metadata (key, value) VALUES (?1, '1')",
            [&metadata_key],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

fn parse_project_task(
    content: &[u8],
    source: RepoPath,
    destination: &ScheduledTaskDestination,
) -> Result<ProjectTaskDefinition, String> {
    let text = std::str::from_utf8(content).map_err(|_| "task file must be UTF-8".to_owned())?;
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| "missing YAML frontmatter".to_owned())?;
    offset += first.len();
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Err("task file must start with YAML frontmatter".to_owned());
    }
    let mut fields = HashMap::new();
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
        if !matches!(key, "id" | "frequency" | "title" | "description" | "model") {
            return Err(format!("unknown project task field `{key}`"));
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
    let fallback_key = source
        .as_path()
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "project task filename must be UTF-8".to_owned())?;
    let key = fields
        .remove("id")
        .unwrap_or_else(|| fallback_key.to_owned());
    if key.is_empty()
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(
            "project task id may contain only letters, numbers, `-`, `_`, and `.`".to_owned(),
        );
    }
    let title = fields
        .remove("title")
        .ok_or_else(|| "missing frontmatter field `title`".to_owned())?;
    let interval_minutes = parse_frequency(
        &fields
            .remove("frequency")
            .ok_or_else(|| "missing frontmatter field `frequency`".to_owned())?,
    )?;
    let edit = ScheduledTaskEdit {
        title,
        description: fields.remove("description").unwrap_or_default(),
        prompt: text[offset..].trim_matches(['\r', '\n']).to_owned(),
        model: fields.remove("model").unwrap_or_default(),
        discord_webhook_id: String::new(),
        destination: destination.path.clone(),
        repository: destination.repository.clone(),
        branch: destination.branch.clone(),
        enabled: false,
        interval_minutes,
    };
    validate_task(&edit)?;
    Ok(ProjectTaskDefinition {
        key,
        source,
        content: content.to_vec(),
        edit,
    })
}

fn parse_task_file(
    content: &[u8],
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
                | "discord_webhook"
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
    let discord_webhook_id = fields.remove("discord_webhook").unwrap_or_default();
    let prompt = content[offset..].trim_matches(['\r', '\n']).to_owned();
    let edit = ScheduledTaskEdit {
        title,
        description,
        prompt,
        model,
        discord_webhook_id,
        destination: validate_destination(&destination)?,
        repository,
        branch,
        enabled,
        interval_minutes,
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

#[cfg(test)]
fn render_task_file(task: &ScheduledTaskEdit) -> String {
    let title = serde_json::to_string(&task.title).expect("strings serialize as JSON");
    let description = serde_json::to_string(&task.description).expect("strings serialize as JSON");
    let model = serde_json::to_string(&task.model).expect("strings serialize as JSON");
    let discord_webhook =
        serde_json::to_string(&task.discord_webhook_id).expect("strings serialize as JSON");
    let destination = task.destination.to_str().map_or_else(
        || format!("base64:{}", STANDARD.encode(encode_path(&task.destination))),
        str::to_owned,
    );
    let destination = serde_json::to_string(&destination).expect("strings serialize as JSON");
    let repository = serde_json::to_string(&task.repository).expect("strings serialize as JSON");
    let branch = serde_json::to_string(&task.branch).expect("strings serialize as JSON");
    format!(
        "---\nstatus: {}\nfrequency: {}m\ntitle: {}\ndescription: {}\nmodel: {}\ndiscord_webhook: {}\ndestination: {}\nrepository: {}\nbranch: {}\n---\n\n{}\n",
        if task.enabled { "enabled" } else { "disabled" },
        task.interval_minutes,
        title,
        description,
        model,
        discord_webhook,
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

fn toggle_task(db: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let task = db
        .query_row(
            "SELECT source_kind, destination, repository, branch, source_path, project_key FROM scheduled_tasks WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "scheduled task not found".to_owned())?;
    if task.0 == "local" || !enabled {
        return changed(
            db.execute(
                "UPDATE scheduled_tasks SET enabled = ?2 WHERE id = ?1",
                params![id, enabled],
            ),
            "scheduled task not found",
        );
    }
    let destination = decode_path(task.1).map_err(db_error)?;
    let source = task
        .4
        .map(decode_path)
        .transpose()
        .map_err(db_error)?
        .map(RepoPath::from)
        .ok_or_else(|| "project task source is unavailable".to_owned())?;
    let content = match read_workspace_file(&destination, &source) {
        Ok(content) => content,
        Err(error) => {
            db.execute(
                "UPDATE scheduled_tasks SET enabled = 0, source_missing = 1 WHERE id = ?1",
                [id],
            )
            .map_err(db_error)?;
            return Err(format!("Could not read {}: {error}", source.display()));
        }
    };
    let definition = parse_project_task(
        &content,
        source,
        &ScheduledTaskDestination {
            path: destination,
            repository: task.2,
            branch: task.3,
        },
    )?;
    if definition.key != task.5 {
        return Err("project task id changed; refresh project tasks first".to_owned());
    }
    let next = now_ms()
        .checked_add(interval_ms(definition.edit.interval_minutes)?)
        .ok_or_else(|| "schedule is too large".to_owned())?;
    changed(
        db.execute(
            "UPDATE scheduled_tasks SET title = ?2, description = ?3, prompt = ?4, model = ?5, enabled = 1, interval_minutes = ?6, next_run_ms = ?7, source_content = ?8, approved_content = ?8, source_missing = 0 WHERE id = ?1",
            params![id, definition.edit.title, definition.edit.description, definition.edit.prompt, definition.edit.model, definition.edit.interval_minutes, next, content],
        ),
        "project task not found",
    )
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
        let task: Option<(
            i64,
            String,
            String,
            String,
            String,
            Vec<u8>,
            i64,
            i64,
            String,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            bool,
        )> = tx
            .query_row(
                "SELECT id, title, prompt, model, discord_webhook_id, destination, interval_minutes, next_run_ms, source_kind, source_path, approved_content, source_missing FROM scheduled_tasks WHERE (?1 IS NULL AND enabled = 1 AND next_run_ms <= ?2) OR id = ?1 ORDER BY next_run_ms, id LIMIT 1",
                params![requested, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((
            task_id,
            title,
            prompt,
            model,
            discord_webhook_id,
            path,
            minutes,
            next,
            source_kind,
            source_path,
            approved_content,
            source_missing,
        )) = task
        else {
            tx.commit().map_err(db_error)?;
            return Ok(None);
        };
        if source_kind == "project" {
            let unavailable = if source_missing || approved_content.is_none() {
                Some("project task requires approval".to_owned())
            } else {
                let destination = decode_path(path.clone()).map_err(db_error)?;
                let source = source_path
                    .map(decode_path)
                    .transpose()
                    .map_err(db_error)?
                    .map(RepoPath::from);
                match source {
                    Some(source) => match read_workspace_file(&destination, &source) {
                        Ok(content) if approved_content.as_deref() == Some(content.as_slice()) => {
                            None
                        }
                        Ok(content) => {
                            tx.execute(
                                "UPDATE scheduled_tasks SET enabled = 0, source_content = ?2, source_missing = 0 WHERE id = ?1",
                                params![task_id, content],
                            )
                            .map_err(db_error)?;
                            Some("project task changed and requires approval".to_owned())
                        }
                        Err(_) => {
                            tx.execute(
                                "UPDATE scheduled_tasks SET enabled = 0, source_missing = 1 WHERE id = ?1",
                                [task_id],
                            )
                            .map_err(db_error)?;
                            Some("project task source is missing".to_owned())
                        }
                    },
                    None => Some("project task source is unavailable".to_owned()),
                }
            };
            if let Some(error) = unavailable {
                tx.commit().map_err(db_error)?;
                if requested.is_some() {
                    return Err(error);
                }
                continue;
            }
        }
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
                "INSERT OR IGNORE INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms, discord_webhook_id) VALUES (?1, ?2, 'launching', ?3, ?4)",
                params![task_id, scheduled, now_ms(), discord_webhook_id],
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
    complete: &mut impl FnMut(&Connection, i64) -> Result<(), String>,
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
    if status == ScheduledRunStatus::Completed {
        complete(db, claim.run_id)?;
    }
    Ok(())
}

fn execute_claim_with_update(
    db: &Connection,
    claim: Claim,
    launch: &mut impl FnMut(SchedulerLaunchRequest) -> SchedulerLaunchResult,
    complete: &mut impl FnMut(&Connection, i64) -> Result<(), String>,
    updates: &Sender<Update>,
) -> Result<(), String> {
    // Publish the claimed row before the Herdr launch, which can take several seconds.
    let _ = updates.send(load_state(db));
    execute_claim(db, claim, launch, complete)
}

fn refresh(
    db: &Connection,
    requested: Option<i64>,
    observe: &mut impl FnMut(&str, Option<&str>) -> SchedulerObserveResult,
    complete: &mut impl FnMut(&Connection, i64) -> Result<(), String>,
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
                    complete,
                )?;
            }
        } else {
            apply_observation(db, id, observation, complete)?;
        }
    }
    Ok(())
}

fn apply_observation(
    db: &Connection,
    run_id: i64,
    result: SchedulerObserveResult,
    complete: &mut impl FnMut(&Connection, i64) -> Result<(), String>,
) -> Result<(), String> {
    let delivery_error = db
        .query_row(
            "SELECT error FROM scheduled_runs WHERE id = ?1",
            [run_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(db_error)?
        .flatten()
        .filter(|error| error.starts_with(DISCORD_DELIVERY_ERROR));
    let (status, output, error) = match result {
        SchedulerObserveResult::Missing(error) => {
            return fail_run(db, run_id, &error);
        }
        SchedulerObserveResult::Unavailable(error) => {
            (ScheduledRunStatus::Unknown, None::<String>, Some(error))
        }
        SchedulerObserveResult::Observed(status) => (agent_status(status), None, delivery_error),
    };
    db.execute(
        "UPDATE scheduled_runs SET status = ?2, output = COALESCE(?3, output), error = ?4 WHERE id = ?1",
        params![run_id, status.text(), output, error],
    )
    .map_err(db_error)?;
    if status == ScheduledRunStatus::Completed {
        complete(db, run_id)?;
    }
    Ok(())
}

fn fetch_scheduled_result(session_id: &str) -> Result<String, String> {
    let mut result = Err("OpenCode session has no assistant response".to_owned());
    for attempt in 0..RESULT_FETCH_ATTEMPTS {
        result = super::latest_message::final_assistant_text(session_id);
        if result.is_ok() || attempt + 1 == RESULT_FETCH_ATTEMPTS {
            break;
        }
        thread::sleep(RESULT_FETCH_RETRY_DELAY);
    }
    result
}

fn finalize_completed_run(
    db: &Connection,
    run_id: i64,
    final_text: &mut impl FnMut(&str) -> Result<String, String>,
    publish: &mut impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    let (title, session_id, stored_output, error) = db
        .query_row(
            "SELECT task.title, run.session_id, run.output, run.error FROM scheduled_runs run JOIN scheduled_tasks task ON task.id = run.task_id WHERE run.id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(db_error)?;
    let retrying = error
        .as_deref()
        .is_some_and(|error| error.starts_with(DISCORD_DELIVERY_ERROR));
    if !stored_output.is_empty() && !retrying {
        return Ok(());
    }
    let output = if stored_output.is_empty() {
        let Some(session_id) = session_id else {
            return record_delivery_error(db, run_id, None, "OpenCode session is unavailable");
        };
        match final_text(&session_id) {
            Ok(output) => output,
            Err(error) => return record_delivery_error(db, run_id, None, &error),
        }
    } else {
        stored_output
    };
    if let Err(error) = publish(&title, &output) {
        return record_delivery_error(db, run_id, Some(&output), &error);
    }
    db.execute(
        "UPDATE scheduled_runs SET output = ?2, error = NULL WHERE id = ?1",
        params![run_id, output],
    )
    .map_err(db_error)?;
    Ok(())
}

fn retry_delivery(
    db: &Connection,
    run_id: i64,
    complete: &mut impl FnMut(&Connection, i64) -> Result<(), String>,
) -> Result<bool, String> {
    let retrying = db
        .query_row(
            "SELECT error FROM scheduled_runs WHERE id = ?1",
            [run_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(db_error)?
        .flatten()
        .is_some_and(|error| error.starts_with(DISCORD_DELIVERY_ERROR));
    if retrying {
        complete(db, run_id)?;
    }
    Ok(retrying)
}

fn record_delivery_error(
    db: &Connection,
    run_id: i64,
    output: Option<&str>,
    error: &str,
) -> Result<(), String> {
    let error = format!("{DISCORD_DELIVERY_ERROR}{error}");
    db.execute(
        "UPDATE scheduled_runs SET output = COALESCE(?2, output), error = ?3 WHERE id = ?1",
        params![run_id, output, error],
    )
    .map_err(db_error)?;
    Ok(())
}

fn discord_message(title: &str, output: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = truncate_utf8(&title, 160);
    let prefix = format!("Hunkle scheduled task: {title}\n\n");
    let output = output.trim();
    if prefix.len().saturating_add(output.len()) <= DISCORD_MESSAGE_BYTES {
        return format!("{prefix}{output}");
    }
    let suffix = "\n\n[Result truncated by Hunkle]";
    let available = DISCORD_MESSAGE_BYTES.saturating_sub(prefix.len() + suffix.len());
    format!("{prefix}{}{suffix}", truncate_utf8(output, available))
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &value[..end]
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
        0 => "CREATE TABLE scheduled_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, description TEXT NOT NULL, prompt TEXT NOT NULL, model TEXT NOT NULL DEFAULT '', discord_webhook_id TEXT NOT NULL DEFAULT '', destination BLOB NOT NULL, repository TEXT NOT NULL, branch TEXT NOT NULL, enabled INTEGER NOT NULL, interval_minutes INTEGER NOT NULL CHECK (interval_minutes > 0), next_run_ms INTEGER NOT NULL, source_path BLOB, source_kind TEXT NOT NULL DEFAULT 'local', project_key TEXT NOT NULL DEFAULT '', source_content BLOB, approved_content BLOB, source_missing INTEGER NOT NULL DEFAULT 0, project_repository BLOB);
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, terminal_id TEXT, session_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, discord_webhook_id TEXT NOT NULL DEFAULT '', UNIQUE (task_id, scheduled_for_ms));
              CREATE TABLE scheduler_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
               CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id); CREATE UNIQUE INDEX scheduled_project_tasks ON scheduled_tasks(project_repository, project_key) WHERE source_kind = 'project';",
        1 => "ALTER TABLE scheduled_runs RENAME TO scheduled_runs_v1;
              CREATE TABLE scheduled_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE, scheduled_for_ms INTEGER NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, pane_id TEXT, terminal_id TEXT, session_id TEXT, output TEXT NOT NULL DEFAULT '', error TEXT, UNIQUE (task_id, scheduled_for_ms));
              INSERT INTO scheduled_runs (id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, terminal_id, output, error) SELECT id, task_id, scheduled_for_ms, status, created_at_ms, pane_id, terminal_id, output, error FROM scheduled_runs_v1 WHERE task_id IS NOT NULL;
              DROP TABLE scheduled_runs_v1; CREATE INDEX scheduled_runs_task_history ON scheduled_runs(task_id, created_at_ms DESC, id DESC); CREATE INDEX scheduled_runs_active ON scheduled_runs(status, task_id);",
        2 => "ALTER TABLE scheduled_runs ADD COLUMN terminal_id TEXT; ALTER TABLE scheduled_runs ADD COLUMN session_id TEXT;",
        3 => "ALTER TABLE scheduled_runs ADD COLUMN session_id TEXT;",
        4 => "ALTER TABLE scheduled_tasks ADD COLUMN source_path BLOB; CREATE UNIQUE INDEX scheduled_tasks_source ON scheduled_tasks(destination, source_path) WHERE source_path IS NOT NULL;",
        5 => "ALTER TABLE scheduled_tasks ADD COLUMN model TEXT NOT NULL DEFAULT '';",
        6 => "",
        7 => "",
        8 => "",
        9 => "",
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
    if version > 0 && version < 7 {
        for table in ["scheduled_tasks", "scheduled_runs"] {
            let exists = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(db_error)?;
            if exists {
                tx.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN discord_webhook_id TEXT NOT NULL DEFAULT '';"
                ))
                .map_err(db_error)?;
            }
        }
    }
    if version > 0 && version < 8 {
        let has_tasks = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'scheduled_tasks')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(db_error)?;
        if has_tasks {
            tx.execute_batch(
                "ALTER TABLE scheduled_tasks ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'local';
                 ALTER TABLE scheduled_tasks ADD COLUMN project_key TEXT NOT NULL DEFAULT '';
                 ALTER TABLE scheduled_tasks ADD COLUMN source_content BLOB;
                 ALTER TABLE scheduled_tasks ADD COLUMN approved_content BLOB;
                 ALTER TABLE scheduled_tasks ADD COLUMN source_missing INTEGER NOT NULL DEFAULT 0;
                 DROP INDEX IF EXISTS scheduled_tasks_source;
                 CREATE UNIQUE INDEX scheduled_project_tasks ON scheduled_tasks(destination, project_key) WHERE source_kind = 'project';",
            )
            .map_err(db_error)?;
        }
    }
    if version > 0 && version < 9 {
        let has_tasks = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'scheduled_tasks')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(db_error)?;
        if has_tasks {
            tx.execute_batch(
                "ALTER TABLE scheduled_tasks ADD COLUMN project_repository BLOB;
                 UPDATE scheduled_tasks SET project_repository = destination WHERE source_kind = 'project';
                 DROP INDEX IF EXISTS scheduled_project_tasks;
                 CREATE UNIQUE INDEX scheduled_project_tasks ON scheduled_tasks(project_repository, project_key) WHERE source_kind = 'project';",
            )
            .map_err(db_error)?;
        }
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduler_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(db_error)?;
    if version < 9 {
        tx.execute_batch("PRAGMA user_version = 9;")
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
        "SELECT id, title, description, prompt, model, discord_webhook_id, destination, repository, branch, enabled, interval_minutes, next_run_ms, source_path, source_kind, source_content, approved_content, source_missing FROM scheduled_tasks ORDER BY id",
        |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                prompt: row.get(3)?,
                model: row.get(4)?,
                discord_webhook_id: row.get(5)?,
                destination: decode_path(row.get(6)?)?,
                repository: row.get(7)?,
                branch: row.get(8)?,
                enabled: row.get(9)?,
                interval_minutes: row.get(10)?,
                next_run_ms: row.get(11)?,
                source: row
                    .get::<_, Option<Vec<u8>>>(12)?
                    .map(decode_path)
                    .transpose()?
                    .map(RepoPath::from),
                project_status: if row.get::<_, String>(13)? == "project" {
                    let source_content = row.get::<_, Option<Vec<u8>>>(14)?;
                    let approved_content = row.get::<_, Option<Vec<u8>>>(15)?;
                    Some(if row.get::<_, bool>(16)? {
                        ProjectTaskStatus::Missing
                    } else if approved_content.is_none() {
                        ProjectTaskStatus::Pending
                    } else if approved_content == source_content {
                        ProjectTaskStatus::Current
                    } else {
                        ProjectTaskStatus::Changed
                    })
                } else {
                    None
                },
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

    #[test]
    fn discord_test_reports_missing_configuration() {
        let mut scheduler = SchedulerService::open(None, None, Vec::new()).unwrap();
        scheduler.test_discord_webhook("123456".to_owned()).unwrap();

        let mut notice = None;
        for _ in 0..20 {
            let (_, next) = scheduler.poll_completions();
            notice = notice.or(next);
            if notice.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(notice.as_deref(), Some("Discord webhook is not configured"));
    }

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
        let content = br#"---
status: disabled
frequency: 2h
title: 'Nightly: review'
description: "Review \"open\" changes"
---

Inspect the diff.
Summarize risks.
"#;

        let mut task = parse_task_file(content, Some(destination(directory.path()))).unwrap();

        assert_eq!(task.title, "Nightly: review");
        assert_eq!(task.description, "Review \"open\" changes");
        assert_eq!(task.prompt, "Inspect the diff.\nSummarize risks.");
        assert_eq!(task.interval_minutes, 120);
        assert_eq!(task.model, "");
        assert!(!task.enabled);
        task.model = "opencode-go/deepseek-flash-v4".to_owned();
        task.discord_webhook_id = "123456".to_owned();
        let reparsed = parse_task_file(render_task_file(&task).as_bytes(), None).unwrap();
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
            9
        );
        assert!(db.prepare("SELECT model FROM scheduled_tasks").is_ok());
    }

    #[test]
    fn discovers_approves_changes_and_marks_missing_project_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let scheduled = directory.path().join(".agents/scheduled");
        std::fs::create_dir_all(&scheduled).unwrap();
        let source = scheduled.join("review.md");
        let initial = "---\nid: review\nfrequency: 1d\ntitle: Review\ndescription: Check changes\n---\n\nReview the repository.\n";
        std::fs::write(&source, initial).unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();

        discover_project_tasks(
            &mut db,
            &destination(directory.path()),
            directory.path().as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.interval_minutes, 24 * 60);
        assert_eq!(
            task.source,
            Some(RepoPath::from(".agents/scheduled/review.md"))
        );
        assert_eq!(task.project_status, Some(ProjectTaskStatus::Pending));
        assert!(!task.enabled);
        assert!(source.exists());

        toggle_task(&db, task.id, true).unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.project_status, Some(ProjectTaskStatus::Current));
        assert!(task.enabled);
        discover_project_tasks(
            &mut db,
            &destination(directory.path()),
            directory.path().as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        assert!(load_state(&db).unwrap().0.pop().unwrap().enabled);

        std::fs::write(
            &source,
            initial.replace("Review the repository.", "Review carefully."),
        )
        .unwrap();
        assert_eq!(
            claim(&mut db, Some(task.id), now_ms()).err().unwrap(),
            "project task changed and requires approval"
        );
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.project_status, Some(ProjectTaskStatus::Changed));
        assert!(!task.enabled);
        discover_project_tasks(
            &mut db,
            &destination(directory.path()),
            directory.path().as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.project_status, Some(ProjectTaskStatus::Changed));
        assert!(!task.enabled);

        std::fs::remove_file(source).unwrap();
        discover_project_tasks(
            &mut db,
            &destination(directory.path()),
            directory.path().as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.project_status, Some(ProjectTaskStatus::Missing));
        assert!(!task.enabled);
    }

    #[test]
    fn linked_worktree_discovery_does_not_duplicate_or_retarget_project_tasks() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let content =
            "---\nid: review\nfrequency: 1d\ntitle: Review\n---\n\nReview the repository.\n";
        for root in [first.path(), second.path()] {
            let scheduled = root.join(".agents/scheduled");
            std::fs::create_dir_all(&scheduled).unwrap();
            std::fs::write(scheduled.join("review.md"), content).unwrap();
        }
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();
        discover_project_tasks(&mut db, &destination(first.path()), b"repository").unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        toggle_task(&db, task.id, true).unwrap();

        discover_project_tasks(&mut db, &destination(second.path()), b"repository").unwrap();

        let tasks = load_state(&db).unwrap().0;
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].destination,
            std::fs::canonicalize(first.path()).unwrap()
        );
        assert!(tasks[0].enabled);
        assert_eq!(tasks[0].project_status, Some(ProjectTaskStatus::Current));
    }

    #[test]
    fn imports_legacy_repository_markdown_without_removing_it() {
        let directory = tempfile::tempdir().unwrap();
        let scheduled = directory.path().join(".hunkle/scheduled");
        std::fs::create_dir_all(&scheduled).unwrap();
        let source = scheduled.join("review.md");
        std::fs::write(
            &source,
            "---\nstatus: enabled\nfrequency: 1h\ntitle: Review\ndescription: Check changes\n---\n\nReview the repository.\n",
        )
        .unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();

        discover_project_tasks(&mut db, &destination(directory.path()), b"repository").unwrap();
        discover_project_tasks(&mut db, &destination(directory.path()), b"repository").unwrap();

        let tasks = load_state(&db).unwrap().0;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Review");
        assert!(tasks[0].project_status.is_none());
        assert!(tasks[0].enabled);
        assert!(source.exists());
    }

    #[test]
    fn imports_legacy_global_markdown_as_a_local_database_task_once() {
        let directory = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        std::fs::create_dir(files.path().join("scheduled")).unwrap();
        let source = files.path().join("scheduled/review.md");
        std::fs::write(
            &source,
            "---\nstatus: enabled\nfrequency: 1h\ntitle: Review\ndescription: Check changes\ndestination: \"",
        )
        .unwrap();
        let mut content = std::fs::read_to_string(&source).unwrap();
        content.push_str(&directory.path().to_string_lossy());
        content.push_str("\"\nrepository: repo\nbranch: main\n---\n\nReview the repository.\n");
        std::fs::write(&source, content).unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();
        import_legacy_task_files(&db, files.path()).unwrap();
        import_legacy_task_files(&db, files.path()).unwrap();
        let task = load_state(&db).unwrap().0.pop().unwrap();
        assert_eq!(task.title, "Review");
        assert!(task.project_status.is_none());
        assert!(task.source.is_none());
        assert!(source.exists());
        assert_eq!(load_state(&db).unwrap().0.len(), 1);
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
            &mut |_, _| Ok(()),
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
            &mut |_, _| Ok(()),
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
        refresh(
            &first,
            None,
            &mut |_, _| SchedulerObserveResult::Observed(AgentStatus::Done),
            &mut |_, _| Ok(()),
        )
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
    fn completed_run_publishes_once_and_persists_the_result() {
        let (db, run_id) = completed_run_database();
        let mut fetches = 0;
        let mut deliveries = Vec::new();
        let mut fetch = |session_id: &str| {
            fetches += 1;
            assert_eq!(session_id, "ses-result");
            Ok("Final report".to_owned())
        };
        let mut publish = |title: &str, output: &str| {
            deliveries.push((title.to_owned(), output.to_owned()));
            Ok(())
        };

        finalize_completed_run(&db, run_id, &mut fetch, &mut publish).unwrap();
        finalize_completed_run(&db, run_id, &mut fetch, &mut publish).unwrap();

        assert_eq!(fetches, 1);
        assert_eq!(
            deliveries,
            [("Review".to_owned(), "Final report".to_owned())]
        );
        let (output, error) = db
            .query_row(
                "SELECT output, error FROM scheduled_runs WHERE id = ?1",
                [run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(output, "Final report");
        assert_eq!(error, None);
    }

    #[test]
    fn failed_delivery_retries_the_persisted_result() {
        let (db, run_id) = completed_run_database();
        let mut fetches = 0;
        let mut fetch = |_: &str| {
            fetches += 1;
            Ok("Final report".to_owned())
        };
        finalize_completed_run(&db, run_id, &mut fetch, &mut |_, _| {
            Err("offline".to_owned())
        })
        .unwrap();
        let error = db
            .query_row(
                "SELECT error FROM scheduled_runs WHERE id = ?1",
                [run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert_eq!(error.as_deref(), Some("Discord delivery failed: offline"));
        assert!(retry_delivery(&db, run_id, &mut |_, _| Ok(())).unwrap());

        let mut delivered = false;
        finalize_completed_run(&db, run_id, &mut fetch, &mut |_, output| {
            delivered = output == "Final report";
            Ok(())
        })
        .unwrap();

        assert!(delivered);
        assert_eq!(fetches, 1);
        assert!(!retry_delivery(&db, run_id, &mut |_, _| Ok(())).unwrap());
    }

    #[test]
    fn discord_message_is_bounded_and_collapses_title_whitespace() {
        let message = discord_message("  Nightly\n review  ", &"é".repeat(2_000));

        assert!(message.starts_with("Hunkle scheduled task: Nightly review\n\n"));
        assert!(message.ends_with("\n\n[Result truncated by Hunkle]"));
        assert!(message.len() <= DISCORD_MESSAGE_BYTES);
    }

    #[test]
    fn claim_snapshots_the_tasks_discord_webhook() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();
        save_task(
            &db,
            None,
            ScheduledTaskEdit {
                title: "Review".to_owned(),
                description: String::new(),
                prompt: "Review it".to_owned(),
                model: String::new(),
                discord_webhook_id: "123456".to_owned(),
                destination: directory.path().to_owned(),
                repository: "repo".to_owned(),
                branch: "main".to_owned(),
                enabled: true,
                interval_minutes: 60,
            },
            1,
        )
        .unwrap();

        let claim = claim(&mut db, Some(1), 1).unwrap().unwrap();

        assert_eq!(
            db.query_row(
                "SELECT discord_webhook_id FROM scheduled_runs WHERE id = ?1",
                [claim.run_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "123456"
        );
    }

    #[test]
    fn completed_run_without_discord_opt_in_does_not_publish() {
        let (db, run_id) = completed_run_database();

        complete_run(&db, run_id, &HashMap::new()).unwrap();

        assert_eq!(
            db.query_row(
                "SELECT output FROM scheduled_runs WHERE id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            ""
        );
    }

    fn completed_run_database() -> (Connection, i64) {
        let mut db = Connection::open_in_memory().unwrap();
        prepare_database(&mut db).unwrap();
        save_task(
            &db,
            None,
            ScheduledTaskEdit {
                title: "Review".to_owned(),
                description: String::new(),
                prompt: "Review it".to_owned(),
                model: String::new(),
                discord_webhook_id: String::new(),
                destination: std::env::temp_dir(),
                repository: "repo".to_owned(),
                branch: "main".to_owned(),
                enabled: true,
                interval_minutes: 60,
            },
            1,
        )
        .unwrap();
        db.execute(
            "INSERT INTO scheduled_runs (task_id, scheduled_for_ms, status, created_at_ms, session_id) VALUES (1, 1, 'completed', 1, 'ses-result')",
            [],
        )
        .unwrap();
        let run_id = db.last_insert_rowid();
        (db, run_id)
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
            9
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
            9
        );
        assert!(db.prepare("SELECT session_id FROM scheduled_runs").is_ok());
    }
}
