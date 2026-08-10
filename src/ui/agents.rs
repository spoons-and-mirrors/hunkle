use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentActivityPreview, AgentDestinationMetadata, AgentEntryState, AgentKey, AgentListMode,
    AgentPromptDelivery, AgentPromptOutcome, AgentRequestPartPreview, AgentRequestPreview,
    AgentStatus, AgentTranscript, AgentUserMessage, HerdrSession, HitTarget, LinkedWorktreeCatalog,
    ScheduledRun, ScheduledRunStatus, ScheduledTask, Settings, TextInput,
};
use crate::theme::Palette;

use super::{
    fill, palette, preview::hard_wrap_preview_lines, text::styled_markdown_preserving_breaks,
    text_input_lines, truncate_width,
};

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_AGENT_TRANSCRIPT_PRESENTATIONS: usize = 8;
const MIN_EXPANDED_USER_RESPONSE_ROWS: u16 = 3;
const EXPANDED_USER_RESPONSE_ROWS: u16 = 6;

fn expanded_user_response_rows(user_height: usize, available: u16, fallback: u16) -> u16 {
    let minimum = MIN_EXPANDED_USER_RESPONSE_ROWS.min(available);
    let full_height = u16::try_from(user_height).unwrap_or(u16::MAX);
    if full_height <= available.saturating_sub(minimum) {
        minimum
    } else {
        fallback.min(available)
    }
}

