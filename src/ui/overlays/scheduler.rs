use super::super::header_card::draw_header_card;
use super::super::location_picker::{
    LocationPickerRow, LocationPickerRowKind, LocationPickerView, draw_location_picker,
};
use super::*;

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
    let outer = if profile.is_single() {
        centered_min(frame.area(), 96, 90, 40, 24)
    } else {
        centered_min(frame.area(), 78, 90, 112, 30)
    };
    frame.render_widget(Clear, outer);
    fill(frame, outer, palette().panel);
    fill(
        frame,
        Rect::new(outer.x, outer.y, outer.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(outer.x, outer.bottom().saturating_sub(2), outer.width, 2),
        palette().surface_alt,
    );
    bold_text(
        frame,
        Rect::new(outer.x + 2, outer.y + 1, outer.width.saturating_sub(12), 1),
        "SCHEDULER  Automated Herdr tasks",
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
        outer.height.saturating_sub(5),
    );
    if profile.is_single() {
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
        let columns = Layout::horizontal([Constraint::Length(30), Constraint::Min(42)])
            .spacing(1)
            .split(content);
        draw_tasks(frame, app, columns[0], &mut regions);
        draw_detail(frame, app, columns[1], &mut regions);
    }

    let (footer, footer_color) = app.scheduler.error.as_deref().map_or(
        (
            "Schedules run while Hunkle is open. Interval is measured in minutes.",
            palette().faint,
        ),
        |error| (error, palette().red),
    );
    draw_text(
        frame,
        Rect::new(
            outer.x + 2,
            outer.bottom().saturating_sub(1),
            outer.width.saturating_sub(4),
            1,
        ),
        truncate_width(footer, usize::from(outer.width.saturating_sub(4))),
        Style::default().fg(footer_color),
    );
    regions
}

fn draw_tasks(frame: &mut Frame<'_>, app: &App, area: Rect, regions: &mut SchedulerRegions) {
    bold_text(
        frame,
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(11), 1),
        " TASKS ",
    );
    let new = Rect::new(area.right().saturating_sub(7), area.y, 6, 1);
    button(frame, regions, new, " + NEW", Target::New, palette().accent);
    let list = Rect::new(
        area.x + 1,
        area.y + 2,
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    );
    regions.scroll(ScrollTarget::SchedulerTasks, list);
    let tasks = app.herdr.scheduled_tasks();
    if tasks.is_empty() {
        frame.render_widget(
            Paragraph::new("No scheduled tasks\n\nChoose + NEW to create one.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(palette().faint)),
            list,
        );
        return;
    }
    for (row, task) in tasks
        .iter()
        .skip(app.scheduler.task_scroll)
        .take(usize::from(list.height))
        .enumerate()
    {
        let rect = Rect::new(list.x, list.y + row as u16, list.width, 1);
        let selected = app.scheduler.selected_task_id == Some(task.id);
        let marker = if task.enabled { "●" } else { "○" };
        let label = truncate_width(
            &format!(" {marker} {}", task.title),
            usize::from(rect.width),
        );
        draw_text(
            frame,
            rect,
            label,
            Style::default()
                .fg(if task.enabled {
                    palette().ink
                } else {
                    palette().faint
                })
                .bg(if selected {
                    palette().selected
                } else {
                    palette().panel
                }),
        );
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
            Paragraph::new("Select a task to see its schedule, runs, and output.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(palette().faint)),
            Rect::new(area.x + 2, area.y + 2, area.width.saturating_sub(4), 3),
        );
        return;
    };
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    bold_text(
        frame,
        Rect::new(inner.x, inner.y, inner.width, 1),
        truncate_width(&task.title, usize::from(inner.width)),
    );
    let action_y = inner.y + 2;
    let actions = [
        (
            if task.enabled { " PAUSE " } else { " ENABLE " },
            Target::Toggle,
            palette().cyan,
        ),
        (" RUN NOW ", Target::RunNow, palette().accent),
        (" DELETE ", Target::Delete, palette().red),
    ];
    let mut x = inner.x;
    for (label, target, color) in actions {
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(x, action_y, width, 1);
        button(frame, regions, rect, label, target, color);
        x = rect.right().saturating_add(1);
    }
    let schedule = format!(
        "every {} minute{}",
        task.interval_minutes,
        if task.interval_minutes == 1 { "" } else { "s" }
    );
    let metadata = format!(
        "{}  ·  {}  ·  {}\n{}\n{}",
        if task.enabled { "Enabled" } else { "Paused" },
        schedule,
        task.repository,
        task.branch,
        task.destination.display()
    );
    frame.render_widget(
        Paragraph::new(metadata)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, action_y + 2, inner.width, 4),
    );
    let runs_y = action_y + 7;
    if runs_y >= inner.bottom() {
        return;
    }
    bold_text(
        frame,
        Rect::new(inner.x, runs_y, inner.width, 1),
        "RUN HISTORY",
    );
    let available = inner.bottom().saturating_sub(runs_y + 1);
    let runs_height = available.min(6);
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
        let color = run_status_color(run.status);
        draw_text(
            frame,
            rect,
            format!(" {:>9}", run.status.text()),
            Style::default().fg(color).bg(if selected {
                palette().selected
            } else {
                palette().panel
            }),
        );
        regions.target(Target::Run(run.id), rect);
    }
    let output_y = run_list.bottom().saturating_add(1);
    if output_y >= inner.bottom() {
        return;
    }
    let refresh = Rect::new(inner.right().saturating_sub(10), output_y, 10, 1);
    button(
        frame,
        regions,
        refresh,
        " REFRESH ",
        Target::Refresh,
        palette().cyan,
    );
    bold_text(
        frame,
        Rect::new(inner.x, output_y, inner.width.saturating_sub(11), 1),
        "OUTPUT",
    );
    let output = Rect::new(
        inner.x,
        output_y + 1,
        inner.width,
        inner.bottom().saturating_sub(output_y + 1),
    );
    regions.scroll(ScrollTarget::SchedulerOutput, output);
    let selected_run = app
        .scheduler
        .selected_run_id
        .and_then(|id| runs.iter().find(|run| run.id == id).copied());
    let text = selected_run.map_or("No run selected", |run| {
        if !run.output.is_empty() {
            run.output.as_str()
        } else if let Some(error) = run.error.as_deref() {
            error
        } else {
            "No output yet"
        }
    });
    frame.render_widget(
        Paragraph::new(text)
            .scroll((app.scheduler.output_scroll.min(u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(palette().muted)),
        output,
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
    draw_text(frame, area, label, Style::default().fg(color));
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
