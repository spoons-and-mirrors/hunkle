use super::*;
use crate::app::{WorktreePickerField, WorktreePickerStep};

#[test]
fn repository_picker_labels_linked_worktrees_by_repository() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("hunkle");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-b", "main"]);
    run_git(&root, &["config", "user.name", "Header Test"]);
    run_git(&root, &["config", "user.email", "header@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(&root, &["add", "tracked.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);
    let linked = directory.path().join("hunkle-restructure");
    run_git(
        &root,
        &["worktree", "add", "-b", "dev", linked.to_str().unwrap()],
    );

    let mut app = App::new(linked.clone());
    wait_for(&mut app, |app| {
        app.repository().is_some_and(|repository| {
            repository.details_ready
                && repository.root == linked
                && app
                    .linked_worktrees
                    .repository(repository.common_dir.as_deref().unwrap())
                    .is_some()
        })
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let repository = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap();
    click(&mut app, repository.x, repository.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(0))
        .unwrap();
    let label = (row.x..row.x.saturating_add(24).min(row.right()))
        .map(|x| terminal.backend().buffer()[(x, row.y)].symbol())
        .collect::<String>();
    let rendered = (row.x..row.right())
        .map(|x| terminal.backend().buffer()[(x, row.y)].symbol())
        .collect::<String>();
    assert!(label.contains("hunkle"));
    assert!(!label.contains("hunkle-restructure"));
    assert!(rendered.contains("hunkle-restructure"));
}

#[test]
fn worktree_picker_deletes_a_clean_linked_worktree_after_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repository");
    let linked = directory.path().join("linked");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-b", "main"]);
    run_git(&root, &["config", "user.name", "Header Test"]);
    run_git(&root, &["config", "user.email", "header@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(&root, &["add", "tracked.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);
    run_git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );

    let mut app = App::new(root.clone());
    wait_for(&mut app, |app| {
        app.repository()
            .and_then(|repository| repository.common_dir.as_deref())
            .and_then(|common_dir| app.linked_worktrees.repository(common_dir))
            .is_some_and(|repository| repository.worktrees.len() == 2)
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktrees = app
        .regions
        .hit_target_rect(HitTarget::HeaderWorktrees)
        .unwrap();
    click(&mut app, worktrees.x, worktrees.y);
    let linked_index = app
        .header_picker
        .items
        .iter()
        .position(|item| {
            matches!(item, HeaderPickerItem::Worktree { worktree, .. } if worktree.path == linked)
        })
        .unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderPickerDeleteWorktree(linked_index))
            .is_none()
    );
    let linked_row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(linked_index))
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        linked_row.x.saturating_add(1),
        linked_row.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let delete = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerDeleteWorktree(linked_index))
        .unwrap();
    assert_eq!(delete.right(), linked_row.right());
    assert_eq!(
        terminal.backend().buffer()[(delete.x + 1, delete.y)].symbol(),
        "X"
    );
    assert_eq!(
        terminal.backend().buffer()[(delete.x + 1, delete.y)].bg,
        palette().red
    );

    click(&mut app, delete.x + 1, delete.y);
    assert!(app.header_picker.deleting_worktree());
    assert_eq!(
        app.header_picker.worktree_delete.as_deref(),
        Some(linked.as_path())
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let confirmation = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerConfirmDeleteWorktree)
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(confirmation.x, confirmation.y)].bg,
        palette().red
    );
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("DELETE WORKTREE"));
    assert!(rendered.contains("Uncommitted changes prevent deletion"));

    click(&mut app, confirmation.x, confirmation.y);
    wait_for(&mut app, |app| {
        !linked.exists()
            && app
                .notice
                .as_deref()
                .is_some_and(|notice| notice.starts_with("Deleted worktree"))
    });
    wait_for(&mut app, |app| {
        app.repository()
            .and_then(|repository| repository.common_dir.as_deref())
            .and_then(|common_dir| app.linked_worktrees.repository(common_dir))
            .is_some_and(|repository| repository.worktrees.len() == 1)
    });
}

