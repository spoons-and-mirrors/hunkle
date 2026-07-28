use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{AuthorFilter, CommitSummaryCache, GraphHitTarget, HitTarget, Mode},
    git::{Commit, RepositoryData},
};

use super::{draw_empty, fill, palette, truncate_width};

pub(super) struct GraphRegions {
    pub table: Option<Rect>,
    pub targets: Vec<(HitTarget, Rect)>,
}

pub(super) fn draw_graph(
    frame: &mut Frame<'_>,
    repo: Option<&RepositoryData>,
    summaries: &CommitSummaryCache,
    author_filter: &AuthorFilter,
    state: &mut TableState,
    scroll_to_selection: &mut bool,
    area: Rect,
) -> GraphRegions {
    let Some(repo) = repo else {
        draw_empty(frame, area, "Open a repository to inspect its graph");
        return GraphRegions {
            table: None,
            targets: Vec::new(),
        };
    };
    if repo.commits.is_empty() {
        draw_empty(frame, area, "This repository has no commits yet");
        return GraphRegions {
            table: None,
            targets: Vec::new(),
        };
    }
    fill(frame, area, palette().panel);
    let graph_header = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        1,
    );
    let mut graph_title = vec![
        Span::styled(
            "ALL BRANCHES",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  date order", Style::default().fg(palette().faint)),
    ];
    if repo.graph_truncated {
        graph_title.push(Span::styled(
            format!("  first {} commits", repo.commits.len()),
            Style::default().fg(palette().yellow),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(graph_title)), graph_header);
    let table_area = Rect::new(
        graph_header.x,
        graph_header.y.saturating_add(2),
        graph_header.width,
        area.bottom()
            .saturating_sub(graph_header.y.saturating_add(3)),
    );
    let graph_region = Rect::new(
        table_area.x,
        table_area.y.saturating_add(2),
        table_area.width,
        table_area.height.saturating_sub(2),
    );

    let maximum_graph_width = table_area.width.saturating_sub(42).clamp(8, 40);
    let graph_width = repo.graph_width.clamp(8, maximum_graph_width as usize) as u16;
    let compact = table_area.width < 110;
    let widths = if compact {
        vec![
            Constraint::Length(graph_width),
            Constraint::Min(8),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Length(7),
        ]
    } else {
        vec![
            Constraint::Length(graph_width),
            Constraint::Min(24),
            Constraint::Length(11),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(7),
        ]
    };

    let visible = author_filter.visible_indices();
    let viewport = usize::from(graph_region.height);
    let selected = state.selected();
    let mut offset = state.offset().min(visible.len().saturating_sub(1));
    if *scroll_to_selection && let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        } else if selected >= offset.saturating_add(viewport) {
            offset = selected.saturating_add(1).saturating_sub(viewport);
        }
    }
    *scroll_to_selection = false;
    *state.offset_mut() = offset;
    let rows = visible.iter().skip(offset).take(viewport).map(|index| {
        let commit = &repo.commits[*index];
        graph_row(commit, summaries.get(&commit.oid), compact)
    });
    let author_label = if author_filter.active_count() == author_filter.entries().len() {
        "AUTHOR ▾".to_owned()
    } else {
        format!(
            "AUTHOR {}/{}",
            author_filter.active_count(),
            author_filter.entries().len()
        )
    };
    let headers = if compact {
        Row::new([
            "GRAPH".to_owned(),
            "DESCRIPTION".to_owned(),
            "CHANGES".to_owned(),
            author_label,
            "COMMIT".to_owned(),
        ])
    } else {
        Row::new([
            "GRAPH".to_owned(),
            "DESCRIPTION".to_owned(),
            "CHANGES".to_owned(),
            "DATE".to_owned(),
            author_label,
            "COMMIT".to_owned(),
        ])
    }
    .style(
        Style::default()
            .fg(palette().muted)
            .bg(palette().surface_alt)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let table = Table::new(rows, widths)
        .header(headers)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(palette().selected));
    let mut visible_state = TableState::default();
    visible_state.select(selected.and_then(|selected| selected.checked_sub(offset)));
    frame.render_stateful_widget(table, table_area, &mut visible_state);
    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new("No commits match the author filter")
                .style(Style::default().fg(palette().faint)),
            graph_region,
        );
    }
    let author_width = if compact { 12 } else { 16 };
    let author_x = table_area.right().saturating_sub(7 + 1 + author_width);
    let author_header = Rect::new(author_x, table_area.y, author_width, 1);
    GraphRegions {
        table: Some(graph_region),
        targets: vec![(
            HitTarget::Graph(GraphHitTarget::AuthorHeader),
            author_header,
        )],
    }
}