struct TranscriptBlock {
    user: bool,
    headerless_response: bool,
    lines: Arc<[Line<'static>]>,
    animated_rows: Arc<[usize]>,
    start: usize,
    height: usize,
    elapsed: Option<String>,
    request_count: usize,
    request: Option<usize>,
    expandable: bool,
}

struct RequestSummary {
    lines: Vec<Line<'static>>,
    reasoning: Option<(Line<'static>, bool)>,
    hidden: bool,
    animated_rows: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgentTranscriptMode {
    Agent,
    Scheduled,
}

struct AgentTranscriptCacheKey {
    identity: String,
    revision: u64,
    message: usize,
    width: usize,
    expanded_requests: Vec<usize>,
    live: bool,
    mode: AgentTranscriptMode,
    palette: Palette,
}

struct CachedAgentTranscript {
    key: AgentTranscriptCacheKey,
    user_lines: Arc<[Line<'static>]>,
    user_elapsed: Option<String>,
    request_count: usize,
    blocks: Vec<TranscriptBlock>,
    request_height: usize,
}

struct AgentTranscriptPresentationInput<'a> {
    transcript: AgentTranscript<'a>,
    message: usize,
    width: usize,
    expanded_requests: &'a [usize],
    live: bool,
    mode: AgentTranscriptMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AgentTranscriptBuildStats {
    presentations: usize,
    markdown_parses: usize,
    static_wraps: usize,
    markdown_input_bytes: usize,
}

#[derive(Default)]
pub(crate) struct AgentTranscriptPresentation {
    cache: Vec<CachedAgentTranscript>,
    build_stats: AgentTranscriptBuildStats,
}

impl AgentTranscriptPresentation {
    fn prepare(&mut self, input: AgentTranscriptPresentationInput<'_>) -> &CachedAgentTranscript {
        self.cache.retain(|cached| {
            cached.key.identity != input.transcript.identity
                || cached.key.revision == input.transcript.revision
        });
        if let Some(index) = self
            .cache
            .iter()
            .position(|cached| cached.key.matches(&input, *palette()))
        {
            let cached = self.cache.remove(index);
            self.cache.push(cached);
            return self
                .cache
                .last()
                .expect("transcript cache hit was retained");
        }

        let message = &input.transcript.messages[input.message];
        self.build_stats.presentations = self.build_stats.presentations.saturating_add(1);
        let user_lines = Arc::from(styled_agent_text_counted(
            &message.text,
            input.width,
            &mut self.build_stats,
        ));
        let user_elapsed = message_total_duration(message).map(format_preview_duration);
        let (blocks, request_height) = build_request_transcript_counted(
            message,
            input.width,
            input.live,
            input.expanded_requests,
            &mut self.build_stats,
        );
        let mut expanded_requests = input.expanded_requests.to_vec();
        expanded_requests.sort_unstable();
        expanded_requests.dedup();
        if self.cache.len() == MAX_AGENT_TRANSCRIPT_PRESENTATIONS {
            self.cache.remove(0);
        }
        self.cache.push(CachedAgentTranscript {
            key: AgentTranscriptCacheKey {
                identity: input.transcript.identity.to_owned(),
                revision: input.transcript.revision,
                message: input.message,
                width: input.width,
                expanded_requests,
                live: input.live,
                mode: input.mode,
                palette: *palette(),
            },
            user_lines,
            user_elapsed,
            request_count: message.requests.len(),
            blocks,
            request_height,
        });
        self.cache
            .last()
            .expect("transcript cache miss was inserted")
    }

    pub(crate) fn retain_conversations(&mut self, mut retain: impl FnMut(&str) -> bool) {
        self.cache.retain(|cached| retain(&cached.key.identity));
    }

    #[cfg(test)]
    fn build_stats(&self) -> AgentTranscriptBuildStats {
        self.build_stats
    }

    #[cfg(test)]
    pub(crate) fn build_counts_for_test(&self) -> (usize, usize, usize) {
        (
            self.build_stats.presentations,
            self.build_stats.markdown_parses,
            self.build_stats.static_wraps,
        )
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    fn contains_conversation(&self, identity: &str) -> bool {
        self.cache
            .iter()
            .any(|cached| cached.key.identity == identity)
    }
}

impl AgentTranscriptCacheKey {
    fn matches(&self, input: &AgentTranscriptPresentationInput<'_>, palette: Palette) -> bool {
        self.identity == input.transcript.identity
            && self.revision == input.transcript.revision
            && self.message == input.message
            && self.width == input.width
            && self.live == input.live
            && self.mode == input.mode
            && self.palette == palette
            && self.expanded_requests.len() == input.expanded_requests.len()
            && input
                .expanded_requests
                .iter()
                .all(|request| self.expanded_requests.binary_search(request).is_ok())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    frame: &mut Frame<'_>,
    herdr: &mut HerdrSession,
    scheduled_tasks: &[ScheduledTask],
    scheduled_runs: &[ScheduledRun],
    linked_worktrees: &LinkedWorktreeCatalog,
    settings: &Settings,
    header: Rect,
    list: Rect,
    dragging: bool,
    hovered: Option<HitTarget>,
) -> (Vec<(HitTarget, Rect)>, bool) {
    let mut targets = Vec::new();
    let mut animation_presented = false;
    if header.width == 0 || header.height == 0 {
        return (targets, false);
    }
    let mode = herdr.agent_list_mode();
    let (section_name, count, toggle_label) = match mode {
        AgentListMode::Agents => ("AGENTS", herdr.agents.len(), " SCHEDULED "),
        AgentListMode::Scheduled => ("SCHEDULED", scheduled_runs.len(), " STASH "),
        AgentListMode::Stash => ("STASHED", herdr.stashed_agents().len(), " AGENTS "),
    };
    let toggle_width = u16::try_from(UnicodeWidthStr::width(toggle_label)).unwrap_or(0);
    let toggle = Rect::new(
        header.right().saturating_sub(toggle_width),
        header.y,
        toggle_width.min(header.width),
        1,
    );
    if header.height == 0 || list.height == 0 {
        return (targets, false);
    }
    let section_header = Rect::new(
        header.x,
        header.y,
        toggle.x.saturating_sub(header.x).saturating_sub(1),
        1,
    );
    let title = truncate_width(
        &format!("{section_name} {count}"),
        usize::from(section_header.width),
    );
    let separator_width = usize::from(section_header.width)
        .saturating_sub(UnicodeWidthStr::width(title.as_str()).saturating_add(1));
    let mut header_spans = vec![Span::styled(
        title,
        Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if separator_width > 0 {
        header_spans.push(Span::raw(" "));
        header_spans.push(Span::styled(
            "─".repeat(separator_width),
            Style::default().fg(if dragging {
                palette().cyan
            } else {
                palette().faint
            }),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), section_header);
    frame.render_widget(
        Paragraph::new(toggle_label).style(
            Style::default()
                .fg(if hovered == Some(HitTarget::AgentListModeToggle) {
                    palette().canvas
                } else {
                    palette().cyan
                })
                .bg(if hovered == Some(HitTarget::AgentListModeToggle) {
                    palette().selected
                } else {
                    palette().raised
                })
                .add_modifier(Modifier::BOLD),
        ),
        toggle,
    );
    targets.push((HitTarget::AgentListModeToggle, toggle));
    match mode {
        AgentListMode::Agents => {}
        AgentListMode::Scheduled => {
            let animation_presented = draw_scheduled_runs(
                frame,
                herdr,
                scheduled_tasks,
                scheduled_runs,
                list,
                hovered,
                &mut targets,
            );
            return (targets, animation_presented);
        }
        AgentListMode::Stash => {
            draw_stashed_agents(frame, herdr, list, hovered, &mut targets);
            return (targets, false);
        }
    }
    if herdr.agents.is_empty() {
        let message = herdr.error.as_deref().unwrap_or(if herdr.loading {
            "Loading Herdr agents…"
        } else {
            "No agents detected"
        });
        if list.height > 0 {
            frame.render_widget(
                Paragraph::new(format!(
                    "  {}",
                    truncate_width(message, usize::from(list.width).saturating_sub(2))
                ))
                .style(
                    Style::default()
                        .fg(if herdr.error.is_some() {
                            palette().red
                        } else {
                            palette().faint
                        })
                        .bg(palette().surface_alt),
                ),
                list,
            );
        }
        return (targets, false);
    }

    let card_height = if list.height >= 2 { 2 } else { 1 };
    let card_gap = 1;
    let item_step = card_height + card_gap;
    let top_padding = u16::from(list.height > card_height);
    let card_groups = herdr.agent_card_groups();
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(top_padding),
        list.width.saturating_sub(1),
        list.height.saturating_sub(top_padding),
    );
    let viewport = complete_card_viewport(card_list.height, item_step);
    let scroll = herdr
        .agent_scroll
        .min(card_groups.len().saturating_sub(viewport));
    let hovered_agent = match hovered.as_ref() {
        Some(HitTarget::Agent(index) | HitTarget::AgentStash(index)) => Some(index),
        Some(
            HitTarget::AgentPreviewPicker(agent)
            | HitTarget::AgentPreviewPickerItem(agent)
            | HitTarget::AgentPreviewMessageTimeline(agent)
            | HitTarget::AgentPreviewRequest { agent, .. }
            | HitTarget::AgentTooltip { agent, .. }
            | HitTarget::AgentMessage { agent, .. },
        ) => Some(agent),
        _ => None,
    }
    .and_then(|key| herdr.agent_index(key));
    let hovered_card = hovered_agent.and_then(|index| herdr.agent_card_index(index));
    let mut last_card = None;
    for (group_index, (index, agent_count)) in card_groups.iter().copied().enumerate().skip(scroll)
    {
        let screen_row = group_index - scroll;
        if screen_row >= viewport {
            break;
        }
        let offset = u16::try_from(screen_row).unwrap_or(0) * item_step;
        let row_area = Rect::new(
            card_list.x,
            card_list.y.saturating_add(offset),
            card_list.width,
            card_height.min(card_list.height.saturating_sub(offset)),
        );
        let full_row_area = Rect::new(list.x, row_area.y, list.width, row_area.height);
        let agent = &herdr.agents[index];
        let Some(agent_key) = herdr.agent_key(index) else {
            continue;
        };
        let state = herdr.agent_entry_state(index);
        let in_host_tab = herdr.agent_is_in_host_tab(index);
        let workspace = herdr
            .workspaces
            .iter()
            .find(|workspace| workspace.id == agent.workspace_id);
        let workspace_name = workspace.map_or("unassigned", |workspace| workspace.label.as_str());
        let destination = agent
            .destination_cwd
            .as_deref()
            .and_then(|path| linked_worktrees.agent_destination(path));
        let destination = agent_card_destination(
            destination,
            workspace_name,
            workspace.and_then(|workspace| workspace.branch.as_deref()),
        );
        let session = herdr
            .agent_display_name(index)
            .unwrap_or("terminal session");
        let elapsed = herdr
            .agent_elapsed(index, settings.agent_time_display)
            .map(format_duration);
        let change_stats = agent
            .destination_cwd
            .as_deref()
            .and_then(|path| linked_worktrees.change_stats(path));
        let is_hovered = hovered_card == Some(group_index);
        let pane_id_hovered =
            hovered.as_ref() == Some(&HitTarget::AgentPaneId(agent.pane_id.clone()));
        let background = row_background(&state, is_hovered);
        if row_area.y > list.y {
            let previous_background = group_index
                .checked_sub(1)
                .and_then(|previous| card_groups.get(previous))
                .map(|(previous, _)| {
                    row_background(
                        &herdr.agent_entry_state(*previous),
                        hovered_card == group_index.checked_sub(1),
                    )
                });
            draw_agent_gap(
                frame,
                Rect::new(list.x, row_area.y - 1, list.width, 1),
                previous_background.unwrap_or(palette().panel),
                background,
            );
        }
        fill(frame, full_row_area, background);
        let pane_id_area = draw_row(
            frame,
            row_area,
            destination,
            Some(&agent.pane_id),
            session,
            agent_count,
            change_stats,
            elapsed.as_deref(),
            agent.runtime.status,
            herdr.spinner_frame(),
            state,
            in_host_tab,
            is_hovered,
            pane_id_hovered,
        );
        last_card = Some((full_row_area, background));
        targets.push((HitTarget::Agent(agent_key.clone()), full_row_area));
        if let Some(pane_id_area) = pane_id_area {
            targets.push((HitTarget::AgentPaneId(agent.pane_id.clone()), pane_id_area));
        }
        let stash_presented = is_hovered && full_row_area.width >= 7;
        if stash_presented {
            let stash = Rect::new(full_row_area.right() - 7, full_row_area.y, 7, 1);
            frame.render_widget(
                Paragraph::new(" STASH ").style(
                    Style::default()
                        .fg(palette().canvas)
                        .bg(palette().red)
                        .add_modifier(Modifier::BOLD),
                ),
                stash,
            );
            targets.push((HitTarget::AgentStash(agent_key), stash));
        }
        animation_presented |=
            agent.runtime.status == AgentStatus::Working && row_area.width > 0 && !stash_presented;
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        draw_trailing_agent_gap(frame, gap, list, background);
    }
    (targets, animation_presented)
}

fn draw_scheduled_runs(
    frame: &mut Frame<'_>,
    herdr: &mut HerdrSession,
    scheduled_tasks: &[ScheduledTask],
    scheduled_runs: &[ScheduledRun],
    list: Rect,
    hovered: Option<HitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) -> bool {
    if scheduled_runs.is_empty() {
        frame.render_widget(
            Paragraph::new("  No scheduled runs").style(
                Style::default()
                    .fg(palette().faint)
                    .bg(palette().surface_alt),
            ),
            list,
        );
        return false;
    }
    let card_height = if list.height >= 2 { 2 } else { 1 };
    let card_gap = 1;
    let item_step = card_height + card_gap;
    let top_padding = u16::from(list.height > card_height);
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(top_padding),
        list.width.saturating_sub(1),
        list.height.saturating_sub(top_padding),
    );
    let viewport = complete_card_viewport(card_list.height, item_step);
    let scroll = herdr
        .scheduled_run_scroll
        .min(scheduled_runs.len().saturating_sub(viewport));
    let hovered_run = match hovered {
        Some(HitTarget::AgentScheduledRun(run_id)) => Some(run_id),
        _ => None,
    };
    let mut last_card = None;
    let mut animation_presented = false;
    for (screen_row, run) in scheduled_runs
        .iter()
        .skip(scroll)
        .take(viewport)
        .enumerate()
    {
        let offset = u16::try_from(screen_row).unwrap_or(0) * item_step;
        let row_area = Rect::new(
            card_list.x,
            card_list.y.saturating_add(offset),
            card_list.width,
            card_height.min(card_list.height.saturating_sub(offset)),
        );
        let full_row_area = Rect::new(list.x, row_area.y, list.width, row_area.height);
        let hovered = hovered_run == Some(run.id);
        let background = if hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        if row_area.y > list.y {
            draw_agent_gap(
                frame,
                Rect::new(list.x, row_area.y - 1, list.width, 1),
                last_card.map_or(palette().panel, |(_, color)| color),
                background,
            );
        }
        fill(frame, full_row_area, background);
        let task = scheduled_tasks.iter().find(|task| task.id == run.task_id);
        let title = task.map_or_else(
            || format!("Scheduled run #{}", run.id),
            |task| task.title.clone(),
        );
        let active = run.status.is_active();
        let status = if active {
            SPINNER_FRAMES[herdr.spinner_frame() % SPINNER_FRAMES.len()]
        } else {
            SPINNER_FRAMES[0]
        };
        let status_width = u16::try_from(UnicodeWidthStr::width(status)).unwrap_or(0);
        let title_width = row_area
            .width
            .saturating_sub(status_width.saturating_add(3));
        frame.render_widget(
            Paragraph::new(truncate_width(&title, usize::from(title_width))).style(
                Style::default()
                    .fg(palette().ink)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(row_area.x + 1, row_area.y, title_width, 1),
        );
        frame.render_widget(
            Paragraph::new(status).style(
                Style::default()
                    .fg(scheduled_run_status_color(run.status))
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(
                row_area.right().saturating_sub(status_width + 1),
                row_area.y,
                status_width,
                1,
            ),
        );
        if row_area.height > 1 {
            let mut detail = task.map_or_else(
                || format!("Run #{}", run.id),
                |task| format!("Run #{}  {} / {}", run.id, task.repository, task.branch),
            );
            if let Some(completed_at_ms) = run.completed_at_ms {
                detail.push_str("  finished ");
                detail.push_str(&relative_completion_time(completed_at_ms));
            }
            frame.render_widget(
                Paragraph::new(truncate_width(
                    &detail,
                    usize::from(row_area.width.saturating_sub(2)),
                ))
                .style(Style::default().fg(palette().soft).bg(background)),
                Rect::new(
                    row_area.x + 1,
                    row_area.y + 1,
                    row_area.width.saturating_sub(2),
                    1,
                ),
            );
        }
        targets.push((HitTarget::AgentScheduledRun(run.id), full_row_area));
        last_card = Some((full_row_area, background));
        animation_presented |= active;
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        draw_trailing_agent_gap(frame, gap, list, background);
    }
    animation_presented
}

fn scheduled_run_status_color(status: ScheduledRunStatus) -> Color {
    match status {
        ScheduledRunStatus::Launching
        | ScheduledRunStatus::Working
        | ScheduledRunStatus::Blocked
        | ScheduledRunStatus::Unknown => palette().accent,
        ScheduledRunStatus::Completed => palette().green,
        ScheduledRunStatus::Failed => palette().red,
    }
}

fn relative_completion_time(completed_at_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        });
    let age_seconds = now_ms.saturating_sub(completed_at_ms).max(0) / 1_000;
    match age_seconds {
        0..=59 => "now".to_owned(),
        60..=3_599 => format!("{}m ago", age_seconds / 60),
        3_600..=86_399 => format!("{}h ago", age_seconds / 3_600),
        _ => format!("{}d ago", age_seconds / 86_400),
    }
}

fn draw_stashed_agents(
    frame: &mut Frame<'_>,
    herdr: &mut HerdrSession,
    list: Rect,
    hovered: Option<HitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if herdr.stashed_agents().is_empty() {
        frame.render_widget(
            Paragraph::new("  No stashed agents").style(
                Style::default()
                    .fg(palette().faint)
                    .bg(palette().surface_alt),
            ),
            list,
        );
        return;
    }
    let card_height = if list.height >= 2 { 2 } else { 1 };
    let card_gap = 1;
    let item_step = card_height + card_gap;
    let top_padding = u16::from(list.height > card_height);
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(top_padding),
        list.width.saturating_sub(1),
        list.height.saturating_sub(top_padding),
    );
    let viewport = complete_card_viewport(card_list.height, item_step);
    let scroll = herdr
        .stash_scroll
        .min(herdr.stashed_agents().len().saturating_sub(viewport));
    let hovered_agent = match hovered {
        Some(HitTarget::StashedAgent(index)) => Some(index),
        _ => None,
    };
    let now_ms = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    let mut last_card = None;
    for (screen_row, index) in (scroll..herdr.stashed_agents().len()).enumerate() {
        if screen_row >= viewport {
            break;
        }
        let offset = u16::try_from(screen_row).unwrap_or(0) * item_step;
        let row_area = Rect::new(
            card_list.x,
            card_list.y.saturating_add(offset),
            card_list.width,
            card_height.min(card_list.height.saturating_sub(offset)),
        );
        let full_row_area = Rect::new(list.x, row_area.y, list.width, row_area.height);
        let agent = &herdr.stashed_agents()[index];
        let is_hovered = hovered_agent == Some(index);
        let background = if is_hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        if row_area.y > list.y {
            let previous_background = index.checked_sub(1).map_or(palette().panel, |previous| {
                if hovered_agent == Some(previous) {
                    palette().selected
                } else {
                    palette().surface_alt
                }
            });
            draw_agent_gap(
                frame,
                Rect::new(list.x, row_area.y - 1, list.width, 1),
                previous_background,
                background,
            );
        }
        fill(frame, full_row_area, background);
        draw_row(
            frame,
            row_area,
            AgentCardDestination {
                repository: &agent.repository_label,
                branch: &agent.branch,
            },
            None,
            agent.session_name.as_deref().unwrap_or(&agent.harness),
            1,
            None,
            Some(&format_duration(Duration::from_millis(
                now_ms.saturating_sub(agent.stashed_at_ms),
            ))),
            AgentStatus::Idle,
            herdr.spinner_frame(),
            AgentEntryState::default(),
            false,
            is_hovered,
            false,
        );
        last_card = Some((full_row_area, background));
        targets.push((HitTarget::StashedAgent(index), full_row_area));
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        draw_trailing_agent_gap(frame, gap, list, background);
    }
}

pub(super) fn draw_history(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    presentation: &mut AgentTranscriptPresentation,
    index: usize,
    selected_message: Option<usize>,
    transcript_scroll: Option<usize>,
    expanded_requests: &[usize],
    user_message_expanded: bool,
    picker_open: bool,
    hovered: Option<HitTarget>,
    prompt: &TextInput,
    prompt_focused: bool,
    prompt_error: Option<&str>,
    prompt_delivery: AgentPromptDelivery,
    status_area: Rect,
    repository_area: Option<Rect>,
    prompt_bottom_padding: u16,
    card_left_inset: u16,
    prompt_delivery_inside: bool,
    area: Rect,
) -> (Vec<(HitTarget, Rect)>, usize, usize, bool) {
    if area.width < 24 || area.height < 4 {
        return (Vec::new(), 0, 0, false);
    }
    let Some(agent_key) = herdr.agent_key(index) else {
        return (Vec::new(), 0, 0, false);
    };
    fill(frame, area, palette().panel);
    let prompt_text_width = usize::from(area.width.saturating_sub(4)).max(1);
    let (prompt_cursor_row, prompt_visual_height) = prompt.visual_metrics(prompt_text_width);
    let desired_prompt_height = u16::try_from(prompt_visual_height)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .max(3);
    let maximum_prompt_height = area.height.saturating_sub(11).max(3).min(area.height);
    let prompt_height = desired_prompt_height.min(maximum_prompt_height);
    let prompt_space = prompt_height
        .saturating_add(prompt_bottom_padding.saturating_mul(2))
        .min(area.height);
    let prompt_bottom_padding = prompt_space.saturating_sub(prompt_height) / 2;
    let prompt_area = Rect::new(
        area.x,
        area.bottom()
            .saturating_sub(prompt_bottom_padding)
            .saturating_sub(prompt_height),
        area.width,
        prompt_height,
    );
    draw_agent_prompt(
        frame,
        herdr,
        index,
        prompt,
        prompt_focused,
        prompt_error,
        prompt_cursor_row,
        prompt_visual_height,
        prompt_area,
    );
    let prompt_pending = herdr.agent_prompt_pending(index);
    let delivery_label = match prompt_pending {
        Some(AgentPromptOutcome::Queued) => "waiting for idle",
        Some(AgentPromptOutcome::Sending) => "sending",
        None if prompt_delivery_inside => match prompt_delivery {
            AgentPromptDelivery::NextRequest => "now",
            AgentPromptDelivery::OnIdle => "on idle",
        },
        None => prompt_delivery.label(),
    };
    let delivery_label = if prompt_delivery_inside {
        format!(" {delivery_label} ")
    } else {
        delivery_label.to_owned()
    };
    let delivery_width = badge_width(&delivery_label).min(area.width.saturating_sub(2));
    let delivery_area = if prompt_delivery_inside {
        Rect::new(
            prompt_area.right().saturating_sub(delivery_width),
            prompt_area.bottom().saturating_sub(1),
            delivery_width,
            1,
        )
    } else {
        Rect::new(
            area.right()
                .saturating_sub(delivery_width)
                .saturating_sub(1),
            prompt_area.y.saturating_sub(1),
            delivery_width,
            1,
        )
    };
    let delivery_background = if prompt_delivery_inside {
        if prompt_focused && prompt_pending.is_none() {
            palette().selected
        } else {
            palette().surface_alt
        }
    } else {
        palette().panel
    };
    draw_badge(
        frame,
        delivery_area,
        &delivery_label,
        palette().cyan,
        delivery_background,
    );
    let area = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(prompt_space),
    );
    let mut navigation_targets = vec![(
        HitTarget::AgentPreviewPrompt(agent_key.clone()),
        prompt_area,
    )];
    if !delivery_area.is_empty() && prompt_pending.is_none() {
        navigation_targets.push((
            HitTarget::AgentPreviewPromptDelivery(agent_key.clone()),
            delivery_area,
        ));
    }
    if area.height < 7 {
        return (navigation_targets, 0, 0, false);
    }
    let transcript = herdr.agent_transcript(index);
    let messages = transcript.map_or(&[][..], |transcript| transcript.messages);
    let message_error = herdr.agent_user_message_error(index);
    let status = herdr
        .agents
        .get(index)
        .map_or(AgentStatus::Unknown, |agent| agent.runtime.status);
    let phase = match status {
        AgentStatus::Working => Some(("LIVE", palette().orange)),
        AgentStatus::Blocked => Some(("PAUSED", palette().red)),
        AgentStatus::Done => Some(("COMPLETE", palette().green)),
        AgentStatus::Idle => None,
        AgentStatus::Unknown => Some(("UNKNOWN", palette().faint)),
    };
    let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
    let repository_width = badge_width(repository).min(area.width);
    let repository_area = repository_area.unwrap_or_else(|| {
        Rect::new(
            area.x
                .saturating_add(area.width.saturating_sub(repository_width) / 2),
            area.y.saturating_sub(1),
            repository_width,
            1,
        )
    });
    if let Some((phase, phase_color)) = phase {
        let phase_width = u16::try_from(UnicodeWidthStr::width(phase)).unwrap_or(u16::MAX);
        let phase_area = Rect::new(status_area.x, status_area.y, status_area.width, 1);
        if phase_area.width >= phase_width.saturating_add(2) {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(phase_color)),
                    Span::styled(
                        phase,
                        Style::default()
                            .fg(phase_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]))
                .alignment(Alignment::Right)
                .style(Style::default().bg(palette().panel)),
                phase_area,
            );
        }
    }
    if repository_area.width >= 3 {
        navigation_targets.push((
            HitTarget::AgentPreviewPicker(agent_key.clone()),
            repository_area,
        ));
    }
    if messages.is_empty() {
        frame.render_widget(
            Paragraph::new(message_error.unwrap_or("Waiting for conversation history…"))
                .style(
                    Style::default()
                        .fg(if message_error.is_some() {
                            palette().red
                        } else {
                            palette().faint
                        })
                        .bg(palette().panel),
                )
                .wrap(Wrap { trim: true }),
            Rect::new(area.x, area.y, area.width, area.height),
        );
        draw_badge(
            frame,
            repository_area,
            repository,
            palette().cyan,
            palette().panel,
        );
        draw_agent_preview_picker(
            frame,
            herdr,
            index,
            repository_area,
            area,
            picker_open,
            hovered,
            &mut navigation_targets,
        );
        return (navigation_targets, 0, 0, false);
    }
    let selected_message = selected_message
        .unwrap_or_else(|| messages.len().saturating_sub(1))
        .min(messages.len().saturating_sub(1));
    let main = Rect::new(area.x, area.y, area.width, area.height);
    let message_selector = Rect::new(main.x, main.y, main.width, 2.min(main.height));
    let content_width = usize::from(main.width.saturating_sub(4).max(1));
    let live = status == AgentStatus::Working && selected_message + 1 == messages.len();
    let cached = presentation.prepare(AgentTranscriptPresentationInput {
        transcript: transcript.expect("non-empty agent messages have a transcript"),
        message: selected_message,
        width: content_width,
        expanded_requests,
        live,
        mode: AgentTranscriptMode::Agent,
    });
    let user_top_padding = u16::from(cached.user_elapsed.is_some());
    let full_user_card_height = cached.user_lines.len().saturating_add(2).max(3);
    let desired_user_height = full_user_card_height;
    let desired_user_height = if user_message_expanded {
        desired_user_height
    } else {
        desired_user_height.min(8)
    }
    .max(3)
    .saturating_add(usize::from(user_top_padding));
    let user_y = message_selector.bottom();
    let user_height = u16::try_from(desired_user_height).unwrap_or(u16::MAX).min(
        main.bottom()
            .saturating_sub(user_y)
            .saturating_sub(if user_message_expanded {
                expanded_user_response_rows(
                    desired_user_height,
                    main.bottom().saturating_sub(user_y),
                    EXPANDED_USER_RESPONSE_ROWS,
                )
            } else {
                0
            }),
    );
    let user_viewport = Rect::new(main.x.saturating_sub(1), user_y, main.width, user_height);
    let viewport = Rect::new(
        main.x.saturating_sub(1),
        user_viewport.bottom(),
        main.width,
        main.bottom().saturating_sub(user_viewport.bottom()),
    );
    let user_cards = Rect::new(
        main.x.saturating_add(card_left_inset),
        user_viewport.y.saturating_add(user_top_padding),
        main.width.saturating_sub(card_left_inset),
        user_viewport.height.saturating_sub(user_top_padding),
    );
    let cards = Rect::new(
        main.x.saturating_add(card_left_inset),
        viewport.y,
        main.width.saturating_sub(card_left_inset),
        viewport.height,
    );
    let scroll_max = if user_message_expanded {
        full_user_card_height.saturating_sub(usize::from(user_cards.height))
    } else {
        cached
            .request_height
            .saturating_sub(usize::from(viewport.height))
    };
    let scroll = if user_message_expanded {
        transcript_scroll.unwrap_or(0).min(scroll_max)
    } else {
        transcript_scroll.unwrap_or(scroll_max).min(scroll_max)
    };
    let mut targets = vec![(
        HitTarget::AgentTooltip {
            agent: agent_key.clone(),
            message: selected_message,
        },
        area,
    )];
    let mut animation_presented = false;
    draw_message_timeline(
        frame,
        Rect::new(
            message_selector.x,
            message_selector.y.saturating_add(1),
            message_selector.width,
            message_selector.height.saturating_sub(1).min(1),
        ),
        Some(agent_key.clone()),
        selected_message,
        messages.len(),
        &mut targets,
    );
    targets.extend(navigation_targets);
    draw_badge(
        frame,
        repository_area,
        repository,
        palette().cyan,
        palette().panel,
    );
    let user_block = TranscriptBlock {
        user: true,
        headerless_response: false,
        lines: cached.user_lines.clone(),
        animated_rows: Arc::default(),
        start: 0,
        height: if user_message_expanded {
            full_user_card_height
        } else {
            usize::from(user_cards.height)
        },
        elapsed: cached.user_elapsed.clone(),
        request_count: cached.request_count,
        request: None,
        expandable: false,
    };
    if let Some((rect, _)) = draw_transcript_card(
        frame,
        &user_block,
        user_cards,
        user_cards,
        if user_message_expanded { scroll } else { 0 },
        herdr.spinner_frame(),
    ) {
        targets.push((
            if user_message_expanded {
                HitTarget::AgentExpandedMessage {
                    agent: agent_key.clone(),
                    message: selected_message,
                }
            } else {
                HitTarget::AgentMessage {
                    agent: agent_key.clone(),
                    message: selected_message,
                }
            },
            rect,
        ));
    }
    let request_scroll = if user_message_expanded { 0 } else { scroll };
    for block in &cached.blocks {
        if let Some((rect, block_animation_presented)) = draw_transcript_card(
            frame,
            block,
            cards,
            viewport,
            request_scroll,
            herdr.spinner_frame(),
        ) {
            animation_presented |= block_animation_presented;
            if block.expandable
                && let Some(request) = block.request
            {
                targets.push((
                    HitTarget::AgentPreviewRequest {
                        agent: agent_key.clone(),
                        message: selected_message,
                        request,
                    },
                    rect,
                ));
            }
        }
    }
    draw_agent_preview_picker(
        frame,
        herdr,
        index,
        repository_area,
        area,
        picker_open,
        hovered,
        &mut targets,
    );
    (targets, scroll_max, scroll, animation_presented)
}

fn draw_agent_prompt(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    index: usize,
    input: &TextInput,
    focused: bool,
    local_error: Option<&str>,
    cursor_row: usize,
    visual_height: usize,
    area: Rect,
) {
    let sending = herdr.agent_prompt_sending(index);
    let error = local_error.or_else(|| herdr.agent_prompt_error(index));
    draw_preview_prompt(
        frame,
        input,
        focused,
        error,
        sending,
        cursor_row,
        visual_height,
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_preview_prompt(
    frame: &mut Frame<'_>,
    input: &TextInput,
    focused: bool,
    error: Option<&str>,
    sending: bool,
    cursor_row: usize,
    visual_height: usize,
    area: Rect,
) {
    let active = focused && !sending;
    let background = if active {
        palette().selected
    } else {
        palette().surface_alt
    };
    fill(frame, area, background);
    if area.is_empty() {
        return;
    }
    let input_area = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    frame.render_widget(
        Paragraph::new("›").style(Style::default().fg(palette().cyan).bg(background)),
        Rect::new(
            area.x.saturating_add(1),
            input_area.y,
            2.min(area.width.saturating_sub(1)),
            1,
        ),
    );
    if input.text().is_empty() && !active && error.is_some() {
        frame.render_widget(
            Paragraph::new(error.unwrap_or_default())
                .style(Style::default().fg(palette().red).bg(background)),
            input_area,
        );
    } else {
        let height = usize::from(input_area.height).max(1);
        let scroll = cursor_row
            .saturating_sub(height.saturating_sub(1))
            .min(visual_height.saturating_sub(height));
        frame.render_widget(
            Paragraph::new(text_input_lines(input, active, palette().ink))
                .wrap(Wrap { trim: false })
                .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
                .style(Style::default().bg(background)),
            input_area,
        );
    }
}

pub(super) fn draw_scheduled_history(
    frame: &mut Frame<'_>,
    run_id: i64,
    transcript: Option<AgentTranscript<'_>>,
    presentation: &mut AgentTranscriptPresentation,
    selected_message: Option<usize>,
    transcript_scroll: Option<usize>,
    expanded_requests: &[usize],
    user_message_expanded: bool,
    prompt: &TextInput,
    prompt_focused: bool,
    prompt_error: Option<&str>,
    prompt_sending: bool,
    prompt_available: bool,
    conversation_message: Option<&str>,
    prompt_bottom_padding: u16,
    prompt_delivery_inside: bool,
    mut area: Rect,
) -> (Vec<(HitTarget, Rect)>, usize, usize) {
    fill(frame, area, palette().panel);
    let mut targets = Vec::new();
    if prompt_available {
        let prompt_text_width = usize::from(area.width.saturating_sub(4)).max(1);
        let (prompt_cursor_row, prompt_visual_height) = prompt.visual_metrics(prompt_text_width);
        let desired_prompt_height = u16::try_from(prompt_visual_height)
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .max(3);
        let prompt_height =
            desired_prompt_height.min(area.height.saturating_sub(11).max(3).min(area.height));
        let prompt_space = prompt_height
            .saturating_add(prompt_bottom_padding.saturating_mul(2))
            .min(area.height);
        let prompt_area = Rect::new(
            area.x,
            area.bottom()
                .saturating_sub(prompt_space.saturating_sub(prompt_height) / 2)
                .saturating_sub(prompt_height),
            area.width,
            prompt_height,
        );
        draw_preview_prompt(
            frame,
            prompt,
            prompt_focused,
            prompt_error,
            prompt_sending,
            prompt_cursor_row,
            prompt_visual_height,
            prompt_area,
        );
        let delivery_label = if prompt_sending { "sending" } else { "now" };
        let delivery_label = if prompt_delivery_inside {
            format!(" {delivery_label} ")
        } else {
            delivery_label.to_owned()
        };
        let delivery_width = badge_width(&delivery_label).min(area.width.saturating_sub(2));
        let delivery_area = if prompt_delivery_inside {
            Rect::new(
                prompt_area.right().saturating_sub(delivery_width),
                prompt_area.bottom().saturating_sub(1),
                delivery_width,
                1,
            )
        } else {
            Rect::new(
                area.right()
                    .saturating_sub(delivery_width)
                    .saturating_sub(1),
                prompt_area.y.saturating_sub(1),
                delivery_width,
                1,
            )
        };
        let delivery_background = if prompt_delivery_inside {
            if prompt_focused && !prompt_sending {
                palette().selected
            } else {
                palette().surface_alt
            }
        } else {
            palette().panel
        };
        draw_badge(
            frame,
            delivery_area,
            &delivery_label,
            palette().cyan,
            delivery_background,
        );
        targets.push((HitTarget::AgentPreviewScheduledPrompt(run_id), prompt_area));
        area.height = area.height.saturating_sub(prompt_space);
    }
    let messages = transcript.map_or(&[][..], |transcript| transcript.messages);
    if messages.is_empty() {
        frame.render_widget(
            Paragraph::new(conversation_message.unwrap_or("Loading conversation history…"))
                .style(Style::default().fg(palette().faint).bg(palette().panel)),
            area,
        );
        return (targets, 0, 0);
    }
    let selected_message = selected_message
        .unwrap_or_else(|| messages.len().saturating_sub(1))
        .min(messages.len().saturating_sub(1));
    let selector = Rect::new(area.x, area.y, area.width, 2.min(area.height));
    let content_width = usize::from(area.width.saturating_sub(4).max(1));
    let cached = presentation.prepare(AgentTranscriptPresentationInput {
        transcript: transcript.expect("non-empty scheduled messages have a transcript"),
        message: selected_message,
        width: content_width,
        expanded_requests,
        live: false,
        mode: AgentTranscriptMode::Scheduled,
    });
    let user_top_padding = u16::from(cached.user_elapsed.is_some());
    let full_user_card_height = cached.user_lines.len().saturating_add(2).max(3);
    let desired_user_height = full_user_card_height;
    let desired_user_height = if user_message_expanded {
        desired_user_height
    } else {
        desired_user_height.min(8)
    }
    .max(3)
    .saturating_add(usize::from(user_top_padding));
    let user_height = u16::try_from(desired_user_height).unwrap_or(u16::MAX).min(
        area.bottom()
            .saturating_sub(selector.bottom())
            .saturating_sub(if user_message_expanded {
                expanded_user_response_rows(
                    desired_user_height,
                    area.bottom().saturating_sub(selector.bottom()),
                    0,
                )
            } else {
                0
            }),
    );
    let user_viewport = Rect::new(
        area.x.saturating_sub(1),
        selector.bottom(),
        area.width,
        user_height,
    );
    let viewport = Rect::new(
        area.x.saturating_sub(1),
        user_viewport.bottom(),
        area.width,
        area.bottom().saturating_sub(user_viewport.bottom()),
    );
    let user_cards = Rect::new(
        area.x,
        user_viewport.y.saturating_add(user_top_padding),
        area.width,
        user_viewport.height.saturating_sub(user_top_padding),
    );
    let cards = Rect::new(area.x, viewport.y, area.width, viewport.height);
    let scroll_max = if user_message_expanded {
        full_user_card_height.saturating_sub(usize::from(user_cards.height))
    } else {
        cached
            .request_height
            .saturating_sub(usize::from(viewport.height))
    };
    let scroll = if user_message_expanded {
        transcript_scroll.unwrap_or(0).min(scroll_max)
    } else {
        transcript_scroll.unwrap_or(scroll_max).min(scroll_max)
    };
    draw_message_timeline(
        frame,
        Rect::new(
            selector.x,
            selector.y.saturating_add(1),
            selector.width,
            selector.height.saturating_sub(1).min(1),
        ),
        None,
        selected_message,
        messages.len(),
        &mut targets,
    );
    let user_block = TranscriptBlock {
        user: true,
        headerless_response: false,
        lines: cached.user_lines.clone(),
        animated_rows: Arc::default(),
        start: 0,
        height: if user_message_expanded {
            full_user_card_height
        } else {
            usize::from(user_cards.height)
        },
        elapsed: cached.user_elapsed.clone(),
        request_count: cached.request_count,
        request: None,
        expandable: false,
    };
    if let Some((rect, _)) = draw_transcript_card(
        frame,
        &user_block,
        user_cards,
        user_cards,
        if user_message_expanded { scroll } else { 0 },
        0,
    ) {
        targets.push((
            HitTarget::AgentScheduledMessage {
                run_id,
                message: selected_message,
            },
            rect,
        ));
    }
    let request_scroll = if user_message_expanded { 0 } else { scroll };
    for block in &cached.blocks {
        if let Some((rect, _)) =
            draw_transcript_card(frame, block, cards, viewport, request_scroll, 0)
            && block.expandable
            && let Some(request) = block.request
        {
            targets.push((
                HitTarget::AgentPreviewScheduledRequest { run_id, request },
                rect,
            ));
        }
    }
    (targets, scroll_max, scroll)
}

fn draw_agent_preview_picker(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    selected: usize,
    anchor: Rect,
    bounds: Rect,
    open: bool,
    hovered: Option<HitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if !open || anchor.width == 0 || herdr.agents.is_empty() {
        return;
    }
    let y = anchor.bottom();
    let available_height = bounds.bottom().saturating_sub(y);
    if available_height < 2 {
        return;
    }
    let desired_width = herdr
        .agents
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
            let agent = herdr.agent_display_name(index).unwrap_or("agent");
            UnicodeWidthStr::width(repository)
                .saturating_add(UnicodeWidthStr::width(agent))
                .saturating_add(7)
        })
        .max()
        .unwrap_or(0);
    let width = u16::try_from(desired_width)
        .unwrap_or(u16::MAX)
        .max(anchor.width)
        .max(24.min(bounds.width))
        .min(58)
        .min(bounds.width);
    let maximum_x = bounds.right().saturating_sub(width);
    let x = anchor
        .x
        .saturating_add(anchor.width / 2)
        .saturating_sub(width / 2)
        .clamp(bounds.x, maximum_x);
    let item_count = herdr
        .agents
        .len()
        .min(usize::from(available_height.saturating_sub(1)));
    let height = u16::try_from(item_count.saturating_add(1)).unwrap_or(available_height);
    let popover = Rect::new(x, y, width, height);
    frame.render_widget(Clear, popover);
    fill(frame, popover, palette().surface_alt);
    frame.render_widget(
        Paragraph::new(format!(" AGENTS · {}", herdr.agents.len())).style(
            Style::default()
                .fg(palette().muted)
                .bg(palette().surface_alt),
        ),
        Rect::new(x, y, width, 1),
    );
    if let Some(agent_key) = herdr.agent_key(selected) {
        targets.push((HitTarget::AgentPreviewPicker(agent_key), popover));
    }
    for (row, index) in (0..herdr.agents.len()).take(item_count).enumerate() {
        let rect = Rect::new(
            x,
            y.saturating_add(1)
                .saturating_add(u16::try_from(row).unwrap_or(0)),
            width,
            1,
        );
        let current = index == selected;
        let Some(agent_key) = herdr.agent_key(index) else {
            continue;
        };
        let hovered =
            hovered.as_ref() == Some(&HitTarget::AgentPreviewPickerItem(agent_key.clone()));
        let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
        let agent = herdr.agent_display_name(index).unwrap_or("agent");
        let background = if hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        fill(frame, rect, background);
        let detail_width = u16::try_from(UnicodeWidthStr::width(agent))
            .unwrap_or(u16::MAX)
            .min(rect.width.saturating_sub(8))
            .min(rect.width / 2);
        let detail = Rect::new(
            rect.right().saturating_sub(detail_width).saturating_sub(1),
            rect.y,
            detail_width,
            1,
        );
        let label = Rect::new(rect.x, rect.y, detail.x.saturating_sub(rect.x), 1);
        frame.render_widget(
            Paragraph::new(truncate_width(
                &format!(" {} {repository}", if current { "●" } else { " " }),
                usize::from(label.width),
            ))
            .style(
                Style::default()
                    .fg(if current {
                        palette().accent
                    } else {
                        palette().ink
                    })
                    .bg(background)
                    .add_modifier(if current {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            label,
        );
        frame.render_widget(
            Paragraph::new(truncate_width(agent, usize::from(detail.width)))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted).bg(background)),
            detail,
        );
        targets.push((HitTarget::AgentPreviewPickerItem(agent_key), rect));
    }
}

const REQUEST_CARD_EXTRA_ROWS: usize = 2;

fn build_request_transcript_counted(
    message: &AgentUserMessage,
    width: usize,
    live: bool,
    expanded_requests: &[usize],
    stats: &mut AgentTranscriptBuildStats,
) -> (Vec<TranscriptBlock>, usize) {
    let mut blocks = Vec::new();
    let mut document_height = 0usize;
    let request_count = message.requests.len();
    for (request_index, request) in message.requests.iter().enumerate() {
        let headerless_response = request
            .parts
            .iter()
            .any(|part| matches!(part, AgentRequestPartPreview::Text(_)))
            && !request.parts.iter().any(|part| {
                matches!(
                    part,
                    AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning)
                )
            });
        let extra_rows = if headerless_response {
            1
        } else {
            REQUEST_CARD_EXTRA_ROWS
        };
        let final_request = request_index + 1 == request_count;
        let request_live = live && request_index + 1 == request_count;
        let expanded = expanded_requests.contains(&request_index);
        let (lines, height, animated_rows, expandable) = if final_request {
            let (lines, content_height, animated_rows) =
                request_content(Some(request), width, request_live, stats);
            (
                lines,
                content_height.max(1).saturating_add(extra_rows),
                animated_rows,
                false,
            )
        } else {
            let RequestSummary {
                lines: summary,
                reasoning,
                hidden,
                animated_rows: summary_animated_rows,
            } = request_summary(request, width, request_live, stats);
            if hidden && !expanded {
                let mut lines = Vec::new();
                let mut animated_rows = Vec::new();
                if let Some((reasoning, animated)) = reasoning {
                    if animated {
                        animated_rows.push(lines.len());
                    }
                    lines.push(reasoning);
                    if request
                        .parts
                        .iter()
                        .any(|part| matches!(part, AgentRequestPartPreview::Text(_)))
                    {
                        lines.push(agent_output_transition_line('▄'));
                    }
                }
                let summary_offset = lines.len();
                animated_rows.extend(
                    summary_animated_rows
                        .into_iter()
                        .map(|row| summary_offset.saturating_add(row)),
                );
                lines.extend(summary);
                lines.push(Line::styled(
                    "⌄ more",
                    Style::default()
                        .fg(palette().cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                let height = lines.len().saturating_add(extra_rows);
                (lines, height, animated_rows, true)
            } else {
                let (lines, content_height, animated_rows) =
                    request_content(Some(request), width, request_live, stats);
                (
                    lines,
                    content_height.max(1).saturating_add(extra_rows),
                    animated_rows,
                    hidden,
                )
            }
        };
        let elapsed = request.duration_ms.map(format_preview_duration);
        document_height = document_height.saturating_add(usize::from(elapsed.is_some()));
        blocks.push(TranscriptBlock {
            user: false,
            headerless_response,
            lines: lines.into(),
            animated_rows: animated_rows.into(),
            start: document_height,
            height,
            elapsed,
            request_count: 0,
            request: Some(request_index),
            expandable,
        });
        document_height = document_height.saturating_add(height);
    }
    if blocks.is_empty() {
        let (lines, height, animated_rows) = request_content(None, width, live, stats);
        blocks.push(TranscriptBlock {
            user: false,
            headerless_response: false,
            lines: lines.into(),
            animated_rows: animated_rows.into(),
            start: 0,
            height: height.saturating_add(REQUEST_CARD_EXTRA_ROWS),
            elapsed: None,
            request_count: 0,
            request: None,
            expandable: false,
        });
        document_height = height.saturating_add(REQUEST_CARD_EXTRA_ROWS);
    }
    (blocks, document_height)
}

#[cfg(test)]
fn build_request_transcript(
    message: &AgentUserMessage,
    width: usize,
    live: bool,
    _spinner_frame: usize,
    expanded_requests: &[usize],
) -> (Vec<TranscriptBlock>, usize) {
    build_request_transcript_counted(
        message,
        width,
        live,
        expanded_requests,
        &mut AgentTranscriptBuildStats::default(),
    )
}

fn styled_agent_text(text: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    hard_wrap_preview_lines(
        styled_markdown_preserving_breaks(text, width, true),
        width,
        0,
        usize::MAX,
        false,
        false,
    )
}

fn styled_agent_text_counted(
    text: &str,
    width: usize,
    stats: &mut AgentTranscriptBuildStats,
) -> Vec<Line<'static>> {
    stats.markdown_parses = stats.markdown_parses.saturating_add(1);
    stats.static_wraps = stats.static_wraps.saturating_add(1);
    stats.markdown_input_bytes = stats.markdown_input_bytes.saturating_add(text.len());
    styled_agent_text(text, width)
}

fn styled_agent_output_text_counted(
    text: &str,
    width: usize,
    stats: &mut AgentTranscriptBuildStats,
) -> Vec<Line<'static>> {
    style_agent_output_lines(styled_agent_text_counted(text, width, stats), width)
}

#[cfg(test)]
fn styled_agent_output_text(text: &str, width: usize) -> Vec<Line<'static>> {
    style_agent_output_lines(styled_agent_text(text, width), width)
}

fn styled_agent_output_text_window_counted(
    text: &str,
    width: usize,
    rows: usize,
    stats: &mut AgentTranscriptBuildStats,
) -> (Vec<Line<'static>>, bool) {
    let byte_limit = width
        .max(1)
        .saturating_mul(rows.max(1))
        .saturating_mul(8)
        .clamp(512, 8_192);
    let mut end = text.len().min(byte_limit);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let source = &text[..end];
    let mut lines = styled_agent_output_text_counted(source, width, stats);
    let truncated = end < text.len() || lines.len() > rows;
    lines.truncate(rows);
    (lines, truncated)
}

fn style_agent_output_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut output = Vec::new();
    let mut code_block = false;
    for mut line in lines {
        let code_row = line.style.bg == Some(palette().surface_alt);
        if code_row && !code_block {
            output.push(agent_code_border('▄', width));
        } else if !code_row && code_block {
            output.push(agent_code_border('▀', width));
        }
        if code_row {
            line.style = Style::default().bg(palette().raised);
            for span in &mut line.spans {
                span.style = span.style.bg(palette().raised);
            }
        } else {
            line.style = Style::default().bg(palette().panel);
        }
        output.push(line);
        code_block = code_row;
    }
    if code_block {
        output.push(agent_code_border('▀', width));
    }
    output
}

fn agent_code_border(glyph: char, width: usize) -> Line<'static> {
    Line::styled(
        glyph.to_string().repeat(width.max(1)),
        Style::default().fg(palette().raised).bg(palette().panel),
    )
}

