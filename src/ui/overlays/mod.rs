pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::app::{
    ACTION_ITEMS, ActionsState, AgentPaneDirection, App, CommandLineSource, CommandStatus,
    Explorer, ExplorerHitTarget, FileDialog, FileDialogKind, FileNameAction, FileSearch,
    FileSearchHitTarget, FileSearchRow, HerdrPrompt, HitTarget, LayoutProfile, PickerAction,
    PickerEntry, ScheduledRunStatus, SchedulerDestinationCard, SchedulerField, SchedulerHitTarget,
    SchedulerSurface, ScrollTarget, SearchScope, Settings, SettingsHitTarget, SettingsPage,
    ShortcutAction, Shortcuts, SurroundingEntry,
};

pub(super) use super::{
    fill, location_picker_capacity, location_picker_row, palette, text::word_wrapped_height,
    text_input_lines, truncate_start_width, truncate_width,
};

mod actions;
pub(super) use actions::*;
mod agent_preview;
pub(super) use agent_preview::*;
mod editor;
pub(super) use editor::*;
mod explorer;
pub(super) use explorer::*;
mod help;
pub(super) use help::*;
mod herdr;
pub(super) use herdr::*;
mod settings;
pub(super) use settings::*;
mod scheduler;
pub(super) use scheduler::draw_scheduler;

pub(super) struct FileSearchRegions {
    pub(super) overlay: Rect,
    pub(super) list: Rect,
    pub(super) targets: Vec<(HitTarget, Rect)>,
}

pub(super) struct ActionMenuRegions {
    pub(super) overlay: Rect,
    pub(super) list: Rect,
}

pub(super) struct CommandRegions {
    pub(super) overlay: Rect,
    pub(super) output: Rect,
}

pub(super) struct FileDialogRegions {
    pub(super) overlay: Rect,
    pub(super) primary: Rect,
    pub(super) secondary: Rect,
}

const MAX_COMMAND_LAYOUT_LINES: usize = 50_000;

fn ensure_command_layout(actions: &mut ActionsState, width: usize) {
    let width = width.max(1);
    if actions.command_layout.revision == actions.presentation_revision
        && actions.command_layout.width == width
    {
        return;
    }
    let sources = command_line_sources(actions);
    let mut starts = Vec::with_capacity(sources.len());
    let mut height = 0usize;
    for source in &sources {
        starts.push(height);
        height = height.saturating_add(word_wrapped_height(
            command_line_content(actions, source).as_ref(),
            width,
        ));
    }
    actions.command_layout = crate::app::CommandLayout {
        revision: actions.presentation_revision,
        width,
        sources,
        starts,
        height,
    };
}

fn command_line_sources(actions: &ActionsState) -> Vec<CommandLineSource> {
    let mut sources = Vec::new();
    if actions.status == CommandStatus::Input && actions.transcript.is_empty() {
        if actions.stderr.is_empty() {
            sources.extend([
                CommandLineSource::Intro,
                CommandLineSource::IntroSpacer,
                CommandLineSource::IntroExamples,
                CommandLineSource::IntroShellNote,
            ]);
        } else {
            push_command_ranges(&mut sources, &actions.stderr, |range| {
                CommandLineSource::CurrentError(range)
            });
        }
        return sources;
    }
    'records: for (record_index, record) in actions.transcript.iter().enumerate() {
        if record_index > 0 {
            sources.push(CommandLineSource::RecordSpacer);
        }
        sources.push(CommandLineSource::RecordHeader(record_index));
        if !record.stdout.is_empty()
            && !push_command_ranges(&mut sources, &record.stdout, |range| {
                CommandLineSource::Stdout {
                    record: record_index,
                    range,
                }
            })
        {
            break 'records;
        }
        if !record.stderr.is_empty()
            && !push_command_ranges(&mut sources, &record.stderr, |range| {
                CommandLineSource::Stderr {
                    record: record_index,
                    range,
                }
            })
        {
            break 'records;
        }
        if record.stdout.is_empty() && record.stderr.is_empty() {
            sources.push(CommandLineSource::EmptyRecord);
        }
    }
    if actions.status == CommandStatus::Input && !actions.stderr.is_empty() {
        if !sources.is_empty() {
            sources.push(CommandLineSource::RecordSpacer);
        }
        push_command_ranges(&mut sources, &actions.stderr, |range| {
            CommandLineSource::CurrentError(range)
        });
    }
    if actions.status == CommandStatus::Running {
        if !sources.is_empty() {
            sources.push(CommandLineSource::RecordSpacer);
        }
        sources.push(CommandLineSource::Waiting);
    }
    if sources.is_empty() {
        sources.push(CommandLineSource::Empty);
    }
    sources
}

