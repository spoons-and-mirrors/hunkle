use super::*;

#[test]
fn shutdown_joins_the_match_worker_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut explorer = Explorer::new(directory.path().to_path_buf());
    explorer.shutdown();
    explorer.shutdown();
    assert!(explorer.match_worker.is_none());
    assert!(explorer.match_wake.is_none());
    assert!(explorer.index_worker.is_none());
    assert!(explorer.index_wake.is_none());
    assert!(explorer.browse_worker.is_none());
    assert!(explorer.browse_wake.is_none());
}

fn wait_for_matches(picker: &mut Explorer) {
    for _ in 0..100 {
        picker.poll_index();
        if !picker.searching {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("Explorer search did not finish");
}

fn wait_for_browse(picker: &mut Explorer) {
    for _ in 0..100 {
        picker.poll_index();
        if !picker.loading {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("Explorer browse did not finish");
}

#[test]
fn browse_worker_keeps_only_the_latest_pending_request() {
    let pending = Arc::new(Mutex::new(None::<BrowseRequest>));
    let worker_pending = Arc::clone(&pending);
    let (wake_tx, wake_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_browse_worker(worker_pending, wake_rx, result_tx, |directory| {
            started_tx.send(directory.to_path_buf()).unwrap();
            continue_rx.recv().unwrap();
            Ok(BrowseResult {
                entries: Vec::new(),
                surroundings: Vec::new(),
                selected_surrounding: None,
            })
        });
    });

    *pending.lock().unwrap() = Some(BrowseRequest {
        generation: 1,
        directory: PathBuf::from("first"),
    });
    wake_tx.try_send(()).unwrap();
    assert_eq!(started_rx.recv().unwrap(), Path::new("first"));

    *pending.lock().unwrap() = Some(BrowseRequest {
        generation: 2,
        directory: PathBuf::from("second"),
    });
    wake_tx.try_send(()).unwrap();
    *pending.lock().unwrap() = Some(BrowseRequest {
        generation: 3,
        directory: PathBuf::from("latest"),
    });
    assert!(wake_tx.try_send(()).is_err());

    continue_tx.send(()).unwrap();
    assert_eq!(result_rx.recv().unwrap().generation, 1);
    assert_eq!(started_rx.recv().unwrap(), Path::new("latest"));
    continue_tx.send(()).unwrap();
    assert_eq!(result_rx.recv().unwrap().generation, 3);

    drop(wake_tx);
    worker.join().unwrap();
}

#[test]
fn index_worker_keeps_only_the_latest_pending_request() {
    let pending = Arc::new(Mutex::new(None::<IndexRequest>));
    let worker_pending = Arc::clone(&pending);
    let (wake_tx, wake_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_index_worker(worker_pending, wake_rx, result_tx, |roots| {
            let path = roots[0].clone();
            started_tx.send(path.clone()).unwrap();
            continue_rx.recv().unwrap();
            vec![IndexedDirectory {
                name_lower: path.to_string_lossy().into_owned(),
                depth: 1,
                is_repo: false,
                path,
            }]
        });
    });

    *pending.lock().unwrap() = Some(IndexRequest {
        generation: 1,
        roots: vec![PathBuf::from("first")],
    });
    wake_tx.try_send(()).unwrap();
    assert_eq!(started_rx.recv().unwrap(), Path::new("first"));

    *pending.lock().unwrap() = Some(IndexRequest {
        generation: 2,
        roots: vec![PathBuf::from("second")],
    });
    wake_tx.try_send(()).unwrap();
    *pending.lock().unwrap() = Some(IndexRequest {
        generation: 3,
        roots: vec![PathBuf::from("latest")],
    });
    assert!(wake_tx.try_send(()).is_err());

    continue_tx.send(()).unwrap();
    assert_eq!(result_rx.recv().unwrap().generation, 1);
    assert_eq!(started_rx.recv().unwrap(), Path::new("latest"));
    continue_tx.send(()).unwrap();
    assert_eq!(result_rx.recv().unwrap().generation, 3);

    drop(wake_tx);
    worker.join().unwrap();
}

#[test]
fn stale_browse_and_index_completions_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut explorer = Explorer::new(directory.path().to_path_buf());

    explorer.loading = true;
    let browse_generation = explorer.browse_generation;
    assert!(!explorer.apply_browse_completion(BrowseCompletion {
        generation: browse_generation.wrapping_sub(1),
        result: Err("stale browse".to_owned()),
    }));
    assert!(explorer.loading);
    assert!(explorer.error.is_none());

    let expected_entry = PickerEntry {
        label: "current".to_owned(),
        path: directory.path().join("current"),
        action: PickerAction::Navigate,
        is_repo: false,
    };
    assert!(explorer.apply_browse_completion(BrowseCompletion {
        generation: browse_generation,
        result: Ok(BrowseResult {
            entries: vec![expected_entry.clone()],
            surroundings: Vec::new(),
            selected_surrounding: None,
        }),
    }));
    assert!(!explorer.loading);
    assert_eq!(explorer.entries[0].path, expected_entry.path);

    explorer.index_loading = true;
    let index_generation = explorer.index_generation;
    assert!(!explorer.apply_index_completion(IndexCompletion {
        generation: index_generation.wrapping_sub(1),
        index: vec![IndexedDirectory {
            path: PathBuf::from("stale"),
            name_lower: "stale".to_owned(),
            depth: 1,
            is_repo: false,
        }],
    }));
    assert!(explorer.index_loading);
    assert!(explorer.directory_index.is_empty());

    assert!(explorer.apply_index_completion(IndexCompletion {
        generation: index_generation,
        index: vec![IndexedDirectory {
            path: PathBuf::from("current"),
            name_lower: "current".to_owned(),
            depth: 1,
            is_repo: false,
        }],
    }));
    assert!(!explorer.index_loading);
    assert_eq!(explorer.directory_index[0].path, Path::new("current"));
}

#[test]
fn fuzzy_repository_paths_resolve_and_complete() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let code = root.join("code");
    let hunkle = code.join("hunkle");
    let gitlab = code.join("gitlab-runner");
    fs::create_dir_all(hunkle.join(".git")).unwrap();
    fs::create_dir_all(&gitlab).unwrap();

    assert_eq!(resolve_fuzzy_path("cod/hunk", root), Some(hunkle.clone()));

    let mut picker = Explorer::new(root.to_path_buf());
    picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
    picker.begin_search(Some("hnk"));
    wait_for_matches(&mut picker);
    assert_eq!(picker.matches[0].path, hunkle);
    assert!(picker.matches[0].is_repo);
    assert!(fuzzy_text_score("hunkle", "go-genai-streamed-function-args").is_none());

    let completed = picker.matches[0].path.clone();
    picker.accept_completion();
    assert_eq!(PathBuf::from(&picker.path_input), completed);
    assert!(picker.path_input.ends_with(std::path::MAIN_SEPARATOR));
}

