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
        GraphSearch, HitTarget, Settings, ShortcutAction, Shortcuts,
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
    pub search: &'a GraphSearch,
    pub search_focused: bool,
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
        search,
        search_focused,
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
    let table_area = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    let search_area = Rect::new(area.x, table_area.y, area.width, 1);
    let commit_table_area = Rect::new(
        table_area.x,
        table_area.y.saturating_add(2),
        table_area.width,
        table_area.height.saturating_sub(2),
    );
    let graph_region = Rect::new(
        commit_table_area.x,
        commit_table_area.y.saturating_add(2),
        commit_table_area.width,
        commit_table_area.height.saturating_sub(2),
    );
    draw_graph_search(frame, search_area, search, search_focused);

    let column_widths = graph_column_widths(table_area.width, repo.graph_width, settings);
    let widths = column_widths.map(Constraint::Length);

    let visible = search.visible_indices();
    let viewport = usize::from(graph_region.height);
    let selected = state.selected();
    let selected_head_background = selected
        .and_then(|selected| visible.get(selected))
        .and_then(|index| repo.commits.get(*index))
        .filter(|commit| commit_is_head(commit))
        .map(commit_graph_highlight);
    let mut offset = state.offset().min(visible.len().saturating_sub(1));
    if *scroll_to_selection && let Some(selected) = selected {
        offset = graph_scroll_offset(
            offset,
            selected,
            visible.len(),
            viewport,
            search.current_match_position() == Some(selected),
        );
    }
    *scroll_to_selection = false;
    *state.offset_mut() = offset;
    let search_active = search.match_status().is_some();
    let changes_width = column_widths[2];
    let rows = visible.iter().skip(offset).take(viewport).map(|index| {
        let commit = &repo.commits[*index];
        graph_row(
            commit,
            summaries.get(&commit.oid),
            changes_width,
            search_active,
        )
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
        if repo.graph_truncated {
            "GRAPH*".to_owned()
        } else {
            "GRAPH".to_owned()
        },
        "DESCRIPTION".to_owned(),
        "CHANGES".to_owned(),
        "DATE".to_owned(),
        author_label,
        "COMMIT".to_owned(),
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
        .row_highlight_style(
            Style::default().bg(selected_head_background.unwrap_or(palette().selected)),
        );
    let mut visible_state = TableState::default();
    visible_state.select(selected.and_then(|selected| selected.checked_sub(offset)));
    frame.render_stateful_widget(table, commit_table_area, &mut visible_state);
    let column_starts = graph_column_starts(commit_table_area.x, column_widths);
    let graph_columns = [
        GraphColumn::Graph,
        GraphColumn::Description,
        GraphColumn::Changes,
        GraphColumn::Date,
        GraphColumn::Author,
        GraphColumn::Commit,
    ];
    let columns = (1..graph_columns.len())
        .map(|index| {
            let left = graph_columns[index - 1];
            let right = graph_columns[index];
            let splitter_line = Rect::new(
                column_starts[index].saturating_sub(1),
                commit_table_area.y,
                1,
                1,
            );
            frame.render_widget(
                Paragraph::new("│").style(Style::default().fg(if dragging_column == Some(right) {
                    palette().accent
                } else {
                    palette().faint
                })),
                splitter_line,
            );
            GraphColumnRegion {
                left,
                right,
                left_width: column_widths[index - 1],
                right_width: column_widths[index],
                splitter: Rect::new(splitter_line.x.saturating_sub(1), splitter_line.y, 2, 1),
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
    let author_header = Rect::new(author_x, commit_table_area.y, author_width, 1);
    GraphRegions {
        table: Some(graph_region),
        targets: vec![
            (HitTarget::Graph(GraphHitTarget::Search), search_area),
            (
                HitTarget::Graph(GraphHitTarget::AuthorHeader),
                author_header,
            ),
        ],
        columns,
    }
}

fn graph_scroll_offset(
    offset: usize,
    selected: usize,
    len: usize,
    viewport: usize,
    center_selection: bool,
) -> usize {
    if center_selection {
        return selected
            .saturating_sub(viewport / 2)
            .min(len.saturating_sub(viewport));
    }
    if selected < offset {
        selected
    } else if selected >= offset.saturating_add(viewport) {
        selected.saturating_add(1).saturating_sub(viewport)
    } else {
        offset
    }
}

fn draw_graph_search(frame: &mut Frame<'_>, area: Rect, search: &GraphSearch, focused: bool) {
    let input = &search.input;
    let mut text = input.text().to_owned();
    if focused && input.cursor_visible() {
        text.insert(input.cursor(), '▌');
    }
    let line = if text.is_empty() {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(palette().accent)),
            Span::styled(
                "Press Space and search by description, date, or hash",
                Style::default().fg(palette().faint),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(palette().accent)),
            Span::styled(text, Style::default().fg(palette().ink)),
        ])
    };
    let background = if focused {
        palette().surface_alt
    } else {
        palette().panel
    };
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(background)),
        area,
    );
    let status = search
        .match_status()
        .map(|(current, total)| format!("{current}/{total}"));
    let status_width = status
        .as_ref()
        .map_or(0, |status| status.len().min(usize::from(area.width)) as u16);
    let status_right_padding = u16::from(status.is_some())
        .saturating_mul(2)
        .min(area.width.saturating_sub(status_width));
    let input_width = area.width.saturating_sub(
        status_width
            .saturating_add(status_right_padding)
            .saturating_add(u16::from(status.is_some())),
    );
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, input_width, 1),
    );
    if let Some(status) = status {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(palette().muted)),
            Rect::new(
                area.right()
                    .saturating_sub(status_width.saturating_add(status_right_padding)),
                area.y,
                status_width,
                1,
            ),
        );
    }
}