fn push_command_ranges(
    sources: &mut Vec<CommandLineSource>,
    content: &str,
    mut source: impl FnMut(std::ops::Range<usize>) -> CommandLineSource,
) -> bool {
    for line in content.lines() {
        if sources.len() >= MAX_COMMAND_LAYOUT_LINES {
            sources.push(CommandLineSource::Truncated);
            return false;
        }
        let start = line.as_ptr() as usize - content.as_ptr() as usize;
        sources.push(source(start..start + line.len()));
    }
    true
}

fn command_line_content<'a>(
    actions: &'a ActionsState,
    source: &CommandLineSource,
) -> std::borrow::Cow<'a, str> {
    match source {
        CommandLineSource::Intro => {
            "Run any non-interactive Git command from this repository.".into()
        }
        CommandLineSource::IntroSpacer | CommandLineSource::RecordSpacer => "".into(),
        CommandLineSource::IntroExamples => {
            "Examples: status --short · log --oneline -10 · remote -v".into()
        }
        CommandLineSource::IntroShellNote => {
            "Shell pipes and redirects are not interpreted.".into()
        }
        CommandLineSource::RecordHeader(index) => {
            let record = &actions.transcript[*index];
            let status = if record.success {
                format!("exit {}", record.exit_code.unwrap_or(0))
            } else {
                record
                    .exit_code
                    .map_or_else(|| "failed".to_owned(), |code| format!("exit {code}"))
            };
            format!("{}  {status}", record.command).into()
        }
        CommandLineSource::Stdout { record, range } => {
            actions.transcript[*record].stdout[range.clone()].into()
        }
        CommandLineSource::Stderr { record, range } => {
            actions.transcript[*record].stderr[range.clone()].into()
        }
        CommandLineSource::EmptyRecord => "Completed without output.".into(),
        CommandLineSource::CurrentError(range) => actions.stderr[range.clone()].into(),
        CommandLineSource::Waiting => "Waiting for Git...".into(),
        CommandLineSource::Truncated => "Output truncated after 50,000 lines.".into(),
        CommandLineSource::Empty => "Command completed without output.".into(),
    }
}

fn command_line<'a>(actions: &'a ActionsState, source: &CommandLineSource) -> Line<'a> {
    let style = match source {
        CommandLineSource::RecordHeader(_) => Style::default()
            .fg(palette().accent)
            .add_modifier(Modifier::BOLD),
        CommandLineSource::Stderr { .. } | CommandLineSource::CurrentError(_) => {
            Style::default().fg(palette().red)
        }
        CommandLineSource::Waiting => Style::default().fg(palette().yellow),
        CommandLineSource::IntroExamples
        | CommandLineSource::IntroShellNote
        | CommandLineSource::EmptyRecord
        | CommandLineSource::Truncated
        | CommandLineSource::Empty => Style::default().fg(palette().faint),
        _ => Style::default().fg(palette().ink),
    };
    Line::styled(command_line_content(actions, source), style)
}

fn centered_min(
    area: Rect,
    width_percent: u16,
    height_percent: u16,
    minimum_width: u16,
    minimum_height: u16,
) -> Rect {
    let width = area
        .width
        .saturating_mul(width_percent)
        .checked_div(100)
        .unwrap_or(0)
        .max(minimum_width)
        .min(area.width.saturating_sub(4));
    let height = area
        .height
        .saturating_mul(height_percent)
        .checked_div(100)
        .unwrap_or(0)
        .max(minimum_height)
        .min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::git::CommandOutput;

    #[test]
    fn command_layout_caps_lines_and_reuses_unchanged_layout() {
        let mut actions = ActionsState::default();
        actions.begin_command("git status".to_owned());
        actions.complete(CommandOutput {
            stdout: "row\n".repeat(MAX_COMMAND_LAYOUT_LINES + 100),
            stderr: String::new(),
            success: true,
            exit_code: Some(0),
        });

        ensure_command_layout(&mut actions, 80);
        assert!(matches!(
            actions.command_layout.sources.last(),
            Some(CommandLineSource::Truncated)
        ));
        assert!(actions.command_layout.sources.len() <= MAX_COMMAND_LAYOUT_LINES + 1);
        let starts = actions.command_layout.starts.as_ptr();
        ensure_command_layout(&mut actions, 80);
        assert_eq!(actions.command_layout.starts.as_ptr(), starts);
    }
}