#[test]
fn directory_index_skips_build_trees() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("projects/hunkle")).unwrap();
    fs::create_dir_all(root.join("target/debug/deps")).unwrap();
    fs::create_dir_all(root.join("archive.git/objects/pack")).unwrap();
    fs::create_dir_all(root.join("archive.git/refs")).unwrap();
    fs::write(root.join("archive.git/HEAD"), "ref: refs/heads/main\n").unwrap();

    let index = index_directories(&[root.to_path_buf()]);
    let paths: Vec<_> = index.iter().map(|entry| &entry.path).collect();
    assert!(paths.contains(&&root.join("projects/hunkle")));
    assert!(!paths.contains(&&root.join("target")));
    assert!(paths.contains(&&root.join("archive.git")));
    assert!(!paths.contains(&&root.join("archive.git/objects")));
}

#[test]
fn includes_config_directories_in_browsing_and_global_search() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let opencode = root.join(".config/opencode");
    fs::create_dir_all(opencode.join("themes")).unwrap();
    fs::create_dir_all(root.join(".cache/ignored")).unwrap();
    fs::create_dir_all(root.join(".git/objects")).unwrap();

    let browse = load_directory_entries(root).unwrap();
    assert!(
        browse
            .entries
            .iter()
            .any(|entry| entry.path == root.join(".config"))
    );
    assert!(
        !browse
            .entries
            .iter()
            .any(|entry| entry.path == root.join(".git"))
    );

    let index = index_directories(&[root.to_path_buf()]);
    let paths: Vec<_> = index.iter().map(|entry| &entry.path).collect();
    assert!(paths.contains(&&opencode));
    assert!(!paths.contains(&&root.join(".cache")));

    let mut picker = Explorer::new(root.to_path_buf());
    picker.directory_index = Arc::new(index);
    picker.begin_search(Some("opencode"));
    wait_for_matches(&mut picker);
    assert_eq!(picker.matches[0].path, opencode);
}