fn graph_column_widths(width: u16, graph_width: usize, settings: &Settings) -> [u16; 6] {
    const COLUMN_SPACING: u16 = 5;
    const PREFERRED_MINIMUMS: [u16; 6] = [5, 8, 7, 9, 12, 7];
    const ABSOLUTE_MINIMUMS: [u16; 6] = [2, 1, 3, 4, 3, 3];
    let available = width.saturating_sub(COLUMN_SPACING).max(1);
    let mut widths = [
        match settings.graph_column_width(GraphColumn::Graph) {
            0 => graph_width.clamp(8, 40) as u16,
            width => width,
        },
        match settings.graph_column_width(GraphColumn::Description) {
            0 => PREFERRED_MINIMUMS[1],
            width => width,
        },
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
    let requested = widths.iter().sum::<u16>();
    if requested < available {
        widths[1] = widths[1].saturating_add(available - requested);
        return widths;
    }
    let mut overflow = requested.saturating_sub(available);
    for index in [1, 4, 2, 0, 5, 3] {
        let reduction = widths[index]
            .saturating_sub(PREFERRED_MINIMUMS[index])
            .min(overflow);
        widths[index] = widths[index].saturating_sub(reduction);
        overflow = overflow.saturating_sub(reduction);
    }
    for index in [1, 4, 2, 0, 5, 3] {
        let reduction = widths[index]
            .saturating_sub(ABSOLUTE_MINIMUMS[index])
            .min(overflow);
        widths[index] = widths[index].saturating_sub(reduction);
        overflow = overflow.saturating_sub(reduction);
    }
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

fn graph_row(
    commit: &Commit,
    summary: Option<&crate::git::DiffSummary>,
    changes_width: u16,
    search_active: bool,
) -> Row<'static> {
    let is_head = commit_is_head(commit);
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
    if is_head {
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
    let changes = commit_changes(summary, changes_width);
    Row::new([
        Cell::from(graph),
        Cell::from(Line::from(description)),
        changes,
        Cell::from(commit.date.clone()).style(Style::default().fg(palette().muted)),
        Cell::from(commit.author.clone()).style(Style::default().fg(palette().muted)),
        Cell::from(short_oid).style(Style::default().fg(palette().muted)),
    ])
    .style(if search_active {
        Style::default().bg(palette().surface_alt)
    } else if is_head {
        Style::default().bg(commit_graph_highlight(commit))
    } else {
        Style::default()
    })
}

fn commit_is_head(commit: &Commit) -> bool {
    commit
        .refs
        .iter()
        .any(|reference| reference == "HEAD" || reference.starts_with("HEAD -> "))
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

pub(super) fn commit_graph_highlight(commit: &Commit) -> Color {
    match (palette().panel, commit_graph_color(commit)) {
        (
            Color::Rgb(background_red, background_green, background_blue),
            Color::Rgb(red, green, blue),
        ) => Color::Rgb(
            blend_channel(background_red, red),
            blend_channel(background_green, green),
            blend_channel(background_blue, blue),
        ),
        (_, color) => color,
    }
}

fn blend_channel(background: u8, color: u8) -> u8 {
    ((u16::from(background) * 4 + u16::from(color)) / 5) as u8
}

fn commit_changes(summary: Option<&crate::git::DiffSummary>, width: u16) -> Cell<'static> {
    let Some(summary) = summary else {
        return Cell::from("…").style(Style::default().fg(palette().faint));
    };
    let (additions, deletions) = commit_change_columns(summary, width);
    Cell::from(Line::from(vec![
        Span::styled(additions, Style::default().fg(palette().green)),
        Span::raw(" "),
        Span::styled(deletions, Style::default().fg(palette().red)),
        Span::raw(" "),
    ]))
}

fn commit_change_columns(summary: &crate::git::DiffSummary, width: u16) -> (String, String) {
    const STAT_WIDTH: usize = 5;
    let content_width = usize::from(width.saturating_sub(2));
    let additions_width = (content_width / 2).min(STAT_WIDTH);
    let deletions_width = content_width
        .saturating_sub(additions_width)
        .min(STAT_WIDTH);
    let left_padding = content_width.saturating_sub(additions_width + deletions_width);
    let additions = format!("+{}", summary.additions);
    let deletions = format!("-{}", summary.deletions);
    (
        format!(
            "{additions:>width$}",
            width = left_padding + additions_width
        ),
        format!("{deletions:>deletions_width$}"),
    )
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
    use ratatui::style::Styled;

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
    fn head_commit_row_uses_its_graph_color_for_the_background() {
        let mut commit = Commit {
            oid: "abc".to_owned(),
            parents: Vec::new(),
            refs: vec!["HEAD -> main".to_owned()],
            author: "Ada".to_owned(),
            date: "today".to_owned(),
            subject: "Current commit".to_owned(),
            message: String::new(),
            graph: vec![GraphCell {
                symbol: '●',
                color: 3,
            }],
        };

        assert_eq!(
            Styled::style(&graph_row(&commit, None, 11, false)).bg,
            Some(commit_graph_highlight(&commit))
        );
        assert_ne!(commit_graph_highlight(&commit), palette().add_bg);

        commit.refs = vec!["main".to_owned()];
        assert_eq!(Styled::style(&graph_row(&commit, None, 11, false)).bg, None);
        assert_eq!(
            Styled::style(&graph_row(&commit, None, 11, true)).bg,
            Some(palette().surface_alt)
        );
    }

    #[test]
    fn commit_changes_use_two_right_aligned_columns() {
        let summary = crate::git::DiffSummary {
            additions: 12,
            deletions: 3,
            ..Default::default()
        };

        assert_eq!(
            commit_change_columns(&summary, 12),
            ("  +12".to_owned(), "   -3".to_owned())
        );

        let (additions, deletions) = commit_change_columns(&summary, 31);
        let resized = format!("{additions} {deletions}");
        assert_eq!(resized.len(), 30);
        assert!(resized.ends_with("  +12    -3"));
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

    #[test]
    fn search_navigation_keeps_the_selected_commit_centered_when_possible() {
        assert_eq!(graph_scroll_offset(0, 50, 100, 11, true), 45);
        assert_eq!(graph_scroll_offset(0, 50, 100, 10, true), 45);
        assert_eq!(graph_scroll_offset(45, 2, 100, 11, true), 0);
        assert_eq!(graph_scroll_offset(0, 98, 100, 11, true), 89);

        assert_eq!(graph_scroll_offset(10, 12, 100, 11, false), 10);
        assert_eq!(graph_scroll_offset(10, 25, 100, 11, false), 15);
    }
}
