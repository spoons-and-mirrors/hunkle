use super::super::header_card::draw_header_card;
use super::super::location_picker::{
    LocationPickerRow, LocationPickerRowKind, LocationPickerView, draw_location_picker,
};
use super::*;
use ansi_to_tui::IntoText;

type Target = SchedulerHitTarget;

pub(crate) struct SchedulerRegions {
    pub(crate) targets: Vec<(HitTarget, Rect)>,
    pub(crate) scrolls: Vec<(ScrollTarget, Rect)>,
}

impl SchedulerRegions {
    fn target(&mut self, target: SchedulerHitTarget, area: Rect) {
        self.targets.push((HitTarget::Scheduler(target), area));
    }

    fn scroll(&mut self, target: ScrollTarget, area: Rect) {
        self.scrolls.push((target, area));
    }
}

pub(crate) fn draw_scheduler(
    frame: &mut Frame<'_>,
    app: &App,
    profile: LayoutProfile,
) -> SchedulerRegions {
    let pane_open = app.scheduler.surface == SchedulerSurface::Pane;
    let outer = scheduler_area(frame.area(), profile, pane_open);
    frame.render_widget(Clear, outer);
    fill(frame, outer, palette().panel);
    fill(
        frame,
        Rect::new(outer.x, outer.y, outer.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(outer.x, outer.bottom().saturating_sub(1), outer.width, 1),
        palette().surface_alt,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "SCHEDULER",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Automated Herdr tasks",
                Style::default().fg(palette().faint),
            ),
        ]))
        .style(Style::default().bg(palette().surface_alt)),
        Rect::new(outer.x + 2, outer.y + 1, outer.width.saturating_sub(12), 1),
    );
    let close = Rect::new(outer.right().saturating_sub(9), outer.y + 1, 7, 1);
    let mut regions = SchedulerRegions {
        targets: Vec::new(),
        scrolls: Vec::new(),
    };
    button(
        frame,
        &mut regions,
        close,
        " CLOSE ",
        Target::Close,
        palette().cyan,
    );

    let content = Rect::new(
        outer.x + 1,
        outer.y + 3,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(4),
    );
    if pane_open {
        draw_live_pane(frame, app, content, &mut regions);
    } else if profile.is_single() {
        if app.scheduler.surface == SchedulerSurface::Tasks {
            draw_tasks(frame, app, content, &mut regions);
        } else {
            let back = Rect::new(content.x + 1, content.y, 8, 1);
            button(
                frame,
                &mut regions,
                back,
                " < BACK ",
                Target::Back,
                palette().cyan,
            );
            let detail = Rect::new(
                content.x,
                content.y + 2,
                content.width,
                content.height.saturating_sub(2),
            );
            draw_detail(frame, app, detail, &mut regions);
        }
    } else {
        let columns = Layout::horizontal([
            Constraint::Length(33),
            Constraint::Length(1),
            Constraint::Min(42),
        ])
        .split(content);
        draw_tasks(frame, app, columns[0], &mut regions);
        fill(frame, columns[1], palette().surface_alt);
        draw_detail(frame, app, columns[2], &mut regions);
    }

    let footer = Rect::new(
        outer.x + 2,
        outer.bottom().saturating_sub(1),
        outer.width.saturating_sub(4),
        1,
    );
    if let Some(error) = app.scheduler.error.as_deref() {
        draw_text(
            frame,
            footer,
            truncate_width(error, usize::from(footer.width)),
            Style::default().fg(palette().red).bg(palette().surface_alt),
        );
    } else {
        let hints = if pane_open {
            &[
                ("←→", "pan"),
                ("↑↓", "scroll"),
                ("V", "conversation"),
                ("P/Esc", "back"),
            ][..]
        } else if app.scheduler.composer.is_some() {
            &[
                ("Ctrl+S", "save"),
                ("Ctrl+E", "expand prompt"),
                ("Esc", "cancel"),
            ][..]
        } else if profile.is_single() && app.scheduler.surface == SchedulerSurface::Tasks {
            &[
                ("N", "new"),
                ("↑↓", "select"),
                ("Enter", "open"),
                ("Esc", "close"),
            ][..]
        } else {
            &[
                ("N", "new"),
                ("Tab", "tasks/runs"),
                ("R", "run"),
                ("P", "live pane"),
                ("V", "conversation"),
                ("Esc", "close"),
            ][..]
        };
        frame.render_widget(
            Paragraph::new(key_hint_line(hints, usize::from(footer.width)))
                .style(Style::default().bg(palette().surface_alt)),
            footer,
        );
    }
    regions
}