#[test]
fn path_completion_adds_a_separator_and_immediately_lists_children() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let config = root.join(".config");
    let opencode = config.join("opencode");
    fs::create_dir_all(opencode.join("themes")).unwrap();
    fs::create_dir_all(config.join("other")).unwrap();

    let mut picker = Explorer::new(root.to_path_buf());
    picker.begin_search(Some(&format!("{}/.conf", root.display())));
    wait_for_matches(&mut picker);
    assert_eq!(picker.matches[0].path, config);
    assert!(
        picker
            .preview_entries
            .iter()
            .any(|entry| entry.path == opencode)
    );

    picker.accept_completion();
    wait_for_matches(&mut picker);
    assert!(
        picker
            .path_input
            .ends_with(&format!(".config{}", std::path::MAIN_SEPARATOR))
    );
    assert!(picker.matches.iter().any(|entry| entry.path == opencode));

    assert!(matches!(picker.confirm_path(), PickerCommand::None));
    assert_eq!(picker.directory, config);
}

#[test]
fn edits_paths_at_the_cursor_and_deletes_previous_segments() {
    let temp = tempfile::tempdir().unwrap();
    let mut picker = Explorer::new(temp.path().to_path_buf());
    picker.begin_search(Some("~/projects/alpha/"));

    picker.handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        true,
    );
    assert_eq!(picker.path_input, "~/projects/");
    picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT), true);
    assert_eq!(picker.path_input, "~/");

    picker.begin_search(Some("~/foo/bar"));
    picker.path_cursor = "~/foo".len();
    picker.handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        true,
    );
    assert_eq!(picker.path_input, "~/bar");

    picker.begin_search(Some("/foo bar/"));
    picker.handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        true,
    );
    assert_eq!(picker.path_input, "/");

    picker.begin_search(Some("cafe\u{301}"));
    picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
    assert_eq!(picker.path_input, "caf");

    picker.begin_search(Some("👩👩"));
    picker.path_cursor = "👩".len();
    picker.handle_key(
        KeyEvent::new(KeyCode::Char('\u{200d}'), KeyModifiers::NONE),
        true,
    );
    assert_eq!(picker.path_cursor, picker.path_input.len());
    picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
    assert!(picker.path_input.is_empty());

    picker.begin_search(Some("ac"));
    picker.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), true);
    picker.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE), true);
    assert_eq!(picker.path_input, "abc");
    picker.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), true);
    picker.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE), true);
    assert_eq!(picker.path_input, "bc");

    picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true);
    assert_eq!(picker.path_input, display_search_path(temp.path()));
    assert_eq!(picker.path_cursor, picker.path_input.len());
}

#[test]
fn invalidates_fuzzy_index_when_roaming_to_another_root() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();

    let mut picker = Explorer::new(first.clone());
    picker.directory_index = Arc::new(vec![IndexedDirectory {
        path: first.join("stale"),
        name_lower: "stale".to_owned(),
        depth: 1,
        is_repo: false,
    }]);
    picker.index_loading = true;
    let generation = picker.index_generation;

    picker.navigate(second);

    assert!(picker.directory_index.is_empty());
    assert!(!picker.index_loading);
    assert_ne!(picker.index_generation, generation);
}

