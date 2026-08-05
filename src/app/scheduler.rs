use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchedulerSurface {
    #[default]
    Tasks,
    Detail,
    Pane,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchedulerField {
    Title,
    Description,
    Prompt,
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
            Self::Worktree => &destination.worktree,
            Self::Branch => &destination.branch,
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
    pub(crate) path: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) worktree: String,
}

#[derive(Debug)]
pub(crate) struct ScheduledTaskComposer {
    pub(crate) title: TextInput,
    pub(crate) description: TextInput,
    pub(crate) prompt: TextInput,
    pub(crate) prompt_expanded: bool,
    pub(crate) prompt_scroll: usize,
    pub(crate) schedule: TextInput,
    pub(crate) destinations: Vec<SchedulerDestination>,
    pub(crate) destination: usize,
    pub(crate) destination_card: SchedulerDestinationCard,
    pub(crate) destination_picker_open: bool,
    pub(crate) picker: PickerState,
    pub(crate) field: SchedulerField,
}

impl ScheduledTaskComposer {
    fn new(destinations: Vec<SchedulerDestination>) -> Self {
        let mut schedule = TextInput::default();
        schedule.set("15");
        Self {
            title: TextInput::default(),
            description: TextInput::default(),
            prompt: TextInput::default(),
            prompt_expanded: false,
            prompt_scroll: 0,
            schedule,
            destinations,
            destination: 0,
            destination_card: SchedulerDestinationCard::Repository,
            destination_picker_open: false,
            picker: PickerState::default(),
            field: SchedulerField::Title,
        }
    }

    pub(crate) fn destination_candidates(&self) -> Vec<usize> {
        let Some(selected) = self.destinations.get(self.destination) else {
            return Vec::new();
        };
        let card = self.destination_card;
        let query = self.picker.query.text().to_lowercase();
        let mut candidates = Vec::new();
        for (index, destination) in self.destinations.iter().enumerate() {
            let searchable = format!(
                "{} {} {} {}",
                destination.repository,
                destination.worktree,
                destination.branch,
                destination.path.display()
            )
            .to_lowercase();
            if (card == SchedulerDestinationCard::Repository
                || destination.repository == selected.repository)
                && query
                    .split_whitespace()
                    .all(|term| searchable.contains(term))
                && !candidates.iter().any(|candidate| {
                    card.value(&self.destinations[*candidate]) == card.value(destination)
                })
            {
                candidates.push(index);
            }
        }
        candidates
    }

    fn open_destination_picker(&mut self, card: SchedulerDestinationCard) {
        self.destination_card = card;
        self.destination_picker_open = true;
        self.picker.query.clear();
        let candidates = self.destination_candidates();
        let selected = self.destinations.get(self.destination).and_then(|current| {
            candidates
                .iter()
                .position(|index| card.value(&self.destinations[*index]) == card.value(current))
        });
        self.picker
            .reset_items(candidates.len(), selected.unwrap_or(0));
    }

    fn close_destination_picker(&mut self) {
        self.destination_picker_open = false;
    }

    fn update_destination_picker(&mut self, delta: Option<isize>) {
        let candidates = self.destination_candidates();
        if let Some(delta) = delta {
            self.picker.move_selection(delta);
        } else {
            let selected = candidates
                .iter()
                .position(|index| *index == self.destination)
                .unwrap_or(0);
            self.picker.reset_items(candidates.len(), selected);
        }
        if let Some(destination) = candidates.get(self.picker.selected) {
            self.destination = *destination;
        }
    }

    pub(crate) fn input(&self, field: SchedulerField) -> &TextInput {
        match field {
            SchedulerField::Title => &self.title,
            SchedulerField::Description => &self.description,
            SchedulerField::Prompt => &self.prompt,
            SchedulerField::Schedule => &self.schedule,
            SchedulerField::Destination => unreachable!(),
        }
    }