fn scheduler_area(area: Rect, profile: LayoutProfile, pane_open: bool) -> Rect {
    let mut outer = if pane_open {
        centered_min(area, 96, 94, 88, 28)
    } else if profile.is_single() {
        centered_min(area, 96, 92, 40, 24)
    } else {
        centered_min(area, 88, 82, 88, 28)
    };
    let height = outer.height.min(if pane_open { 70 } else { 46 });
    let width = outer.width.min(if pane_open { 220 } else { 132 });
    outer.y = area.y + area.height.saturating_sub(height) / 2;
    outer.x = area.x + area.width.saturating_sub(width) / 2;
    outer.width = width;
    outer.height = height;
    outer
}

fn draw_live_pane(frame: &mut Frame<'_>, app: &App, area: Rect, regions: &mut SchedulerRegions) {
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let back = Rect::new(inner.x, inner.y, 9, 1);
    button(
        frame,
        regions,
        back,
        " < BACK ",
        Target::ClosePane,
        palette().cyan,
    );
    let conversation = Rect::new(inner.right().saturating_sub(20), inner.y, 20, 1);
    button(
        frame,
        regions,
        conversation,
        " OPEN CONVERSATION ",
        Target::OpenConversation,
        palette().accent,
    );
    bold_text(
        frame,
        Rect::new(
            back.right().saturating_add(2),
            inner.y,
            conversation.x.saturating_sub(back.right() + 4),
            1,
        ),
        "LIVE PANE",
    );
    let Some(pane_id) = app.selected_scheduled_run_pane_id() else {
        draw_text(
            frame,
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
            "This run has no available Herdr pane.",
            Style::default().fg(palette().red),
        );
        return;
    };
    let viewport = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    fill(frame, viewport, Color::Black);
    regions.scroll(ScrollTarget::SchedulerPane, viewport);
    if let Some(error) = app.herdr.pane_preview_error(&pane_id) {
        draw_text(
            frame,
            Rect::new(
                viewport.x + 2,
                viewport.y + 1,
                viewport.width.saturating_sub(4),
                2,
            ),
            format!("Could not read {pane_id}: {error}"),
            Style::default().fg(palette().red).bg(Color::Black),
        );
        return;
    }
    let Some(preview) = app.herdr.pane_preview(&pane_id) else {
        draw_text(
            frame,
            Rect::new(
                viewport.x + 2,
                viewport.y + 1,
                viewport.width.saturating_sub(4),
                1,
            ),
            format!("Reading {pane_id}…"),
            Style::default().fg(palette().faint).bg(Color::Black),
        );
        return;
    };
    let Ok(text) = preview.ansi.as_bytes().into_text() else {
        draw_text(
            frame,
            Rect::new(
                viewport.x + 2,
                viewport.y + 1,
                viewport.width.saturating_sub(4),
                1,
            ),
            "Herdr returned an ANSI pane snapshot Hunkle could not render.",
            Style::default().fg(palette().red).bg(Color::Black),
        );
        return;
    };
    let source_width = text.width();
    let source_height = text.height();
    let maximum_x = source_width.saturating_sub(usize::from(viewport.width));
    let maximum_y = source_height.saturating_sub(usize::from(viewport.height));
    let scroll_x = app.scheduler.pane_scroll_x.min(maximum_x);
    let scroll_y = maximum_y.saturating_sub(app.scheduler.pane_scroll_bottom.min(maximum_y));
    frame.render_widget(
        Paragraph::new(text).scroll((
            scroll_y.min(u16::MAX as usize) as u16,
            scroll_x.min(u16::MAX as usize) as u16,
        )),
        viewport,
    );
    let dimensions = format!(
        " {pane_id}  {source_width}×{source_height} → {}×{} ",
        viewport.width, viewport.height
    );
    let width = UnicodeWidthStr::width(dimensions.as_str()) as u16;
    draw_text(
        frame,
        Rect::new(
            inner.right().saturating_sub(width.min(inner.width)),
            inner.y + 1,
            width.min(inner.width),
            1,
        ),
        dimensions,
        Style::default().fg(palette().cyan).bg(palette().panel),
    );
}

