use super::*;

#[test]
fn header_cards_open_pickers_and_checkout_branches() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repository");
    fs::create_dir(&root).unwrap();
    let root = root.as_path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Header Test"]);
    run_git(root, &["config", "user.email", "header@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "initial"]);
    run_git(root, &["branch", "topic"]);
    run_git(root, &["branch", "linked"]);
    let linked = directory.path().join("linked");
    run_git(
        root,
        &["worktree", "add", linked.to_str().unwrap(), "linked"],
    );
    let long_branch = "feature/header-branch-name-is-never-truncated";
    run_git(root, &["branch", "-m", long_branch]);
    fs::write(root.join("feature.txt"), "feature branch\n").unwrap();
    run_git(root, &["add", "feature.txt"]);
    run_git(root, &["commit", "-m", "feature change"]);

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let repository = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap();
    let worktrees = app
        .regions
        .hit_target_rect(HitTarget::HeaderWorktrees)
        .unwrap();
    let branch = app
        .regions
        .hit_target_rect(HitTarget::HeaderBranch)
        .unwrap();
    let diff = app.regions.hit_target_rect(HitTarget::HeaderDiff).unwrap();
    assert_eq!(repository.x, 1);
    assert_eq!(terminal.backend().buffer()[(0, 1)].symbol(), "▀");
    assert_eq!(
        terminal.backend().buffer()[(0, 1)].fg,
        super::palette().surface_alt
    );
    assert_eq!(
        terminal.backend().buffer()[(0, 1)].bg,
        super::palette().panel
    );
    assert_eq!(repository.right().saturating_add(1), worktrees.x);
    assert!(worktrees.right() <= branch.x);
    assert_eq!(branch.right().saturating_add(1), diff.x);
    assert_eq!(
        terminal.backend().buffer()[(repository.x, repository.y)].bg,
        super::palette().yellow
    );
    assert_eq!(
        terminal.backend().buffer()[(repository.x, repository.y)].fg,
        super::palette().canvas
    );
    assert_eq!(
        terminal.backend().buffer()[(worktrees.x, worktrees.y)].bg,
        super::palette().orange
    );
    assert_eq!(
        terminal.backend().buffer()[(branch.x, branch.y)].bg,
        super::palette().accent
    );
    assert_eq!(
        terminal.backend().buffer()[(diff.x, diff.y)].bg,
        super::palette().purple
    );
    assert_eq!(worktrees.width, " worktree ".width() as u16);
    let worktree_text = (worktrees.x..worktrees.right())
        .map(|x| terminal.backend().buffer()[(x, worktrees.y)].symbol())
        .collect::<String>();
    assert_eq!(worktree_text, " worktree ");
    let branch_text = (branch.x..branch.right())
        .map(|x| terminal.backend().buffer()[(x, branch.y)].symbol())
        .collect::<String>();
    assert_eq!(branch_text, format!(" {long_branch} "));

    click(&mut app, diff.x, diff.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.header_picker.kind, Some(HeaderPickerKind::DiffTargets));
    for character in "topic".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.header_picker.items.len(), 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    assert_eq!((picker.x, picker.y), (repository.x, repository.bottom()));
    let topic_index = app
        .header_picker
        .items
        .iter()
        .position(
            |item| matches!(item, HeaderPickerItem::DiffTarget(branch) if branch.name == "topic"),
        )
        .unwrap();
    let topic_target = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(topic_index))
        .unwrap();
    click(&mut app, topic_target.x, topic_target.y);
    wait_for(&mut app, |app| {
        app.changes
            .branch_comparison()
            .is_some_and(|comparison| comparison.target == "topic")
            && app.changes.diff.contains("feature.txt")
    });
    assert_eq!(app.visible_view(), View::Changes);
    assert_eq!(app.repository().unwrap().branch, long_branch);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("topic...feature/header"));
    let diff = app.regions.diff.unwrap();
    let diff_header = (diff.x..diff.right())
        .map(|x| terminal.backend().buffer()[(x, diff.y + 1)].symbol())
        .collect::<String>();
    assert!(!diff_header.contains("topic..."));

    click(&mut app, repository.x, repository.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.header_picker.kind, Some(HeaderPickerKind::Repositories));
    for character in "repository".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.header_picker.items.len(), 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    assert_eq!((picker.x, picker.y), (repository.x, repository.bottom()));
    let repository_row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(0))
        .unwrap();
    let repository_text = (repository_row.x..repository_row.right())
        .map(|x| terminal.backend().buffer()[(x, repository_row.y)].symbol())
        .collect::<String>();
    assert!(repository_text.contains("repository"));
    assert!(repository_text.contains("+0"));
    assert!(repository_text.contains("-0"));
    assert!(
        repository_text
            .trim_end()
            .ends_with(&root.display().to_string())
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktrees = app
        .regions
        .hit_target_rect(HitTarget::HeaderWorktrees)
        .unwrap();
    click(&mut app, worktrees.x, worktrees.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.header_picker.kind, Some(HeaderPickerKind::Worktrees));
    assert_eq!(app.header_picker.items.len(), 2);
    for character in "linked".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.header_picker.items.len(), 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    assert_eq!((picker.x, picker.y), (repository.x, repository.bottom()));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderPickerItem(0))
            .is_some()
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let branch = app
        .regions
        .hit_target_rect(HitTarget::HeaderBranch)
        .unwrap();
    click(&mut app, branch.x, branch.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.header_picker.kind, Some(HeaderPickerKind::Branches));
    for character in "topic".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.header_picker.items.len(), 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    assert_eq!((picker.x, picker.y), (repository.x, repository.bottom()));
    let topic_index = app
        .header_picker
        .items
        .iter()
        .position(|item| matches!(item, HeaderPickerItem::Branch(branch) if branch.name == "topic"))
        .unwrap();
    let topic = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(topic_index))
        .unwrap();
    click(&mut app, topic.x, topic.y);
    assert_eq!(app.mode, Mode::Normal);

    wait_for(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.details_ready && repo.branch == "topic")
    });
    assert!(app.changes.branch_comparison().is_none());
    assert_eq!(
        String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["branch", "--show-current"])
                .output()
                .unwrap()
                .stdout
        )
        .trim(),
        "topic"
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let branch = app
        .regions
        .hit_target_rect(HitTarget::HeaderBranch)
        .unwrap();
    click(&mut app, branch.x, branch.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    let picker_search = (picker.x..picker.right())
        .map(|x| terminal.backend().buffer()[(x, picker.y + 1)].symbol())
        .collect::<String>();
    assert!(picker_search.contains("▌Search branch..."));
    let footer_y = terminal.backend().buffer().area.height - 1;
    let background = &terminal.backend().buffer()[(0, footer_y)];
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(background.modifier.contains(Modifier::DIM));
    for (target, undimmed) in [
        (HitTarget::HeaderRepository, true),
        (HitTarget::HeaderWorktrees, true),
        (HitTarget::HeaderBranch, true),
        (HitTarget::HeaderDiff, true),
    ] {
        let control = app.regions.hit_target_rect(target).unwrap();
        assert_eq!(
            !terminal.backend().buffer()[(control.x, control.y)]
                .modifier
                .contains(Modifier::DIM),
            undimmed
        );
    }
    let search_top = &terminal.backend().buffer()[(picker.x, picker.y)];
    assert_eq!(search_top.symbol(), "▄");
    assert_eq!(search_top.fg, super::palette().surface_alt);
    assert_eq!(search_top.bg, Color::Rgb(0, 0, 0));
    let search_bottom = &terminal.backend().buffer()[(picker.x, picker.y + 2)];
    assert_eq!(search_bottom.symbol(), "▀");
    assert_eq!(search_bottom.fg, super::palette().surface_alt);
    assert_eq!(search_bottom.bg, super::palette().raised);
    let list_bottom = &terminal.backend().buffer()[(picker.x, picker.bottom() - 1)];
    assert_eq!(list_bottom.symbol(), "▀");
    assert_eq!(list_bottom.fg, super::palette().raised);
    assert_eq!(list_bottom.bg, Color::Rgb(0, 0, 0));
    let current_index = app
        .header_picker
        .items
        .iter()
        .position(|item| matches!(item, HeaderPickerItem::Branch(branch) if branch.current))
        .unwrap();
    let current_row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(current_index))
        .unwrap();
    let current_text = (current_row.x..current_row.right())
        .map(|x| terminal.backend().buffer()[(x, current_row.y)].symbol())
        .collect::<String>();
    assert!(current_text.trim_end().ends_with("current"));
    let new_branch = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerNewBranch)
        .unwrap();
    assert_eq!(new_branch.right() + 1, picker.right());
    assert_eq!(
        terminal.backend().buffer()[(new_branch.x, new_branch.y)].bg,
        super::palette().green
    );
    assert_eq!(new_branch.height, 1);
    click(&mut app, new_branch.x, new_branch.y);
    assert_eq!(app.header_picker.branch_step, super::BranchPickerStep::Base);
    for character in "header-branch".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.header_picker.items.len(), 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let base_index = app
        .header_picker
        .items
        .iter()
        .position(|item| {
            matches!(item, HeaderPickerItem::BranchBase(branch) if branch.name == long_branch)
        })
        .unwrap();
    let base = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(base_index))
        .unwrap();
    click(&mut app, base.x, base.y);
    assert!(app.header_picker.naming_branch());
    let created_branch = "feature/new-from-header";
    for character in created_branch.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.details_ready && repo.branch == created_branch)
    });
    let created_head = run_git_output(root, &["rev-parse", created_branch]);
    let base_head = run_git_output(root, &["rev-parse", long_branch]);
    assert_eq!(created_head, base_head);

    drop(app);
    let mut linked_app = App::new(linked.clone());
    wait_for(&mut linked_app, |app| {
        app.linked_worktrees.worktree_name(&linked).is_some()
    });
    terminal.draw(|frame| draw(frame, &mut linked_app)).unwrap();
    let linked_card = linked_app
        .regions
        .hit_target_rect(HitTarget::HeaderWorktrees)
        .unwrap();
    let linked_text = (linked_card.x..linked_card.right())
        .map(|x| terminal.backend().buffer()[(x, linked_card.y)].symbol())
        .collect::<String>();
    assert_eq!(linked_text, " linked ");
}
