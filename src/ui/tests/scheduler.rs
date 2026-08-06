use super::*;
use crate::app::{SchedulerDestination, SchedulerDestinationCard, SchedulerField};
use std::path::{Path, PathBuf};

fn destination(path: &str, repository: &str, branch: &str, worktree: &str) -> SchedulerDestination {
    SchedulerDestination {
        path: Some(path.into()),
        repository_root: PathBuf::from(format!("/tmp/{repository}")),
        repository: repository.to_owned(),
        branch: crate::git::Branch {
            name: branch.to_owned(),
            upstream: None,
            remote: false,
            current: branch == "main",
            default: branch == "main",
            last_touched_at: None,
        },
        checkout_branch: branch.to_owned(),
        worktree: Some(worktree.to_owned()),
    }
}

fn scheduler_rect(app: &App, target: SchedulerHitTarget) -> Option<Rect> {
    app.regions.hit_target_rect(HitTarget::Scheduler(target))
}

#[test]
fn schedule_footer_control_is_herdr_gated() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderSchedule)
            .is_none()
    );
    enable_herdr(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let schedule = app
        .regions
        .hit_target_rect(HitTarget::HeaderSchedule)
        .unwrap();
    let text = screen_text(&terminal);
    assert_eq!(schedule.y, 23);
    assert!(!text.contains("SCHEDULE F4"));
    assert!(text.contains("F4 Schedule"));
    click(&mut app, schedule.x, schedule.y);
    assert_eq!(app.mode, Mode::Scheduler);
}

#[test]
fn narrow_scheduler_uses_semantic_new_target_and_shared_composer() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    let mut terminal = Terminal::new(TestBackend::new(50, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(scheduler_rect(&app, SchedulerHitTarget::New).is_some());
    assert!(screen_text(&terminal).contains("Runs while Hunkle is open"));
    app.begin_scheduled_task();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let text = screen_text(&terminal);
    assert!(text.contains("NEW SCHEDULED TASK"));
    assert!(text.contains("Minutes"));
    assert!(text.contains("Model"));
}

#[test]
fn scheduler_prompt_is_a_five_row_expandable_editor() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    app.begin_scheduled_task();
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        scheduler_rect(&app, SchedulerHitTarget::Field(SchedulerField::Prompt))
            .unwrap()
            .height,
        5
    );
    app.activate_scheduler_target(SchedulerHitTarget::PromptExpand);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        scheduler_rect(&app, SchedulerHitTarget::Field(SchedulerField::Prompt))
            .unwrap()
            .height,
        20
    );
    assert!(screen_text(&terminal).contains("COLLAPSE Ctrl+E"));
}

#[test]
fn scheduler_destination_reuses_repository_and_full_branch_cards() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    app.begin_scheduled_task();
    app.scheduler.composer.as_mut().unwrap().destinations = vec![
        destination("/tmp/alpha-main", "alpha", "main", "basetree"),
        destination("/tmp/alpha-feature", "alpha", "feature", "feature"),
        destination("/tmp/beta-main", "beta", "main", "basetree"),
    ];
    app.linked_worktrees
        .set_change_stats_for_test(PathBuf::from("/tmp/alpha-main"), (11, 3));
    app.linked_worktrees
        .set_change_stats_for_test(PathBuf::from("/tmp/beta-main"), (22, 4));
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        scheduler_rect(
            &app,
            SchedulerHitTarget::DestinationCard(SchedulerDestinationCard::Repository)
        )
        .is_some()
    );
    assert!(
        scheduler_rect(
            &app,
            SchedulerHitTarget::DestinationCard(SchedulerDestinationCard::Worktree)
        )
        .is_some()
    );
    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Repository,
    ));
    app.poll_worker();
    assert!(
        app.scheduler
            .composer
            .as_ref()
            .unwrap()
            .destination_picker
            .items
            .iter()
            .any(|item| matches!(
                item,
                HeaderPickerItem::Repository { path, stats: Some((22, 4)), .. }
                    if path == Path::new("/tmp/beta-main")
            ))
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("Search repositories..."));
    app.paste_scheduler("beta");
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(scheduler_rect(&app, SchedulerHitTarget::Destination(0)).is_none());
    assert!(scheduler_rect(&app, SchedulerHitTarget::Destination(2)).is_some());
    app.activate_scheduler_target(SchedulerHitTarget::Destination(2));
    assert_eq!(app.scheduler.composer.as_ref().unwrap().destination, 2);
    assert!(
        !app.scheduler
            .composer
            .as_ref()
            .unwrap()
            .destination_picker_open()
    );

    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Repository,
    ));
    app.activate_scheduler_target(SchedulerHitTarget::Destination(0));
    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Branch,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("Search branch..."));
    assert!(screen_text(&terminal).contains("feature"));

    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Worktree,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("Search worktrees..."));
    assert!(screen_text(&terminal).contains("alpha-main"));
    assert!(screen_text(&terminal).contains("alpha-feature"));
}

#[test]
fn wide_scheduler_reserves_a_full_master_detail_surface() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(
        screen_text(&terminal).contains("Select a task to review its schedule and run history.")
    );
}