fn draw_tasks(frame: &mut Frame<'_>, app: &App, area: Rect, regions: &mut SchedulerRegions) {
    let tasks = app.herdr.scheduled_tasks();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "TASKS",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", tasks.len()),
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(11), 1),
    );
    let new = Rect::new(area.right().saturating_sub(7), area.y, 6, 1);
    button(frame, regions, new, " + NEW", Target::New, palette().accent);
    draw_text(
        frame,
        Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1),
        "Runs while Hunkle is open",
        Style::default().fg(palette().faint),
    );
    let list = Rect::new(
        area.x + 1,
        area.y + 3,
        area.width.saturating_sub(2),
        area.height.saturating_sub(4),
    );
    regions.scroll(ScrollTarget::SchedulerTasks, list);
    if tasks.is_empty() {
        frame.render_widget(
            Paragraph::new("No scheduled tasks\n\nChoose + NEW to create one.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(palette().faint)),
            list,
        );
        return;
    }
    let visible = usize::from(list.height.saturating_add(1) / 3).max(1);
    for (row, task) in tasks
        .iter()
        .skip(app.scheduler.task_scroll)
        .take(visible)
        .enumerate()
    {
        let y = list.y + u16::try_from(row).unwrap_or(u16::MAX).saturating_mul(3);
        let rect = Rect::new(
            list.x,
            y,
            list.width,
            2.min(list.bottom().saturating_sub(y)),
        );
        let selected = app.scheduler.selected_task_id == Some(task.id);
        let background = if selected {
            palette().selected
        } else {
            palette().surface_alt
        };
        fill(frame, rect, background);
        draw_text(
            frame,
            Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(2), 1),
            Line::from(vec![
                Span::styled(
                    if task.enabled { "● " } else { "○ " },
                    Style::default().fg(if task.enabled {
                        palette().green
                    } else {
                        palette().faint
                    }),
                ),
                Span::styled(
                    truncate_width(&task.title, usize::from(rect.width.saturating_sub(4))),
                    Style::default()
                        .fg(palette().ink)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Style::default().bg(background),
        );
        if rect.height > 1 {
            draw_text(
                frame,
                Rect::new(rect.x + 3, rect.y + 1, rect.width.saturating_sub(4), 1),
                truncate_width(
                    &format!(
                        "{} / {}  ·  {}m",
                        task.repository, task.branch, task.interval_minutes
                    ),
                    usize::from(rect.width.saturating_sub(4)),
                ),
                Style::default().fg(palette().muted).bg(background),
            );
        }
        regions.target(Target::Task(task.id), rect);
    }
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, area: Rect, regions: &mut SchedulerRegions) {
    if let Some(composer) = app.scheduler.composer.as_ref() {
        draw_composer(frame, composer, area, regions);
        return;
    }
    let Some(task) = app.selected_scheduled_task() else {
        frame.render_widget(
            Paragraph::new("Select a task to review its schedule and run history.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(palette().faint)),
            Rect::new(area.x + 2, area.y + 2, area.width.saturating_sub(4), 3),
        );
        return;
    };
    let inner = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );
    let status = if task.enabled { "ENABLED" } else { "PAUSED" };
    let status_width = UnicodeWidthStr::width(status) as u16;
    bold_text(
        frame,
        Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(status_width + 2),
            1,
        ),
        truncate_width(
            &task.title,
            usize::from(inner.width.saturating_sub(status_width + 2)),
        ),
    );
    draw_text(
        frame,
        Rect::new(
            inner.right().saturating_sub(status_width),
            inner.y,
            status_width,
            1,
        ),
        status,
        Style::default()
            .fg(if task.enabled {
                palette().green
            } else {
                palette().faint
            })
            .add_modifier(Modifier::BOLD),
    );
    let action_y = inner.y + 2;
    let actions = [
        (" RUN NOW ", Target::RunNow, palette().accent),
        (
            if task.enabled { " PAUSE " } else { " ENABLE " },
            Target::Toggle,
            palette().cyan,
        ),
        (" DELETE ", Target::Delete, palette().red),
    ];
    let mut x = inner.x;
    for (label, target, color) in actions {
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(x, action_y, width, 1);
        button(frame, regions, rect, label, target, color);
        x = rect.right().saturating_add(1);
    }
    draw_text(
        frame,
        Rect::new(inner.x, action_y + 2, inner.width, 1),
        Line::from(vec![
            Span::styled(&task.repository, Style::default().fg(palette().yellow)),
            Span::styled("  /  ", Style::default().fg(palette().faint)),
            Span::styled(&task.branch, Style::default().fg(palette().accent)),
            Span::styled(
                format!(
                    "  ·  every {} minute{}",
                    task.interval_minutes,
                    if task.interval_minutes == 1 { "" } else { "s" }
                ),
                Style::default().fg(palette().muted),
            ),
        ]),
        Style::default(),
    );
    draw_text(
        frame,
        Rect::new(inner.x, action_y + 3, inner.width, 1),
        truncate_width(
            &task.destination.display().to_string(),
            usize::from(inner.width),
        ),
        Style::default().fg(palette().faint),
    );
    if !task.description.is_empty() {
        draw_text(
            frame,
            Rect::new(inner.x, action_y + 5, inner.width, 1),
            truncate_width(&task.description, usize::from(inner.width)),
            Style::default().fg(palette().soft),
        );
    }
    let prompt_y = action_y + 7;
    bold_text(
        frame,
        Rect::new(inner.x, prompt_y, inner.width, 1),
        "PROMPT",
    );
    let prompt = Rect::new(inner.x, prompt_y + 1, inner.width, 2);
    fill(frame, prompt, palette().surface_alt);
    frame.render_widget(
        Paragraph::new(task.prompt.as_str())
            .wrap(Wrap { trim: true })
            .style(
                Style::default()
                    .fg(palette().soft)
                    .bg(palette().surface_alt),
            ),
        Rect::new(
            prompt.x + 1,
            prompt.y,
            prompt.width.saturating_sub(2),
            prompt.height,
        ),
    );
    let runs_y = prompt.bottom().saturating_add(1);
    if runs_y >= inner.bottom() {
        return;
    }
    bold_text(
        frame,
        Rect::new(inner.x, runs_y, inner.width, 1),
        format!(
            "RUN HISTORY  {}",
            app.scheduled_runs_for_selected_task().len()
        ),
    );
    let available = inner.bottom().saturating_sub(runs_y + 1);
    let runs_height = available.saturating_sub(7).clamp(3, 7).min(available);
    let run_list = Rect::new(inner.x, runs_y + 1, inner.width, runs_height);
    regions.scroll(ScrollTarget::SchedulerRuns, run_list);
    let runs = app.scheduled_runs_for_selected_task();
    for (row, run) in runs
        .iter()
        .skip(app.scheduler.run_scroll)
        .take(usize::from(run_list.height))
        .enumerate()
    {
        let rect = Rect::new(run_list.x, run_list.y + row as u16, run_list.width, 1);
        let selected = app.scheduler.selected_run_id == Some(run.id);
        let background = if selected {
            palette().selected
        } else {
            palette().panel
        };
        fill(frame, rect, background);
        draw_text(
            frame,
            Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(12), 1),
            format!("●  Run #{}", run.id),
            Style::default()
                .fg(run_status_color(run.status))
                .bg(background),
        );
        let label = run.status.text().to_uppercase();
        let width = UnicodeWidthStr::width(label.as_str()) as u16;
        draw_text(
            frame,
            Rect::new(rect.right().saturating_sub(width + 1), rect.y, width, 1),
            label,
            Style::default()
                .fg(run_status_color(run.status))
                .bg(background)
                .add_modifier(Modifier::BOLD),
        );
        regions.target(Target::Run(run.id), rect);
    }
    let conversation_y = run_list.bottom().saturating_add(1);
    if conversation_y >= inner.bottom() {
        return;
    }
    let refresh = Rect::new(inner.right().saturating_sub(9), conversation_y, 9, 1);
    button(
        frame,
        regions,
        refresh,
        " REFRESH ",
        Target::Refresh,
        palette().cyan,
    );
    let open_width = 20.min(inner.width.saturating_sub(10));
    let open = Rect::new(
        refresh.x.saturating_sub(open_width + 1),
        conversation_y,
        open_width,
        1,
    );
    button(
        frame,
        regions,
        open,
        " OPEN CONVERSATION ",
        Target::OpenConversation,
        palette().accent,
    );
    let pane_width = 13.min(open.x.saturating_sub(inner.x + 1));
    let pane = Rect::new(
        open.x.saturating_sub(pane_width + 1),
        conversation_y,
        pane_width,
        1,
    );
    button(
        frame,
        regions,
        pane,
        " LIVE PANE ",
        Target::OpenPane,
        palette().cyan,
    );
    bold_text(
        frame,
        Rect::new(
            inner.x,
            conversation_y,
            pane.x.saturating_sub(inner.x + 1),
            1,
        ),
        "CONVERSATION",
    );
    let conversation = Rect::new(
        inner.x,
        conversation_y + 1,
        inner.width,
        inner.bottom().saturating_sub(conversation_y + 1),
    );
    let selected_run = app
        .scheduler
        .selected_run_id
        .and_then(|id| runs.iter().find(|run| run.id == id).copied());
    draw_conversation_handoff(frame, selected_run, conversation);
}