fn agent_output_background_line() -> Line<'static> {
    Line::default().style(Style::default().bg(palette().panel))
}

fn agent_output_transition_line(glyph: char) -> Line<'static> {
    Line::styled(
        glyph.to_string(),
        Style::default().fg(palette().panel).bg(palette().canvas),
    )
}

fn agent_output_transition_glyph(line: &Line<'_>) -> Option<char> {
    let [span] = line.spans.as_slice() else {
        return None;
    };
    match span.content.as_ref() {
        "▀" => Some('▀'),
        "▄" => Some('▄'),
        _ => None,
    }
}

fn is_agent_output_row(line: &Line<'_>) -> bool {
    line.style.bg == Some(palette().panel) && agent_output_transition_glyph(line).is_none()
}

fn draw_transcript_card(
    frame: &mut Frame<'_>,
    block: &TranscriptBlock,
    cards: Rect,
    viewport: Rect,
    scroll: usize,
    spinner_frame: usize,
) -> Option<(Rect, bool)> {
    let visible_start = block.start.max(scroll);
    let visible_end = block
        .start
        .saturating_add(block.height)
        .min(scroll.saturating_add(usize::from(viewport.height)));
    if visible_start >= visible_end {
        return None;
    }
    let local_start = visible_start.saturating_sub(block.start);
    let local_end = visible_end.saturating_sub(block.start);
    let y = viewport
        .y
        .saturating_add(u16::try_from(visible_start.saturating_sub(scroll)).unwrap_or(u16::MAX));
    let visible = Rect::new(
        cards.x,
        y,
        cards.width,
        u16::try_from(visible_end - visible_start).unwrap_or(u16::MAX),
    );
    let background = palette().canvas;
    let accent = if block.user {
        palette().yellow
    } else {
        palette().cyan
    };
    let content_offset = usize::from(!block.headerless_response);
    let middle_start = local_start.max(content_offset);
    let middle_end = local_end.min(block.height.saturating_sub(1));
    if middle_start < middle_end {
        let middle = Rect::new(
            cards.x,
            y.saturating_add(u16::try_from(middle_start - local_start).unwrap_or(0)),
            cards.width,
            u16::try_from(middle_end - middle_start).unwrap_or(u16::MAX),
        );
        fill(frame, middle, background);
        frame.render_widget(
            Paragraph::new("┃\n".repeat(usize::from(middle.height))).style(
                Style::default()
                    .fg(accent)
                    .bg(background)
                    .remove_modifier(Modifier::DIM),
            ),
            Rect::new(middle.x, middle.y, 1, middle.height),
        );
    }
    if local_start == 0 && !block.headerless_response {
        {
            frame.render_widget(
                Paragraph::new("▄".repeat(usize::from(cards.width)))
                    .style(Style::default().fg(background).bg(palette().panel)),
                Rect::new(cards.x, y, cards.width, 1),
            );
        }
        let label = if block.user {
            let request_word = if block.request_count == 1 {
                "request"
            } else {
                "requests"
            };
            if let Some(elapsed) = block.elapsed.as_deref() {
                Some(format!(
                    " {} {request_word} · total {elapsed} ",
                    block.request_count
                ))
            } else {
                Some(format!(" {} {request_word} ", block.request_count))
            }
        } else if block.headerless_response {
            None
        } else {
            block
                .elapsed
                .as_deref()
                .map(|elapsed| format!(" {elapsed} "))
        };
        if let Some(label) = label {
            let width = u16::try_from(UnicodeWidthStr::width(label.as_str())).unwrap_or(u16::MAX);
            if cards.width > width.saturating_add(2) {
                frame.render_widget(
                    Paragraph::new(label).style(
                        Style::default()
                            .fg(accent)
                            .bg(background)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Rect::new(
                        cards.right().saturating_sub(width).saturating_sub(1),
                        y,
                        width,
                        1,
                    ),
                );
            }
        }
    }
    let mut animation_presented = false;
    let content_start = local_start.max(content_offset);
    let content_end = local_end.min(block.height.saturating_sub(content_offset));
    if content_start < content_end {
        let content = Rect::new(
            cards.x.saturating_add(2),
            y.saturating_add(u16::try_from(content_start - local_start).unwrap_or(0)),
            cards.width.saturating_sub(3),
            u16::try_from(content_end - content_start).unwrap_or(u16::MAX),
        );
        let line_start = content_start.saturating_sub(content_offset);
        let line_end = line_start.saturating_add(usize::from(content.height));
        animation_presented = block
            .animated_rows
            .iter()
            .any(|row| (line_start..line_end).contains(row));
        for (row, line) in block
            .lines
            .iter()
            .skip(line_start)
            .take(usize::from(content.height))
            .enumerate()
        {
            if agent_output_transition_glyph(line).is_none()
                && matches!(line.style.bg, Some(background) if background == palette().panel || background == palette().raised)
            {
                let code_row = line.style.bg == Some(palette().raised);
                let band = Rect::new(
                    if code_row { content.x } else { cards.x },
                    content
                        .y
                        .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                    if code_row { content.width } else { cards.width },
                    1,
                );
                frame.render_widget(Clear, band);
                fill(
                    frame,
                    band,
                    if code_row {
                        palette().raised
                    } else {
                        palette().panel
                    },
                );
            }
        }
        for (row, line) in block
            .lines
            .iter()
            .skip(line_start)
            .take(usize::from(content.height))
            .enumerate()
        {
            let line_area = Rect::new(
                content.x,
                content
                    .y
                    .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                content.width,
                1,
            );
            frame
                .buffer_mut()
                .set_style(line_area, Style::default().fg(palette().soft));
            frame.render_widget(line, line_area);
        }
        let spinner = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
        for row in block
            .animated_rows
            .iter()
            .copied()
            .filter(|row| (line_start..line_end).contains(row))
        {
            let y = content
                .y
                .saturating_add(u16::try_from(row.saturating_sub(line_start)).unwrap_or(u16::MAX));
            if let Some(cell) = frame.buffer_mut().cell_mut((content.x, y)) {
                cell.set_symbol(spinner);
            }
        }
        for (row, line) in block
            .lines
            .iter()
            .skip(line_start)
            .take(usize::from(content.height))
            .enumerate()
        {
            let Some(glyph) = agent_output_transition_glyph(line) else {
                continue;
            };
            let symbol = glyph.to_string();
            let y = content
                .y
                .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
            for x in cards.x..cards.right() {
                if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                    cell.set_symbol(&symbol)
                        .set_fg(palette().panel)
                        .set_bg(background);
                }
            }
        }
    }
    if block.headerless_response
        && local_start == 0
        && local_end > 0
        && let Some(elapsed) = block.elapsed.as_deref()
    {
        let width = u16::try_from(UnicodeWidthStr::width(elapsed)).unwrap_or(u16::MAX);
        if cards.width > width.saturating_add(2) {
            frame.render_widget(
                Paragraph::new(elapsed).style(
                    Style::default()
                        .fg(accent)
                        .bg(palette().panel)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(
                    cards.right().saturating_sub(width).saturating_sub(2),
                    y,
                    width,
                    1,
                ),
            );
        }
    }
    if local_end == block.height {
        let bottom_y = y.saturating_add(
            u16::try_from(local_end.saturating_sub(local_start).saturating_sub(1)).unwrap_or(0),
        );
        let bottom = Rect::new(cards.x, bottom_y, cards.width, 1);
        if !block.user && block.lines.last().is_some_and(is_agent_output_row) {
            frame.render_widget(Clear, bottom);
            fill(frame, bottom, palette().panel);
        } else {
            frame.render_widget(
                Paragraph::new("▀".repeat(usize::from(cards.width)))
                    .style(Style::default().fg(background).bg(palette().panel)),
                bottom,
            );
        }
    }
    Some((visible, animation_presented))
}

fn request_content(
    request: Option<&AgentRequestPreview>,
    width: usize,
    live: bool,
    stats: &mut AgentTranscriptBuildStats,
) -> (Vec<Line<'static>>, usize, Vec<usize>) {
    let Some(request) = request else {
        return (
            vec![Line::styled(
                "Waiting for agent output...",
                Style::default().fg(palette().faint),
            )],
            1,
            Vec::new(),
        );
    };
    let mut lines = Vec::new();
    let mut animated_rows = Vec::new();
    let mut height = 0;
    let mut reasoning_seen = false;
    for (index, part) in request.parts.iter().enumerate() {
        let AgentRequestPartPreview::Activity(activity) = part else {
            let AgentRequestPartPreview::Text(text) = part else {
                unreachable!();
            };
            if index > 0
                && request
                    .parts
                    .get(index - 1)
                    .is_some_and(|part| matches!(part, AgentRequestPartPreview::Activity(_)))
            {
                lines.push(agent_output_transition_line('▄'));
                height += 1;
            }
            if index == 0
                || request
                    .parts
                    .get(index - 1)
                    .is_some_and(|part| matches!(part, AgentRequestPartPreview::Activity(_)))
            {
                lines.push(agent_output_background_line());
                height += 1;
            }
            let text_lines = styled_agent_output_text_counted(text, width, stats);
            height += text_lines.len().max(1);
            lines.extend(text_lines);
            let followed_by_activity = request
                .parts
                .get(index + 1)
                .is_some_and(|part| matches!(part, AgentRequestPartPreview::Activity(_)));
            if followed_by_activity || index + 1 == request.parts.len() {
                lines.push(agent_output_background_line());
                height += 1;
            }
            if followed_by_activity {
                lines.push(agent_output_transition_line('▀'));
                height += 1;
            }
            continue;
        };
        if matches!(activity, AgentActivityPreview::Reasoning) {
            if reasoning_seen {
                continue;
            }
            reasoning_seen = true;
        }
        let (line, animated) = match activity {
            AgentActivityPreview::Reasoning => reasoning_line(request, width, live),
            AgentActivityPreview::Tool { .. } => tool_line(activity, width, live),
        };
        height += 1;
        if animated {
            animated_rows.push(lines.len());
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "Waiting for agent output...",
            Style::default().fg(palette().faint),
        ));
        height = 1;
    }
    (lines, height, animated_rows)
}

