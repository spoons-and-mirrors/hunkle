use super::*;
use crate::app::{SchedulerDestination, SchedulerDestinationCard, SchedulerField};

fn destination(path: &str, repository: &str, branch: &str, worktree: &str) -> SchedulerDestination {
    SchedulerDestination {
        path: path.into(),
        repository: repository.to_owned(),
        branch: branch.to_owned(),
        worktree: worktree.to_owned(),
    }
}

fn scheduler_rect(app: &App, target: SchedulerHitTarget) -> Option<Rect> {
    app.regions.hit_target_rect(HitTarget::Scheduler(target))
}

#[test]
fn schedule_header_control_is_herdr_gated() {
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
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderSchedule)
            .is_some()
    );
    assert!(screen_text(&terminal).contains("SCHEDULE F4"));
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
    app.begin_scheduled_task();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let text = screen_text(&terminal);
    assert!(text.contains("NEW SCHEDULED TASK"));
    assert!(text.contains("Schedules run while Hunkle is open"));
    assert!(text.contains("Minutes"));
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
fn scheduler_destination_reuses_repository_worktree_and_branch_cards() {
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
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        scheduler_rect(
            &app,
            SchedulerHitTarget::DestinationCard(SchedulerDestinationCard::Repository)
        )
        .is_some()
    );
    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Repository,
    ));
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
            .destination_picker_open
    );

    for (card, destination) in [
        (SchedulerDestinationCard::Repository, 0),
        (SchedulerDestinationCard::Worktree, 1),
    ] {
        app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(card));
        app.activate_scheduler_target(SchedulerHitTarget::Destination(destination));
    }
    assert_eq!(app.scheduler.composer.as_ref().unwrap().destination, 1);
    app.activate_scheduler_target(SchedulerHitTarget::DestinationCard(
        SchedulerDestinationCard::Branch,
    ));
    app.activate_scheduler_target(SchedulerHitTarget::Destination(0));
    assert_eq!(app.scheduler.composer.as_ref().unwrap().destination, 0);
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
        screen_text(&terminal).contains("Select a task to see its schedule, runs, and output.")
    );
}
