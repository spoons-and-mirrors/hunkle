use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    AgentKey, AgentPromptDelivery, AgentTranscript, AgentUserMessage, EditOutcome, HitTarget, Mode,
    ScrollTarget, TextInput, opencode_session,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct TranscriptScroll {
    agent: AgentKey,
    message: usize,
    offset: usize,
}

#[derive(Debug)]
struct MessageSelection {
    agent: AgentKey,
    message: usize,
}

#[derive(Debug)]
struct ExpandedRequests {
    agent: AgentKey,
    message: usize,
    requests: Vec<usize>,
}

#[derive(Debug)]
struct ExpandedUserMessage {
    agent: AgentKey,
    message: usize,
}

#[derive(Default)]
struct ScheduledConversationState {
    scroll: Option<usize>,
    message: Option<usize>,
    expanded_requests: Vec<usize>,
    user_expanded: bool,
}

impl ScheduledConversationState {
    fn reset(&mut self) {
        self.scroll = None;
        self.message = None;
        self.expanded_requests.clear();
        self.user_expanded = false;
    }

    fn scroll_by(&mut self, delta: isize, maximum: usize) {
        let current = self.scroll.unwrap_or(maximum);
        self.scroll = Some(current.saturating_add_signed(delta).min(maximum));
    }

    fn move_message(&mut self, count: usize, delta: isize) {
        if count == 0 {
            return;
        }
        let current = self.message.unwrap_or(count - 1);
        self.message = Some(
            current
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1)),
        );
        self.scroll = None;
        self.expanded_requests.clear();
        self.user_expanded = false;
    }

    fn toggle_request(&mut self, request: usize) {
        if let Some(index) = self
            .expanded_requests
            .iter()
            .position(|value| *value == request)
        {
            self.expanded_requests.remove(index);
        } else {
            self.expanded_requests.push(request);
        }
    }
}

struct ScheduledTranscript {
    session_id: String,
    revision: u64,
    messages: Vec<AgentUserMessage>,
}

enum Completion {
    Session {
        run_id: i64,
        result: Result<String, String>,
    },
    Conversation {
        session_id: String,
        result: Result<opencode_session::TranscriptFetch, String>,
    },
}

#[derive(Default)]
pub(crate) struct AgentPreviewPoll {
    pub(crate) changed: bool,
    pub(crate) resolved_sessions: Vec<(i64, String)>,
}

#[derive(Clone)]
pub(crate) struct LiveAgentPreviewContext {
    pub(crate) key: AgentKey,
    pub(crate) message: usize,
    pub(crate) message_count: usize,
    pub(crate) scroll_max: usize,
}

pub(crate) enum AgentPreviewEffect {
    Handled,
    Close(Mode),
    FocusPrompt(Option<AgentKey>),
    SubmitPrompt,
    PromptEdited,
    SelectAgent(AgentKey),
    TogglePromptDelivery(AgentKey),
    MessageSelected { agent: AgentKey, message: usize },
    MoveMessage { agent: AgentKey, forward: bool },
    MoveScheduledMessage { run_id: i64, forward: bool },
    TogglePicker(AgentKey),
}

pub(crate) struct ScheduledPreviewRenderState<'a> {
    pub(crate) transcript: Option<AgentTranscript<'a>>,
    pub(crate) presentation: &'a mut crate::ui::AgentTranscriptPresentation,
    pub(crate) message: Option<usize>,
    pub(crate) scroll: Option<usize>,
    pub(crate) expanded_requests: &'a [usize],
    pub(crate) prompt: &'a TextInput,
    pub(crate) prompt_focused: bool,
    pub(crate) prompt_error: Option<&'a str>,
    pub(crate) conversation_error: Option<&'a str>,
    pub(crate) user_message_expanded: bool,
}

