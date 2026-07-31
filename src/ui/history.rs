use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Clear, List, ListItem, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{
        AuthorFilter, CommitSummaryCache, GraphColumn, GraphColumnRegion, GraphHitTarget,
        HitTarget, Settings, ShortcutAction, Shortcuts,
    },
    git::{Commit, RepositoryData},
};

use super::{draw_empty, fill, palette, truncate_width};

pub(super) struct GraphRegions {
    pub table: Option<Rect>,
    pub targets: Vec<(HitTarget, Rect)>,
    pub columns: Vec<GraphColumnRegion>,
}

pub(super) struct GraphView<'a> {
    pub repo: Option<&'a RepositoryData>,
    pub summaries: &'a CommitSummaryCache,
    pub author_filter: &'a AuthorFilter,
    pub state: &'a mut TableState,
    pub scroll_to_selection: &'a mut bool,
    pub settings: &'a Settings,
    pub dragging_column: Option<GraphColumn>,
}

pub(super) fn draw_graph(frame: &mut Frame<'_>, area: Rect, view: GraphView<'_>) -> GraphRegions {
    let GraphView {
        repo,
        summaries,
        author_filter,
        state,
        scroll_to_selection,
        settings,
        dragging_column,
    } = view;
    let Some(repo) = repo else {
        draw_empty(frame, area, "Open a repository to inspect its graph");
        return GraphRegions {
            table: None,
            targets: Vec::new(),
            columns: Vec::new(),
        };
    };
    if !repo.is_local() && !repo.details_ready {
        draw_empty(frame, area, "Loading commit graph…");
        return GraphRegions {
            table: None,
            targets: Vec::new(),
            columns: Vec::new(),
        };
    }
    if repo.commits.is_empty() {
        draw_empty(frame, area, "This repository has no commits yet");
        return GraphRegions {
            table: None,
            targets: Vec::new(),
            columns: Vec::new(),
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
            format!("  graph limited ({} commits)", repo.commits.len()),
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

    let column_widths = graph_column_widths(table_area.width, repo.graph_width, settings);
    let widths = column_widths.map(Constraint::Length);

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
        graph_row(commit, summaries.get(&commit.oid))
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
    let headers = Row::new([
        "GRAPH".to_owned(),
        "DESCRIPTION".to_owned(),
        "CHANGES".to_owned(),
        "DATE".to_owned(),
        author_label,
        " COMMIT".to_owned(),
    ])
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
    let column_starts = graph_column_starts(table_area.x, column_widths);
    let columns = [
        (GraphColumn::Changes, 2),
        (GraphColumn::Date, 3),
        (GraphColumn::Author, 4),
        (GraphColumn::Commit, 5),
    ]
    .into_iter()
    .map(|(column, index)| {
        let splitter_x = if column == GraphColumn::Commit {
            column_starts[index]
        } else {
            column_starts[index].saturating_add(column_widths[index])
        };
        let splitter = Rect::new(splitter_x, table_area.y, 1, 1);
        frame.render_widget(
            Paragraph::new(if column == GraphColumn::Commit {
                "↔"
            } else {
                "│"
            })
            .style(Style::default().fg(if dragging_column == Some(column) {
                palette().accent
            } else {
                palette().faint
            })),
            splitter,
        );
        GraphColumnRegion {
            column,
            start_x: column_starts[index],
            end_x: column_starts[index].saturating_add(column_widths[index]),
            splitter,
        }
    })
    .collect::<Vec<_>>();
    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new("No commits match the author filter")
                .style(Style::default().fg(palette().faint)),
            graph_region,
        );
    }
    let author_width = column_widths[4];
    let author_x = column_starts[4];
    let author_header = Rect::new(author_x, table_area.y, author_width, 1);
    GraphRegions {
        table: Some(graph_region),
        targets: vec![(
            HitTarget::Graph(GraphHitTarget::AuthorHeader),
            author_header,
        )],
        columns,
    }
}