fn request_summary(
    request: &AgentRequestPreview,
    width: usize,
    live: bool,
    stats: &mut AgentTranscriptBuildStats,
) -> RequestSummary {
    const TEXT_ROWS: usize = 3;
    const TOOL_ROWS: usize = 3;

    let mut text = Vec::new();
    let mut hidden_text = false;
    for text_part in request.parts.iter().filter_map(|part| match part {
        AgentRequestPartPreview::Text(text) => Some(text.as_str()),
        AgentRequestPartPreview::Activity(_) => None,
    }) {
        if text.len() >= TEXT_ROWS {
            hidden_text |= !text_part.is_empty();
            continue;
        }
        let remaining = TEXT_ROWS.saturating_sub(text.len());
        let (lines, truncated) =
            styled_agent_output_text_window_counted(text_part, width, remaining, stats);
        hidden_text |= truncated;
        text.extend(lines);
    }
    let tool_count = request
        .parts
        .iter()
        .filter(|part| {
            matches!(
                part,
                AgentRequestPartPreview::Activity(AgentActivityPreview::Tool { .. })
            )
        })
        .count();
    let tools = request
        .parts
        .iter()
        .filter_map(|part| match part {
            AgentRequestPartPreview::Activity(activity @ AgentActivityPreview::Tool { .. }) => {
                Some(tool_line(activity, width, live))
            }
            AgentRequestPartPreview::Text(_)
            | AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning) => None,
        })
        .take(TOOL_ROWS)
        .collect::<Vec<_>>();
    let reasoning = request.parts.iter().find_map(|part| match part {
        AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning) => {
            Some(reasoning_line(request, width, live))
        }
        AgentRequestPartPreview::Text(_)
        | AgentRequestPartPreview::Activity(AgentActivityPreview::Tool { .. }) => None,
    });
    let hidden = hidden_text || tool_count > TOOL_ROWS;
    let visible_text = text;
    let visible_tools = tools;
    let mut lines = Vec::new();
    let mut animated_rows = Vec::new();
    if !visible_text.is_empty() {
        lines.push(agent_output_background_line());
        lines.extend(visible_text);
        lines.push(agent_output_background_line());
        if !visible_tools.is_empty() {
            lines.push(agent_output_transition_line('▀'));
        }
    }
    for (line, animated) in visible_tools {
        if animated {
            animated_rows.push(lines.len());
        }
        lines.push(line);
    }
    RequestSummary {
        lines,
        reasoning,
        hidden,
        animated_rows,
    }
}