pub(crate) struct AgentPreview {
    pub(crate) selection: Option<AgentKey>,
    transcript_scroll: Option<TranscriptScroll>,
    message_selection: Option<MessageSelection>,
    expanded_requests: Option<ExpandedRequests>,
    expanded_user_message: Option<ExpandedUserMessage>,
    pub(crate) presentation: crate::ui::AgentTranscriptPresentation,
    pub(crate) picker_open: bool,
    pub(crate) prompt: TextInput,
    pub(crate) prompt_focused: bool,
    pub(crate) prompt_error: Option<String>,
    pub(crate) prompt_delivery: AgentPromptDelivery,
    pub(crate) scheduled_run: Option<i64>,
    scheduled: ScheduledConversationState,
    return_mode: Mode,
    active_session: Option<String>,
    scheduled_transcript: Option<ScheduledTranscript>,
    scheduled_conversation_error: Option<(String, String)>,
    scheduled_session_errors: HashMap<i64, String>,
    session_requests: HashSet<i64>,
    session_refreshes: HashMap<i64, Instant>,
    conversation_requests: HashSet<String>,
    conversation_refreshes: HashMap<String, Instant>,
    transcript_revision: u64,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
}

impl Default for AgentPreview {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            selection: None,
            transcript_scroll: None,
            message_selection: None,
            expanded_requests: None,
            expanded_user_message: None,
            presentation: crate::ui::AgentTranscriptPresentation::default(),
            picker_open: false,
            prompt: TextInput::default(),
            prompt_focused: false,
            prompt_error: None,
            prompt_delivery: AgentPromptDelivery::default(),
            scheduled_run: None,
            scheduled: ScheduledConversationState::default(),
            return_mode: Mode::Normal,
            active_session: None,
            scheduled_transcript: None,
            scheduled_conversation_error: None,
            scheduled_session_errors: HashMap::new(),
            session_requests: HashSet::new(),
            session_refreshes: HashMap::new(),
            conversation_requests: HashSet::new(),
            conversation_refreshes: HashMap::new(),
            transcript_revision: 0,
            sender,
            receiver,
        }
    }
}

impl AgentPreview {
    pub(crate) fn handle_prompt_key(
        &mut self,
        key: KeyEvent,
        input_width: usize,
    ) -> AgentPreviewEffect {
        match key.code {
            KeyCode::Esc => {
                self.blur_prompt();
                AgentPreviewEffect::Handled
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
            {
                self.prompt.insert_char('\n');
                self.clear_prompt_error();
                AgentPreviewEffect::PromptEdited
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt.insert_char('\n');
                self.clear_prompt_error();
                AgentPreviewEffect::PromptEdited
            }
            KeyCode::Enter => AgentPreviewEffect::SubmitPrompt,
            KeyCode::Up => {
                self.prompt.move_up(input_width);
                AgentPreviewEffect::Handled
            }
            KeyCode::Down => {
                self.prompt.move_down(input_width);
                AgentPreviewEffect::Handled
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt.clear();
                self.clear_prompt_error();
                AgentPreviewEffect::PromptEdited
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.prompt.delete_word();
                self.clear_prompt_error();
                AgentPreviewEffect::PromptEdited
            }
            _ if self.prompt.handle_edit_key(key) == EditOutcome::Edited => {
                self.clear_prompt_error();
                AgentPreviewEffect::PromptEdited
            }
            _ => AgentPreviewEffect::Handled,
        }
    }

    pub(crate) fn paste_prompt(&mut self, text: &str) -> AgentPreviewEffect {
        self.prompt.insert(text);
        self.clear_prompt_error();
        AgentPreviewEffect::PromptEdited
    }

    pub(crate) fn handle_modal_key(
        &mut self,
        key: KeyEvent,
        live: Option<LiveAgentPreviewContext>,
        scheduled_scroll_max: usize,
    ) -> AgentPreviewEffect {
        if self.prompt_focused {
            return AgentPreviewEffect::Handled;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('v') => AgentPreviewEffect::Close(self.close()),
            KeyCode::Enter => AgentPreviewEffect::FocusPrompt(None),
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_context(live.as_ref(), -1, scheduled_scroll_max)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_context(live.as_ref(), 1, scheduled_scroll_max)
            }
            KeyCode::PageUp => self.scroll_context(live.as_ref(), -10, scheduled_scroll_max),
            KeyCode::PageDown => self.scroll_context(live.as_ref(), 10, scheduled_scroll_max),
            KeyCode::Home => self.scroll_context(live.as_ref(), isize::MIN, scheduled_scroll_max),
            KeyCode::End => self.scroll_context(live.as_ref(), isize::MAX, scheduled_scroll_max),
            KeyCode::Char('[') => self.move_context_message(live.as_ref(), false),
            KeyCode::Char(']') => self.move_context_message(live.as_ref(), true),
            _ => AgentPreviewEffect::Handled,
        }
    }