fn draw_conversation_handoff(
    frame: &mut Frame<'_>,
    run: Option<&crate::app::ScheduledRun>,
    area: Rect,
) {
    if area.is_empty() {
        return;
    }
    let Some(run) = run else {
        draw_text(
            frame,
            area,
            "Select a run to inspect its conversation.",
            Style::default().fg(palette().faint),
        );
        return;
    };
    let accent = if run.error.is_some() {
        palette().red
    } else {
        palette().cyan
    };
    fill(frame, area, palette().surface_alt);
    fill(frame, Rect::new(area.x, area.y, 1, area.height), accent);
    let body = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let (label, text) = if let Some(error) = run.error.as_deref() {
        ("RUN ERROR", error)
    } else if run.pane_id.is_some() {
        (
            "OPEN IN AGENTS",
            "View the structured OpenCode transcript, reasoning, and tool activity in the existing Agents preview.",
        )
    } else {
        (
            "NO LIVE CONVERSATION",
            "Herdr did not report an available agent pane for this run.",
        )
    };
    draw_text(
        frame,
        Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
        label,
        Style::default()
            .fg(accent)
            .bg(palette().surface_alt)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: true }).style(
            Style::default()
                .fg(palette().soft)
                .bg(palette().surface_alt),
        ),
        body,
    );
}