#[test]
fn explicit_reload_invalidates_the_fuzzy_index_for_the_same_root() {
    let temp = tempfile::tempdir().unwrap();
    let mut picker = Explorer::new(temp.path().to_path_buf());
    picker.directory_index = Arc::new(vec![IndexedDirectory {
        path: temp.path().join("stale"),
        name_lower: "stale".to_owned(),
        depth: 1,
        is_repo: false,
    }]);
    picker.index_loading = true;
    let generation = picker.index_generation;

    picker.reload();

    assert!(picker.directory_index.is_empty());
    assert!(!picker.index_loading);
    assert_ne!(picker.index_generation, generation);
}

#[test]
fn enter_opens_the_current_directory_while_its_rows_load() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("workspace");
    fs::create_dir(&directory).unwrap();
    let mut picker = Explorer::new(temp.path().to_path_buf());
    picker.navigate(directory.clone());

    let PickerCommand::Open(opened) = picker.activate_selected(true) else {
        panic!("Enter should open the directory being browsed");
    };
    assert_eq!(opened, directory);
}

#[test]
fn semantic_targets_activate_exact_entries_and_reject_stale_rows() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    fs::create_dir(&child).unwrap();
    let mut picker = Explorer::new(temp.path().to_path_buf());
    picker.entries = vec![PickerEntry {
        label: "child/".to_owned(),
        path: child.clone(),
        action: PickerAction::Navigate,
        is_repo: false,
    }];
    picker.state.select(Some(0));
    let target = picker.entry_target(0);

    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, temp.path());
    assert_eq!(picker.state.selected(), Some(0));

    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, child);

    let generation = picker.content_generation;
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.content_generation, generation);
}

#[test]
fn single_clicks_select_and_double_clicks_traverse_repository_folders() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let child = root.join("child");
    let grandchild = child.join("grandchild");
    fs::create_dir_all(&grandchild).unwrap();
    fs::create_dir(child.join(".git")).unwrap();
    fs::write(root.join("note.txt"), "x\n").unwrap();
    let mut picker = Explorer::new(root.to_path_buf());
    wait_for_browse(&mut picker);

    let child_row = picker
        .entries
        .iter()
        .position(|entry| entry.path == child)
        .unwrap();
    let child_target = picker.entry_target(child_row);
    assert!(matches!(
        picker.activate_target(child_target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, root);
    assert_eq!(picker.state.selected(), Some(child_row));

    picker.activate_target(child_target);
    assert_eq!(picker.directory, child);
    wait_for_browse(&mut picker);

    let grandchild_row = picker
        .entries
        .iter()
        .position(|entry| entry.path == grandchild)
        .unwrap();
    let parent_row = picker
        .entries
        .iter()
        .position(|entry| entry.label == "..")
        .unwrap();
    picker.activate_target(picker.entry_target(grandchild_row));
    assert_eq!(picker.directory, child);
    let parent_target = picker.entry_target(parent_row);
    picker.activate_target(parent_target);
    assert_eq!(picker.directory, child);
    picker.activate_target(parent_target);
    assert_eq!(picker.directory, root);
    wait_for_browse(&mut picker);

    let file = root.join("note.txt");
    let file_row = picker
        .entries
        .iter()
        .position(|entry| entry.path == file)
        .unwrap();
    let file_target = picker.entry_target(file_row);
    assert!(matches!(
        picker.activate_target(file_target),
        PickerCommand::None
    ));
    let PickerCommand::OpenFile(opened) = picker.activate_target(file_target) else {
        panic!("double-clicking a file entry should open it");
    };
    assert_eq!(opened, file);
}

#[test]
fn single_click_on_a_match_previews_and_double_click_confirms() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let child = root.join("child");
    fs::create_dir_all(child.join("inside")).unwrap();
    let mut picker = Explorer::new(root.to_path_buf());
    picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
    picker.begin_search(Some("child"));
    wait_for_matches(&mut picker);

    let target = picker.match_target(0);
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, root);
    assert_eq!(picker.match_state.selected(), Some(0));
    assert!(
        picker
            .preview_entries
            .iter()
            .any(|entry| entry.path == child.join("inside"))
    );

    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, child);
}