    fn input_mut(&mut self, field: SchedulerField) -> &mut TextInput {
        match field {
            SchedulerField::Title => &mut self.title,
            SchedulerField::Description => &mut self.description,
            SchedulerField::Prompt => &mut self.prompt,
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
    pub(crate) pane_scroll_x: usize,
    pub(crate) pane_scroll_bottom: usize,
    pub(crate) conversation_scroll: Option<usize>,
    pub(crate) conversation_scroll_max: usize,
    pub(crate) conversation_message: Option<usize>,
    pub(crate) conversation_expanded_requests: Vec<usize>,
    pub(crate) runs_focused: bool,
    pub(crate) error: Option<String>,
}

impl App {
    pub(crate) fn scheduler_destinations(&self) -> Vec<SchedulerDestination> {
        self.linked_worktrees
            .snapshot()
            .repositories
            .iter()
            .flat_map(|repository| {
                repository.worktrees.iter().filter_map(|worktree| {
                    (!worktree.is_bare && !worktree.prunable).then(|| SchedulerDestination {
                        worktree: if worktree.is_main {
                            "basetree".to_owned()
                        } else {
                            worktree
                                .path
                                .file_name()
                                .unwrap_or(worktree.path.as_os_str())
                                .to_string_lossy()
                                .into_owned()
                        },
                        path: worktree.path.clone(),
                        repository: repository.label.clone(),
                        branch: worktree
                            .branch
                            .as_deref()
                            .map(|branch| branch.strip_prefix("refs/heads/").unwrap_or(branch))
                            .unwrap_or("detached HEAD")
                            .to_owned(),
                    })
                })
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
        self.scheduler.error = None;
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
    }

    pub(crate) fn close_scheduler(&mut self) {
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

    pub(crate) fn cancel_scheduled_task(&mut self) {
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
                    if composer.destination_picker_open && composer.destination_card == card {
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
            SchedulerHitTarget::OpenPane => self.open_selected_scheduled_run_pane(),
            SchedulerHitTarget::ClosePane => self.scheduler.surface = SchedulerSurface::Detail,
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
            SchedulerHitTarget::CloseConversation => {
                self.scheduler.surface = SchedulerSurface::Detail;
                self.herdr.clear_scheduled_conversation();
            }
        }
    }

    pub(crate) fn handle_scheduler(&mut self, key: KeyEvent) {
        if self.scheduler.composer.is_some() {
            self.handle_scheduler_composer(key);
            return;
        }
        if self.scheduler.surface == SchedulerSurface::Pane {
            match key.code {
                KeyCode::Esc | KeyCode::Char('p') => {
                    self.scheduler.surface = SchedulerSurface::Detail;
                }
                KeyCode::Char('v') => self.open_selected_scheduled_run_conversation(),
                KeyCode::Left | KeyCode::Char('h') => {
                    self.scheduler.pane_scroll_x = self.scheduler.pane_scroll_x.saturating_sub(3);
                }
                KeyCode::Right | KeyCode::Char('l') => self.scheduler.pane_scroll_x += 3,
                KeyCode::PageUp => self.scroll_scheduled_run_pane(true, true),
                KeyCode::PageDown => self.scroll_scheduled_run_pane(false, true),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scheduler.pane_scroll_bottom += 3;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scheduler.pane_scroll_bottom =
                        self.scheduler.pane_scroll_bottom.saturating_sub(3);
                }
                KeyCode::Home => self.scheduler.pane_scroll_x = 0,
                KeyCode::End => self.scheduler.pane_scroll_bottom = 0,
                KeyCode::F(4) => self.close_scheduler(),
                _ => {}
            }
            return;
        }
        if self.scheduler.surface == SchedulerSurface::Conversation {
            match key.code {
                KeyCode::Esc | KeyCode::Char('v') | KeyCode::Left | KeyCode::Char('h') => {
                    self.scheduler.surface = SchedulerSurface::Detail;
                    self.herdr.clear_scheduled_conversation();
                }
                KeyCode::Up | KeyCode::Char('k') => self.scroll_scheduler_conversation(-3),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_scheduler_conversation(3),
                KeyCode::PageUp => self.scroll_scheduler_conversation(-12),
                KeyCode::PageDown => self.scroll_scheduler_conversation(12),
                KeyCode::Home => self.scheduler.conversation_scroll = Some(0),
                KeyCode::End => self.scheduler.conversation_scroll = None,
                KeyCode::Char('[') => self.move_scheduler_conversation_message(-1),
                KeyCode::Char(']') => self.move_scheduler_conversation_message(1),
                KeyCode::F(4) => self.close_scheduler(),
                _ => {}
            }
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
            KeyCode::Char('p') => self.open_selected_scheduled_run_pane(),
            KeyCode::Char('v') => self.open_selected_scheduled_run_conversation(),
            _ => {}
        }
    }

    fn handle_scheduler_composer(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            let composer = self.scheduler.composer.as_mut().unwrap();
            if composer.destination_picker_open {
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
        if !composer.destination_picker_open {
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
        composer.picker.set_viewport_rows(viewport);
        match key.code {
            KeyCode::Enter => composer.close_destination_picker(),
            KeyCode::Down => composer.update_destination_picker(Some(1)),
            KeyCode::Up => composer.update_destination_picker(Some(-1)),
            KeyCode::PageDown | KeyCode::PageUp => {
                let delta = if key.code == KeyCode::PageDown { 1 } else { -1 };
                composer.picker.move_selection_page(delta);
                composer.update_destination_picker(Some(0));
            }
            _ => {
                if composer.picker.query.handle_edit_key(key) != EditOutcome::Unhandled {
                    composer.update_destination_picker(None);
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
            SchedulerField::Destination if composer.destination_picker_open => {
                composer.picker.query.insert_single_line(text);
                composer.update_destination_picker(None);
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
        let mut changed = composer
            .picker
            .query
            .poll_blink(scheduler_active && composer.destination_picker_open);
        for input_field in [
            SchedulerField::Title,
            SchedulerField::Description,
            SchedulerField::Prompt,
            SchedulerField::Schedule,
        ] {
            let focused = scheduler_active && field == input_field;
            changed |= composer.input_mut(input_field).poll_blink(focused);
        }
        changed
    }

    fn save_scheduled_task(&mut self) {
        let Some(composer) = self.scheduler.composer.as_ref() else {
            return;
        };
        let Some(destination) = composer.destinations.get(composer.destination) else {
            self.scheduler.error = Some("Select a linked worktree destination".to_owned());
            return;
        };
        let edit = ScheduledTaskEdit {
            title: composer.title.text().to_owned(),
            description: composer.description.text().to_owned(),
            prompt: composer.prompt.text().to_owned(),
            destination: destination.path.clone(),
            repository: destination.repository.clone(),
            branch: destination.branch.clone(),
            enabled: true,
            interval_minutes: composer.schedule.text().parse().unwrap_or(0),
        };
        match self.herdr.save_scheduled_task(edit) {
            Ok(()) => {
                self.scheduler.composer = None;
                self.scheduler.error = None;
                self.scheduler.surface = SchedulerSurface::Tasks;
                self.notice = Some("Scheduled task saved".to_owned());
            }
            Err(error) => self.scheduler.error = Some(error),
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

    pub(crate) fn selected_scheduled_run_pane_id(&self) -> Option<String> {
        let run = self
            .scheduler
            .selected_run_id
            .and_then(|id| self.herdr.scheduled_runs().iter().find(|run| run.id == id))?;
        let pane_id = run.pane_id.as_deref()?;
        Some(
            self.herdr
                .scheduled_agent_pane_id(pane_id, run.terminal_id.as_deref())
                .unwrap_or(pane_id)
                .to_owned(),
        )
    }

    fn open_selected_scheduled_run_pane(&mut self) {
        if self.selected_scheduled_run_pane_id().is_none() {
            self.scheduler.error = Some("This run has no available Herdr pane".to_owned());
            return;
        }
        self.scheduler.surface = SchedulerSurface::Pane;
        self.scheduler.pane_scroll_x = 0;
        self.scheduler.pane_scroll_bottom = 0;
        self.scheduler.error = None;
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
            self.herdr.request_scheduled_conversation(session_id);
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
            );
        }
        self.scheduler.surface = SchedulerSurface::Conversation;
        self.scheduler.conversation_scroll = None;
        self.scheduler.conversation_message = None;
        self.scheduler.conversation_expanded_requests.clear();
        self.scheduler.error = None;
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
            if !composer.destination_picker_open {
                composer.open_destination_picker(composer.destination_card);
            }
            composer.picker.set_viewport_rows(viewport);
            composer.picker.scroll_by(delta);
            return;
        }
        if target == ScrollTarget::SchedulerPane {
            if delta != 0 {
                self.scroll_scheduled_run_pane(delta < 0, false);
            }
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

    fn scroll_scheduler_conversation(&mut self, delta: isize) {
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

    fn move_scheduler_conversation_message(&mut self, delta: isize) {
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

    fn scroll_scheduled_run_pane(&mut self, up: bool, page: bool) {
        let Some(pane_id) = self.selected_scheduled_run_pane_id() else {
            self.scheduler.error = Some("This run has no available Herdr pane".to_owned());
            return;
        };
        self.scheduler.error = None;
        self.herdr.scroll_pane_preview(pane_id, up, page);
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