    pub(crate) fn activate_target(&mut self, target: &HitTarget) -> AgentPreviewEffect {
        match target {
            HitTarget::AgentPreviewModalBackdrop | HitTarget::AgentPreviewModalClose => {
                AgentPreviewEffect::Close(self.close())
            }
            HitTarget::AgentPreviewPrompt(key) => {
                AgentPreviewEffect::FocusPrompt(Some(key.clone()))
            }
            HitTarget::AgentPreviewScheduledPrompt(run_id)
                if self.scheduled_run == Some(*run_id) =>
            {
                AgentPreviewEffect::FocusPrompt(None)
            }
            HitTarget::AgentPreviewPromptDelivery(key) => {
                AgentPreviewEffect::TogglePromptDelivery(key.clone())
            }
            HitTarget::AgentPreviewRequest {
                agent,
                message,
                request,
            } => {
                self.toggle_request(agent.clone(), *message, *request);
                AgentPreviewEffect::Handled
            }
            HitTarget::AgentPreviewScheduledRequest { run_id, request }
                if self.scheduled_run == Some(*run_id) =>
            {
                self.toggle_scheduled_request(*request);
                AgentPreviewEffect::Handled
            }
            HitTarget::AgentMessage { agent, message }
            | HitTarget::AgentExpandedMessage { agent, message } => {
                self.toggle_user_message(agent.clone(), *message);
                AgentPreviewEffect::Handled
            }
            HitTarget::AgentScheduledMessage { run_id, .. }
                if self.scheduled_run == Some(*run_id) =>
            {
                self.toggle_scheduled_user_message();
                AgentPreviewEffect::Handled
            }
            HitTarget::AgentPreviewPicker(key) => AgentPreviewEffect::TogglePicker(key.clone()),
            HitTarget::AgentPreviewPickerItem(key) => AgentPreviewEffect::SelectAgent(key.clone()),
            HitTarget::AgentPreviewMessageStep { agent, forward } => {
                AgentPreviewEffect::MoveMessage {
                    agent: agent.clone(),
                    forward: *forward,
                }
            }
            HitTarget::AgentPreviewScheduledMessageStep { run_id, forward }
                if self.scheduled_run == Some(*run_id) =>
            {
                AgentPreviewEffect::MoveScheduledMessage {
                    run_id: *run_id,
                    forward: *forward,
                }
            }
            _ => AgentPreviewEffect::Handled,
        }
    }

    pub(crate) fn owns_target(target: &HitTarget) -> bool {
        matches!(
            target,
            HitTarget::AgentPreviewModalBackdrop
                | HitTarget::AgentPreviewModalOverlay
                | HitTarget::AgentPreviewModalClose
                | HitTarget::AgentPreviewPrompt(_)
                | HitTarget::AgentPreviewScheduledPrompt(_)
                | HitTarget::AgentPreviewPromptDelivery(_)
                | HitTarget::AgentPreviewRequest { .. }
                | HitTarget::AgentPreviewScheduledRequest { .. }
                | HitTarget::AgentMessage { .. }
                | HitTarget::AgentExpandedMessage { .. }
                | HitTarget::AgentScheduledMessage { .. }
                | HitTarget::AgentPreviewPicker(_)
                | HitTarget::AgentPreviewPickerItem(_)
                | HitTarget::AgentPreviewMessageStep { .. }
                | HitTarget::AgentPreviewScheduledMessageStep { .. }
        )
    }

    pub(crate) fn handle_scroll(
        &mut self,
        target: &ScrollTarget,
        delta: isize,
        live: Option<LiveAgentPreviewContext>,
        maximum: usize,
    ) -> AgentPreviewEffect {
        match (target, live.as_ref()) {
            (ScrollTarget::AgentTimeline(target), Some(live)) if target == &live.key => {
                self.move_live_message(live, delta > 0)
            }
            (ScrollTarget::AgentTranscript(target), Some(live)) if target == &live.key => {
                self.scroll_transcript(live.key.clone(), live.message, delta, maximum);
                AgentPreviewEffect::Handled
            }
            (ScrollTarget::AgentScheduledTranscript(run_id), _)
                if self.scheduled_run == Some(*run_id) =>
            {
                self.scroll_scheduled(delta, maximum);
                AgentPreviewEffect::Handled
            }
            _ => AgentPreviewEffect::Handled,
        }
    }

