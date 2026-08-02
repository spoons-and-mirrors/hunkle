pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
};

pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::{git::Branch, repo_path::RepoPath};

pub(super) use crate::app::{
    ACTION_ITEMS, ActionsState, AgentPaneDirection, BranchDeleteDialog, BrowserTab, CommandRecord,
    CommandStatus, Explorer, ExplorerHitTarget, ExplorerTab, FileDialog, FileDialogKind,
    FileNameAction, FileSearch, HerdrPrompt, HitTarget, Issue, PickerAction, PickerEntry,
    PullRequest, RemoteItems, RepositoryBrowser, RepositoryBrowserHitTarget, Settings,
    SettingsPage, ShortcutAction, Shortcuts, SurroundingEntry, WorktreeCreateDialog,
    WorktreeCreateField, WorktreeManager, WorktreeManagerHitTarget, WorktreeManagerRow,
    WorktreeRemoveDialog, short_head, worktree_label,
};

pub(super) use super::{
    fill, palette, text::word_wrapped_height, truncate_start_width, truncate_width,
};

mod actions;
pub(super) use actions::*;
mod editor;
pub(super) use editor::*;
mod explorer;
pub(super) use explorer::*;
mod help;
pub(super) use help::*;
mod herdr;
pub(super) use herdr::*;
mod repository_browser;
pub(super) use repository_browser::*;
mod settings;
pub(super) use settings::*;
mod worktree_manager;
pub(super) use worktree_manager::*;

pub(super) struct FileSearchRegions {
    pub(super) overlay: Rect,
    pub(super) list: Rect,
}

pub(super) struct SettingsRegions {
    pub(super) overlay: Rect,
    pub(super) general_tab: Rect,
    pub(super) shortcuts_tab: Rect,
    pub(super) opencode_tab: Rect,
    pub(super) auto_fetch: Option<Rect>,
    pub(super) fetch_interval: Option<Rect>,
    pub(super) fetch_interval_down: Option<Rect>,
    pub(super) fetch_interval_up: Option<Rect>,
    pub(super) format_on_save: Option<Rect>,
    pub(super) opencode_model: Option<Rect>,
    pub(super) opencode_reasoning: Option<Rect>,
    pub(super) cross_workspace_agents: Option<Rect>,
    pub(super) agent_harness: Option<Rect>,
    pub(super) agent_time: Option<Rect>,
    pub(super) clear_agent_timings: Option<Rect>,
    pub(super) media_preview: Option<Rect>,
    pub(super) editor: Option<Rect>,
    pub(super) shortcut_rows: Vec<(ShortcutAction, Rect)>,
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

fn status_row(message: &str, color: Color) -> ListItem<'_> {
    ListItem::new(Line::styled(message, Style::default().fg(color)))
}

fn command_lines<'a>(
    status: CommandStatus,
    transcript: &'a [CommandRecord],
    stderr: &'a str,
) -> Vec<Line<'a>> {
    if status == CommandStatus::Input && transcript.is_empty() {
        return if stderr.is_empty() {
            vec![
                Line::styled(
                    "Run any non-interactive Git command from this repository.",
                    Style::default().fg(palette().ink),
                ),
                Line::raw(""),
                Line::styled(
                    "Examples: status --short · log --oneline -10 · remote -v",
                    Style::default().fg(palette().faint),
                ),
                Line::styled(
                    "Shell pipes and redirects are not interpreted.",
                    Style::default().fg(palette().faint),
                ),
            ]
        } else {
            vec![Line::styled(stderr, Style::default().fg(palette().red))]
        };
    }
    let mut lines = Vec::new();
    for (index, record) in transcript.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        let status = if record.success {
            format!("exit {}", record.exit_code.unwrap_or(0))
        } else {
            record
                .exit_code
                .map_or_else(|| "failed".to_owned(), |code| format!("exit {code}"))
        };
        lines.push(Line::from(vec![
            Span::styled(
                record.command.as_str(),
                Style::default()
                    .fg(palette().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {status}"), Style::default().fg(palette().muted)),
        ]));
        if !record.stdout.is_empty() {
            lines.extend(
                record
                    .stdout
                    .lines()
                    .map(|line| Line::styled(line, Style::default().fg(palette().ink))),
            );
        }
        if !record.stderr.is_empty() {
            lines.extend(
                record
                    .stderr
                    .lines()
                    .map(|line| Line::styled(line, Style::default().fg(palette().red))),
            );
        }
        if record.stdout.is_empty() && record.stderr.is_empty() {
            lines.push(Line::styled(
                "Completed without output.",
                Style::default().fg(palette().faint),
            ));
        }
    }
    if status == CommandStatus::Input && !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(stderr, Style::default().fg(palette().red)));
    }
    if status == CommandStatus::Running {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            "Waiting for Git...",
            Style::default().fg(palette().yellow),
        ));
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "Command completed without output.",
            Style::default().fg(palette().faint),
        ));
    }
    lines
}

fn rendered_height(lines: &[Line<'_>], width: usize) -> usize {
    let width = width.max(1);
    lines
        .iter()
        .map(|line| {
            let content = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            word_wrapped_height(&content, width)
        })
        .sum()
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