#[test]
fn surrounding_tree_can_navigate_up_and_back_down() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let spoon = home.join("spoon");
    let code = spoon.join("code");
    fs::create_dir_all(&code).unwrap();
    let mut picker = Explorer::new(code.clone());
    wait_for_browse(&mut picker);

    let home_row = picker
        .surroundings
        .iter()
        .position(|entry| entry.path == home)
        .unwrap();
    let target = picker.surrounding_target(home_row);
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, code);
    assert_eq!(picker.surroundings_state.selected(), Some(home_row));
    assert!(picker.surroundings_focused);
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, home);
    wait_for_browse(&mut picker);

    let spoon_row = picker
        .surroundings
        .iter()
        .position(|entry| entry.path == spoon)
        .expect("the current directory's child should remain in the left tree");
    let target = picker.surrounding_target(spoon_row);
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, spoon);
    wait_for_browse(&mut picker);
    assert!(picker.surroundings.iter().any(|entry| entry.path == code));
}

#[test]
fn fuzzy_search_keeps_only_the_best_twelve_matches() {
    let mut picker = Explorer::new(PathBuf::from("/"));
    picker.directory_index = Arc::new(
        (0..30)
            .map(|index| {
                let name = if index == 29 {
                    "needle".to_owned()
                } else {
                    format!("needle-{index:02}")
                };
                IndexedDirectory {
                    path: PathBuf::from("/").join(&name),
                    name_lower: name,
                    depth: 1,
                    is_repo: false,
                }
            })
            .collect(),
    );

    picker.begin_search(Some("needle"));
    wait_for_matches(&mut picker);

    assert_eq!(picker.matches.len(), 12);
    assert_eq!(picker.matches[0].path, Path::new("/needle"));
}

#[test]
fn every_unmodified_character_starts_path_input_instead_of_a_browse_command() {
    let temp = tempfile::tempdir().unwrap();
    for character in ['h', 'j', 'k', 'l', 'p', 'q', 'r', '/', '~'] {
        let mut picker = Explorer::new(temp.path().to_path_buf());
        assert!(matches!(
            picker.handle_key(
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                false,
            ),
            PickerCommand::None
        ));
        assert!(picker.editing_path);
        assert_eq!(picker.path_input, character.to_string());
        assert_eq!(picker.directory, temp.path());
    }
}

#[test]
fn paste_starts_path_input_from_browse_mode() {
    let temp = tempfile::tempdir().unwrap();
    let mut picker = Explorer::new(temp.path().to_path_buf());

    picker.paste("~/shared");

    assert!(picker.editing_path);
    assert_eq!(picker.path_input, "~/shared");
}

#[test]
fn favorites_persist_navigate_and_toggle_from_the_active_directory() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let favorites_path = temp.path().join("explorer-favorites.json");
    let mut picker = Explorer::with_favorites(first.clone(), Some(favorites_path.clone()));

    picker.handle_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        true,
    );
    assert!(picker.naming_favorite);
    for character in "Projects".chars() {
        picker.handle_key(
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            true,
        );
    }
    picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true);
    assert!(!picker.naming_favorite);
    assert_eq!(picker.favorites.len(), 1);
    assert_eq!(picker.favorites[0].name, "Projects");
    drop(picker);

    let mut picker = Explorer::with_favorites(second.clone(), Some(favorites_path.clone()));
    assert_eq!(picker.favorites.len(), 1);
    let target = picker.favorite_target(0);
    assert!(matches!(
        picker.activate_target(target),
        PickerCommand::None
    ));
    assert_eq!(picker.directory, first);

    picker.handle_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        true,
    );
    assert!(picker.favorites.is_empty());
    drop(picker);

    let picker = Explorer::with_favorites(second, Some(favorites_path));
    assert!(picker.favorites.is_empty());
}