    pub(crate) fn handle_horizontal_scroll(
        &mut self,
        target: &ScrollTarget,
        forward: bool,
        live: Option<LiveAgentPreviewContext>,
    ) -> AgentPreviewEffect {
        match target {
            ScrollTarget::AgentTimeline(current) | ScrollTarget::AgentTranscript(current) => live
                .as_ref()
                .filter(|live| &live.key == current)
                .map_or(AgentPreviewEffect::Handled, |live| {
                    self.move_live_message(live, forward)
                }),
            _ => AgentPreviewEffect::Handled,
        }
    }

    fn scroll_context(
        &mut self,
        live: Option<&LiveAgentPreviewContext>,
        delta: isize,
        scheduled_scroll_max: usize,
    ) -> AgentPreviewEffect {
        if let Some(live) = live {
            self.scroll_transcript(live.key.clone(), live.message, delta, live.scroll_max);
        } else if self.scheduled_run.is_some() {
            self.scroll_scheduled(delta, scheduled_scroll_max);
        }
        AgentPreviewEffect::Handled
    }

    fn move_context_message(
        &mut self,
        live: Option<&LiveAgentPreviewContext>,
        forward: bool,
    ) -> AgentPreviewEffect {
        if let Some(live) = live {
            return self.move_live_message(live, forward);
        }
        if self.scheduled_run.is_some() {
            self.move_scheduled_message(
                if forward { 1 } else { -1 },
                self.scheduled_message_count(),
            );
        }
        AgentPreviewEffect::Handled
    }

    fn move_live_message(
        &mut self,
        live: &LiveAgentPreviewContext,
        forward: bool,
    ) -> AgentPreviewEffect {
        self.select_message(live.key.clone(), live.message_count, live.message, forward)
            .map_or(AgentPreviewEffect::Handled, |message| {
                AgentPreviewEffect::MessageSelected {
                    agent: live.key.clone(),
                    message,
                }
            })
    }

    pub(crate) fn select_agent(&mut self, key: AgentKey) {
        self.selection = Some(key);
        self.scheduled_run = None;
        self.clear_scheduled_conversation();
        self.reset_conversation();
    }

    pub(crate) fn set_return_mode(&mut self, return_mode: Mode) {
        self.return_mode = return_mode;
    }

    pub(crate) fn open_scheduled_run(&mut self, run_id: i64, return_mode: Mode) {
        self.scheduled_run = Some(run_id);
        self.return_mode = return_mode;
        self.picker_open = false;
        self.reset_conversation();
    }

    pub(crate) fn close(&mut self) -> Mode {
        self.picker_open = false;
        self.scheduled_run = None;
        self.clear_scheduled_conversation();
        self.reset_prompt();
        let mode = self.return_mode;
        self.return_mode = Mode::Normal;
        mode
    }

    pub(crate) fn dismiss(&mut self) {
        self.selection = None;
        self.scheduled_run = None;
        self.clear_scheduled_conversation();
        self.reset_conversation();
    }

    pub(crate) fn reset_conversation(&mut self) {
        self.transcript_scroll = None;
        self.message_selection = None;
        self.expanded_requests = None;
        self.expanded_user_message = None;
        self.picker_open = false;
        self.scheduled.reset();
        self.reset_prompt();
    }

    pub(crate) fn focus_agent(&mut self, key: AgentKey) {
        self.selection = Some(key);
    }

    pub(crate) fn restore_selection(&mut self, selection: Option<AgentKey>) {
        self.selection = selection;
    }

    pub(crate) fn reset_prompt(&mut self) {
        self.prompt.clear();
        self.prompt_focused = false;
        self.prompt_error = None;
    }

    pub(crate) fn focus_prompt(&mut self) {
        self.prompt_focused = true;
        self.prompt.focus();
        self.prompt_error = None;
    }

    pub(crate) fn blur_prompt(&mut self) {
        self.prompt_focused = false;
    }