fn reasoning_line(
    request: &AgentRequestPreview,
    width: usize,
    live: bool,
) -> (Line<'static>, bool) {
    let active = live && request.reasoning_active;
    let prefix = if active { SPINNER_FRAMES[0] } else { "›" };
    let mut spans = Vec::new();
    let mut remaining = width;
    push_truncated_span(
        &mut spans,
        prefix,
        Style::default().fg(palette().orange),
        &mut remaining,
    );
    push_truncated_span(
        &mut spans,
        " reasoning",
        Style::default()
            .fg(palette().orange)
            .add_modifier(Modifier::BOLD),
        &mut remaining,
    );
    if let Some(duration_ms) = request.reasoning_duration_ms {
        push_truncated_span(
            &mut spans,
            &format!("  {}", format_preview_duration(duration_ms)),
            Style::default().fg(palette().faint),
            &mut remaining,
        );
    }
    (Line::from(spans), active)
}

fn tool_line(activity: &AgentActivityPreview, width: usize, live: bool) -> (Line<'static>, bool) {
    let AgentActivityPreview::Tool {
        name,
        title,
        running,
    } = activity
    else {
        unreachable!();
    };
    let active = live && *running;
    let prefix = if active { SPINNER_FRAMES[0] } else { "›" };
    let mut spans = Vec::new();
    let mut remaining = width;
    push_truncated_span(
        &mut spans,
        prefix,
        Style::default().fg(palette().cyan),
        &mut remaining,
    );
    push_truncated_span(
        &mut spans,
        " tool  ",
        Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD),
        &mut remaining,
    );
    push_truncated_span(
        &mut spans,
        name,
        Style::default().fg(palette().accent),
        &mut remaining,
    );
    if let Some(title) = title {
        push_truncated_span(&mut spans, "  ", Style::default(), &mut remaining);
        push_truncated_span(
            &mut spans,
            title,
            Style::default().fg(palette().soft),
            &mut remaining,
        );
    }
    (Line::from(spans), active)
}

