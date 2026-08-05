use super::super::agents::draw_scheduled_history;
use super::super::header_card::draw_header_card;
use super::super::location_picker::{LocationPickerView, draw_location_picker};
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

type Target = SchedulerHitTarget;

pub(crate) struct SchedulerRegions {
    pub(crate) targets: Vec<(HitTarget, Rect)>,
    pub(crate) scrolls: Vec<(ScrollTarget, Rect)>,
    pub(crate) conversation_scroll_max: usize,
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
    app: &mut App,
    profile: LayoutProfile,
) -> SchedulerRegions {
    let conversation_open = app.scheduler.surface == SchedulerSurface::Conversation;
    let outer = scheduler_area(frame.area(), profile, conversation_open);
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
        conversation_scroll_max: 0,
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
    if conversation_open {
        draw_conversation(frame, app, content, &mut regions);
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
        let hints = if conversation_open {
            &[
                ("↑↓", "scroll"),
                ("[ ]", "messages"),
                ("PgUp/PgDn", "page"),
                ("V/Esc", "back"),
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
                ("E", "edit"),
                ("Tab", "tasks/runs"),
                ("R", "run"),
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

fn draw_conversation(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    regions: &mut SchedulerRegions,
) {
    let back = Rect::new(area.x + 1, area.y, 8, 1);
    button(
        frame,
        regions,
        back,
        " < BACK ",
        Target::CloseConversation,
        palette().cyan,
    );
    let body = Rect::new(
        area.x + 3,
        area.y + 2,
        area.width.saturating_sub(6),
        area.height.saturating_sub(2),
    );
    let Some(run) = app
        .scheduler
        .selected_run_id
        .and_then(|id| app.herdr.scheduled_runs().iter().find(|run| run.id == id))
    else {
        draw_text(
            frame,
            body,
            "Select a run to view its conversation",
            Style::default().fg(palette().faint).bg(palette().panel),
        );
        return;
    };
    let Some(session_id) = run.session_id.as_deref() else {
        let message = app
            .herdr
            .scheduled_session_error(run.id)
            .unwrap_or("Finding this run's OpenCode session…");
        draw_text(
            frame,
            body,
            message,
            Style::default().fg(palette().faint).bg(palette().panel),
        );
        return;
    };
    if let Some(error) = app.herdr.scheduled_conversation_error(session_id) {
        draw_text(
            frame,
            body,
            error,
            Style::default().fg(palette().red).bg(palette().panel),
        );
        return;
    }
    let selected_message = app.scheduler.conversation_message;
    let transcript_scroll = app.scheduler.conversation_scroll;
    let expanded_requests = app.scheduler.conversation_expanded_requests.clone();
    let transcript = app.herdr.scheduled_transcript(session_id);
    let (targets, maximum, _) = draw_scheduled_history(
        frame,
        transcript,
        &mut app.agent_transcript_presentation,
        selected_message,
        transcript_scroll,
        &expanded_requests,
        body,
    );
    regions.targets.extend(targets);
    regions.conversation_scroll_max = maximum;
    regions.scroll(ScrollTarget::SchedulerConversation, body);
}

fn scheduler_area(area: Rect, profile: LayoutProfile, conversation_open: bool) -> Rect {
    let mut outer = if conversation_open {
        centered_min(
            area,
            if profile.is_single() { 96 } else { 88 },
            94,
            if profile.is_single() { 40 } else { 88 },
            32,
        )
    } else if profile.is_single() {
        centered_min(area, 96, 92, 40, 24)
    } else {
        centered_min(area, 88, 82, 88, 28)
    };
    let height = outer.height.min(if conversation_open { 70 } else { 46 });
    let width = outer.width.min(132);
    outer.y = area.y + area.height.saturating_sub(height) / 2;
    outer.x = area.x + area.width.saturating_sub(width) / 2;
    outer.width = width;
    outer.height = height;
    outer
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
                        "{} / {}  ·  {}",
                        task.repository,
                        task.branch,
                        next_run_label(task)
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
        (" EDIT ", Target::Edit, palette().yellow),
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
            Span::styled(
                format!("  ·  {}", next_run_label(task)),
                Style::default().fg(if task.enabled {
                    palette().cyan
                } else {
                    palette().faint
                }),
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
    bold_text(
        frame,
        Rect::new(
            inner.x,
            conversation_y,
            open.x.saturating_sub(inner.x + 1),
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

fn next_run_label(task: &crate::app::ScheduledTask) -> String {
    if !task.enabled {
        return "Paused".to_owned();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let remaining = task.next_run_ms.saturating_sub(now).max(0) as u64;
    if remaining == 0 {
        return "Due now".to_owned();
    }
    let seconds = remaining.saturating_add(999) / 1_000;
    let hours = seconds / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("Next in {hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("Next in {minutes}m {seconds:02}s")
    } else {
        format!("Next in {seconds}s")
    }
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
            "DURABLE CONVERSATION",
            "View the structured OpenCode transcript, reasoning, and tool activity even after its Herdr pane is closed.",
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
        if composer.task_id.is_some() {
            "EDIT SCHEDULED TASK"
        } else {
            "NEW SCHEDULED TASK"
        },
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
                "No repository branches are available.",
                Style::default().fg(palette().red),
            );
        } else if !composer.destination_picker_open()
            && let Some(destination) = composer.destinations.get(composer.destination)
        {
            draw_text(
                frame,
                Rect::new(inner.x, y, inner.width, 1),
                truncate_width(
                    &destination.path.as_ref().map_or_else(
                        || "A linked worktree will be created when this task is saved".to_owned(),
                        |path| format!("Target: {}", path.display()),
                    ),
                    usize::from(inner.width),
                ),
                Style::default().fg(palette().muted),
            );
        }
        if composer.destination_picker_open() {
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
    let card = composer.destination_card;
    let selected = composer.destinations.get(composer.destination);
    let rows = composer
        .destination_picker
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let destination = composer.destination_index_for_item(item)?;
            location_picker_row(
                item,
                HitTarget::Scheduler(Target::Destination(destination)),
                selected.and_then(|destination| destination.path.as_deref()),
                composer.destination_picker.selected == index,
                false,
                None,
            )
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
            query: &composer.destination_picker.query,
            placeholder,
            rows: &rows,
            visible_start: composer.destination_picker.visible_start(),
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
