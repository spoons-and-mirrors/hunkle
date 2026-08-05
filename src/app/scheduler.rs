use super::*;

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
    pub(crate) source: Option<ScheduledTaskSource>,
    pub(crate) title: TextInput,
    pub(crate) description: TextInput,
    pub(crate) prompt: TextInput,
    pub(crate) model: TextInput,
    pub(crate) prompt_expanded: bool,
    pub(crate) prompt_scroll: usize,
    pub(crate) schedule: TextInput,
    pub(crate) destinations: Vec<SchedulerDestination>,
    pub(crate) destination: usize,
    pub(crate) destination_card: SchedulerDestinationCard,
    pub(crate) destination_picker: HeaderPicker,
    pub(crate) field: SchedulerField,
}

impl ScheduledTaskComposer {
    fn new(destinations: Vec<SchedulerDestination>) -> Self {
        let mut schedule = TextInput::default();
        schedule.set("15");
        Self {
            task_id: None,
            enabled: true,
            source: None,
            title: TextInput::default(),
            description: TextInput::default(),
            prompt: TextInput::default(),
            model: TextInput::default(),
            prompt_expanded: false,
            prompt_scroll: 0,
            schedule,
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
        source: Option<ScheduledTaskSource>,
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
        let mut composer = Self::new(destinations);
        composer.task_id = Some(task.id);
        composer.enabled = task.enabled;
        composer.source = source;
        composer.destination = destination;
        composer.title.set(&task.title);
        composer.description.set(&task.description);
        composer.prompt.set(&task.prompt);
        composer.model.set(&task.model);
        composer.schedule.set(&task.interval_minutes.to_string());
        composer
    }

    fn destination_indices(&self, card: SchedulerDestinationCard) -> Vec<usize> {
        let Some(selected) = self.destinations.get(self.destination) else {
            return Vec::new();
        };
        let mut candidates: Vec<usize> = Vec::new();
        for (index, destination) in self.destinations.iter().enumerate() {
            if card == SchedulerDestinationCard::Worktree && destination.path.is_none() {
                continue;
            }
            if (card == SchedulerDestinationCard::Repository
                || destination.repository_root == selected.repository_root)
                && !candidates.iter().any(|candidate| match card {
                    SchedulerDestinationCard::Repository => {
                        self.destinations[*candidate].repository_root == destination.repository_root
                    }
                    SchedulerDestinationCard::Worktree => {
                        self.destinations[*candidate].path == destination.path
                    }
                    SchedulerDestinationCard::Branch => {
                        self.destinations[*candidate].branch.name == destination.branch.name
                    }
                })
            {
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
        self.destination_picker.start_change_details();
    }

    fn close_destination_picker(&mut self) {
        self.destination_picker.close();
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
            SchedulerField::Destination => unreachable!(),
        }
    }

    fn input_mut(&mut self, field: SchedulerField) -> &mut TextInput {
        match field {
            SchedulerField::Title => &mut self.title,
            SchedulerField::Description => &mut self.description,
            SchedulerField::Prompt => &mut self.prompt,
            SchedulerField::Model => &mut self.model,
            SchedulerField::Schedule => &mut self.schedule,
            SchedulerField::Destination => unreachable!(),
        }
    }

    fn focus(&mut self, field: SchedulerField) {
        self.field = field;
        if field != SchedulerField::Destination {
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
    pub(crate) conversation_scroll: Option<usize>,
    pub(crate) conversation_scroll_max: usize,
    pub(crate) conversation_message: Option<usize>,
    pub(crate) conversation_expanded_requests: Vec<usize>,
    pub(crate) runs_focused: bool,
    pub(crate) error: Option<String>,
    pub(crate) pending_worktree: Option<usize>,
    pub(crate) preview_pending: bool,
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
                            || repository.branches.iter().any(|local| {
                                !local.remote
                                    && local.name == checkout_branch
                                    && local.upstream.as_deref() == Some(branch.name.as_str())
                            });
                        let worktree = local_tracks_remote
                            .then(|| {
                                worktrees.iter().find(|worktree| {
                                    worktree
                                        .branch
                                        .as_deref()
                                        .and_then(|branch| branch.strip_prefix("refs/heads/"))
                                        == Some(checkout_branch)
                                })
                            })
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
                for worktree in worktrees {
                    if destinations.iter().any(|destination| {
                        destination
                            .path
                            .as_deref()
                            .is_some_and(|path| same_path(path, &worktree.path))
                    }) {
                        continue;
                    }
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
        if !self.herdr_available() {
            return;
        }
        self.header_picker.close();
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
        let destinations = self
            .scheduler_destinations()
            .into_iter()
            .filter_map(|destination| {
                Some(ScheduledTaskDestination {
                    path: destination.path?,
                    repository: destination.repository,
                    branch: destination.checkout_branch,
                })
            })
            .collect();
        if let Err(error) = self.herdr.sync_scheduled_task_files(destinations) {
            self.scheduler.error = Some(error);
        }
        self.sync_scheduler_selection();
    }

    pub(crate) fn sync_scheduler_selection(&mut self) {
        let tasks = self.herdr.scheduled_tasks();
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
        self.herdr.clear_scheduled_conversation();
    }

    pub(crate) fn begin_scheduled_task(&mut self) {
        self.scheduler.composer = Some(ScheduledTaskComposer::new(self.scheduler_destinations()));
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.error = None;
    }

    pub(crate) fn edit_selected_scheduled_task(&mut self) {
        let Some(task) = self.selected_scheduled_task().cloned() else {
            return;
        };
        let source = match self.herdr.scheduled_task_source(&task) {
            Ok(source) => source,
            Err(error) => {
                self.scheduler.error = Some(error);
                return;
            }
        };
        self.scheduler.composer = Some(ScheduledTaskComposer::edit(
            &task,
            self.scheduler_destinations(),
            source,
        ));
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
        self.herdr
            .scheduled_runs()
            .iter()
            .filter(|run| run.task_id == selected)
            .collect()
    }

    fn select_default_scheduled_run(&mut self) {
        let task_id = self.scheduler.selected_task_id;
        let runs = self.herdr.scheduled_runs();
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
            .herdr
            .scheduled_tasks()
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
            .herdr
            .scheduled_runs()
            .iter()
            .any(|run| Some(run.task_id) == self.scheduler.selected_task_id && run.id == id)
        {
            self.scheduler.selected_run_id = Some(id);
            self.scheduler.runs_focused = true;
        }
    }

    pub(crate) fn activate_scheduler_target(&mut self, target: SchedulerHitTarget) {
        match target {
            SchedulerHitTarget::Close => self.close_scheduler(),
            SchedulerHitTarget::Back => self.cancel_scheduled_task(),
            SchedulerHitTarget::New => self.begin_scheduled_task(),
            SchedulerHitTarget::Edit => self.edit_selected_scheduled_task(),
            SchedulerHitTarget::Save => self.save_scheduled_task(),
            SchedulerHitTarget::Cancel => self.cancel_scheduled_task(),
            SchedulerHitTarget::Task(id) => self.select_scheduled_task(id),
            SchedulerHitTarget::Run(id) => self.select_scheduled_run(id),
            SchedulerHitTarget::Field(field) => {
                if let Some(composer) = self.scheduler.composer.as_mut() {
                    composer.focus(field);
                }
            }
            SchedulerHitTarget::PromptExpand => self.toggle_scheduler_prompt_expansion(),
            SchedulerHitTarget::DestinationPickerOverlay => {}
            SchedulerHitTarget::DestinationCard(card) => {
                if let Some(composer) = self.scheduler.composer.as_mut() {
                    composer.field = SchedulerField::Destination;
                    if composer.destination_picker_open() && composer.destination_card == card {
                        composer.close_destination_picker();
                    } else {
                        composer.open_destination_picker(card);
                    }
                }
            }
            SchedulerHitTarget::Destination(index) => {
                if let Some(composer) = self.scheduler.composer.as_mut()
                    && index < composer.destinations.len()
                {
                    composer.destination = index;
                    composer.close_destination_picker();
                }
            }
            SchedulerHitTarget::Toggle => self.toggle_selected_scheduled_task(),
            SchedulerHitTarget::RunNow => self.run_selected_scheduled_task(),
            SchedulerHitTarget::Delete => self.delete_selected_scheduled_task(),
            SchedulerHitTarget::Refresh => self.refresh_selected_scheduled_run(),
            SchedulerHitTarget::OpenConversation => {
                self.open_selected_scheduled_run_conversation();
            }
            SchedulerHitTarget::ConversationRequest(request) => {
                if let Some(index) = self
                    .scheduler
                    .conversation_expanded_requests
                    .iter()
                    .position(|value| *value == request)
                {
                    self.scheduler.conversation_expanded_requests.remove(index);
                } else {
                    self.scheduler.conversation_expanded_requests.push(request);
                }
            }
        }
    }

    pub(crate) fn handle_scheduler(&mut self, key: KeyEvent) {
        if self.scheduler.composer.is_some() {
            self.handle_scheduler_composer(key);
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h')
                if self.layout_profile().is_single()
                    && self.scheduler.surface == SchedulerSurface::Detail =>
            {
                self.scheduler.surface = SchedulerSurface::Tasks;
            }
            KeyCode::Esc | KeyCode::F(4) => self.close_scheduler(),
            KeyCode::Char('n') => self.begin_scheduled_task(),
            KeyCode::Char('e') => self.edit_selected_scheduled_task(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.scheduler.runs_focused = !self.scheduler.runs_focused;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_scheduler_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_scheduler_selection(-1),
            KeyCode::Enter if self.scheduler.runs_focused => {
                self.open_selected_scheduled_run_conversation();
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if self.layout_profile().is_single() {
                    self.scheduler.surface = SchedulerSurface::Detail;
                }
            }
            KeyCode::Char(' ') => self.toggle_selected_scheduled_task(),
            KeyCode::Char('r') => self.run_selected_scheduled_task(),
            KeyCode::Char('d') => self.delete_selected_scheduled_task(),
            KeyCode::Char('o') => self.refresh_selected_scheduled_run(),
            KeyCode::Char('v') => self.open_selected_scheduled_run_conversation(),
            _ => {}
        }
    }

    fn handle_scheduler_composer(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            let composer = self.scheduler.composer.as_mut().unwrap();
            if composer.destination_picker_open() {
                composer.close_destination_picker();
                return;
            }
            if composer.prompt_expanded {
                self.toggle_scheduler_prompt_expansion();
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
                && !matches!(field, SchedulerField::Prompt | SchedulerField::Destination))
        {
            self.save_scheduled_task();
            return;
        }
        if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.toggle_scheduler_prompt_expansion();
            return;
        }
        if field == SchedulerField::Destination {
            self.handle_scheduler_destination(key);
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
            _ => composer.input_mut(field).insert_single_line(text),
        }
        self.scheduler.error = None;
        if field == SchedulerField::Prompt {
            self.sync_scheduler_prompt_scroll();
        }
    }

    pub(crate) fn poll_scheduler_inputs(&mut self) -> bool {
        let scheduler_active = self.mode == Mode::Scheduler;
        let Some(composer) = self.scheduler.composer.as_mut() else {
            return false;
        };
        let field = composer.field;
        let destination_picker_open = composer.destination_picker_open();
        let mut changed = composer
            .destination_picker
            .query
            .poll_blink(scheduler_active && destination_picker_open);
        changed |= composer.destination_picker.poll_change_details();
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
            destination: destination_path.clone(),
            repository: destination.repository.clone(),
            branch: destination.checkout_branch.clone(),
            enabled: composer.enabled,
            interval_minutes: composer.schedule.text().parse().unwrap_or(0),
            source: composer.source.as_ref().map(|source| source.path.clone()),
        };
        let task_id = composer.task_id;
        let source = composer.source.clone();
        match self.herdr.save_scheduled_task(task_id, edit, source) {
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
                self.linked_worktrees.refresh();
                self.save_scheduled_task();
            }
            Err(error) => {
                self.scheduler.error = Some(format!("Could not create worktree: {error}"));
            }
        }
    }

    pub(crate) fn selected_scheduled_task(&self) -> Option<&ScheduledTask> {
        let selected = self.scheduler.selected_task_id?;
        self.herdr
            .scheduled_tasks()
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
        self.scheduler.error = self.herdr.toggle_scheduled_task(id, enabled).err();
    }

    fn run_selected_scheduled_task(&mut self) {
        let Some(id) = self.selected_scheduled_task().map(|task| task.id) else {
            return;
        };
        self.scheduler.error = self.herdr.run_scheduled_task_now(id).err();
        if self.scheduler.error.is_none() {
            // The next scheduler update contains the newly claimed run at the front.
            self.scheduler.selected_run_id = None;
            self.scheduler.run_scroll = 0;
            self.scheduler.surface = SchedulerSurface::Detail;
            self.scheduler.preview_pending = true;
            self.scheduler.conversation_scroll = None;
            self.scheduler.conversation_message = None;
            self.scheduler.conversation_expanded_requests.clear();
            self.herdr.clear_scheduled_conversation();
        }
    }

    fn delete_selected_scheduled_task(&mut self) {
        let Some(id) = self.selected_scheduled_task().map(|task| task.id) else {
            return;
        };
        if let Err(error) = self.herdr.delete_scheduled_task(id) {
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
        self.scheduler.error = self.herdr.refresh_scheduled_run(id).err();
    }

    fn open_selected_scheduled_run_conversation(&mut self) {
        let Some(run) = self
            .scheduler
            .selected_run_id
            .and_then(|id| self.herdr.scheduled_runs().iter().find(|run| run.id == id))
            .cloned()
        else {
            self.scheduler.error = Some("Select a run to open its conversation".to_owned());
            return;
        };
        if let Some(session_id) = run.session_id.as_deref() {
            self.herdr
                .request_scheduled_conversation(session_id, run.status.is_active());
        } else if let Some(task) = self
            .herdr
            .scheduled_tasks()
            .iter()
            .find(|task| task.id == run.task_id)
        {
            self.herdr.resolve_scheduled_run_session(
                run.id,
                task.destination.clone(),
                task.prompt.clone(),
                run.created_at_ms,
            );
        }
        self.scheduler.surface = SchedulerSurface::Detail;
        self.scheduler.conversation_scroll = None;
        self.scheduler.conversation_message = None;
        self.scheduler.conversation_expanded_requests.clear();
        self.scheduler.error = None;
        self.agent_preview_scheduled_run = Some(run.id);
        self.agent_preview_return_mode = Mode::Scheduler;
        self.agent_preview_picker_open = false;
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
        let tasks = self.herdr.scheduled_tasks();
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
        if target == ScrollTarget::SchedulerConversation {
            self.scroll_scheduler_conversation(delta.saturating_mul(3));
            return;
        }
        let scroll = match target {
            ScrollTarget::SchedulerTasks => &mut self.scheduler.task_scroll,
            ScrollTarget::SchedulerRuns => &mut self.scheduler.run_scroll,
            _ => return,
        };
        *scroll = scroll.saturating_add_signed(delta.saturating_mul(3));
    }

    pub(crate) fn scroll_scheduler_conversation(&mut self, delta: isize) {
        let current = self
            .scheduler
            .conversation_scroll
            .unwrap_or(self.scheduler.conversation_scroll_max);
        self.scheduler.conversation_scroll = Some(
            current
                .saturating_add_signed(delta)
                .min(self.scheduler.conversation_scroll_max),
        );
    }

    pub(crate) fn move_scheduler_conversation_message(&mut self, delta: isize) {
        let count = self
            .scheduler
            .selected_run_id
            .and_then(|id| self.herdr.scheduled_runs().iter().find(|run| run.id == id))
            .and_then(|run| run.session_id.as_deref())
            .and_then(|session| self.herdr.scheduled_conversation(session))
            .map(<[_]>::len)
            .unwrap_or(0);
        if count == 0 {
            return;
        }
        let current = self.scheduler.conversation_message.unwrap_or(count - 1);
        self.scheduler.conversation_message = Some(
            current
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1)),
        );
        self.scheduler.conversation_scroll = None;
        self.scheduler.conversation_expanded_requests.clear();
    }

    fn toggle_scheduler_prompt_expansion(&mut self) {
        let width = self.scheduler_prompt_size(5).0;
        let Some(composer) = self.scheduler.composer.as_mut() else {
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