fn push_truncated_span(
    spans: &mut Vec<Span<'static>>,
    value: &str,
    style: Style,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    let value = truncate_width(value, *remaining);
    *remaining = remaining.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    spans.push(Span::styled(value, style));
}

fn draw_message_timeline(
    frame: &mut Frame<'_>,
    area: Rect,
    agent: Option<AgentKey>,
    message: usize,
    message_count: usize,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if area.height == 0 || area.width == 0 || message_count == 0 {
        return;
    }
    let capacity = usize::from(area.width).div_ceil(2).max(1);
    let visible = message_count.min(capacity);
    let start = message
        .saturating_sub(visible / 2)
        .min(message_count.saturating_sub(visible));
    let timeline_width = u16::try_from(visible.saturating_mul(2).saturating_sub(1))
        .unwrap_or(u16::MAX)
        .min(area.width);
    let timeline = Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(timeline_width) / 2),
        area.y,
        timeline_width,
        1,
    );
    let mut spans = Vec::with_capacity(visible.saturating_mul(2).saturating_sub(1));
    for index in start..start.saturating_add(visible) {
        if index > start {
            spans.push(Span::raw(" "));
        }
        let selected = index == message;
        spans.push(Span::styled(
            if selected { "●" } else { "○" },
            Style::default()
                .fg(if selected {
                    palette().cyan
                } else {
                    palette().faint
                })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(palette().panel)),
        timeline,
    );
    if let Some(agent) = agent {
        targets.push((HitTarget::AgentPreviewMessageTimeline(agent), area));
    }
}

fn message_total_duration(message: &AgentUserMessage) -> Option<u64> {
    message
        .requests
        .iter()
        .filter_map(|request| request.duration_ms)
        .reduce(u64::saturating_add)
}

fn row_background(state: &AgentEntryState, hovered: bool) -> Color {
    if state.selected || hovered {
        palette().selected
    } else {
        palette().surface_alt
    }
}

#[derive(Clone, Copy)]
struct AgentCardDestination<'a> {
    repository: &'a str,
    branch: &'a str,
}