pub(super) fn draw_author_filter(
    frame: &mut Frame<'_>,
    anchor: Rect,
    filter: &mut AuthorFilter,
) -> Vec<(HitTarget, Rect)> {
    let width = filter
        .entries()
        .iter()
        .map(|entry| UnicodeWidthStr::width(entry.name.as_str()) + 12)
        .max()
        .unwrap_or(28)
        .clamp(28, 48) as u16;
    let list_height = filter.entries().len().clamp(1, 10) as u16;
    let height = list_height.saturating_add(1);
    let minimum_x = frame.area().x.saturating_add(1);
    let maximum_x = frame
        .area()
        .right()
        .saturating_sub(width.saturating_add(1))
        .max(minimum_x);
    let x = anchor.x.clamp(minimum_x, maximum_x);
    let below = anchor.y.saturating_add(1);
    let y = if below.saturating_add(height) <= frame.area().bottom() {
        below
    } else {
        anchor.y.saturating_sub(height)
    };
    let area = Rect::new(x, y, width, height);
    let list = Rect::new(area.x, area.y, area.width, list_height);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().raised);

    let selected = filter.state.selected();
    let items: Vec<ListItem<'static>> = filter
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let count = format!("{} commits", entry.commits);
            let name = truncate_width(
                &entry.name,
                usize::from(list.width).saturating_sub(count.len() + 7),
            );
            let padding = usize::from(list.width)
                .saturating_sub(UnicodeWidthStr::width(name.as_str()) + count.len() + 5);
            let foreground = if selected == Some(index) {
                palette().ink
            } else {
                palette().muted
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    if entry.enabled { " ▣ " } else { " ▢ " },
                    Style::default().fg(if entry.enabled {
                        palette().accent
                    } else {
                        palette().faint
                    }),
                ),
                Span::styled(name, Style::default().fg(foreground)),
                Span::raw(" ".repeat(padding)),
                Span::styled(count, Style::default().fg(foreground)),
                Span::raw(" "),
            ]))
        })
        .collect();
    let authors = List::new(items).highlight_style(Style::default().bg(palette().selected));
    frame.render_stateful_widget(authors, list, &mut filter.state);
    frame.render_widget(
        Paragraph::new("Space toggle   a all   n none   Esc close")
            .style(Style::default().fg(palette().faint)),
        Rect::new(
            area.x.saturating_add(1),
            area.bottom().saturating_sub(1),
            area.width - 1,
            1,
        ),
    );

    let mut targets = vec![(HitTarget::Graph(GraphHitTarget::FilterOverlay), area)];
    let offset = filter.state.offset();
    for row in 0..usize::from(list.height) {
        let index = offset + row;
        if index >= filter.entries().len() {
            break;
        }
        targets.push((
            HitTarget::Graph(GraphHitTarget::FilterItem(index)),
            Rect::new(list.x, list.y + row as u16, list.width, 1),
        ));
    }
    targets
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_branch(
    frame: &mut Frame<'_>,
    commits: &[Commit],
    branch: &str,
    header: Rect,
    list: Rect,
    dragging: bool,
    focused: bool,
    mode: Mode,
    state: &mut ListState,
) {
    fill(
        frame,
        header,
        if dragging {
            palette().selected
        } else {
            palette().surface_alt
        },
    );
    let history_title = if header.width >= 20 {
        format!("HISTORY  {branch}")
    } else {
        "HISTORY".to_owned()
    };
    let history_meta = format!("↕  {}", commits.len());
    let history_padding = usize::from(header.width).saturating_sub(
        UnicodeWidthStr::width(history_title.as_str())
            + UnicodeWidthStr::width(history_meta.as_str()),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                truncate_width(
                    &history_title,
                    usize::from(header.width)
                        .saturating_sub(UnicodeWidthStr::width(history_meta.as_str()) + 1),
                ),
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(history_padding)),
            Span::styled(history_meta, Style::default().fg(palette().faint)),
        ])),
        header,
    );
    if commits.is_empty() {
        let items = vec![ListItem::new(Line::styled(
            "  No commits on this branch",
            Style::default().fg(palette().faint),
        ))];
        frame.render_stateful_widget(List::new(items), list, state);
        return;
    }

    let selected = state.selected();
    let mut offset = state.offset().min(commits.len().saturating_sub(1));
    if let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        }
        while offset < selected
            && commits[offset..=selected]
                .iter()
                .map(history_item_height)
                .sum::<usize>()
                > usize::from(list.height)
        {
            offset += 1;
        }
    }
    *state.offset_mut() = offset;
    let mut height = 0usize;
    let items: Vec<ListItem<'_>> = commits
        .iter()
        .skip(offset)
        .take_while(|commit| {
            let item_height = history_item_height(commit);
            let include =
                height == 0 || height.saturating_add(item_height) <= usize::from(list.height);
            if include {
                height = height.saturating_add(item_height);
            }
            include
        })
        .map(|commit| history_item(commit, usize::from(list.width)))
        .collect();
    let history =
        List::new(items).highlight_style(Style::default().bg(if focused && mode == Mode::Normal {
            palette().selected
        } else {
            palette().inactive_selected
        }));
    let mut visible_state = ListState::default();
    visible_state.select(selected.and_then(|selected| selected.checked_sub(offset)));
    frame.render_stateful_widget(history, list, &mut visible_state);
}