#[test]
fn path_completion_finds_files_and_enter_opens_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let file = root.join("auth.json");
    fs::write(&file, "{}\n").unwrap();
    fs::create_dir_all(root.join("themes")).unwrap();

    let mut picker = Explorer::new(root.to_path_buf());
    picker.begin_search(Some(&format!("{}/au", root.display())));
    wait_for_matches(&mut picker);

    assert_eq!(picker.matches[0].path, file);
    assert_eq!(picker.matches[0].action, PickerAction::OpenFile);

    picker.accept_completion();
    assert!(picker.path_input.ends_with("auth.json"));
    assert!(!picker.path_input.ends_with(std::path::MAIN_SEPARATOR));

    let PickerCommand::OpenFile(opened) = picker.confirm_path() else {
        panic!("Enter should open the completed file");
    };
    assert_eq!(opened, root.join("auth.json"));
}

#[test]
fn enter_opens_an_exact_file_in_the_current_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("auth.json"), "{}\n").unwrap();

    let mut picker = Explorer::new(root.to_path_buf());
    picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
    picker.begin_search(Some("auth.json"));
    wait_for_matches(&mut picker);

    let PickerCommand::OpenFile(opened) = picker.confirm_path() else {
        panic!("Enter should open the exact file path");
    };
    assert_eq!(opened, root.join("auth.json"));
}

#[test]
fn enter_reports_paths_that_do_not_exist() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let mut picker = Explorer::new(root.to_path_buf());
    picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
    picker.begin_search(Some("missing.json"));
    wait_for_matches(&mut picker);

    assert!(matches!(picker.confirm_path(), PickerCommand::None));
    assert!(
        picker
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Path not found: "))
    );
}

#[test]
fn browsing_a_directory_lists_everything_directories_first() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for index in 0..15 {
        fs::write(root.join(format!("file-{index:02}.txt")), "x\n").unwrap();
    }
    for index in 0..5 {
        fs::create_dir_all(root.join(format!("dir-{index:02}"))).unwrap();
    }

    let mut picker = Explorer::new(root.to_path_buf());
    picker.begin_search(Some(&format!("{}/", root.display())));
    wait_for_matches(&mut picker);

    assert_eq!(picker.matches.len(), 20);
    assert!(
        picker
            .matches
            .iter()
            .take(5)
            .all(|entry| entry.action == PickerAction::Navigate)
    );
    assert!(
        picker
            .matches
            .iter()
            .skip(5)
            .all(|entry| entry.action == PickerAction::OpenFile)
    );
    assert_eq!(picker.matches[0].path, root.join("dir-00"));
    assert_eq!(picker.matches[5].path, root.join("file-00.txt"));
}

#[test]
fn fuzzy_fragments_keep_only_the_best_twelve_completions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for index in 0..20 {
        fs::create_dir_all(root.join(format!("needle-{index:02}"))).unwrap();
    }

    let mut picker = Explorer::new(root.to_path_buf());
    picker.begin_search(Some(&format!("{}/need", root.display())));
    wait_for_matches(&mut picker);

    assert_eq!(picker.matches.len(), 12);
    assert!(
        picker
            .matches
            .iter()
            .all(|entry| entry.path.starts_with(root))
    );
}

#[test]
fn browsing_lists_directories_before_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("zeta")).unwrap();
    fs::write(root.join("auth.json"), "{}\n").unwrap();

    let browse = load_directory_entries(root).unwrap();
    let directory = browse
        .entries
        .iter()
        .position(|entry| entry.path == root.join("zeta"))
        .unwrap();
    let file = browse
        .entries
        .iter()
        .position(|entry| entry.path == root.join("auth.json"))
        .unwrap();
    assert!(directory < file);
    assert_eq!(browse.entries[file].action, PickerAction::OpenFile);
    assert_eq!(browse.entries[file].label, "auth.json");

    let mut picker = Explorer::new(root.to_path_buf());
    picker.entries = browse.entries;
    picker.state.select(Some(file));
    let PickerCommand::OpenFile(opened) = picker.activate_selected(true) else {
        panic!("activating a file entry should open it");
    };
    assert_eq!(opened, root.join("auth.json"));
}
