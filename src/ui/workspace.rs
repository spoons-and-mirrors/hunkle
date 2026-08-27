//! Responsive composition for the shared workspace.
//!
//! This module selects and arranges feature surfaces. Feature renderers receive
//! explicit areas and must not choose the global composition themselves. See
//! `docs/adr/0009-responsive-workspace-composition.md` before extending it.

use super::*;

enum WorkspacePlan {
    Search,
    Single(SingleSurface),
    Columns {
        files: Rect,
        detail_area: Rect,
        changes: Rect,
        detail: DetailSurface,
        companion: Option<Rect>,
    },
}

enum SingleSurface {
    Sidebar(LeftPane),
    Preview(LeftPane),
    Agents,
    AgentHistory,
    Graph,
}

enum DetailSurface {
    Preview(LeftPane),
    Agents,
    Graph,
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect, profile: LayoutProfile) {
    let plan = plan(app, area, profile);
    app.regions.agent_cards_presented = plan.agent_cards_presented(app.repository().is_some());
    match plan {
        WorkspacePlan::Search => draw_search(frame, app, area),
        WorkspacePlan::Single(surface) => draw_single(frame, app, area, surface),
        WorkspacePlan::Columns {
            files,
            detail_area,
            changes,
            detail,
            companion,
        } => {
            let preview_pane = match detail {
                DetailSurface::Preview(pane) => Some(pane),
                DetailSurface::Agents | DetailSurface::Graph => None,
            };
            changes::draw(
                frame,
                app,
                changes::ChangesPlan::Columns {
                    files_area: files,
                    editor_area: detail_area,
                    changes_area: changes,
                    preview_pane,
                },
            );
            match detail {
                DetailSurface::Graph => draw_graph(frame, app, detail_area),
                DetailSurface::Agents => changes::draw_agents_panel(frame, app, detail_area),
                DetailSurface::Preview(_) => {}
            }
            if let Some(area) = companion {
                changes::draw_agent_preview_companion(frame, app, area);
            }
        }
    }
}

impl WorkspacePlan {
    fn agent_cards_presented(&self, repository_present: bool) -> bool {
        match self {
            Self::Single(SingleSurface::Agents) => true,
            Self::Columns {
                detail: DetailSurface::Agents,
                ..
            } if repository_present => true,
            _ => false,
        }
    }
}

fn plan(app: &App, area: Rect, profile: LayoutProfile) -> WorkspacePlan {
    let view = app.visible_view();
    if view == View::RepositorySearch {
        return WorkspacePlan::Search;
    }

    let sidebar_pane = app.sidebar_pane();
    let preview_pane = app.changes.preview.pane();
    if profile.is_single() {
        let surface = if app.agents_pane_visible() {
            if app.workspace_detail_open() {
                SingleSurface::AgentHistory
            } else {
                SingleSurface::Agents
            }
        } else if view == View::Graph && !app.graph_commit_open() {
            SingleSurface::Graph
        } else if app.workspace_detail_open() || app.mode == Mode::FileEdit {
            SingleSurface::Preview(preview_pane)
        } else {
            SingleSurface::Sidebar(sidebar_pane)
        };
        return WorkspacePlan::Single(surface);
    }

    let detail = if app.agents_pane_visible() {
        DetailSurface::Agents
    } else if view == View::Graph && !app.graph_commit_open() {
        DetailSurface::Graph
    } else {
        DetailSurface::Preview(preview_pane)
    };
    let columns = column_areas(
        app.settings.worktree_width,
        area,
        (app.herdr_available() && app.agent_preview_index().is_some())
            .then_some(app.settings.agent_preview_split_width),
    );
    WorkspacePlan::Columns {
        files: columns.files,
        detail_area: columns.detail,
        changes: columns.changes,
        detail,
        companion: columns.companion,
    }
}

struct ColumnAreas {
    files: Rect,
    detail: Rect,
    changes: Rect,
    companion: Option<Rect>,
}

fn column_areas(worktree_width: u16, area: Rect, companion_min_width: Option<u16>) -> ColumnAreas {
    let files_width = (worktree_width.saturating_sub(10)).clamp(20, area.width.saturating_sub(50).max(20));
    let changes_width = (worktree_width.saturating_sub(2)).clamp(24, area.width.saturating_sub(files_width).saturating_sub(26).max(24));
    let files = Rect::new(area.x, area.y, files_width, area.height);
    let changes_x = area.right().saturating_sub(changes_width);
    let changes = Rect::new(changes_x, area.y, changes_width, area.height);
    let viewer_x = files.right().saturating_add(1);
    let viewer_width = changes_x.saturating_sub(viewer_x).saturating_sub(1);
    let viewer = Rect::new(viewer_x, area.y, viewer_width, area.height);
    let (detail, companion) = if companion_min_width.is_some_and(|width| viewer.width >= width) {
        let detail_width = viewer.width.saturating_sub(1) / 2;
        let detail = Rect::new(viewer.x, viewer.y, detail_width, viewer.height);
        let companion = Rect::new(
            detail.right().saturating_add(1),
            viewer.y,
            viewer
                .right()
                .saturating_sub(detail.right().saturating_add(1)),
            viewer.height,
        );
        (detail, Some(companion))
    } else {
        (viewer, None)
    };
    ColumnAreas {
        files,
        detail,
        changes,
        companion,
    }
}

