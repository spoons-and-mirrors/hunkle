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
    },
}

enum SingleSurface {
    Sidebar(LeftPane),
    Preview(LeftPane),
    Graph,
}

enum DetailSurface {
    Preview(LeftPane),
    Graph,
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect, profile: LayoutProfile) {
    let plan = plan(app, area, profile);
    match plan {
        WorkspacePlan::Search => draw_search(frame, app, area),
        WorkspacePlan::Single(surface) => draw_single(frame, app, area, surface),
        WorkspacePlan::Columns {
            files,
            detail_area,
            changes,
            detail,
        } => {
            let preview_pane = match detail {
                DetailSurface::Preview(pane) => Some(pane),
                DetailSurface::Graph => None,
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
                DetailSurface::Preview(_) => {}
            }
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
        let surface = if view == View::Graph && !app.graph_commit_open() {
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
        app.settings.files_width,
        app.settings.worktree_width,
        area,
        app.sidebar_layout(),
    );
    WorkspacePlan::Columns {
        files: columns.files,
        detail_area: columns.detail,
        changes: columns.changes,
        detail,
    }
}

struct ColumnAreas {
    files: Rect,
    detail: Rect,
    changes: Rect,
}

fn column_areas(
    files_width_setting: u16,
    worktree_width: u16,
    area: Rect,
    layout: crate::app::SidebarLayout,
) -> ColumnAreas {
    let show_files = layout.show_files();
    let show_changes = layout.show_changes();
    if !show_files && !show_changes {
        return ColumnAreas {
            files: Rect::new(area.x, area.y, 0, area.height),
            detail: area,
            changes: Rect::new(area.x, area.y, 0, area.height),
        };
    }
    let files_width = if show_files {
        files_width_setting.clamp(20, area.width.saturating_sub(50).max(20))
    } else {
        0
    };
    let changes_width = if show_changes {
        (worktree_width.saturating_sub(2)).clamp(
            24,
            area.width
                .saturating_sub(files_width)
                .saturating_sub(26)
                .max(24),
        )
    } else {
        0
    };
    let files = Rect::new(area.x, area.y, files_width, area.height);
    let changes_x = area.right().saturating_sub(changes_width);
    let changes = Rect::new(changes_x, area.y, changes_width, area.height);
    let viewer_x = files.right().saturating_add(1);
    let viewer_width = changes_x.saturating_sub(viewer_x).saturating_sub(1);
    let viewer = Rect::new(viewer_x, area.y, viewer_width, area.height);
    let detail = viewer;
    ColumnAreas {
        files,
        detail,
        changes,
    }
}

fn draw_single(frame: &mut Frame<'_>, app: &mut App, area: Rect, surface: SingleSurface) {
    let plan = match surface {
        SingleSurface::Sidebar(pane) => changes::ChangesPlan::SingleMaster { area, pane },
        SingleSurface::Preview(pane) => changes::ChangesPlan::SinglePreview { area, pane },
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
        let columns = column_areas(
            21,
            31,
            Rect::new(2, 3, 100, 40),
            crate::app::SidebarLayout::Both,
        );

        assert_eq!(columns.files, Rect::new(2, 3, 21, 40));
        assert_eq!(columns.detail, Rect::new(24, 3, 48, 40));
        assert_eq!(columns.changes, Rect::new(73, 3, 29, 40));
    }

    #[test]
    fn sidebar_layout_cycles_through_all_visibility_states() {
        use crate::app::SidebarLayout;
        assert_eq!(SidebarLayout::Both.cycle(), SidebarLayout::FilesOnly);
        assert_eq!(
            SidebarLayout::FilesOnly.cycle(),
            SidebarLayout::ChangesOnly
        );
        assert_eq!(SidebarLayout::ChangesOnly.cycle(), SidebarLayout::Hidden);
        assert_eq!(SidebarLayout::Hidden.cycle(), SidebarLayout::Both);

        let area = Rect::new(2, 3, 100, 40);
        let hidden = column_areas(21, 31, area, SidebarLayout::Hidden);
        assert_eq!(hidden.files.width, 0);
        assert_eq!(hidden.changes.width, 0);
        assert_eq!(hidden.detail, area);
        let files_only = column_areas(21, 31, area, SidebarLayout::FilesOnly);
        assert!(files_only.files.width > 0);
        assert_eq!(files_only.changes.width, 0);
        let changes_only = column_areas(21, 31, area, SidebarLayout::ChangesOnly);
        assert_eq!(changes_only.files.width, 0);
        assert!(changes_only.changes.width > 0);
    }

    #[test]
    fn files_and_changes_widths_resize_independently() {
        use crate::app::SidebarLayout;
        let area = Rect::new(0, 0, 120, 40);
        let narrow_files = column_areas(20, 40, area, SidebarLayout::Both);
        let wide_files = column_areas(40, 40, area, SidebarLayout::Both);
        assert_eq!(narrow_files.files.width, 20);
        assert_eq!(wide_files.files.width, 40);
        assert_eq!(narrow_files.changes.width, wide_files.changes.width);
    }
}