fn draw_composer(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    area: Rect,
    regions: &mut SchedulerRegions,
) {
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    bold_text(
        frame,
        Rect::new(inner.x, inner.y, inner.width, 1),
        "NEW SCHEDULED TASK",
    );
    let mut y = inner.y + 2;
    if !composer.prompt_expanded {
        for field in [
            ("Title", SchedulerField::Title),
            ("Description", SchedulerField::Description),
        ] {
            draw_field(frame, composer, inner, &mut y, field, regions);
        }
    }

    draw_prompt_label(frame, composer, inner, y, regions);
    let prompt_height = if composer.prompt_expanded {
        inner.bottom().saturating_sub(y + 3).min(20)
    } else {
        5
    };
    let prompt = Rect::new(inner.x, y + 1, inner.width, prompt_height);
    draw_prompt_input(frame, composer, prompt, regions);

    if !composer.prompt_expanded {
        y = prompt.bottom();
        draw_field(
            frame,
            composer,
            inner,
            &mut y,
            ("Minutes", SchedulerField::Schedule),
            regions,
        );
        draw_text(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            "DESTINATION  Scheduler only; active workspace stays unchanged.",
            Style::default().fg(palette().faint),
        );
        y += 1;
        let destination_cards = draw_destination_cards(
            frame,
            composer,
            Rect::new(inner.x, y, inner.width, 3),
            regions,
        );
        y += 4;
        if composer.destinations.is_empty() {
            draw_text(
                frame,
                Rect::new(inner.x, y, inner.width, 2),
                "No linked worktree destinations are available.",
                Style::default().fg(palette().red),
            );
        } else if !composer.destination_picker_open
            && let Some(destination) = composer.destinations.get(composer.destination)
        {
            draw_text(
                frame,
                Rect::new(inner.x, y, inner.width, 1),
                truncate_width(
                    &format!("Target: {}", destination.path.display()),
                    usize::from(inner.width),
                ),
                Style::default().fg(palette().muted),
            );
        }
        if composer.destination_picker_open {
            draw_destination_picker(
                frame,
                composer,
                destination_cards,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(1),
                ),
                regions,
            );
        }
    }
    let mut x = inner.x;
    for (label, width, target, color) in [
        (" CANCEL ", 9, Target::Cancel, palette().faint),
        (" SAVE ", 8, Target::Save, palette().accent),
    ] {
        let rect = Rect::new(x, inner.bottom().saturating_sub(1), width, 1);
        button(frame, regions, rect, label, target, color);
        x = rect.right().saturating_add(1);
    }
}