fn draw_single(frame: &mut Frame<'_>, app: &mut App, area: Rect, surface: SingleSurface) {
    let plan = match surface {
        SingleSurface::Sidebar(pane) => changes::ChangesPlan::SingleMaster { area, pane },
        SingleSurface::Preview(pane) => changes::ChangesPlan::SinglePreview { area, pane },
        SingleSurface::Agents => changes::ChangesPlan::SingleAgents { area },
        SingleSurface::AgentHistory => changes::ChangesPlan::SingleAgentHistory { area },
        SingleSurface::Graph => {
            draw_graph(frame, app, area);
            return;
        }
    };
    changes::draw(frame, app, plan);
}

fn draw_search(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    app.reset_media_presentation();
    let search_root = app.repository().map(|repository| repository.root.clone());
    let regions =
        overlays::draw_file_search(frame, &mut app.file_search, search_root.as_deref(), area);
    app.regions.file_search = Some(regions.overlay);
    app.regions.file_search_list = Some(regions.list);
    app.regions
        .register_scroll_target(ScrollTarget::RepositorySearch, regions.list);
    for (target, rect) in regions.targets {
        app.regions.register_hit_target(target, rect);
    }
}

fn draw_graph(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    app.reset_media_presentation();
    let graph_regions = history::draw_graph(
        frame,
        area,
        history::GraphView {
            repo: app.session.data(),
            summaries: &app.commit_summaries,
            author_filter: &app.author_filter,
            search: &app.graph_search,
            search_focused: app.graph_search_focused,
            state: &mut app.graph_state,
            scroll_to_selection: &mut app.graph_scroll_to_selection,
            settings: &app.settings,
            dragging_column: app.dragging_graph_column.map(|drag| drag.right),
        },
    );
    app.regions.graph_table = graph_regions.table;
    app.regions.graph_columns = graph_regions.columns;
    if let Some(table) = graph_regions.table {
        app.regions
            .register_scroll_target(ScrollTarget::Graph, table);
    }
    for (target, rect) in graph_regions.targets {
        app.regions.register_hit_target(target, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_respect_the_persisted_master_width() {
        let columns = column_areas(31, Rect::new(2, 3, 100, 40), None);

        assert_eq!(columns.files, Rect::new(2, 3, 21, 40));
        assert_eq!(columns.detail, Rect::new(24, 3, 48, 40));
        assert_eq!(columns.changes, Rect::new(73, 3, 29, 40));
    }

    #[test]
    fn wide_viewer_adds_an_equal_companion_column() {
        let columns = column_areas(38, Rect::new(0, 0, 180, 40), Some(100));

        assert_eq!(columns.files, Rect::new(0, 0, 28, 40));
        assert_eq!(columns.detail, Rect::new(29, 0, 56, 40));
        assert_eq!(columns.companion, Some(Rect::new(86, 0, 57, 40)));
        assert_eq!(columns.changes, Rect::new(144, 0, 36, 40));
    }

    #[test]
    fn companion_waits_for_usable_main_viewer_width() {
        let columns = column_areas(38, Rect::new(0, 0, 158, 40), Some(100));

        assert_eq!(columns.detail, Rect::new(29, 0, 92, 40));
        assert_eq!(columns.companion, None);
    }

    #[test]
    fn companion_threshold_uses_the_configured_viewer_width() {
        let columns = column_areas(38, Rect::new(0, 0, 158, 40), Some(80));

        assert_eq!(columns.detail, Rect::new(29, 0, 45, 40));
        assert_eq!(columns.companion, Some(Rect::new(75, 0, 46, 40)));
    }

    #[test]
    fn composition_declares_agent_card_interest() {
        let area = Rect::new(0, 0, 80, 40);
        let columns = |detail| WorkspacePlan::Columns {
            files: area,
            detail_area: area,
            changes: area,
            detail,
            companion: None,
        };

        assert!(WorkspacePlan::Single(SingleSurface::Agents).agent_cards_presented(false));
        assert!(!WorkspacePlan::Single(SingleSurface::AgentHistory).agent_cards_presented(false));
        assert!(columns(DetailSurface::Agents).agent_cards_presented(true));
        assert!(!columns(DetailSurface::Agents).agent_cards_presented(false));
        assert!(!columns(DetailSurface::Preview(LeftPane::Worktree)).agent_cards_presented(true));
        assert!(!WorkspacePlan::Search.agent_cards_presented(true));
    }
}