    pub(crate) fn finish_prompt(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.prompt.clear();
                self.prompt_error = None;
                self.prompt_focused = false;
            }
            Err(error) => self.prompt_error = Some(error),
        }
    }

    pub(crate) fn set_prompt_error(&mut self, error: impl Into<String>) {
        self.prompt_error = Some(error.into());
    }

    pub(crate) fn toggle_prompt_delivery(&mut self) {
        self.prompt_delivery = self.prompt_delivery.toggle();
    }

    pub(crate) fn clear_prompt_error(&mut self) {
        self.prompt_error = None;
    }

    pub(crate) fn toggle_picker(&mut self, key: AgentKey) {
        self.selection = Some(key);
        self.picker_open = !self.picker_open;
    }

    pub(crate) fn close_picker(&mut self) {
        self.picker_open = false;
    }

    pub(crate) fn selected_message(&self, key: &AgentKey, last: usize) -> Option<usize> {
        self.message_selection
            .as_ref()
            .filter(|selection| &selection.agent == key)
            .map(|selection| selection.message.min(last))
    }

    pub(crate) fn transcript_scroll(&self, key: &AgentKey, message: usize) -> Option<usize> {
        self.transcript_scroll
            .as_ref()
            .filter(|scroll| &scroll.agent == key && scroll.message == message)
            .map(|scroll| scroll.offset)
    }

    pub(crate) fn expanded_requests(&self, key: &AgentKey, message: usize) -> &[usize] {
        self.expanded_requests
            .as_ref()
            .filter(|expanded| &expanded.agent == key && expanded.message == message)
            .map_or(&[], |expanded| expanded.requests.as_slice())
    }

    pub(crate) fn user_message_expanded(&self, key: &AgentKey, message: usize) -> bool {
        self.expanded_user_message
            .as_ref()
            .is_some_and(|expanded| &expanded.agent == key && expanded.message == message)
    }

    pub(crate) fn select_message(
        &mut self,
        key: AgentKey,
        message_count: usize,
        message: usize,
        forward: bool,
    ) -> Option<usize> {
        let selected = if forward {
            message
                .saturating_add(1)
                .min(message_count.saturating_sub(1))
        } else {
            message.saturating_sub(1)
        };
        if selected == message {
            return None;
        }
        self.message_selection = (selected + 1 < message_count).then(|| MessageSelection {
            agent: key.clone(),
            message: selected,
        });
        self.expanded_requests = None;
        self.expanded_user_message = None;
        self.transcript_scroll = Some(TranscriptScroll {
            agent: key,
            message: selected,
            offset: 0,
        });
        Some(selected)
    }

    pub(crate) fn toggle_request(&mut self, key: AgentKey, message: usize, request: usize) {
        let same_scope = self
            .expanded_requests
            .as_ref()
            .is_some_and(|expanded| expanded.agent == key && expanded.message == message);
        if !same_scope {
            self.expanded_requests = Some(ExpandedRequests {
                agent: key,
                message,
                requests: vec![request],
            });
            return;
        }
        let expanded = self
            .expanded_requests
            .as_mut()
            .expect("scope checked above");
        if let Some(index) = expanded
            .requests
            .iter()
            .position(|expanded| *expanded == request)
        {
            expanded.requests.remove(index);
        } else {
            expanded.requests.push(request);
        }
    }

    pub(crate) fn toggle_user_message(&mut self, key: AgentKey, message: usize) {
        if self.user_message_expanded(&key, message) {
            self.expanded_user_message = None;
            self.transcript_scroll = None;
        } else {
            self.expanded_user_message = Some(ExpandedUserMessage {
                agent: key.clone(),
                message,
            });
            self.transcript_scroll = Some(TranscriptScroll {
                agent: key,
                message,
                offset: 0,
            });
        }
    }

    pub(crate) fn scroll_transcript(
        &mut self,
        key: AgentKey,
        message: usize,
        delta: isize,
        maximum: usize,
    ) {
        let offset = self
            .transcript_scroll(&key, message)
            .unwrap_or(maximum)
            .saturating_add_signed(delta)
            .min(maximum);
        let user_message_expanded = self.user_message_expanded(&key, message);
        self.transcript_scroll =
            (user_message_expanded || offset < maximum).then_some(TranscriptScroll {
                agent: key,
                message,
                offset,
            });
    }

    pub(crate) fn scroll_scheduled(&mut self, delta: isize, maximum: usize) {
        self.scheduled.scroll_by(delta, maximum);
    }

    pub(crate) fn move_scheduled_message(&mut self, delta: isize, count: usize) {
        self.scheduled.move_message(count, delta);
    }

    pub(crate) fn toggle_scheduled_request(&mut self, request: usize) {
        self.scheduled.toggle_request(request);
    }

    pub(crate) fn toggle_scheduled_user_message(&mut self) {
        self.scheduled.user_expanded = !self.scheduled.user_expanded;
        self.scheduled.scroll = self.scheduled.user_expanded.then_some(0);
    }

    pub(crate) fn scheduled_message_count(&self) -> usize {
        self.scheduled_transcript
            .as_ref()
            .map_or(0, |transcript| transcript.messages.len())
    }

    #[cfg(test)]
    pub(crate) fn scheduled_scroll(&self) -> Option<usize> {
        self.scheduled.scroll
    }

    pub(crate) fn request_scheduled_session(
        &mut self,
        run_id: i64,
        directory: PathBuf,
        prompt: String,
        run_created_at_ms: i64,
    ) {
        let now = Instant::now();
        if self.session_requests.contains(&run_id)
            || self
                .session_refreshes
                .get(&run_id)
                .is_some_and(|refresh| now < *refresh)
        {
            return;
        }
        self.session_requests.insert(run_id);
        self.session_refreshes
            .insert(run_id, now + REFRESH_INTERVAL);
        self.scheduled_session_errors.remove(&run_id);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = opencode_session::resolve_scheduled_session_id(
                &directory,
                &prompt,
                run_created_at_ms,
            );
            let _ = sender.send(Completion::Session { run_id, result });
        });
    }

    pub(crate) fn scheduled_session_error(&self, run_id: i64) -> Option<&str> {
        self.scheduled_session_errors
            .get(&run_id)
            .map(String::as_str)
    }

    pub(crate) fn request_scheduled_conversation(&mut self, session_id: &str, active: bool) {
        self.activate_session(session_id);
        let now = Instant::now();
        if (self.scheduled_transcript.is_some() && !active)
            || self.conversation_requests.contains(session_id)
            || self
                .conversation_refreshes
                .get(session_id)
                .is_some_and(|refresh| now < *refresh)
        {
            return;
        }
        self.start_conversation_fetch(session_id, now + REFRESH_INTERVAL);
    }

    pub(crate) fn refresh_scheduled_conversation(&mut self, session_id: &str) {
        self.activate_session(session_id);
        if self.conversation_requests.contains(session_id) {
            return;
        }
        self.conversation_refreshes.remove(session_id);
        self.start_conversation_fetch(session_id, Instant::now());
    }

    fn activate_session(&mut self, session_id: &str) {
        if self.active_session.as_deref() == Some(session_id) {
            return;
        }
        self.active_session = Some(session_id.to_owned());
        self.scheduled_transcript = None;
        self.scheduled_conversation_error = None;
    }

    fn start_conversation_fetch(&mut self, session_id: &str, next_refresh: Instant) {
        self.conversation_requests.insert(session_id.to_owned());
        self.conversation_refreshes
            .insert(session_id.to_owned(), next_refresh);
        self.scheduled_conversation_error = None;
        let session_id = session_id.to_owned();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = opencode_session::fetch(&session_id, false);
            let _ = sender.send(Completion::Conversation { session_id, result });
        });
    }

    pub(crate) fn scheduled_transcript_identity(&self) -> Option<&str> {
        self.scheduled_transcript
            .as_ref()
            .map(|transcript| transcript.session_id.as_str())
    }

    pub(crate) fn scheduled_render_state(
        &mut self,
        session_id: Option<&str>,
    ) -> ScheduledPreviewRenderState<'_> {
        let transcript = match (session_id, self.scheduled_transcript.as_ref()) {
            (Some(session_id), Some(transcript)) if transcript.session_id == session_id => {
                Some(AgentTranscript {
                    identity: &transcript.session_id,
                    messages: &transcript.messages,
                    revision: transcript.revision,
                })
            }
            _ => None,
        };
        let conversation_error = match (session_id, self.scheduled_conversation_error.as_ref()) {
            (Some(session_id), Some((active, error))) if active == session_id => {
                Some(error.as_str())
            }
            _ => None,
        };
        ScheduledPreviewRenderState {
            transcript,
            presentation: &mut self.presentation,
            message: self.scheduled.message,
            scroll: self.scheduled.scroll,
            expanded_requests: &self.scheduled.expanded_requests,
            prompt: &self.prompt,
            prompt_focused: self.prompt_focused,
            prompt_error: self.prompt_error.as_deref(),
            conversation_error,
            user_message_expanded: self.scheduled.user_expanded,
        }
    }

    pub(crate) fn clear_scheduled_conversation(&mut self) {
        self.active_session = None;
        self.scheduled_transcript = None;
        self.scheduled_conversation_error = None;
    }

    pub(crate) fn poll(&mut self) -> AgentPreviewPoll {
        let mut poll = AgentPreviewPoll::default();
        while let Ok(completion) = self.receiver.try_recv() {
            match completion {
                Completion::Session { run_id, result } => {
                    self.session_requests.remove(&run_id);
                    match result {
                        Ok(session_id) => {
                            self.scheduled_session_errors.remove(&run_id);
                            poll.resolved_sessions.push((run_id, session_id));
                        }
                        Err(error) => {
                            self.scheduled_session_errors.insert(run_id, error);
                        }
                    }
                    poll.changed = true;
                }
                Completion::Conversation { session_id, result } => {
                    self.conversation_requests.remove(&session_id);
                    if self.active_session.as_deref() != Some(session_id.as_str()) {
                        continue;
                    }
                    match result {
                        Ok(opencode_session::TranscriptFetch::Changed(messages)) => {
                            self.transcript_revision = self.transcript_revision.wrapping_add(1);
                            self.scheduled_transcript = Some(ScheduledTranscript {
                                session_id: session_id.clone(),
                                revision: self.transcript_revision,
                                messages,
                            });
                            self.scheduled_conversation_error = None;
                        }
                        Ok(opencode_session::TranscriptFetch::Unchanged) => {
                            self.scheduled_conversation_error = None;
                        }
                        Err(error) => {
                            self.scheduled_conversation_error = Some((session_id, error));
                        }
                    }
                    poll.changed = true;
                }
            }
        }
        poll
    }

    #[cfg(test)]
    pub(crate) fn set_scheduled_conversation_for_test(
        &mut self,
        session_id: &str,
        messages: Vec<AgentUserMessage>,
    ) {
        self.active_session = Some(session_id.to_owned());
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.scheduled_transcript = Some(ScheduledTranscript {
            session_id: session_id.to_owned(),
            revision: self.transcript_revision,
            messages,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> AgentUserMessage {
        AgentUserMessage {
            text: text.to_owned(),
            requests: Vec::new(),
        }
    }

    #[test]
    fn stale_scheduled_conversation_completion_is_discarded() {
        let mut preview = AgentPreview::default();
        preview.activate_session("ses_active");
        preview.conversation_requests.insert("ses_stale".to_owned());
        preview
            .sender
            .send(Completion::Conversation {
                session_id: "ses_stale".to_owned(),
                result: Ok(opencode_session::TranscriptFetch::Changed(vec![message(
                    "stale",
                )])),
            })
            .unwrap();

        let poll = preview.poll();

        assert!(!poll.changed);
        assert!(preview.scheduled_transcript.is_none());
        assert!(!preview.conversation_requests.contains("ses_stale"));
    }

    #[test]
    fn scheduled_preview_lifecycle_resets_owned_interaction_state() {
        let mut preview = AgentPreview::default();
        preview.prompt.insert_char('x');
        preview.prompt_focused = true;
        preview.picker_open = true;
        preview.scheduled.scroll = Some(4);
        preview.scheduled.message = Some(2);
        preview.scheduled.expanded_requests.push(1);

        preview.open_scheduled_run(17, Mode::Scheduler);

        assert_eq!(preview.scheduled_run, Some(17));
        assert!(!preview.picker_open);
        assert!(preview.prompt.text().is_empty());
        assert!(!preview.prompt_focused);
        assert_eq!(preview.scheduled.scroll, None);
        assert_eq!(preview.scheduled.message, None);
        assert!(preview.scheduled.expanded_requests.is_empty());

        assert_eq!(preview.close(), Mode::Scheduler);
        assert_eq!(preview.scheduled_run, None);
    }
}