fn graph_column_widths(width: u16, graph_width: usize, settings: &Settings) -> [u16; 6] {
    const COLUMN_SPACING: u16 = 5;
    const PREFERRED_MINIMUMS: [u16; 6] = [5, 1, 7, 9, 12, 7];
    const ABSOLUTE_MINIMUMS: [u16; 6] = [2, 1, 3, 4, 3, 3];
    let available = width.saturating_sub(COLUMN_SPACING).max(1);
    let mut widths = [
        graph_width.clamp(8, 40) as u16,
        1,
        settings
            .graph_column_width(GraphColumn::Changes)
            .clamp(3, 80),
        settings.graph_column_width(GraphColumn::Date).clamp(4, 80),
        settings
            .graph_column_width(GraphColumn::Author)
            .clamp(3, 80),
        settings
            .graph_column_width(GraphColumn::Commit)
            .clamp(3, 80),
    ];
    let fixed = widths[0] + widths[2] + widths[3] + widths[4] + widths[5];
    let fixed_minimum = PREFERRED_MINIMUMS[0]
        + PREFERRED_MINIMUMS[2]
        + PREFERRED_MINIMUMS[3]
        + PREFERRED_MINIMUMS[4]
        + PREFERRED_MINIMUMS[5];
    let description_minimum = available
        .saturating_sub(fixed_minimum)
        .clamp(PREFERRED_MINIMUMS[1], 8);
    let mut overflow = fixed
        .saturating_add(description_minimum)
        .saturating_sub(available);
    for index in [4, 2, 5, 0, 3] {
        let reduction = widths[index]
            .saturating_sub(PREFERRED_MINIMUMS[index])
            .min(overflow);
        widths[index] = widths[index].saturating_sub(reduction);
        overflow = overflow.saturating_sub(reduction);
    }
    for index in [4, 2, 5, 0, 3] {
        let reduction = widths[index]
            .saturating_sub(ABSOLUTE_MINIMUMS[index])
            .min(overflow);
        widths[index] = widths[index].saturating_sub(reduction);
        overflow = overflow.saturating_sub(reduction);
    }
    widths[1] = available
        .saturating_sub(widths[0] + widths[2] + widths[3] + widths[4] + widths[5])
        .max(ABSOLUTE_MINIMUMS[1]);
    widths
}

fn graph_column_starts(x: u16, widths: [u16; 6]) -> [u16; 6] {
    let mut starts = [x; 6];
    for index in 1..starts.len() {
        starts[index] = starts[index - 1]
            .saturating_add(widths[index - 1])
            .saturating_add(1);
    }
    starts
}

pub(super) fn draw_author_filter(
    frame: &mut Frame<'_>,
    anchor: Rect,
    filter: &mut AuthorFilter,
    shortcuts: &Shortcuts,
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
        Paragraph::new(format!(
            "Space toggle   {} all   {} none   Esc close",
            shortcuts.label(ShortcutAction::AuthorEnableAll),
            shortcuts.label(ShortcutAction::AuthorDisableAll)
        ))
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

fn graph_row(commit: &Commit, summary: Option<&crate::git::DiffSummary>) -> Row<'static> {
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
    let branch_color = commit_graph_color(commit);
    for reference in &commit.refs {
        let (label, color) = if let Some(tag) = reference.strip_prefix("tag: ") {
            (tag, palette().yellow)
        } else if let Some(branch) = reference.strip_prefix("HEAD -> ") {
            (branch, branch_color)
        } else if reference == "HEAD" {
            continue;
        } else {
            (reference.as_str(), branch_color)
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
    Row::new([
        Cell::from(graph),
        Cell::from(Line::from(description)),
        changes,
        Cell::from(commit.date.clone()).style(Style::default().fg(palette().muted)),
        Cell::from(commit.author.clone()).style(Style::default().fg(palette().muted)),
        Cell::from(short_oid).style(Style::default().fg(palette().muted)),
    ])
}

fn commit_graph_color(commit: &Commit) -> Color {
    commit
        .graph
        .iter()
        .find(|cell| cell.symbol == '●')
        .map_or(palette().accent, |cell| {
            palette().graph_colors[cell.color % palette().graph_colors.len()]
        })
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

fn ref_badge(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(color)
            .bg(palette().raised)
            .add_modifier(Modifier::BOLD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GraphCell;

    #[test]
    fn branch_badges_use_the_commit_graph_node_color() {
        let commit = Commit {
            oid: "abc".to_owned(),
            parents: Vec::new(),
            refs: vec!["feature/colors".to_owned()],
            author: "Ada".to_owned(),
            date: "today".to_owned(),
            subject: "Color branches".to_owned(),
            message: String::new(),
            graph: vec![
                GraphCell {
                    symbol: '│',
                    color: 0,
                },
                GraphCell {
                    symbol: '●',
                    color: 3,
                },
            ],
        };

        assert_eq!(commit_graph_color(&commit), palette().graph_colors[3]);
        assert_eq!(
            commit_graph_color(&Commit {
                graph: Vec::new(),
                ..commit
            }),
            palette().accent
        );
    }

    #[test]
    fn graph_columns_keep_date_visible_at_every_supported_width() {
        let settings = Settings::default();
        for width in 24..160 {
            let widths = graph_column_widths(width, 20, &settings);
            assert!(widths[3] >= 4, "date disappeared at width {width}");
            assert!(
                widths.iter().sum::<u16>() + 5 <= width,
                "columns overflowed width {width}: {widths:?}"
            );
        }
    }
}