fn agent_card_destination<'a>(
    destination: Option<AgentDestinationMetadata<'a>>,
    fallback_repository: &'a str,
    fallback_branch: Option<&'a str>,
) -> AgentCardDestination<'a> {
    destination.map_or(
        AgentCardDestination {
            repository: fallback_repository,
            branch: fallback_branch.unwrap_or("unknown"),
        },
        |destination| AgentCardDestination {
            repository: destination.repository(),
            branch: destination.branch(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    frame: &mut Frame<'_>,
    area: Rect,
    destination: AgentCardDestination<'_>,
    pane_id: Option<&str>,
    session: &str,
    agent_count: usize,
    change_stats: Option<(u64, u64)>,
    elapsed: Option<&str>,
    status: AgentStatus,
    spinner_frame: usize,
    state: AgentEntryState,
    in_host_tab: bool,
    hovered: bool,
    pane_id_hovered: bool,
) -> Option<Rect> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let background = row_background(&state, hovered);
    fill(frame, area, background);
    let status_area = draw_agent_status(frame, area, status, spinner_frame, background);
    let pane_id_area = draw_agent_card_header(
        frame,
        Rect::new(area.x, area.y, status_area.x.saturating_sub(area.x), 1),
        destination,
        pane_id,
        elapsed,
        background,
        in_host_tab,
        pane_id_hovered,
    );
    if area.height > 1 {
        draw_agent_card_detail(
            frame,
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
            session,
            agent_count,
            change_stats,
            background,
        );
    }
    pane_id_area
}

fn draw_agent_card_header(
    frame: &mut Frame<'_>,
    area: Rect,
    destination: AgentCardDestination<'_>,
    pane_id: Option<&str>,
    elapsed: Option<&str>,
    background: Color,
    highlighted: bool,
    pane_id_hovered: bool,
) -> Option<Rect> {
    if area.width < 2 || area.height == 0 {
        return None;
    }
    let time_width = elapsed
        .map(|time| u16::try_from(UnicodeWidthStr::width(time)).unwrap_or(u16::MAX))
        .unwrap_or(0)
        .min(area.width.saturating_sub(1));
    let elapsed_x = area.right().saturating_sub(time_width).saturating_sub(1);
    if let Some(elapsed) = elapsed {
        frame.render_widget(
            Paragraph::new(elapsed)
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().fg(palette().soft).bg(background)),
            Rect::new(elapsed_x, area.y, time_width, 1),
        );
    }
    let marker_right = elapsed_x.saturating_sub(u16::from(time_width > 0));
    let available = marker_right.saturating_sub(area.x).saturating_sub(1);
    let mut widths = [
        pane_id.map(|_| badge_width("id")).unwrap_or(0),
        badge_width(destination.repository).min(20),
        badge_width(destination.branch).min(20),
    ];
    let gap_width = if pane_id.is_some() { 2 } else { 1 };
    while widths
        .iter()
        .copied()
        .reduce(u16::saturating_add)
        .unwrap_or(0)
        .saturating_add(gap_width)
        > available
    {
        let Some((index, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > 3)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[index] -= 1;
    }
    let pane_id_area = pane_id.map(|_| Rect::new(area.x, area.y, widths[0].min(available), 1));
    if let Some(pane_id_area) = pane_id_area {
        draw_badge_with_background(
            frame,
            pane_id_area,
            "id",
            if pane_id_hovered {
                palette().canvas
            } else {
                palette().soft
            },
            background,
            if pane_id_hovered {
                palette().accent
            } else {
                palette().raised
            },
        );
    }
    let repository_x = if pane_id.is_some() {
        area.x.saturating_add(widths[0]).saturating_add(1)
    } else {
        area.x
    };
    draw_badge(
        frame,
        Rect::new(
            repository_x,
            area.y,
            widths[1].min(
                area.x
                    .saturating_add(available)
                    .saturating_sub(repository_x),
            ),
            1,
        ),
        destination.repository,
        if highlighted {
            palette().yellow
        } else {
            palette().cyan
        },
        background,
    );
    let branch_x = repository_x.saturating_add(widths[1]).saturating_add(1);
    draw_badge(
        frame,
        Rect::new(
            branch_x,
            area.y,
            widths[2].min(area.x.saturating_add(available).saturating_sub(branch_x)),
            1,
        ),
        destination.branch,
        palette().accent,
        background,
    );
    pane_id_area.filter(|area| area.width >= 3)
}

fn draw_agent_card_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &str,
    agent_count: usize,
    change_stats: Option<(u64, u64)>,
    background: Color,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let stats = change_stats
        .map(|(additions, deletions)| (format!("+{additions}"), format!("-{deletions}")));
    let stats_width = stats.as_ref().map_or(0, |(additions, deletions)| {
        u16::try_from(
            UnicodeWidthStr::width(additions.as_str())
                + 1
                + UnicodeWidthStr::width(deletions.as_str()),
        )
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(2))
    });
    let stats_area = Rect::new(
        area.right().saturating_sub(stats_width),
        area.y,
        stats_width,
        1,
    );
    let group_label = (agent_count > 1).then(|| format!("{agent_count} agents"));
    let group_width = group_label
        .as_ref()
        .map(|label| {
            u16::try_from(UnicodeWidthStr::width(label.as_str()))
                .unwrap_or(u16::MAX)
                .min(area.width.saturating_sub(2))
        })
        .unwrap_or(0);
    let group_right = stats_area.x.saturating_sub(u16::from(stats_width > 0));
    let group_x = group_right.saturating_sub(group_width).max(area.x);
    let group_width = group_right.saturating_sub(group_x);
    let session_x = area.x.saturating_add(1);
    let trailing_x = if group_width > 0 {
        group_x
    } else if stats_width > 0 {
        stats_area.x
    } else {
        area.right()
    };
    let session_gap = u16::from(group_width > 0 || stats_width > 0);
    let session_width = trailing_x
        .saturating_sub(session_x)
        .saturating_sub(session_gap);
    frame.render_widget(
        Paragraph::new(truncate_width(session, usize::from(session_width)))
            .style(Style::default().fg(palette().muted).bg(background)),
        Rect::new(session_x, area.y, session_width, 1),
    );
    if let Some(group_label) = group_label {
        frame.render_widget(
            Paragraph::new(truncate_width(&group_label, usize::from(group_width)))
                .style(Style::default().fg(palette().cyan).bg(background)),
            Rect::new(group_x, area.y, group_width, 1),
        );
    }
    if let Some((additions, deletions)) = stats {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(additions, Style::default().fg(palette().green)),
                Span::raw(" "),
                Span::styled(deletions, Style::default().fg(palette().red)),
            ]))
            .style(Style::default().bg(background)),
            stats_area,
        );
    }
}

fn draw_agent_status(
    frame: &mut Frame<'_>,
    row: Rect,
    status: AgentStatus,
    spinner_frame: usize,
    background: Color,
) -> Rect {
    let area = Rect::new(row.right().saturating_sub(1), row.y, row.width.min(1), 1);
    frame.render_widget(
        Paragraph::new(status_marker(status, spinner_frame)).style(
            Style::default()
                .fg(status_color(status))
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
    area
}

fn draw_agent_gap(frame: &mut Frame<'_>, gap: Rect, above: Color, below: Color) {
    if gap.width > 0 && gap.height > 0 {
        frame.render_widget(
            Paragraph::new("▀".repeat(usize::from(gap.width)))
                .style(Style::default().fg(above).bg(below)),
            gap,
        );
    }
}

fn complete_card_viewport(height: u16, item_step: u16) -> usize {
    usize::from(height.saturating_add(1) / item_step).max(1)
}

fn draw_trailing_agent_gap(frame: &mut Frame<'_>, gap: Rect, list: Rect, background: Color) {
    let below = if gap.y < list.bottom() {
        palette().panel
    } else if gap.y == list.bottom() {
        palette().canvas
    } else {
        return;
    };
    draw_agent_gap(frame, gap, background, below);
}

pub(super) fn badge_width(label: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(label).saturating_add(2)).unwrap_or(u16::MAX)
}

pub(super) fn draw_badge(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    foreground: Color,
    outer_background: Color,
) {
    draw_badge_with_background(
        frame,
        area,
        label,
        foreground,
        outer_background,
        palette().raised,
    );
}

fn draw_badge_with_background(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    foreground: Color,
    outer_background: Color,
    badge_background: Color,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("▐").style(Style::default().fg(badge_background).bg(outer_background)),
        Rect::new(area.x, area.y, 1, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            label,
            usize::from(area.width.saturating_sub(2)),
        ))
        .style(
            Style::default()
                .fg(foreground)
                .bg(badge_background)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            area.x.saturating_add(1),
            area.y,
            area.width.saturating_sub(2),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new("▌").style(Style::default().fg(badge_background).bg(outer_background)),
        Rect::new(area.right().saturating_sub(1), area.y, 1, 1),
    );
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        return format_tenths(seconds, 3_600, 'h');
    }
    if seconds < 604_800 {
        return format_tenths(seconds, 86_400, 'd');
    }
    format_tenths(seconds, 604_800, 'w')
}

fn format_preview_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{duration_ms}ms");
    }
    if duration_ms < 60_000 {
        let tenths = duration_ms.saturating_add(50) / 100;
        return if tenths.is_multiple_of(10) {
            format!("{}s", tenths / 10)
        } else {
            format!("{}.{:01}s", tenths / 10, tenths % 10)
        };
    }
    format_duration(Duration::from_millis(duration_ms))
}

fn format_tenths(seconds: u64, unit: u64, suffix: char) -> String {
    let tenths = seconds.saturating_mul(10).saturating_add(unit / 2) / unit;
    if tenths.is_multiple_of(10) {
        format!("{}{suffix}", tenths / 10)
    } else {
        format!("{}.{}{}", tenths / 10, tenths % 10, suffix)
    }
}

fn status_marker(status: AgentStatus, spinner_frame: usize) -> &'static str {
    match status {
        AgentStatus::Idle => "·",
        AgentStatus::Working => SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()],
        AgentStatus::Blocked => "■",
        AgentStatus::Done => "●",
        AgentStatus::Unknown => "?",
    }
}