fn history_item_height(commit: &Commit) -> usize {
    1 + usize::from(!commit.refs.is_empty())
}

fn graph_row(
    commit: &Commit,
    summary: Option<&crate::git::DiffSummary>,
    compact: bool,
) -> Row<'static> {
    let graph = Line::from(
        commit
            .graph
            .iter()
            .map(|cell| {
                Span::styled(
                    cell.symbol.to_string(),
                    Style::default()
                        .fg(palette().graph_colors[cell.color % palette().graph_colors.len()]),
                )
            })
            .collect::<Vec<_>>(),
    );

    let mut description = Vec::new();
    if commit
        .refs
        .iter()
        .any(|reference| reference == "HEAD" || reference.starts_with("HEAD -> "))
    {
        description.push(ref_badge("HEAD", palette().green));
        description.push(Span::raw(" "));
    }
    for reference in &commit.refs {
        let (label, color) = if let Some(tag) = reference.strip_prefix("tag: ") {
            (tag, palette().yellow)
        } else if let Some(branch) = reference.strip_prefix("HEAD -> ") {
            (branch, palette().accent)
        } else if reference == "HEAD" {
            continue;
        } else {
            (reference.as_str(), palette().accent)
        };
        description.push(ref_badge(label, color));
        description.push(Span::raw(" "));
    }
    description.push(Span::styled(
        commit.subject.clone(),
        Style::default().fg(palette().ink),
    ));

    let short_oid: String = commit.oid.chars().take(7).collect();
    let changes = commit_changes(summary);
    if compact {
        Row::new([
            Cell::from(graph),
            Cell::from(Line::from(description)),
            changes,
            Cell::from(commit.author.clone()).style(Style::default().fg(palette().muted)),
            Cell::from(short_oid).style(Style::default().fg(palette().muted)),
        ])
    } else {
        Row::new([
            Cell::from(graph),
            Cell::from(Line::from(description)),
            changes,
            Cell::from(commit.date.clone()).style(Style::default().fg(palette().muted)),
            Cell::from(commit.author.clone()).style(Style::default().fg(palette().muted)),
            Cell::from(short_oid).style(Style::default().fg(palette().muted)),
        ])
    }
}

fn commit_changes(summary: Option<&crate::git::DiffSummary>) -> Cell<'static> {
    let Some(summary) = summary else {
        return Cell::from("…").style(Style::default().fg(palette().faint));
    };
    Cell::from(Line::from(vec![
        Span::styled(
            format!("+{}", summary.additions),
            Style::default().fg(palette().green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", summary.deletions),
            Style::default().fg(palette().red),
        ),
    ]))
}

fn history_item(commit: &Commit, width: usize) -> ListItem<'static> {
    let short_oid: String = commit.oid.chars().take(7).collect();
    let subject_width = width.saturating_sub(8);
    let subject = truncate_width(&commit.subject, subject_width);
    let subject_padding = width.saturating_sub(
        UnicodeWidthStr::width(subject.as_str()) + UnicodeWidthStr::width(short_oid.as_str()),
    );
    let mut details = Vec::new();
    let has_head = commit
        .refs
        .iter()
        .any(|reference| reference == "HEAD" || reference.starts_with("HEAD -> "));
    if has_head {
        details.push(ref_badge("HEAD", palette().green));
    }
    for reference in &commit.refs {
        if reference == "HEAD" || reference.starts_with("HEAD -> ") {
            continue;
        }
        let (label, color) = if let Some(tag) = reference.strip_prefix("tag: ") {
            (tag, palette().yellow)
        } else if reference.contains('/') {
            (reference.as_str(), palette().purple)
        } else {
            (reference.as_str(), palette().accent)
        };
        details.push(ref_badge(label, color));
    }

    let mut lines = vec![Line::from(vec![
        Span::styled(subject, Style::default().fg(palette().ink)),
        Span::raw(" ".repeat(subject_padding)),
        Span::styled(short_oid, Style::default().fg(palette().faint)),
    ])];
    if !details.is_empty() {
        lines.push(Line::from(details));
    }
    ListItem::new(Text::from(lines))
}

fn ref_badge(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(color)
            .bg(palette().raised)
            .add_modifier(Modifier::BOLD),
    )
}