#[test]
fn repository_picker_clones_and_opens_a_repository() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    fs::create_dir(&source).unwrap();
    run_git(&source, &["init", "-b", "main"]);
    run_git(&source, &["config", "user.name", "Header Test"]);
    run_git(&source, &["config", "user.email", "header@example.com"]);
    fs::write(source.join("tracked.txt"), "initial\n").unwrap();
    run_git(&source, &["add", "tracked.txt"]);
    run_git(&source, &["commit", "-m", "initial"]);
    let destination = directory.path().join("cloned");

    let mut app = App::new(source.clone());
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let repository = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap();
    click(&mut app, repository.x, repository.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let picker = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerOverlay)
        .unwrap();
    let clone = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerClone)
        .unwrap();
    assert_eq!(clone.y, picker.y + 1);
    assert_eq!(clone.right() + 1, picker.right());
    assert_eq!(
        terminal.backend().buffer()[(clone.x, clone.y)].bg,
        palette().green
    );
    click(&mut app, clone.x, clone.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(app.header_picker.cloning_repository());
    let directory_input = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerCloneDirectory)
        .unwrap();
    let url_input = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerCloneUrl)
        .unwrap();
    assert_eq!(directory_input.y + 2, url_input.y);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app.handle_paste(destination.to_str().unwrap());
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_paste(source.to_str().unwrap());
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.header_picker.is_open());
    assert!(app.header_picker.clone_running());

    wait_for(&mut app, |app| {
        app.repository()
            .is_some_and(|repository| repository.root == destination && repository.details_ready)
    });
    assert_eq!(
        fs::read_to_string(destination.join("tracked.txt")).unwrap(),
        "initial\n"
    );
}

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
    let agent = app.regions.hit_target_rect(HitTarget::HeaderAgent).unwrap();
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
    assert_eq!(diff.right().saturating_add(1), agent.x);
    assert_eq!(
        terminal.backend().buffer()[(repository.x, repository.y)].symbol(),
        "▌"
    );
    assert_eq!(
        terminal.backend().buffer()[(repository.x, repository.y)].fg,
        super::palette().yellow
    );
    assert_eq!(
        terminal.backend().buffer()[(repository.x, repository.y)].bg,
        super::palette().surface_alt
    );
    assert_eq!(
        terminal.backend().buffer()[(repository.x + 1, repository.y)].bg,
        super::palette().surface_alt
    );
    assert_eq!(
        terminal.backend().buffer()[(worktrees.x, worktrees.y)].fg,
        super::palette().orange
    );
    assert_eq!(
        terminal.backend().buffer()[(branch.x, branch.y)].fg,
        super::palette().accent
    );
    assert_eq!(
        terminal.backend().buffer()[(diff.x, diff.y)].fg,
        super::palette().purple
    );
    assert_eq!(
        terminal.backend().buffer()[(agent.x, agent.y)].fg,
        super::palette().green
    );
    for card in [repository, worktrees, branch, diff, agent] {
        assert_eq!(
            terminal.backend().buffer()[(card.x + 1, card.y)].bg,
            super::palette().surface_alt
        );
        assert_eq!(
            terminal.backend().buffer()[(card.x + 1, card.y)].fg,
            super::palette().ink
        );
    }
    app.handle_mouse(mouse(MouseEventKind::Moved, repository.x + 1, repository.y));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(repository.x + 1, repository.y)].bg,
        super::lighter(super::palette().yellow)
    );
    app.hovered_hit_target = None;
    assert_eq!(worktrees.width, " basetree ".width() as u16);
    let worktree_text = (worktrees.x..worktrees.right())
        .map(|x| terminal.backend().buffer()[(x, worktrees.y)].symbol())
        .collect::<String>();
    assert_eq!(worktree_text, "▌basetree ");
    let branch_text = (branch.x..branch.right())
        .map(|x| terminal.backend().buffer()[(x, branch.y)].symbol())
        .collect::<String>();
    assert_eq!(branch_text, format!("▌{long_branch} "));

    click(&mut app, agent.x, agent.y);
    assert_eq!(app.header_picker.kind, None);
    assert_eq!(app.herdr_prompt.agent_destination(), Some(root));
    assert_eq!(
        app.herdr_prompt.agent_destination_branch(),
        Some(long_branch)
    );
    assert_eq!(
        app.notice.as_deref(),
        Some("Loading active Herdr tab layout")
    );
    assert!(app.herdr_prompt.cancel_pending_agent());

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
    assert_eq!(app.herdr_prompt.agent_destination(), Some(root));
    assert_eq!(
        app.herdr_prompt.agent_destination_branch(),
        Some(long_branch)
    );
    assert!(app.herdr_prompt.cancel_pending_agent());

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
    assert_eq!(picker.width, 80);
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
    assert!(repository_text.contains("feature/header"));
    assert!(repository_text.trim_end().ends_with("repository"));
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
    wait_for(&mut app, |app| {
        matches!(
            app.header_picker.items.first(),
            Some(HeaderPickerItem::Worktree { stats: Some(_), .. })
        )
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktree_row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(0))
        .unwrap();
    let worktree_text = (worktree_row.x..worktree_row.right())
        .map(|x| terminal.backend().buffer()[(x, worktree_row.y)].symbol())
        .collect::<String>();
    assert!(worktree_text.contains("linked"));
    assert!(worktree_text.contains("+0"));
    assert!(worktree_text.contains("-0"));
    assert!(worktree_text.trim_end().ends_with("linked"));
    let new_worktree = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerNewWorktree)
        .unwrap();
    assert_eq!(new_worktree.y, picker.y + 1);
    assert_eq!(new_worktree.right() + 1, picker.right());
    assert_eq!(
        terminal.backend().buffer()[(new_worktree.x, new_worktree.y)].bg,
        palette().green
    );
    let current_branch = app.repository().unwrap().branch.clone();
    click(&mut app, new_worktree.x, new_worktree.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.header_picker.creating_worktree());
    assert_eq!(app.header_picker.worktree_base.text(), current_branch);
    let name_input = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerWorktreeName)
        .unwrap();
    let base_input = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerWorktreeBase)
        .unwrap();
    assert_eq!(name_input.y + 2, base_input.y);
    app.handle_paste("feature/new-tree");
    assert_eq!(app.header_picker.worktree_name.text(), "feature/new-tree");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.header_picker.worktree_field, WorktreePickerField::Base);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(
        app.header_picker.worktree_step,
        WorktreePickerStep::Worktrees
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
        (HitTarget::HeaderAgent, true),
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
    assert_eq!(search_bottom.bg, super::palette().surface_alt);
    let list_bottom = &terminal.backend().buffer()[(picker.x, picker.bottom() - 1)];
    assert_eq!(list_bottom.symbol(), "▀");
    assert_eq!(list_bottom.fg, super::palette().surface_alt);
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
    let unselected_index = app
        .header_picker
        .items
        .iter()
        .position(|item| matches!(item, HeaderPickerItem::Branch(branch) if !branch.current))
        .unwrap();
    let unselected_row = app
        .regions
        .hit_target_rect(HitTarget::HeaderPickerItem(unselected_index))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(unselected_row.x, unselected_row.y)].bg,
        super::palette().surface_alt
    );
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
    assert_eq!(linked_text, "▌linked ");
}

