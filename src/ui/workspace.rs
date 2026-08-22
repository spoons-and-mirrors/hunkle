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
        areas: [Rect; 2],
        sidebar_pane: LeftPane,
        detail: DetailSurface,
        agents: changes::ColumnAgents,
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
    Graph,
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect, profile: LayoutProfile) {
    let plan = plan(app, area, profile);
    app.regions.agent_cards_presented = plan.agent_cards_presented(app.repository().is_some());
    match plan {
        WorkspacePlan::Search => draw_search(frame, app, area),
        WorkspacePlan::Single(surface) => draw_single(frame, app, area, surface),
        WorkspacePlan::Columns {
            areas,
            sidebar_pane,
            detail,
            agents,
            companion,
        } => {
            let preview_pane = match detail {
                DetailSurface::Preview(pane) => pane,
                DetailSurface::Graph => app.changes.preview.pane(),
            };
            changes::draw(
                frame,
                app,
                changes::ChangesPlan::Columns {
                    areas,
                    sidebar_pane,
                    preview_pane: matches!(detail, DetailSurface::Preview(_))
                        .then_some(preview_pane),
                    agents,
                },
            );
            if matches!(detail, DetailSurface::Graph) {
                draw_graph(frame, app, areas[1]);
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
                agents: changes::ColumnAgents::Master | changes::ColumnAgents::MasterDetail,
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

    let detail = if view == View::Graph && !app.graph_commit_open() {
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
    let agents = if !app.agents_available() || !app.agents_visible {
        changes::ColumnAgents::Hidden
    } else if app.agents_pane_visible() {
        if columns.companion.is_some() {
            changes::ColumnAgents::Master
        } else {
            changes::ColumnAgents::MasterDetail
        }
    } else {
        changes::ColumnAgents::Master
    };
    WorkspacePlan::Columns {
        areas: columns.primary,
        sidebar_pane,
        detail,
        agents,
        companion: columns.companion,
    }
}

struct ColumnAreas {
    primary: [Rect; 2],
    companion: Option<Rect>,
}

fn column_areas(worktree_width: u16, area: Rect, companion_min_width: Option<u16>) -> ColumnAreas {
    let left_width = worktree_width.clamp(24, area.width.saturating_sub(25));
    let master = Rect::new(area.x, area.y, left_width, area.height);
    let viewer = Rect::new(
        master.right().saturating_add(1),
        area.y,
        area.width.saturating_sub(left_width).saturating_sub(1),
        area.height,
    );
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
        primary: [master, detail],
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
        let areas = column_areas(31, Rect::new(2, 3, 100, 40), None).primary;

        assert_eq!(areas[0], Rect::new(2, 3, 31, 40));
        assert_eq!(areas[1], Rect::new(34, 3, 68, 40));
    }

    #[test]
    fn wide_viewer_adds_an_equal_companion_column() {
        let columns = column_areas(38, Rect::new(0, 0, 180, 40), Some(120));

        assert_eq!(columns.primary[0], Rect::new(0, 0, 38, 40));
        assert_eq!(columns.primary[1], Rect::new(39, 0, 70, 40));
        assert_eq!(columns.companion, Some(Rect::new(110, 0, 70, 40)));
    }

    #[test]
    fn companion_waits_for_usable_main_viewer_width() {
        let columns = column_areas(38, Rect::new(0, 0, 158, 40), Some(120));

        assert_eq!(columns.primary[1], Rect::new(39, 0, 119, 40));
        assert_eq!(columns.companion, None);
    }

    #[test]
    fn companion_threshold_uses_the_configured_viewer_width() {
        let columns = column_areas(38, Rect::new(0, 0, 158, 40), Some(100));

        assert_eq!(columns.primary[1], Rect::new(39, 0, 59, 40));
        assert_eq!(columns.companion, Some(Rect::new(99, 0, 59, 40)));
    }

    #[test]
    fn composition_declares_agent_card_interest() {
        let area = Rect::new(0, 0, 80, 40);
        let columns = |agents| WorkspacePlan::Columns {
            areas: [area, area],
            sidebar_pane: LeftPane::Worktree,
            detail: DetailSurface::Preview(LeftPane::Worktree),
            agents,
            companion: None,
        };

        assert!(WorkspacePlan::Single(SingleSurface::Agents).agent_cards_presented(false));
        assert!(!WorkspacePlan::Single(SingleSurface::AgentHistory).agent_cards_presented(false));
        assert!(columns(changes::ColumnAgents::Master).agent_cards_presented(true));
        assert!(!columns(changes::ColumnAgents::Master).agent_cards_presented(false));
        assert!(!columns(changes::ColumnAgents::Hidden).agent_cards_presented(true));
        assert!(!WorkspacePlan::Search.agent_cards_presented(true));
    }
}