fn draw_field(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    area: Rect,
    y: &mut u16,
    field: (&'static str, SchedulerField),
    regions: &mut SchedulerRegions,
) {
    let (label, field) = field;
    draw_text(
        frame,
        Rect::new(area.x, *y, 12, 1),
        label,
        Style::default().fg(palette().faint),
    );
    let rect = Rect::new(area.x + 12, *y, area.width.saturating_sub(12), 1);
    let active = composer.field == field;
    frame.render_widget(
        Paragraph::new(text_input_lines(
            composer.input(field),
            active,
            palette().ink,
        ))
        .style(Style::default().bg(if active {
            palette().selected
        } else {
            palette().surface_alt
        })),
        rect,
    );
    regions.target(Target::Field(field), rect);
    *y += 1;
}

fn draw_destination_cards(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    area: Rect,
    regions: &mut SchedulerRegions,
) -> Rect {
    fill(frame, area, palette().canvas);
    let destination = composer.destinations.get(composer.destination);
    let mut x = area.x;
    for (card, color, empty, maximum_width) in [
        (
            SchedulerDestinationCard::Repository,
            palette().yellow,
            "repository",
            20,
        ),
        (
            SchedulerDestinationCard::Worktree,
            palette().orange,
            "worktree",
            18,
        ),
        (
            SchedulerDestinationCard::Branch,
            palette().accent,
            "branch",
            32,
        ),
    ] {
        let value = destination.map_or(empty, |destination| card.value(destination));
        let width = (UnicodeWidthStr::width(value) as u16 + 2)
            .min(maximum_width)
            .min(area.right().saturating_sub(x));
        let rect = Rect::new(x, area.y + 1, width, 1);
        let active =
            composer.field == SchedulerField::Destination && composer.destination_card == card;
        draw_header_card(frame, rect, &format!(" {value} "), color, active, true);
        regions.target(
            Target::DestinationCard(card),
            Rect::new(rect.x, rect.y.saturating_sub(1), rect.width, 3),
        );
        x = rect.right().saturating_add(2);
    }
    Rect::new(area.x, area.y, x.saturating_sub(area.x), area.height)
}

fn draw_destination_picker(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    anchor: Rect,
    bounds: Rect,
    regions: &mut SchedulerRegions,
) {
    let candidates = composer.destination_candidates();
    let card = composer.destination_card;
    let selected = composer.destinations.get(composer.destination);
    let rows = candidates
        .iter()
        .map(|index| {
            let destination = &composer.destinations[*index];
            LocationPickerRow {
                target: HitTarget::Scheduler(Target::Destination(*index)),
                label: card.value(destination).to_owned(),
                detail: match card {
                    SchedulerDestinationCard::Branch => destination.worktree.clone(),
                    _ => destination.path.display().to_string(),
                },
                current: selected
                    .is_some_and(|selected| card.value(selected) == card.value(destination)),
                stats: None,
                kind: match card {
                    SchedulerDestinationCard::Repository => LocationPickerRowKind::Location {
                        branch: Some(destination.branch.clone()),
                    },
                    SchedulerDestinationCard::Worktree => {
                        LocationPickerRowKind::Location { branch: None }
                    }
                    SchedulerDestinationCard::Branch => LocationPickerRowKind::Choice,
                },
                selected: candidates
                    .get(composer.picker.selected)
                    .copied()
                    .unwrap_or(composer.destination)
                    == *index,
                hovered: false,
                delete_target: None,
            }
        })
        .collect::<Vec<_>>();
    let (placeholder, maximum_width) = match card {
        SchedulerDestinationCard::Repository => ("Search repositories...", 80),
        SchedulerDestinationCard::Worktree => ("Search worktrees...", 58),
        SchedulerDestinationCard::Branch => ("Search branch...", 58),
    };
    let (targets, scroll) = draw_location_picker(
        frame,
        bounds,
        anchor,
        LocationPickerView {
            query: &composer.picker.query,
            placeholder,
            rows: &rows,
            visible_start: composer.picker.visible_start(),
            actions: &[],
            maximum_width,
            overlay_target: HitTarget::Scheduler(Target::DestinationPickerOverlay),
        },
    );
    regions.targets.extend(targets);
    regions.scroll(ScrollTarget::SchedulerDestinations, scroll);
}

fn draw_prompt_label(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    inner: Rect,
    y: u16,
    regions: &mut SchedulerRegions,
) {
    draw_text(
        frame,
        Rect::new(inner.x, y, inner.width.saturating_sub(24), 1),
        "Prompt",
        Style::default().fg(palette().faint),
    );
    let label = if composer.prompt_expanded {
        " COLLAPSE Ctrl+E "
    } else {
        " EXPAND Ctrl+E "
    };
    let width = UnicodeWidthStr::width(label) as u16;
    let expand = Rect::new(inner.right().saturating_sub(width), y, width, 1);
    button(
        frame,
        regions,
        expand,
        label,
        Target::PromptExpand,
        palette().cyan,
    );
}

fn draw_prompt_input(
    frame: &mut Frame<'_>,
    composer: &crate::app::ScheduledTaskComposer,
    rect: Rect,
    regions: &mut SchedulerRegions,
) {
    let width = usize::from(rect.width).max(1);
    let maximum = composer
        .prompt
        .visual_height(width)
        .saturating_sub(usize::from(rect.height));
    let scroll = composer
        .prompt_scroll
        .min(maximum)
        .min(usize::from(u16::MAX));
    let active = composer.field == SchedulerField::Prompt;
    frame.render_widget(
        Paragraph::new(text_input_lines(&composer.prompt, active, palette().ink))
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0))
            .style(Style::default().bg(if active {
                palette().selected
            } else {
                palette().surface_alt
            })),
        rect,
    );
    regions.target(Target::Field(SchedulerField::Prompt), rect);
    regions.scroll(ScrollTarget::SchedulerPrompt, rect);
}