#[test]
fn agent_pane_picker_preserves_tab_geometry_and_excludes_hunkle() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::opening(directory.path().to_path_buf());
    app.herdr_prompt.show_agent_pane_picker(
        directory.path().to_path_buf(),
        "feature/modal".to_owned(),
        "w0:p2".to_owned(),
        HerdrPaneLayout {
            workspace_id: "w0".to_owned(),
            x: 0,
            y: 0,
            width: 120,
            height: 40,
            panes: vec![
                HerdrPaneRect {
                    pane_id: "w0:p1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 72,
                    height: 40,
                },
                HerdrPaneRect {
                    pane_id: "w0:p2".to_owned(),
                    x: 72,
                    y: 0,
                    width: 48,
                    height: 20,
                },
                HerdrPaneRect {
                    pane_id: "w0:p3".to_owned(),
                    x: 72,
                    y: 20,
                    width: 48,
                    height: 20,
                },
            ],
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::AgentPanePickerOverlay),
        Some(ratatui::layout::Rect::new(0, 0, 100, 30))
    );
    let left = app
        .regions
        .hit_target_rect(HitTarget::AgentPane(0))
        .unwrap();
    let bottom_right = app
        .regions
        .hit_target_rect(HitTarget::AgentPane(2))
        .unwrap();
    assert!(left.width > bottom_right.width);
    assert!(left.height > bottom_right.height);
    assert_eq!(left.right(), bottom_right.x);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPane(1))
            .is_none()
    );
    let up = app
        .regions
        .hit_target_rect(HitTarget::AgentPaneSplit(0, AgentPaneDirection::Up))
        .unwrap();
    let left_edge = app
        .regions
        .hit_target_rect(HitTarget::AgentPaneSplit(0, AgentPaneDirection::Left))
        .unwrap();
    assert_eq!(up.width, left.width.saturating_sub(1));
    assert_eq!(up.height, left.height.saturating_sub(1).div_ceil(5));
    assert_eq!(left_edge.height, left.height.saturating_sub(1));
    assert_eq!(left_edge.width, left.width.saturating_sub(1).div_ceil(5));
    for index in 0..3 {
        for direction in [
            AgentPaneDirection::Up,
            AgentPaneDirection::Down,
            AgentPaneDirection::Left,
            AgentPaneDirection::Right,
        ] {
            let edge = app
                .regions
                .hit_target_rect(HitTarget::AgentPaneSplit(index, direction))
                .unwrap();
            let point = Position::new(
                edge.x.saturating_add(edge.width / 2),
                edge.y.saturating_add(edge.height / 2),
            );
            assert_eq!(
                app.regions.hit_target_at(point),
                Some(HitTarget::AgentPaneSplit(index, direction))
            );
        }
    }
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("START AGENT"));
    assert!(screen.contains("feature/modal"));
    assert!(screen.contains("HUNKLE"));
    assert!(screen.contains("CLICK INSIDE TO REPLACE"));
    assert!(!screen.contains("w0:p1"));
    assert!(!screen.contains("w0:p3"));
    assert!(!screen.contains("w0:t1"));
}