fn status_color(status: AgentStatus) -> Color {
    match status {
        AgentStatus::Idle => palette().cyan,
        AgentStatus::Working => palette().orange,
        AgentStatus::Blocked => palette().red,
        AgentStatus::Done => palette().green,
        AgentStatus::Unknown => palette().faint,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn request(parts: Vec<AgentRequestPartPreview>) -> AgentRequestPreview {
        AgentRequestPreview {
            parts,
            reasoning_active: false,
            duration_ms: Some(1_000),
            reasoning_duration_ms: Some(500),
            tool_call_count: 0,
        }
    }

    fn request_message(parts: Vec<AgentRequestPartPreview>) -> AgentUserMessage {
        AgentUserMessage {
            text: "prompt".to_owned(),
            requests: vec![request(parts)],
        }
    }

    fn presentation_input<'a>(
        identity: &'a str,
        revision: u64,
        messages: &'a [AgentUserMessage],
        message: usize,
        width: usize,
        expanded_requests: &'a [usize],
        live: bool,
    ) -> AgentTranscriptPresentationInput<'a> {
        AgentTranscriptPresentationInput {
            transcript: AgentTranscript {
                identity,
                revision,
                messages,
            },
            message,
            width,
            expanded_requests,
            live,
            mode: AgentTranscriptMode::Agent,
        }
    }

    fn running_tool_message(text: &str) -> AgentUserMessage {
        request_message(vec![
            AgentRequestPartPreview::Text(text.to_owned()),
            AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                name: "read".to_owned(),
                title: Some("Inspecting transcript".to_owned()),
                running: true,
            }),
        ])
    }

    #[test]
    fn transcript_presentation_hit_does_no_static_build_work() {
        let messages = vec![running_tool_message("output")];
        let mut presentation = AgentTranscriptPresentation::default();

        presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            true,
        ));
        let built = presentation.build_stats();
        presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            true,
        ));

        assert_eq!(presentation.build_stats(), built);
        assert_eq!(built.presentations, 1);
        assert!(built.markdown_parses > 0);
        assert!(built.static_wraps > 0);
    }

    #[test]
    fn transcript_presentation_invalidates_all_static_inputs() {
        let messages = vec![
            running_tool_message("first"),
            running_tool_message("second"),
        ];
        let mut presentation = AgentTranscriptPresentation::default();
        let prepare = |presentation: &mut AgentTranscriptPresentation,
                       revision,
                       message,
                       width,
                       expanded: &[usize]| {
            presentation.prepare(presentation_input(
                "conversation",
                revision,
                &messages,
                message,
                width,
                expanded,
                true,
            ));
            presentation.build_stats().presentations
        };

        assert_eq!(prepare(&mut presentation, 1, 0, 40, &[]), 1);
        assert_eq!(prepare(&mut presentation, 1, 0, 41, &[]), 2);
        assert_eq!(prepare(&mut presentation, 1, 1, 41, &[]), 3);
        assert_eq!(prepare(&mut presentation, 1, 1, 41, &[0]), 4);
        assert_eq!(prepare(&mut presentation, 2, 1, 41, &[0]), 5);
        assert_eq!(presentation.cache_len(), 1);
    }

    #[test]
    fn collapsed_transcript_builds_only_a_bounded_markdown_window() {
        let hidden = (0..1_000)
            .map(|row| format!("**row {row}** hidden payload"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut message = running_tool_message(&hidden);
        message
            .requests
            .push(request(vec![AgentRequestPartPreview::Text(
                "latest output".to_owned(),
            )]));
        let messages = vec![message];
        let mut presentation = AgentTranscriptPresentation::default();

        let cached = presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            false,
        ));
        let rendered = cached.blocks[0]
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("row 0"));
        assert!(!rendered.contains("row 999"));
        assert!(cached.blocks[0].lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        }));
        assert!(presentation.build_stats().markdown_input_bytes < hidden.len() / 10);
    }

    #[test]
    fn collapsed_transcript_bounds_a_single_long_markdown_event() {
        let hidden = format!("**{}**", "word ".repeat(20_000));
        let mut message = running_tool_message(&hidden);
        message
            .requests
            .push(request(vec![AgentRequestPartPreview::Text(
                "latest output".to_owned(),
            )]));
        let messages = vec![message];
        let mut presentation = AgentTranscriptPresentation::default();

        let cached = presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            false,
        ));

        assert!(cached.blocks[0].expandable);
        assert!(presentation.build_stats().markdown_input_bytes < 2_000);
    }

    #[test]
    fn transcript_presentation_is_bounded_and_prunes_departed_conversations() {
        let messages = vec![running_tool_message("output")];
        let mut presentation = AgentTranscriptPresentation::default();
        let identities = (0..=MAX_AGENT_TRANSCRIPT_PRESENTATIONS)
            .map(|index| format!("conversation-{index}"))
            .collect::<Vec<_>>();
        for (index, identity) in identities.iter().enumerate() {
            presentation.prepare(presentation_input(
                identity,
                u64::try_from(index + 1).unwrap(),
                &messages,
                0,
                40,
                &[],
                false,
            ));
        }

        assert_eq!(presentation.cache_len(), MAX_AGENT_TRANSCRIPT_PRESENTATIONS);
        assert!(!presentation.contains_conversation(&identities[0]));
        let retained = identities.last().unwrap();
        presentation.retain_conversations(|identity| identity == retained);
        assert_eq!(presentation.cache_len(), 1);
        assert!(presentation.contains_conversation(retained));
    }

    #[test]
    fn transcript_presentation_hit_uses_current_spinner_and_reports_visibility() {
        let messages = vec![running_tool_message("output")];
        let mut presentation = AgentTranscriptPresentation::default();
        presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            true,
        ));
        let cached = presentation.prepare(presentation_input(
            "conversation",
            1,
            &messages,
            0,
            40,
            &[],
            true,
        ));
        let block = &cached.blocks[0];
        let area = Rect::new(0, 0, 40, 12);
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut animation_presented = false;

        terminal
            .draw(|frame| {
                animation_presented = draw_transcript_card(frame, block, area, area, 0, 7)
                    .unwrap()
                    .1;
            })
            .unwrap();

        assert!(animation_presented);
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == SPINNER_FRAMES[7])
        );
        assert_eq!(presentation.build_stats().presentations, 1);
    }

    #[test]
    fn formats_agent_durations_as_compact_units() {
        assert_eq!(format_duration(Duration::ZERO), "0s");
        assert_eq!(format_duration(Duration::from_secs(23)), "23s");
        assert_eq!(format_duration(Duration::from_secs(240)), "4m");
        assert_eq!(format_duration(Duration::from_secs(18_360)), "5.1h");
        assert_eq!(format_duration(Duration::from_secs(276_480)), "3.2d");
        assert_eq!(format_duration(Duration::from_secs(665_280)), "1.1w");
    }

    #[test]
    fn separates_cropped_and_completed_text_output() {
        let mut cropped = request_message(vec![
            AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning),
            AgentRequestPartPreview::Text("one\ntwo\nthree\nfour\nfive".to_owned()),
        ]);
        cropped
            .requests
            .push(request(vec![AgentRequestPartPreview::Text(
                "latest output".to_owned(),
            )]));
        let (blocks, _) = build_request_transcript(&cropped, 80, false, 0, &[]);
        let lines = &blocks[0].lines;
        assert_eq!(agent_output_transition_glyph(&lines[1]), Some('▄'));
        assert!(is_agent_output_row(&lines[6]));
        assert!(lines[6].spans.is_empty());
        assert!(
            lines[7]
                .spans
                .iter()
                .any(|span| span.content.contains("⌄ more"))
        );

        let completed = request_message(vec![
            AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning),
            AgentRequestPartPreview::Text("finished".to_owned()),
        ]);
        let (blocks, _) = build_request_transcript(&completed, 80, false, 0, &[]);
        assert!(blocks[0].lines.last().is_some_and(is_agent_output_row));
    }

    #[test]
    fn leaves_a_row_above_each_timed_request_card() {
        let request = |duration_ms| AgentRequestPreview {
            parts: vec![AgentRequestPartPreview::Text("output".to_owned())],
            reasoning_active: false,
            duration_ms,
            reasoning_duration_ms: None,
            tool_call_count: 0,
        };
        let message = AgentUserMessage {
            text: "prompt".to_owned(),
            requests: vec![request(Some(1_000)), request(None), request(Some(2_000))],
        };

        let (blocks, document_height) = build_request_transcript(&message, 80, false, 0, &[]);
        assert_eq!(blocks[0].start, 1);
        assert_eq!(blocks[1].start, blocks[0].start + blocks[0].height);
        assert_eq!(blocks[2].start, blocks[1].start + blocks[1].height + 1);
        assert_eq!(document_height, blocks[2].start + blocks[2].height);
    }

    #[test]
    fn renders_agent_code_blocks_as_raised_cards() {
        let width = 24;
        let lines =
            styled_agent_output_text("Before\n\n```rust\nfn preview() {}\n```\n\nAfter", width);
        let text = |line: &Line<'_>| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        let top = lines
            .iter()
            .position(|line| text(line) == "▄".repeat(width))
            .unwrap();
        let bottom = lines
            .iter()
            .enumerate()
            .skip(top + 1)
            .find_map(|(index, line)| (text(line) == "▀".repeat(width)).then_some(index))
            .unwrap();

        assert!(bottom > top + 1);
        assert!(lines[top + 1..bottom].iter().all(|line| {
            line.style.bg == Some(palette().raised)
                && line
                    .spans
                    .iter()
                    .all(|span| span.style.bg == Some(palette().raised))
        }));
        assert!(lines[top + 1..bottom].iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content == "fn" && span.style.fg == Some(palette().purple))
        }));
        assert_eq!(lines.first().unwrap().style.bg, Some(palette().panel));
        assert_eq!(lines.last().unwrap().style.bg, Some(palette().panel));
    }

    #[test]
    fn final_request_is_expanded_by_default_and_tracks_active_rows() {
        let tool = |name: &str, running| {
            AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                name: name.to_owned(),
                title: None,
                running,
            })
        };
        let message = request_message(vec![
            tool("one", false),
            tool("two", false),
            tool("three", false),
            tool("hidden", true),
        ]);

        let (expanded, _) = build_request_transcript(&message, 80, true, 0, &[]);
        assert!(!expanded[0].expandable);
        assert!(!expanded[0].lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("⌄ more"))
        }));
        assert_eq!(expanded[0].animated_rows.len(), 1);
        assert_eq!(
            expanded[0].lines[expanded[0].animated_rows[0]].spans[0].content,
            SPINNER_FRAMES[0]
        );
    }

    #[test]
    fn final_request_skips_collapsed_summary_parsing() {
        let output = "**complete** response";
        let messages = vec![request_message(vec![AgentRequestPartPreview::Text(
            output.to_owned(),
        )])];
        let mut stats = AgentTranscriptBuildStats::default();

        build_request_transcript_counted(&messages[0], 40, false, &[], &mut stats);

        assert_eq!(stats.markdown_parses, 1);
        assert_eq!(stats.markdown_input_bytes, output.len());
    }

    #[test]
    fn response_without_reasoning_omits_header_line_and_lowers_timer() {
        let message = request_message(vec![
            AgentRequestPartPreview::Text("response".to_owned()),
            AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                name: "skill".to_owned(),
                title: Some("Loaded skill".to_owned()),
                running: false,
            }),
        ]);
        let (blocks, _) = build_request_transcript(&message, 40, false, 0, &[]);
        let block = &blocks[0];
        let area = Rect::new(0, 0, 40, 6);
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();

        terminal
            .draw(|frame| {
                draw_transcript_card(frame, block, area, area, 0, 0);
            })
            .unwrap();

        let row = |y| {
            (0..40)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        };
        let top = u16::try_from(block.start).unwrap();
        assert_eq!(block.height, block.lines.len() + 1);
        assert!(!row(top).contains('▄'));
        assert!(row(top).contains("1s"));
        assert!(row(top + 1).contains("response"));
        assert!(
            block
                .lines
                .iter()
                .any(|line| { line.spans.iter().any(|span| span.content.contains("skill")) })
        );
    }

    #[test]
    fn reasoning_response_keeps_header_line_and_timer() {
        let message = request_message(vec![
            AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning),
            AgentRequestPartPreview::Text("response".to_owned()),
        ]);
        let (blocks, _) = build_request_transcript(&message, 40, false, 0, &[]);
        let block = &blocks[0];
        let area = Rect::new(0, 0, 40, 6);
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();

        terminal
            .draw(|frame| {
                draw_transcript_card(frame, block, area, area, 0, 0);
            })
            .unwrap();

        let top = u16::try_from(block.start).unwrap();
        let top = (0..40)
            .map(|x| terminal.backend().buffer()[(x, top)].symbol())
            .collect::<String>();
        assert!(top.contains('▄'));
        assert!(top.contains("1s"));
    }

    #[test]
    fn transcript_card_reports_only_visible_animated_rows() {
        let animated_row = 15;
        let block = TranscriptBlock {
            user: false,
            headerless_response: false,
            lines: (0..20)
                .map(|row| {
                    if row == animated_row {
                        Line::from(SPINNER_FRAMES[0])
                    } else {
                        Line::from(format!("line {row}"))
                    }
                })
                .collect::<Vec<_>>()
                .into(),
            animated_rows: vec![animated_row].into(),
            start: 0,
            height: 22,
            elapsed: None,
            request_count: 0,
            request: Some(0),
            expandable: false,
        };
        let area = Rect::new(0, 0, 40, 8);
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        let mut animation_presented = true;

        terminal
            .draw(|frame| {
                animation_presented = draw_transcript_card(frame, &block, area, area, 0, 0)
                    .unwrap()
                    .1;
            })
            .unwrap();
        assert!(!animation_presented);

        terminal
            .draw(|frame| {
                animation_presented = draw_transcript_card(frame, &block, area, area, 10, 0)
                    .unwrap()
                    .1;
            })
            .unwrap();
        assert!(animation_presented);
    }
}