fn button(
    frame: &mut Frame,
    regions: &mut SchedulerRegions,
    area: Rect,
    label: &'static str,
    target: SchedulerHitTarget,
    color: Color,
) {
    draw_text(
        frame,
        area,
        label,
        Style::default()
            .fg(color)
            .bg(palette().surface_alt)
            .add_modifier(Modifier::BOLD),
    );
    regions.target(target, area);
}

fn draw_text<'a>(frame: &mut Frame, area: Rect, text: impl Into<Line<'a>>, style: Style) {
    frame.render_widget(Paragraph::new(text.into()).style(style), area);
}

fn bold_text<'a>(frame: &mut Frame, area: Rect, text: impl Into<Line<'a>>) {
    draw_text(
        frame,
        area,
        text,
        Style::default()
            .fg(palette().ink)
            .add_modifier(Modifier::BOLD),
    );
}

fn run_status_color(status: ScheduledRunStatus) -> Color {
    match status {
        ScheduledRunStatus::Launching => palette().cyan,
        ScheduledRunStatus::Working => palette().accent,
        ScheduledRunStatus::Blocked => palette().yellow,
        ScheduledRunStatus::Unknown => palette().faint,
        ScheduledRunStatus::Completed => palette().green,
        ScheduledRunStatus::Failed => palette().red,
    }
}
