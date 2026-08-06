mod service;

use super::*;
pub(crate) use service::{
    ProjectTaskStatus, ScheduledRun, ScheduledRunStatus, ScheduledTask, ScheduledTaskDestination,
    ScheduledTaskEdit,
};
use std::collections::HashSet;

pub(crate) struct ScheduledTasks {
    service: Option<service::SchedulerService>,
    open_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct ScheduledTasksPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
}

pub(crate) struct LegacyScheduledRunBinding {
    pub(crate) run_id: i64,
    pub(crate) agent: Option<(String, String)>,
    pub(crate) session_id: Option<String>,
}

impl ScheduledTasks {
    #[cfg(not(test))]
    pub(crate) fn open(
        config_dir: Option<&Path>,
        discord_webhooks: Vec<DiscordWebhookConfig>,
    ) -> Self {
        let files_root = crate::paths::data_root().map_err(|error| error.to_string());
        match files_root.and_then(|files_root| {
            service::SchedulerService::open(
                config_dir.map(|path| path.join("scheduler.sqlite3")),
                Some(files_root),
                discord_webhooks,
            )
        }) {
            Ok(service) => Self {
                service: Some(service),
                open_error: None,
            },
            Err(error) => Self {
                service: None,
                open_error: Some(error),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            service: Some(service::SchedulerService::open(None, None, Vec::new()).unwrap()),
            open_error: None,
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.service.is_some()
    }

    pub(crate) fn tasks(&self) -> &[ScheduledTask] {
        self.service
            .as_ref()
            .map_or(&[], |service| service.tasks.as_slice())
    }

    pub(crate) fn runs(&self) -> &[ScheduledRun] {
        self.service
            .as_ref()
            .map_or(&[], |service| service.runs.as_slice())
    }

    pub(crate) fn poll(&mut self) -> ScheduledTasksPoll {
        let Some(service) = self.service.as_mut() else {
            return ScheduledTasksPoll {
                notice: self
                    .open_error
                    .take()
                    .map(|error| format!("Could not open scheduler: {error}")),
                ..ScheduledTasksPoll::default()
            };
        };
        let (changed, notice) = service.poll_completions();
        ScheduledTasksPoll {
            changed,
            notice: notice.map(|notice| format!("Scheduler: {notice}")),
        }
    }

    fn service(&self) -> Result<&service::SchedulerService, String> {
        self.service
            .as_ref()
            .ok_or_else(|| "scheduler is unavailable".to_owned())
    }

    pub(crate) fn save_task(&self, id: Option<i64>, edit: ScheduledTaskEdit) -> Result<(), String> {
        self.service()?.save_task(id, edit)
    }

    pub(crate) fn configure_project_task(
        &self,
        id: i64,
        discord_webhook_id: String,
    ) -> Result<(), String> {
        self.service()?
            .configure_project_task(id, discord_webhook_id)
    }

    pub(crate) fn discover_project_tasks(
        &self,
        destination: ScheduledTaskDestination,
        repository_identity: PathBuf,
    ) -> Result<(), String> {
        self.service()?
            .discover_project_tasks(destination, repository_identity)
    }

    pub(crate) fn toggle_task(&self, id: i64, enabled: bool) -> Result<(), String> {
        self.service()?.toggle_task(id, enabled)
    }

    pub(crate) fn delete_task(&self, id: i64) -> Result<(), String> {
        self.service()?.delete_task(id)
    }

    pub(crate) fn run_now(&self, id: i64) -> Result<(), String> {
        self.service()?.run_now(id)
    }

    pub(crate) fn refresh_run(&self, id: i64) -> Result<(), String> {
        self.service()?.refresh_run(id)
    }

    pub(crate) fn prompt_run(&self, id: i64, prompt: String) -> Result<(), String> {
        self.service()?.prompt_run(id, prompt)
    }

    pub(crate) fn configure_discord_webhooks(
        &self,
        webhooks: Vec<DiscordWebhookConfig>,
    ) -> Result<(), String> {
        let Some(service) = self.service.as_ref() else {
            return Ok(());
        };
        service.configure_discord_webhooks(webhooks)
    }

    pub(crate) fn test_discord_webhook(&self, channel: String) -> Result<(), String> {
        self.service()?.test_discord_webhook(channel)
    }

    pub(crate) fn bind_agent(&mut self, id: i64, pane_id: String, terminal_id: String) {
        if let Some(service) = self.service.as_mut() {
            service.bind_agent(id, pane_id, terminal_id);
        }
    }

    pub(crate) fn bind_session(&mut self, id: i64, session_id: String) {
        if let Some(service) = self.service.as_mut() {
            service.bind_session(id, session_id);
        }
    }

    pub(crate) fn bind_legacy_agents(&mut self, herdr: &HerdrSession) {
        for binding in herdr.legacy_scheduled_run_bindings(self.tasks(), self.runs()) {
            if let Some((pane_id, terminal_id)) = binding.agent {
                self.bind_agent(binding.run_id, pane_id, terminal_id);
            }
            if let Some(session_id) = binding.session_id {
                self.bind_session(binding.run_id, session_id);
            }
        }
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(service) = self.service.as_mut() {
            service.shutdown();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_tasks_for_test(&mut self, tasks: Vec<ScheduledTask>) {
        if self.service.is_none() {
            *self = Self::memory();
        }
        self.service.as_mut().unwrap().tasks = tasks;
    }

    #[cfg(test)]
    pub(crate) fn set_runs_for_test(&mut self, runs: Vec<ScheduledRun>) {
        if self.service.is_none() {
            *self = Self::memory();
        }
        self.service.as_mut().unwrap().runs = runs;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchedulerSurface {
    #[default]
    Tasks,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchedulerField {
    Title,
    Description,
    Prompt,
    Model,
    Schedule,
    Discord,
    Destination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchedulerDestinationCard {
    Repository,
    Worktree,
    Branch,
}

impl SchedulerDestinationCard {
    fn next(self, backwards: bool) -> Self {
        cycle(
            self,
            [Self::Repository, Self::Worktree, Self::Branch],
            backwards,
        )
    }

    pub(crate) fn value(self, destination: &SchedulerDestination) -> &str {
        match self {
            Self::Repository => &destination.repository,
            Self::Worktree => destination.worktree.as_deref().unwrap_or("new worktree"),
            Self::Branch => &destination.branch.name,
        }
    }
}

impl SchedulerField {
    fn next(self, backwards: bool) -> Self {
        cycle(
            self,
            [
                Self::Title,
                Self::Description,
                Self::Prompt,
                Self::Model,
                Self::Schedule,
                Self::Discord,
                Self::Destination,
            ],
            backwards,
        )
    }
}

fn cycle<T: Copy + PartialEq, const N: usize>(value: T, values: [T; N], backwards: bool) -> T {
    let index = values.iter().position(|item| *item == value).unwrap_or(0);
    values[(index + if backwards { N - 1 } else { 1 }) % N]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchedulerDestination {
    pub(crate) path: Option<PathBuf>,
    pub(crate) repository_root: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: Branch,
    pub(crate) checkout_branch: String,
    pub(crate) worktree: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ScheduledTaskComposer {
    pub(crate) task_id: Option<i64>,
    pub(crate) enabled: bool,
    pub(crate) project: bool,
    pub(crate) title: TextInput,
    pub(crate) description: TextInput,
    pub(crate) prompt: TextInput,
    pub(crate) model: TextInput,
    pub(crate) prompt_expanded: bool,
    pub(crate) prompt_scroll: usize,
    pub(crate) schedule: TextInput,
    pub(crate) discord_webhooks: Vec<(String, String)>,
    pub(crate) discord_webhook: usize,
    pub(crate) destinations: Vec<SchedulerDestination>,
    pub(crate) destination: usize,
    pub(crate) destination_card: SchedulerDestinationCard,
    pub(crate) destination_picker: HeaderPicker,
    pub(crate) field: SchedulerField,
}

impl ScheduledTaskComposer {
    fn new(
        destinations: Vec<SchedulerDestination>,
        discord_webhooks: Vec<(String, String)>,
    ) -> Self {
        let mut schedule = TextInput::default();
        schedule.set("15");
        Self {
            task_id: None,
            enabled: true,
            project: false,
            title: TextInput::default(),
            description: TextInput::default(),
            prompt: TextInput::default(),
            model: TextInput::default(),
            prompt_expanded: false,
            prompt_scroll: 0,
            schedule,
            discord_webhooks,
            discord_webhook: 0,
            destinations,
            destination: 0,
            destination_card: SchedulerDestinationCard::Repository,
            destination_picker: HeaderPicker::default(),
            field: SchedulerField::Title,
        }
    }

    fn edit(
        task: &ScheduledTask,
        mut destinations: Vec<SchedulerDestination>,
        mut discord_webhooks: Vec<(String, String)>,
    ) -> Self {
        let destination = destinations
            .iter()
            .position(|destination| {
                destination
                    .path
                    .as_deref()
                    .is_some_and(|path| same_path(path, &task.destination))
            })
            .unwrap_or_else(|| {
                destinations.push(SchedulerDestination {
                    path: Some(task.destination.clone()),
                    repository_root: task.destination.clone(),
                    repository: task.repository.clone(),
                    branch: Branch {
                        name: task.branch.clone(),
                        upstream: None,
                        remote: false,
                        current: false,
                        default: false,
                        last_touched_at: None,
                    },
                    checkout_branch: task.branch.clone(),
                    worktree: Some(
                        task.destination
                            .file_name()
                            .unwrap_or(task.destination.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                    ),
                });
                destinations.len() - 1
            });
        let discord_webhook = if task.discord_webhook_id.is_empty() {
            0
        } else {
            discord_webhooks
                .iter()
                .position(|(id, _)| id == &task.discord_webhook_id)
                .map(|index| index + 1)
                .unwrap_or_else(|| {
                    discord_webhooks.push((
                        task.discord_webhook_id.clone(),
                        "Missing configured webhook".to_owned(),
                    ));
                    discord_webhooks.len()
                })
        };
        let mut composer = Self::new(destinations, discord_webhooks);
        composer.task_id = Some(task.id);
        composer.enabled = task.enabled;
        composer.project = task.project_status.is_some();
        composer.destination = destination;
        composer.discord_webhook = discord_webhook;
        composer.title.set(&task.title);
        composer.description.set(&task.description);
        composer.prompt.set(&task.prompt);
        composer.model.set(&task.model);
        composer.schedule.set(&task.interval_minutes.to_string());
        if composer.project {
            composer.field = SchedulerField::Discord;
        }
        composer
    }

    pub(crate) fn discord_webhook_label(&self) -> &str {
        self.discord_webhook
            .checked_sub(1)
            .and_then(|index| self.discord_webhooks.get(index))
            .map(|(_, label)| label.as_str())
            .unwrap_or("Off")
    }

    fn cycle_discord_webhook(&mut self, backwards: bool) {
        let count = self.discord_webhooks.len() + 1;
        self.discord_webhook =
            (self.discord_webhook + if backwards { count - 1 } else { 1 }) % count;
    }

    fn discord_webhook_id(&self) -> String {
        self.discord_webhook
            .checked_sub(1)
            .and_then(|index| self.discord_webhooks.get(index))
            .map(|(id, _)| id.clone())
            .unwrap_or_default()
    }

    fn destination_indices(&self, card: SchedulerDestinationCard) -> Vec<usize> {
        let Some(selected) = self.destinations.get(self.destination) else {
            return Vec::new();
        };
        let mut candidates: Vec<usize> = Vec::new();
        let mut repositories = HashSet::new();
        let mut worktrees = HashSet::new();
        let mut branches = HashSet::new();
        for (index, destination) in self.destinations.iter().enumerate() {
            if card == SchedulerDestinationCard::Worktree && destination.path.is_none() {
                continue;
            }
            if card != SchedulerDestinationCard::Repository
                && destination.repository_root != selected.repository_root
            {
                continue;
            }
            let inserted = match card {
                SchedulerDestinationCard::Repository => {
                    repositories.insert(&destination.repository_root)
                }
                SchedulerDestinationCard::Worktree => worktrees.insert(destination.path.as_ref()),
                SchedulerDestinationCard::Branch => branches.insert(&destination.branch.name),
            };
            if inserted {
                candidates.push(index);
            }
        }
        candidates
    }

    fn destination_item(
        &self,
        card: SchedulerDestinationCard,
        index: usize,
    ) -> Option<HeaderPickerItem> {
        let destination = self.destinations.get(index)?;
        Some(match card {
            SchedulerDestinationCard::Repository => HeaderPickerItem::Repository {
                path: destination
                    .path
                    .clone()
                    .unwrap_or_else(|| destination.repository_root.clone()),
                label: destination.repository.clone(),
                stats: None,
                branch: Some(destination.checkout_branch.clone()),
            },
            SchedulerDestinationCard::Worktree => {
                let path = destination.path.clone()?;
                HeaderPickerItem::Worktree {
                    worktree: crate::git::LinkedWorktree {
                        path,
                        head: None,
                        branch: Some(format!("refs/heads/{}", destination.checkout_branch)),
                        is_main: destination.worktree.as_deref() == Some("basetree"),
                        is_detached: destination.checkout_branch == "detached HEAD",
                        is_bare: false,
                        locked: false,
                        locked_reason: None,
                        prunable: false,
                        prunable_reason: None,
                    },
                    stats: None,
                }
            }
            SchedulerDestinationCard::Branch => {
                HeaderPickerItem::Branch(destination.branch.clone())
            }
        })
    }

    pub(crate) fn destination_index_for_item(&self, item: &HeaderPickerItem) -> Option<usize> {
        let selected = self.destinations.get(self.destination)?;
        match item {
            HeaderPickerItem::Repository { path, .. } => self
                .destinations
                .iter()
                .position(|item| item.path.as_deref().unwrap_or(&item.repository_root) == path),
            HeaderPickerItem::Worktree { worktree, .. } => self
                .destinations
                .iter()
                .position(|item| item.path.as_deref() == Some(worktree.path.as_path())),
            HeaderPickerItem::Branch(branch) => self.destinations.iter().position(|item| {
                item.repository_root == selected.repository_root && item.branch.name == branch.name
            }),
            _ => None,
        }
    }

    fn open_destination_picker(&mut self, card: SchedulerDestinationCard) {
        self.destination_card = card;
        let indices = self.destination_indices(card);
        let selected = indices
            .iter()
            .position(|index| *index == self.destination)
            .unwrap_or(0);
        let items = indices
            .into_iter()
            .filter_map(|index| self.destination_item(card, index))
            .collect();
        let kind = match card {
            SchedulerDestinationCard::Repository => HeaderPickerKind::Repositories,
            SchedulerDestinationCard::Worktree => HeaderPickerKind::Worktrees,
            SchedulerDestinationCard::Branch => HeaderPickerKind::Branches,
        };
        self.destination_picker.open(kind, items, selected);
    }

    fn close_destination_picker(&mut self) {
        self.destination_picker.close();
    }

    fn sync_destinations(&mut self, mut destinations: Vec<SchedulerDestination>) {
        let selected = self.destinations.get(self.destination).cloned();
        self.destination = selected
            .as_ref()
            .and_then(|selected| {
                destinations.iter().position(|destination| {
                    destination.path == selected.path
                        && destination.repository_root == selected.repository_root
                        && destination.branch.name == selected.branch.name
                })
            })
            .or_else(|| {
                selected.map(|selected| {
                    destinations.push(selected);
                    destinations.len() - 1
                })
            })
            .unwrap_or(0);
        self.destinations = destinations;
        self.close_destination_picker();
    }

    pub(crate) fn destination_picker_open(&self) -> bool {
        self.destination_picker.is_open()
    }

    fn select_destination_picker_item(&mut self) {
        if let Some(item) = self
            .destination_picker
            .items
            .get(self.destination_picker.selected)
            && let Some(destination) = self.destination_index_for_item(item)
        {
            self.destination = destination;
        }
        self.close_destination_picker();
    }

    pub(crate) fn input(&self, field: SchedulerField) -> &TextInput {
        match field {
            SchedulerField::Title => &self.title,
            SchedulerField::Description => &self.description,
            SchedulerField::Prompt => &self.prompt,
            SchedulerField::Model => &self.model,
            SchedulerField::Schedule => &self.schedule,
            SchedulerField::Discord | SchedulerField::Destination => unreachable!(),
        }
    }

    fn input_mut(&mut self, field: SchedulerField) -> &mut TextInput {
        match field {
            SchedulerField::Title => &mut self.title,
            SchedulerField::Description => &mut self.description,
            SchedulerField::Prompt => &mut self.prompt,
            SchedulerField::Model => &mut self.model,
            SchedulerField::Schedule => &mut self.schedule,
            SchedulerField::Discord | SchedulerField::Destination => unreachable!(),
        }
    }

    fn focus(&mut self, field: SchedulerField) {
        self.field = field;
        if !matches!(field, SchedulerField::Discord | SchedulerField::Destination) {
            self.input_mut(field).focus();
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SchedulerState {
    pub(crate) surface: SchedulerSurface,
    pub(crate) selected_task_id: Option<i64>,
    pub(crate) selected_run_id: Option<i64>,
    pub(crate) composer: Option<ScheduledTaskComposer>,
    pub(crate) task_scroll: usize,
    pub(crate) run_scroll: usize,
    pub(crate) runs_focused: bool,
    pub(crate) error: Option<String>,
    pub(crate) pending_worktree: Option<usize>,
    pub(crate) preview_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerEffect {
    Handled,
    Close,
    Cancel,
    New,
    Edit,
    Save,
    SelectTask(i64),
    SelectRun(i64),
    MoveSelection(isize),
    Toggle,
    RunNow,
    Delete,
    Refresh,
    OpenConversation,
}

impl SchedulerState {
    fn activate_target(
        &mut self,
        target: SchedulerHitTarget,
        prompt_width: usize,
    ) -> SchedulerEffect {
        match target {
            SchedulerHitTarget::Close => SchedulerEffect::Close,
            SchedulerHitTarget::Back | SchedulerHitTarget::Cancel => SchedulerEffect::Cancel,
            SchedulerHitTarget::New => SchedulerEffect::New,
            SchedulerHitTarget::Edit => SchedulerEffect::Edit,
            SchedulerHitTarget::Save => SchedulerEffect::Save,
            SchedulerHitTarget::Task(id) => SchedulerEffect::SelectTask(id),
            SchedulerHitTarget::Run(id) => SchedulerEffect::SelectRun(id),
            SchedulerHitTarget::Field(field) => {
                if let Some(composer) = self.composer.as_mut() {
                    composer.focus(field);
                }
                SchedulerEffect::Handled
            }
            SchedulerHitTarget::PromptExpand => {
                self.toggle_prompt_expansion(prompt_width);
                SchedulerEffect::Handled
            }
            SchedulerHitTarget::DestinationPickerOverlay => SchedulerEffect::Handled,
            SchedulerHitTarget::DestinationCard(card) => {
                if let Some(composer) = self.composer.as_mut() {
                    composer.field = SchedulerField::Destination;
                    if composer.destination_picker_open() && composer.destination_card == card {
                        composer.close_destination_picker();
                    } else {
                        composer.open_destination_picker(card);
                    }
                }
                SchedulerEffect::Handled
            }
            SchedulerHitTarget::Destination(index) => {
                if let Some(composer) = self.composer.as_mut()
                    && index < composer.destinations.len()
                {
                    composer.destination = index;
                    composer.close_destination_picker();
                }
                SchedulerEffect::Handled
            }
            SchedulerHitTarget::Toggle => SchedulerEffect::Toggle,
            SchedulerHitTarget::RunNow => SchedulerEffect::RunNow,
            SchedulerHitTarget::Delete => SchedulerEffect::Delete,
            SchedulerHitTarget::Refresh => SchedulerEffect::Refresh,
            SchedulerHitTarget::OpenConversation => SchedulerEffect::OpenConversation,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, single_layout: bool) -> SchedulerEffect {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h')
                if single_layout && self.surface == SchedulerSurface::Detail =>
            {
                self.surface = SchedulerSurface::Tasks;
                SchedulerEffect::Handled
            }
            KeyCode::Esc | KeyCode::F(4) => SchedulerEffect::Close,
            KeyCode::Char('n') => SchedulerEffect::New,
            KeyCode::Char('e') => SchedulerEffect::Edit,
            KeyCode::Tab | KeyCode::BackTab => {
                self.runs_focused = !self.runs_focused;
                SchedulerEffect::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => SchedulerEffect::MoveSelection(1),
            KeyCode::Up | KeyCode::Char('k') => SchedulerEffect::MoveSelection(-1),
            KeyCode::Enter if self.runs_focused => SchedulerEffect::OpenConversation,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if single_layout {
                    self.surface = SchedulerSurface::Detail;
                }
                SchedulerEffect::Handled
            }
            KeyCode::Char(' ') => SchedulerEffect::Toggle,
            KeyCode::Char('r') => SchedulerEffect::RunNow,
            KeyCode::Char('d') => SchedulerEffect::Delete,
            KeyCode::Char('o') => SchedulerEffect::Refresh,
            KeyCode::Char('v') => SchedulerEffect::OpenConversation,
            _ => SchedulerEffect::Handled,
        }
    }

    fn toggle_prompt_expansion(&mut self, width: usize) {
        let Some(composer) = self.composer.as_mut() else {
            return;
        };
        composer.field = SchedulerField::Prompt;
        composer.prompt_expanded = !composer.prompt_expanded;
        composer.prompt.focus();
        let height = if composer.prompt_expanded { 20 } else { 5 };
        composer.prompt_scroll = composer
            .prompt
            .visual_cursor_row(width)
            .saturating_sub(height - 1);
    }
}

fn worktree_label(worktree: &crate::git::LinkedWorktree) -> String {
    if worktree.is_main {
        "basetree".to_owned()
    } else {
        worktree
            .path
            .file_name()
            .unwrap_or(worktree.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}

impl App {
    pub(crate) fn scheduler_destinations(&self) -> Vec<SchedulerDestination> {
        self.linked_worktrees
            .snapshot()
            .repositories
            .iter()
            .flat_map(|repository| {
                let worktrees = repository
                    .worktrees
                    .iter()
                    .filter(|worktree| !worktree.is_bare && !worktree.prunable)
                    .collect::<Vec<_>>();
                let Some(repository_root) = worktrees
                    .iter()
                    .find(|worktree| worktree.is_main)
                    .or_else(|| worktrees.first())
                    .map(|worktree| worktree.path.clone())
                else {
                    return Vec::new();
                };
                let local_tracking = repository
                    .branches
                    .iter()
                    .filter(|branch| !branch.remote)
                    .filter_map(|branch| {
                        branch
                            .upstream
                            .as_deref()
                            .map(|upstream| ((branch.name.as_str(), upstream), ()))
                    })
                    .collect::<HashMap<_, _>>();
                let worktrees_by_branch = worktrees
                    .iter()
                    .filter_map(|worktree| {
                        worktree
                            .branch
                            .as_deref()
                            .and_then(|branch| branch.strip_prefix("refs/heads/"))
                            .map(|branch| (branch, *worktree))
                    })
                    .collect::<HashMap<_, _>>();
                let mut destinations = repository
                    .branches
                    .iter()
                    .map(|branch| {
                        let checkout_branch = if branch.remote {
                            branch
                                .name
                                .split_once('/')
                                .map_or(branch.name.as_str(), |(_, branch)| branch)
                        } else {
                            &branch.name
                        };
                        let local_tracks_remote = !branch.remote
                            || local_tracking
                                .contains_key(&(checkout_branch, branch.name.as_str()));
                        let worktree = local_tracks_remote
                            .then(|| worktrees_by_branch.get(checkout_branch).copied())
                            .flatten();
                        SchedulerDestination {
                            path: worktree.map(|worktree| worktree.path.clone()),
                            repository_root: repository_root.clone(),
                            repository: repository.label.clone(),
                            branch: branch.clone(),
                            checkout_branch: checkout_branch.to_owned(),
                            worktree: worktree.map(|worktree| worktree_label(worktree)),
                        }
                    })
                    .collect::<Vec<_>>();
                let unmatched_worktrees = {
                    let destination_paths = destinations
                        .iter()
                        .filter_map(|destination| destination.path.as_deref())
                        .collect::<HashSet<_>>();
                    worktrees
                        .into_iter()
                        .filter(|worktree| !destination_paths.contains(worktree.path.as_path()))
                        .collect::<Vec<_>>()
                };
                for worktree in unmatched_worktrees {
                    let branch = worktree
                        .branch
                        .as_deref()
                        .and_then(|branch| branch.strip_prefix("refs/heads/"))
                        .unwrap_or("detached HEAD");
                    destinations.push(SchedulerDestination {
                        path: Some(worktree.path.clone()),
                        repository_root: repository_root.clone(),
                        repository: repository.label.clone(),
                        branch: Branch {
                            name: branch.to_owned(),
                            upstream: None,
                            remote: false,
                            current: false,
                            default: false,
                            last_touched_at: None,
                        },
                        checkout_branch: branch.to_owned(),
                        worktree: Some(worktree_label(worktree)),
                    });
                }
                destinations
            })
            .collect()
    }

    pub(crate) fn open_scheduler(&mut self) {
        if !self.scheduled_tasks.is_available() {
            return;
        }
        self.header_picker.close();
        self.linked_worktrees.request_branches();
        self.mode = Mode::Scheduler;
        self.scheduler.surface = SchedulerSurface::Tasks;
        self.scheduler.composer = None;
        self.scheduler.error = self
            .linked_worktrees
            .snapshot()
            .repositories
            .iter()
            .find_map(|repository| {
                repository.branch_error.as_ref().map(|error| {
                    format!("Could not load branches for {}: {error}", repository.label)
                })
            });
        self.discover_active_project_tasks();
        self.sync_scheduler_selection();
    }

    pub(crate) fn discover_active_project_tasks(&mut self) {
        if !self.scheduled_tasks.is_available() {
            return;
        }
        let Some(repository) = self.repository() else {
            return;
        };
        let root = repository.root.clone();
        let repository_identity = repository
            .common_dir
            .clone()
            .unwrap_or_else(|| root.clone());
        let branch = repository.branch.clone();
        let repository = self
            .scheduler_destinations()
            .into_iter()
            .find(|destination| {
                destination
                    .path
                    .as_deref()
                    .is_some_and(|path| same_path(path, &root))
            })
            .map(|destination| destination.repository)
            .unwrap_or_else(|| {
                root.file_name()
                    .unwrap_or(root.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            });
        if let Err(error) = self.scheduled_tasks.discover_project_tasks(
            ScheduledTaskDestination {
                path: root,
                repository,
                branch,
            },
            repository_identity,
        ) {
            self.scheduler.error = Some(error);
        }
    }

    pub(crate) fn sync_scheduler_catalog(&mut self) {
        if self.mode != Mode::Scheduler {
            return;
        }
        self.scheduler.error = self
            .linked_worktrees
            .snapshot()
            .repositories
            .iter()
            .find_map(|repository| {
                repository.branch_error.as_ref().map(|error| {
                    format!("Could not load branches for {}: {error}", repository.label)
                })
            });
        let destinations = self.scheduler_destinations();
        if let Some(composer) = &mut self.scheduler.composer {
            composer.sync_destinations(destinations);
        }
    }

    pub(crate) fn sync_scheduler_selection(&mut self) {
        let tasks = self.scheduled_tasks.tasks();
        if !tasks
            .iter()
            .any(|task| Some(task.id) == self.scheduler.selected_task_id)
        {
            self.scheduler.selected_task_id = tasks.first().map(|task| task.id);
        }
        self.select_default_scheduled_run();
        if self.scheduler.preview_pending && self.scheduler.selected_run_id.is_some() {
            self.scheduler.preview_pending = false;
            self.open_selected_scheduled_run_conversation();
        }
    }

    pub(crate) fn close_scheduler(&mut self) {
        if self.scheduler.pending_worktree.is_some() {
            self.scheduler.error = Some("Wait for the linked worktree to be created".to_owned());
            return;
        }
        self.scheduler.composer = None;
        self.scheduler.error = None;
        self.mode = Mode::Normal;
    }

    pub(crate) fn begin_scheduled_task(&mut self) {
        let destinations = self.scheduler_destinations();
        let webhooks = self.scheduler_discord_webhooks();
        self.scheduler.composer = Some(ScheduledTaskComposer::new(destinations, webhooks));
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.error = None;
    }

    pub(crate) fn edit_selected_scheduled_task(&mut self) {
        let Some(task) = self.selected_scheduled_task().cloned() else {
            return;
        };
        let destinations = self.scheduler_destinations();
        let webhooks = self.scheduler_discord_webhooks();
        self.scheduler.composer = Some(ScheduledTaskComposer::edit(&task, destinations, webhooks));
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.error = None;
    }

    pub(crate) fn cancel_scheduled_task(&mut self) {
        if self.scheduler.pending_worktree.is_some() {
            self.scheduler.error = Some("Wait for the linked worktree to be created".to_owned());
            return;
        }
        self.scheduler.composer = None;
        self.scheduler.error = None;
        if self.layout_profile().is_single() {
            self.scheduler.surface = SchedulerSurface::Tasks;
        }
    }

    pub(crate) fn scheduled_runs_for_selected_task(&self) -> Vec<&ScheduledRun> {
        let Some(selected) = self.scheduler.selected_task_id else {
            return Vec::new();
        };
        self.scheduled_tasks
            .runs()
            .iter()
            .filter(|run| run.task_id == selected)
            .collect()
    }

    fn select_default_scheduled_run(&mut self) {
        let task_id = self.scheduler.selected_task_id;
        let runs = self.scheduled_tasks.runs();
        let selected = self.scheduler.selected_run_id;
        if !runs
            .iter()
            .any(|run| Some(run.task_id) == task_id && Some(run.id) == selected)
        {
            self.scheduler.selected_run_id = runs
                .iter()
                .find(|run| Some(run.task_id) == task_id)
                .map(|run| run.id);
        }
    }

    pub(crate) fn select_scheduled_task(&mut self, id: i64) {
        if !self
            .scheduled_tasks
            .tasks()
            .iter()
            .any(|task| task.id == id)
        {
            return;
        }
        self.scheduler.selected_task_id = Some(id);
        self.scheduler.selected_run_id = None;
        self.scheduler.runs_focused = false;
        self.scheduler.run_scroll = 0;
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.composer = None;
        self.scheduler.error = None;
        self.select_default_scheduled_run();
    }

    pub(crate) fn select_scheduled_run(&mut self, id: i64) {
        if self
            .scheduled_tasks
            .runs()
            .iter()
            .any(|run| Some(run.task_id) == self.scheduler.selected_task_id && run.id == id)
        {
            self.scheduler.selected_run_id = Some(id);
            self.scheduler.runs_focused = true;
        }
    }

    pub(crate) fn activate_scheduler_target(&mut self, target: SchedulerHitTarget) {
        let prompt_width = self.scheduler_prompt_size(5).0;
        let effect = self.scheduler.activate_target(target, prompt_width);
        self.apply_scheduler_effect(effect);
    }

    pub(crate) fn handle_scheduler(&mut self, key: KeyEvent) {
        if self.scheduler.composer.is_some() {
            self.handle_scheduler_composer(key);
            return;
        }
        let single_layout = self.layout_profile().is_single();
        let effect = self.scheduler.handle_key(key, single_layout);
        self.apply_scheduler_effect(effect);
    }

    fn apply_scheduler_effect(&mut self, effect: SchedulerEffect) {
        match effect {
            SchedulerEffect::Handled => {}
            SchedulerEffect::Close => self.close_scheduler(),
            SchedulerEffect::Cancel => self.cancel_scheduled_task(),
            SchedulerEffect::New => self.begin_scheduled_task(),
            SchedulerEffect::Edit => self.edit_selected_scheduled_task(),
            SchedulerEffect::Save => self.save_scheduled_task(),
            SchedulerEffect::SelectTask(id) => self.select_scheduled_task(id),
            SchedulerEffect::SelectRun(id) => self.select_scheduled_run(id),
            SchedulerEffect::MoveSelection(delta) => self.move_scheduler_selection(delta),
            SchedulerEffect::Toggle => self.toggle_selected_scheduled_task(),
            SchedulerEffect::RunNow => self.run_selected_scheduled_task(),
            SchedulerEffect::Delete => self.delete_selected_scheduled_task(),
            SchedulerEffect::Refresh => self.refresh_selected_scheduled_run(),
            SchedulerEffect::OpenConversation => self.open_selected_scheduled_run_conversation(),
        }
    }

    fn handle_scheduler_composer(&mut self, key: KeyEvent) {
        if self
            .scheduler
            .composer
            .as_ref()
            .is_some_and(|composer| composer.project)
        {
            match key.code {
                KeyCode::Esc => self.cancel_scheduled_task(),
                KeyCode::Enter | KeyCode::Char('s')
                    if key.code == KeyCode::Enter
                        || key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.save_scheduled_task()
                }
                KeyCode::Left
                | KeyCode::Char('h')
                | KeyCode::Right
                | KeyCode::Char('l')
                | KeyCode::Char(' ') => {
                    self.scheduler
                        .composer
                        .as_mut()
                        .unwrap()
                        .cycle_discord_webhook(matches!(
                            key.code,
                            KeyCode::Left | KeyCode::Char('h')
                        ));
                }
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Esc {
            let composer = self.scheduler.composer.as_mut().unwrap();
            if composer.destination_picker_open() {
                composer.close_destination_picker();
                return;
            }
            if composer.prompt_expanded {
                let width = self.scheduler_prompt_size(5).0;
                self.scheduler.toggle_prompt_expansion(width);
            } else {
                self.cancel_scheduled_task();
            }
            return;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            let composer = self.scheduler.composer.as_mut().unwrap();
            composer.close_destination_picker();
            composer.focus(composer.field.next(key.code == KeyCode::BackTab));
            return;
        }
        let field = self.scheduler.composer.as_ref().unwrap().field;
        if (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Enter
                && !matches!(
                    field,
                    SchedulerField::Prompt | SchedulerField::Discord | SchedulerField::Destination
                ))
        {
            self.save_scheduled_task();
            return;
        }
        if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let width = self.scheduler_prompt_size(5).0;
            self.scheduler.toggle_prompt_expansion(width);
            return;
        }
        if field == SchedulerField::Destination {
            self.handle_scheduler_destination(key);
            return;
        }
        if field == SchedulerField::Discord {
            if matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Char('h')
                    | KeyCode::Right
                    | KeyCode::Char('l')
                    | KeyCode::Char(' ')
            ) {
                self.scheduler
                    .composer
                    .as_mut()
                    .unwrap()
                    .cycle_discord_webhook(matches!(key.code, KeyCode::Left | KeyCode::Char('h')));
            }
            return;
        }
        let prompt_width =
            (field == SchedulerField::Prompt).then(|| self.scheduler_prompt_size(5).0);
        let composer = self.scheduler.composer.as_mut().unwrap();
        let handled = if field == SchedulerField::Prompt {
            match key.code {
                KeyCode::Enter => composer.prompt.insert_char('\n'),
                KeyCode::Up => composer.prompt.move_up(prompt_width.unwrap()),
                KeyCode::Down => composer.prompt.move_down(prompt_width.unwrap()),
                _ => _ = composer.prompt.handle_edit_key(key),
            }
            true
        } else if field == SchedulerField::Schedule {
            match key.code {
                KeyCode::Char(character)
                    if character.is_ascii_digit()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    composer.schedule.insert_char(character);
                    true
                }
                KeyCode::Char(_) => false,
                _ => composer.schedule.handle_edit_key(key) != EditOutcome::Unhandled,
            }
        } else {
            composer.input_mut(field).handle_edit_key(key) != EditOutcome::Unhandled
        };
        if handled {
            self.scheduler.error = None;
        }
        if field == SchedulerField::Prompt {
            self.sync_scheduler_prompt_scroll();
        }
    }

    fn handle_scheduler_destination(&mut self, key: KeyEvent) {
        let viewport = self.scheduler_destination_viewport();
        let composer = self.scheduler.composer.as_mut().unwrap();
        if !composer.destination_picker_open() {
            match key.code {
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                    composer.destination_card = composer
                        .destination_card
                        .next(matches!(key.code, KeyCode::Left | KeyCode::Char('h')));
                }
                KeyCode::Enter | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char(' ') => {
                    composer.open_destination_picker(composer.destination_card);
                }
                _ => {}
            }
            return;
        }
        composer.destination_picker.set_viewport_rows(viewport);
        match key.code {
            KeyCode::Enter => composer.select_destination_picker_item(),
            KeyCode::Down => composer.destination_picker.move_selection(1),
            KeyCode::Up => composer.destination_picker.move_selection(-1),
            KeyCode::PageDown | KeyCode::PageUp => {
                let delta = if key.code == KeyCode::PageDown { 1 } else { -1 };
                composer.destination_picker.move_selection_page(delta);
            }
            _ => {
                if composer.destination_picker.query.handle_edit_key(key) != EditOutcome::Unhandled
                {
                    composer.destination_picker.apply_filter();
                }
            }
        }
    }

    pub(crate) fn paste_scheduler(&mut self, text: &str) {
        let Some(composer) = self.scheduler.composer.as_mut() else {
            return;
        };
        let field = composer.field;
        match field {
            SchedulerField::Prompt => composer.prompt.insert(text),
            SchedulerField::Schedule => composer.schedule.insert(
                &text
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>(),
            ),
            SchedulerField::Destination if composer.destination_picker_open() => {
                composer.destination_picker.query.insert_single_line(text);
                composer.destination_picker.apply_filter();
            }
            SchedulerField::Destination => return,
            SchedulerField::Discord => return,
            _ => composer.input_mut(field).insert_single_line(text),
        }
        self.scheduler.error = None;
        if field == SchedulerField::Prompt {
            self.sync_scheduler_prompt_scroll();
        }
    }

    pub(crate) fn poll_scheduler_inputs(&mut self) -> bool {
        let scheduler_active = self.mode == Mode::Scheduler;
        let picker_roots = self
            .scheduler
            .composer
            .as_ref()
            .filter(|composer| composer.destination_picker_open())
            .map(|composer| composer.destination_picker.change_stats_roots())
            .unwrap_or_default();
        self.linked_worktrees.request_stats(picker_roots.clone());
        let picker_stats = picker_roots
            .into_iter()
            .filter_map(|root| {
                self.linked_worktrees
                    .change_stats(&root)
                    .map(|stats| (root, stats))
            })
            .collect::<Vec<_>>();
        let Some(composer) = self.scheduler.composer.as_mut() else {
            return false;
        };
        let field = composer.field;
        let destination_picker_open = composer.destination_picker_open();
        let mut changed = composer
            .destination_picker
            .query
            .poll_blink(scheduler_active && destination_picker_open);
        changed |= composer.destination_picker.sync_change_stats(&picker_stats);
        for input_field in [
            SchedulerField::Title,
            SchedulerField::Description,
            SchedulerField::Prompt,
            SchedulerField::Model,
            SchedulerField::Schedule,
        ] {
            let focused = scheduler_active && field == input_field;
            changed |= composer.input_mut(input_field).poll_blink(focused);
        }
        changed
    }

    fn save_scheduled_task(&mut self) {
        if self.scheduler.pending_worktree.is_some() {
            return;
        }
        let Some(composer) = self.scheduler.composer.as_ref() else {
            return;
        };
        if composer.project {
            let task_id = composer.task_id.unwrap();
            match self
                .scheduled_tasks
                .configure_project_task(task_id, composer.discord_webhook_id())
            {
                Ok(()) => {
                    self.scheduler.composer = None;
                    self.scheduler.error = None;
                    self.notice = Some("Project task configuration saved".to_owned());
                }
                Err(error) => self.scheduler.error = Some(error),
            }
            return;
        }
        let destination_index = composer.destination;
        let Some(destination) = composer.destinations.get(destination_index).cloned() else {
            self.scheduler.error = Some("Select a branch destination".to_owned());
            return;
        };
        let Some(destination_path) = destination.path.clone() else {
            match self.linked_worktrees.create_worktree_for_branch(
                destination.repository_root,
                destination.branch.name.clone(),
                destination.branch.remote,
            ) {
                Ok(()) => {
                    self.scheduler.pending_worktree = Some(destination_index);
                    self.scheduler.error = None;
                    self.notice = Some(format!(
                        "Creating a worktree for {}…",
                        destination.branch.name
                    ));
                }
                Err(error) => self.scheduler.error = Some(error),
            }
            return;
        };
        let edit = ScheduledTaskEdit {
            title: composer.title.text().to_owned(),
            description: composer.description.text().to_owned(),
            prompt: composer.prompt.text().to_owned(),
            model: composer.model.text().trim().to_owned(),
            discord_webhook_id: composer.discord_webhook_id(),
            destination: destination_path.clone(),
            repository: destination.repository.clone(),
            branch: destination.checkout_branch.clone(),
            enabled: composer.enabled,
            interval_minutes: composer.schedule.text().parse().unwrap_or(0),
        };
        let task_id = composer.task_id;
        match self.scheduled_tasks.save_task(task_id, edit) {
            Ok(()) => {
                self.scheduler.composer = None;
                self.scheduler.error = None;
                self.scheduler.surface = if task_id.is_some() {
                    SchedulerSurface::Detail
                } else {
                    SchedulerSurface::Tasks
                };
                self.notice = Some("Scheduled task saved".to_owned());
            }
            Err(error) => self.scheduler.error = Some(error),
        }
    }

    fn scheduler_discord_webhooks(&self) -> Vec<(String, String)> {
        self.discord_webhooks
            .iter()
            .map(|webhook| {
                (
                    webhook.id.clone(),
                    format!(
                        "{} / #{} / {}",
                        webhook.server, webhook.channel, webhook.webhook_name
                    ),
                )
            })
            .collect()
    }

    pub(crate) fn finish_scheduler_worktree_creation(&mut self, result: Result<PathBuf, String>) {
        let Some(destination_index) = self.scheduler.pending_worktree.take() else {
            return;
        };
        match result {
            Ok(path) => {
                let Some(composer) = self.scheduler.composer.as_mut() else {
                    return;
                };
                let Some(destination) = composer.destinations.get_mut(destination_index) else {
                    return;
                };
                destination.worktree = Some(
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                );
                destination.path = Some(path);
                self.linked_worktrees.refresh_after_topology_change();
                self.save_scheduled_task();
            }
            Err(error) => {
                self.scheduler.error = Some(format!("Could not create worktree: {error}"));
            }
        }
    }

    pub(crate) fn selected_scheduled_task(&self) -> Option<&ScheduledTask> {
        let selected = self.scheduler.selected_task_id?;
        self.scheduled_tasks
            .tasks()
            .iter()
            .find(|task| task.id == selected)
    }

    fn toggle_selected_scheduled_task(&mut self) {
        let Some((id, enabled)) = self
            .selected_scheduled_task()
            .map(|task| (task.id, !task.enabled))
        else {
            return;
        };
        self.scheduler.error = self.scheduled_tasks.toggle_task(id, enabled).err();
    }

    fn run_selected_scheduled_task(&mut self) {
        let Some(id) = self.selected_scheduled_task().map(|task| task.id) else {
            return;
        };
        self.scheduler.error = self.scheduled_tasks.run_now(id).err();
        if self.scheduler.error.is_none() {
            // The next scheduler update contains the newly claimed run at the front.
            self.scheduler.selected_run_id = None;
            self.scheduler.run_scroll = 0;
            self.scheduler.surface = SchedulerSurface::Detail;
            self.scheduler.preview_pending = true;
            self.agent_preview.clear_scheduled_conversation();
        }
    }

    fn delete_selected_scheduled_task(&mut self) {
        let Some(id) = self.selected_scheduled_task().map(|task| task.id) else {
            return;
        };
        if let Err(error) = self.scheduled_tasks.delete_task(id) {
            self.scheduler.error = Some(error);
            return;
        }
        self.scheduler.selected_task_id = None;
        self.scheduler.selected_run_id = None;
        self.scheduler.surface = SchedulerSurface::Tasks;
        self.scheduler.error = None;
    }

    fn refresh_selected_scheduled_run(&mut self) {
        let Some(id) = self.scheduler.selected_run_id else {
            return;
        };
        self.scheduler.error = self.scheduled_tasks.refresh_run(id).err();
    }

    fn open_selected_scheduled_run_conversation(&mut self) {
        let Some(run_id) = self.scheduler.selected_run_id else {
            self.scheduler.error = Some("Select a run to open its conversation".to_owned());
            return;
        };
        self.open_scheduled_run_conversation(run_id, Mode::Scheduler);
    }

    pub(super) fn open_scheduled_run_preview(&mut self, run_id: i64) {
        if self.mode == Mode::Commit {
            self.flush_commit_draft();
        }
        self.open_scheduled_run_conversation(run_id, Mode::Normal);
    }

    pub(super) fn promote_scheduled_run(&mut self, run_id: i64) {
        let (destination, session_id) = match self.scheduled_run_promotion(run_id) {
            Ok(promotion) => promotion,
            Err(error) => {
                self.notice = Some(error);
                return;
            }
        };
        match self
            .herdr_prompt
            .prepare_stashed_agent(destination, session_id)
        {
            Ok(()) => self.notice = Some("Loading active Herdr tab layout".to_owned()),
            Err(error) => self.notice = Some(format!("Could not open scheduled agent: {error}")),
        }
    }

    pub(super) fn scheduled_run_promotion(&self, run_id: i64) -> Result<(PathBuf, String), String> {
        let run = self
            .scheduled_tasks
            .runs()
            .iter()
            .find(|run| run.id == run_id)
            .ok_or_else(|| "Scheduled run is no longer available".to_owned())?;
        let session_id = run
            .session_id
            .clone()
            .ok_or_else(|| "Scheduled run has not started an OpenCode session yet".to_owned())?;
        let task = self
            .scheduled_tasks
            .tasks()
            .iter()
            .find(|task| task.id == run.task_id)
            .ok_or_else(|| "Scheduled task is no longer available".to_owned())?;
        Ok((task.destination.clone(), session_id))
    }

    fn open_scheduled_run_conversation(&mut self, run_id: i64, return_mode: Mode) {
        let Some(run) = self
            .scheduled_tasks
            .runs()
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
        else {
            self.notice = Some("Scheduled run is no longer available".to_owned());
            return;
        };
        self.agent_preview.open_scheduled_run(run.id, return_mode);
        if let Some(session_id) = run.session_id.as_deref() {
            self.agent_preview
                .refresh_scheduled_conversation(session_id);
        } else if let Some(task) = self
            .scheduled_tasks
            .tasks()
            .iter()
            .find(|task| task.id == run.task_id)
        {
            self.agent_preview.request_scheduled_session(
                run.id,
                task.destination.clone(),
                task.prompt.clone(),
                run.created_at_ms,
            );
        }
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.error = None;
        self.mode = Mode::AgentPreview;
    }

    fn move_scheduler_selection(&mut self, delta: isize) {
        let next = |current: usize, len: usize| {
            current
                .saturating_add_signed(delta)
                .min(len.saturating_sub(1))
        };
        if self.scheduler.runs_focused {
            let runs = self.scheduled_runs_for_selected_task();
            let current = self
                .scheduler
                .selected_run_id
                .and_then(|id| runs.iter().position(|run| run.id == id))
                .unwrap_or(0);
            let next = next(current, runs.len());
            if let Some(run) = runs.get(next) {
                self.scheduler.selected_run_id = Some(run.id);
                self.scheduler.run_scroll = next;
            }
            return;
        }
        let tasks = self.scheduled_tasks.tasks();
        let current = self
            .scheduler
            .selected_task_id
            .and_then(|id| tasks.iter().position(|task| task.id == id))
            .unwrap_or(0);
        let next = next(current, tasks.len());
        if let Some(id) = tasks.get(next).map(|task| task.id) {
            self.select_scheduled_task(id);
            self.scheduler.task_scroll = next;
            if !self.layout_profile().is_single() {
                self.scheduler.surface = SchedulerSurface::Tasks;
            }
        }
    }

    pub(crate) fn scroll_scheduler(&mut self, target: ScrollTarget, delta: isize) {
        if target == ScrollTarget::SchedulerPrompt {
            let (width, height) = self.scheduler_prompt_size(1);
            let Some(composer) = self.scheduler.composer.as_mut() else {
                return;
            };
            composer.field = SchedulerField::Prompt;
            let maximum = composer.prompt.visual_height(width).saturating_sub(height);
            composer.prompt_scroll = composer
                .prompt_scroll
                .saturating_add_signed(delta.saturating_mul(3))
                .min(maximum);
            return;
        }
        if target == ScrollTarget::SchedulerDestinations {
            let viewport = self.scheduler_destination_viewport();
            let Some(composer) = self.scheduler.composer.as_mut() else {
                return;
            };
            composer.field = SchedulerField::Destination;
            if !composer.destination_picker_open() {
                composer.open_destination_picker(composer.destination_card);
            }
            composer.destination_picker.set_viewport_rows(viewport);
            composer.destination_picker.scroll_by(delta);
            return;
        }
        let scroll = match target {
            ScrollTarget::SchedulerTasks => &mut self.scheduler.task_scroll,
            ScrollTarget::SchedulerRuns => &mut self.scheduler.run_scroll,
            _ => return,
        };
        *scroll = scroll.saturating_add_signed(delta.saturating_mul(3));
    }

    fn sync_scheduler_prompt_scroll(&mut self) {
        let (width, height) = self.scheduler_prompt_size(5);
        let Some(composer) = self.scheduler.composer.as_mut() else {
            return;
        };
        let cursor_row = composer.prompt.visual_cursor_row(width);
        if cursor_row < composer.prompt_scroll {
            composer.prompt_scroll = cursor_row;
        } else if cursor_row >= composer.prompt_scroll + height {
            composer.prompt_scroll = cursor_row + 1 - height;
        }
        let maximum = composer.prompt.visual_height(width).saturating_sub(height);
        composer.prompt_scroll = composer.prompt_scroll.min(maximum);
    }

    fn scheduler_prompt_size(&self, fallback_height: usize) -> (usize, usize) {
        self.regions
            .hit_target_rect(HitTarget::Scheduler(SchedulerHitTarget::Field(
                SchedulerField::Prompt,
            )))
            .map_or((1, fallback_height), |rect| {
                (
                    usize::from(rect.width).max(1),
                    usize::from(rect.height).max(1),
                )
            })
    }

    fn scheduler_destination_viewport(&self) -> usize {
        self.regions
            .scroll_target_rect(ScrollTarget::SchedulerDestinations)
            .map_or(1, |rect| usize::from(rect.height).max(1))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        Branch, ScheduledTaskComposer, SchedulerDestination, SchedulerEffect, SchedulerHitTarget,
        SchedulerState, SchedulerSurface,
    };

    #[test]
    fn scheduler_state_owns_navigation_and_semantic_targets() {
        let mut state = SchedulerState {
            surface: SchedulerSurface::Detail,
            ..SchedulerState::default()
        };

        let effect = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true);
        assert_eq!(effect, SchedulerEffect::Handled);
        assert_eq!(state.surface, SchedulerSurface::Tasks);
        assert_eq!(
            state.activate_target(SchedulerHitTarget::Run(17), 1),
            SchedulerEffect::SelectRun(17)
        );
    }

    fn destination(path: &str, branch: &str) -> SchedulerDestination {
        SchedulerDestination {
            path: Some(PathBuf::from(path)),
            repository_root: PathBuf::from("/repo"),
            repository: "repo".to_owned(),
            branch: Branch {
                name: branch.to_owned(),
                upstream: None,
                remote: false,
                current: branch == "main",
                default: branch == "main",
                last_touched_at: None,
            },
            checkout_branch: branch.to_owned(),
            worktree: Some("worktree".to_owned()),
        }
    }

    #[test]
    fn branch_refresh_preserves_the_composer_destination() {
        let selected = destination("/repo/feature", "feature");
        let mut composer = ScheduledTaskComposer::new(vec![selected.clone()], Vec::new());
        composer.title.set("Retained task");

        composer.sync_destinations(vec![destination("/repo", "main"), selected.clone()]);

        assert_eq!(composer.destinations[composer.destination], selected);
        assert_eq!(composer.title.text(), "Retained task");
    }
}
