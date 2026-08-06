use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use super::*;

fn linked(path: &str, is_main: bool) -> LinkedWorktree {
    LinkedWorktree {
        path: PathBuf::from(path),
        head: Some("1234567890abcdef".to_owned()),
        branch: Some("refs/heads/feature".to_owned()),
        is_main,
        is_detached: false,
        is_bare: false,
        locked: false,
        locked_reason: None,
        prunable: false,
        prunable_reason: None,
    }
}

fn repository() -> LinkedWorktreeRepository {
    LinkedWorktreeRepository {
        common_dir: PathBuf::from("/repo/.git"),
        label: "repo".to_owned(),
        worktrees: vec![linked("/repo", true), linked("/repo-feature", false)],
        branches: Vec::new(),
        branch_error: None,
        error: None,
    }
}

fn wait_for_stats(catalog: &mut LinkedWorktreeCatalog) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while (catalog.active_stats_generation.is_some() || !catalog.pending_stats.is_empty())
        && Instant::now() < deadline
    {
        catalog.poll();
        thread::sleep(Duration::from_millis(5));
    }
    catalog.poll();
    assert!(catalog.active_stats_generation.is_none());
    assert!(catalog.pending_stats.is_empty());
}

fn wait_for_persistence(catalog: &mut LinkedWorktreeCatalog) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while (catalog.active_persistence_generation.is_some() || catalog.pending_persistence.is_some())
        && Instant::now() < deadline
    {
        catalog.poll();
        thread::sleep(Duration::from_millis(5));
    }
    catalog.poll();
    assert!(catalog.active_persistence_generation.is_none());
    assert!(catalog.pending_persistence.is_none());
}

fn persist_store(catalog: &LinkedWorktreeCatalog) {
    if let Some(request) = catalog.store.persistence_request().unwrap() {
        known_repositories::persist(request).unwrap();
    }
}

#[test]
fn resolves_agent_destination_metadata_from_its_worktree() {
    let snapshot = LinkedWorktreeCatalogSnapshot::for_test(vec![repository()]);

    let basetree = snapshot.agent_destination(Path::new("/repo")).unwrap();
    assert_eq!(basetree.repository(), "repo");
    assert_eq!(basetree.branch(), "feature");

    let linked = snapshot
        .agent_destination(Path::new("/repo-feature"))
        .unwrap();
    assert_eq!(linked.repository(), "repo");
    assert_eq!(linked.branch(), "feature");
}

#[test]
fn defers_branch_discovery_until_scheduler_requests_it() {
    let mut catalog = LinkedWorktreeCatalog::new(None);

    assert!(!catalog.refresh_request().key.load_branches);
    catalog.request_branches();

    assert!(catalog.branches_requested);
    assert!(
        catalog
            .active_refresh
            .as_ref()
            .is_some_and(|refresh| refresh.key.load_branches)
    );
}

#[test]
fn inventory_cache_reuses_only_fresh_matching_topology() {
    let cached = CachedRepository {
        topology_epoch: 3,
        branches_loaded: false,
        checked_at: Instant::now(),
        repository: repository(),
    };
    let known_worktree = PathBuf::from("/repo-feature");

    assert!(cached_repository_is_reusable(
        &cached,
        3,
        false,
        Some(std::slice::from_ref(&known_worktree)),
    ));
    assert!(!cached_repository_is_reusable(&cached, 4, false, None,));
    assert!(!cached_repository_is_reusable(
        &cached,
        3,
        false,
        Some(&[PathBuf::from("/new-worktree")]),
    ));

    let stale = CachedRepository {
        checked_at: Instant::now() - INVENTORY_CACHE_TTL - Duration::from_millis(1),
        ..cached
    };
    assert!(!cached_repository_is_reusable(&stale, 3, false, None));
    assert!(cached_repository_is_reusable(&stale, 3, true, None));
}

#[test]
fn repeated_presented_card_requests_stay_inside_the_backoff_window() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader_calls = Arc::clone(&calls);
    let loader: TestStatsLoader = Arc::new(move |_root, _previous| {
        loader_calls.fetch_add(1, Ordering::SeqCst);
        Ok((WorktreeSignature::for_test(1, 1), Some((12, 4))))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/agent-destination");

    catalog.request_stats([root.clone()]);
    wait_for_stats(&mut catalog);
    for _ in 0..10 {
        catalog.request_stats([root.clone()]);
    }
    catalog.poll();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.change_stats(&root), Some((12, 4)));
}

#[test]
fn duplicate_in_flight_roots_coalesce_to_one_load() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader_calls = Arc::clone(&calls);
    let (started_sender, started_receiver) = mpsc::channel();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let loader_gate = Arc::clone(&gate);
    let loader: TestStatsLoader = Arc::new(move |_root, _previous| {
        loader_calls.fetch_add(1, Ordering::SeqCst);
        started_sender.send(()).unwrap();
        let (lock, wake) = &*loader_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Ok((WorktreeSignature::for_test(1, 1), Some((7, 3))))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/shared-destination");

    catalog.request_stats([root.clone(), root.clone()]);
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    catalog.request_stats([root.clone(), root.clone()]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let (lock, wake) = &*gate;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    wait_for_stats(&mut catalog);
    assert_eq!(catalog.change_stats(&root), Some((7, 3)));
}

#[test]
fn active_repository_observation_avoids_a_git_load() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader_calls = Arc::clone(&calls);
    let loader: TestStatsLoader = Arc::new(move |_root, _previous| {
        loader_calls.fetch_add(1, Ordering::SeqCst);
        Ok((WorktreeSignature::for_test(1, 1), Some((1, 1))))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/active-repository");

    catalog.observe_active_repository(Some((
        root.clone(),
        (31, 9),
        WorktreeSignature::for_test(3, 1),
    )));
    catalog.request_stats([root.clone()]);
    catalog.poll();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.change_stats(&root), Some((31, 9)));
    assert!(catalog.active_repository_stats_are_current(&root, WorktreeSignature::for_test(3, 1)));
}

#[test]
fn changed_root_refreshes_after_the_minimum_recheck() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader_calls = Arc::clone(&calls);
    let loader: TestStatsLoader = Arc::new(move |_root, _previous| {
        let call = loader_calls.fetch_add(1, Ordering::SeqCst);
        Ok((
            WorktreeSignature::for_test(call as u64 + 1, 1),
            Some(if call == 0 { (2, 1) } else { (8, 5) }),
        ))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/changed-repository");

    catalog.request_stats([root.clone()]);
    wait_for_stats(&mut catalog);
    catalog.request_stats([root.clone()]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    catalog.request_stats_at([root.clone()], Instant::now() + MIN_STATS_INTERVAL);
    wait_for_stats(&mut catalog);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(catalog.change_stats(&root), Some((8, 5)));
}

#[test]
fn unchanged_status_probes_do_not_repeat_numstat_hydration() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hydrated = Arc::new(AtomicUsize::new(0));
    let loader_calls = Arc::clone(&calls);
    let loader_hydrated = Arc::clone(&hydrated);
    let signature = WorktreeSignature::for_test(4, 1);
    let loader: TestStatsLoader = Arc::new(move |_root, previous| {
        loader_calls.fetch_add(1, Ordering::SeqCst);
        if previous == Some(signature) {
            Ok((signature, None))
        } else {
            loader_hydrated.fetch_add(1, Ordering::SeqCst);
            Ok((signature, Some((20, 6))))
        }
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/unchanged-repository");

    catalog.request_stats_at([root.clone()], Instant::now());
    wait_for_stats(&mut catalog);
    catalog.request_stats_at([root.clone()], Instant::now() + MIN_STATS_INTERVAL);
    wait_for_stats(&mut catalog);

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(hydrated.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.change_stats(&root), Some((20, 6)));
    assert_eq!(
        catalog.stats.get(&root).unwrap().recheck_interval,
        MIN_STATS_INTERVAL * 2
    );
}

#[test]
fn active_observation_rejects_an_older_in_flight_completion() {
    let (started_sender, started_receiver) = mpsc::channel();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let loader_gate = Arc::clone(&gate);
    let loader: TestStatsLoader = Arc::new(move |_root, _previous| {
        started_sender.send(()).unwrap();
        let (lock, wake) = &*loader_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Ok((WorktreeSignature::for_test(1, 1), Some((1, 1))))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from("/active-overrides-flight");

    catalog.request_stats([root.clone()]);
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    catalog.observe_active_repository(Some((
        root.clone(),
        (55, 13),
        WorktreeSignature::for_test(2, 1),
    )));
    let (lock, wake) = &*gate;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    wait_for_stats(&mut catalog);

    assert_eq!(catalog.change_stats(&root), Some((55, 13)));
}

#[test]
fn repository_picker_items_use_live_catalog_counts() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    let root = PathBuf::from("/picker-repository");
    catalog.store.recent = vec![known_repositories::RecentRepository {
        common_dir: Some(PathBuf::from("/picker-repository/.git")),
        root: root.clone(),
        stats: Some((1, 1)),
    }];

    catalog.set_change_stats_for_test(root, (90, 12));

    assert_eq!(
        catalog.recent_repository_picker_items()[0].stats,
        Some((90, 12))
    );
}

#[cfg(unix)]
#[test]
fn stats_cache_preserves_non_utf8_root_identity() {
    use std::os::unix::ffi::OsStrExt;

    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let loader_observed = Arc::clone(&observed);
    let loader: TestStatsLoader = Arc::new(move |root, _previous| {
        loader_observed.lock().unwrap().push(root.to_path_buf());
        Ok((WorktreeSignature::for_test(1, 1), Some((4, 2))))
    });
    let mut catalog = LinkedWorktreeCatalog::new_with_stats_loader(None, loader);
    let root = PathBuf::from(std::ffi::OsString::from_vec(b"/repository/\xff".to_vec()));

    catalog.request_stats([root.clone()]);
    wait_for_stats(&mut catalog);

    assert_eq!(
        observed.lock().unwrap().as_slice(),
        std::slice::from_ref(&root)
    );
    assert_eq!(catalog.change_stats(&root), Some((4, 2)));
    assert_eq!(
        observed.lock().unwrap()[0].as_os_str().as_bytes(),
        root.as_os_str().as_bytes()
    );
}

#[test]
fn ignores_stale_inventory_completions() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.generation = 2;
    catalog.snapshot.repositories = vec![repository()];
    catalog
        .sender
        .send(InventoryCompletion {
            generation: 1,
            branches_loaded: false,
            topology_epoch: 0,
            repositories: Vec::new(),
            discovered: Vec::new(),
            pruned: Vec::new(),
            relevant: Vec::new(),
        })
        .unwrap();

    assert!(!catalog.poll().changed);
    assert_eq!(catalog.snapshot.repositories.len(), 1);
}

#[test]
fn repeated_identical_refreshes_share_one_generation() {
    let mut catalog = LinkedWorktreeCatalog::new(None);

    catalog.refresh();
    catalog.refresh();
    catalog.refresh();

    assert_eq!(catalog.generation, 1);
    assert!(catalog.active_refresh.is_some());
    assert!(catalog.pending_refresh.is_none());
}

#[test]
fn changed_in_flight_refreshes_coalesce_to_one_follow_up() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.refresh();

    catalog.observe_herdr(LinkedWorktreeObservation {
        candidates: vec![LinkedWorktreeCandidate {
            path: PathBuf::from("/first-change"),
        }],
    });
    catalog.refresh();
    catalog.observe_herdr(LinkedWorktreeObservation {
        candidates: vec![LinkedWorktreeCandidate {
            path: PathBuf::from("/latest-change"),
        }],
    });
    catalog.refresh();
    assert_eq!(catalog.generation, 1);

    catalog
        .sender
        .send(InventoryCompletion {
            generation: 1,
            branches_loaded: false,
            topology_epoch: 0,
            repositories: Vec::new(),
            discovered: Vec::new(),
            pruned: Vec::new(),
            relevant: Vec::new(),
        })
        .unwrap();
    catalog.poll();

    assert_eq!(catalog.generation, 2);
    assert!(catalog.pending_refresh.is_none());
    assert_eq!(
        catalog
            .active_refresh
            .as_ref()
            .unwrap()
            .key
            .candidates
            .as_slice(),
        [LinkedWorktreeCandidate {
            path: PathBuf::from("/latest-change"),
        }]
    );
}

#[test]
fn reverted_intent_still_follows_a_superseded_flight() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    let active_key = catalog.refresh_request().key;
    catalog.generation = 1;
    catalog.snapshot.loading = true;
    catalog.active_refresh = Some(CatalogRefreshFlight {
        key: active_key.clone(),
        generation: 1,
        inventory_pending: true,
        stats_pending: false,
    });
    catalog.observe_herdr(LinkedWorktreeObservation {
        candidates: vec![LinkedWorktreeCandidate {
            path: PathBuf::from("/transient-change"),
        }],
    });
    catalog.refresh();
    catalog.observe_herdr(LinkedWorktreeObservation {
        candidates: Vec::new(),
    });
    catalog.refresh();
    assert!(catalog.pending_refresh.as_ref().unwrap().key == active_key);

    catalog
        .sender
        .send(InventoryCompletion {
            generation: 1,
            branches_loaded: false,
            topology_epoch: 0,
            repositories: Vec::new(),
            discovered: Vec::new(),
            pruned: Vec::new(),
            relevant: Vec::new(),
        })
        .unwrap();
    catalog.poll();

    assert_eq!(catalog.generation, 2);
    assert!(catalog.active_refresh.as_ref().unwrap().key == active_key);
}

#[test]
fn superseded_topology_and_stale_stats_completions_cannot_publish() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.snapshot.repositories = vec![repository()];
    catalog.store.recent = vec![known_repositories::RecentRepository {
        common_dir: Some(PathBuf::from("/repo/.git")),
        root: PathBuf::from("/repo"),
        stats: Some((3, 2)),
    }];
    catalog.stats.insert(
        PathBuf::from("/repo"),
        RepositoryStatsEntry {
            counts: Some((3, 2)),
            signature: Some(WorktreeSignature::for_test(2, 1)),
            checked_at: Some(Instant::now()),
            recheck_interval: MAX_STATS_INTERVAL,
            revision: 0,
        },
    );
    let active_key = catalog.refresh_request().key;
    catalog.topology_epoch = 1;
    let pending_request = catalog.refresh_request();
    catalog.generation = 7;
    catalog.snapshot.loading = true;
    catalog.active_refresh = Some(CatalogRefreshFlight {
        key: active_key,
        generation: 7,
        inventory_pending: true,
        stats_pending: true,
    });
    catalog.pending_refresh = Some(pending_request);
    catalog.active_stats_generation = Some(7);
    catalog.stats_in_flight.insert(
        PathBuf::from("/repo"),
        RepositoryStatsInterest {
            revision: 0,
            standalone: false,
            refresh_generation: Some(7),
        },
    );
    catalog
        .sender
        .send(InventoryCompletion {
            generation: 7,
            branches_loaded: false,
            topology_epoch: 0,
            repositories: Vec::new(),
            discovered: Vec::new(),
            pruned: vec![PathBuf::from("/repo/.git")],
            relevant: Vec::new(),
        })
        .unwrap();
    catalog
        .stats_sender
        .send(RepositoryStatsCompletion {
            generation: 7,
            result: Some(RepositoryStatsResult {
                root: PathBuf::from("/repo"),
                revision: 0,
                result: Ok((WorktreeSignature::for_test(1, 1), Some((99, 88)))),
            }),
        })
        .unwrap();

    let poll = catalog.poll();

    assert!(!poll.changed);
    assert_eq!(catalog.snapshot.repositories.len(), 1);
    assert_eq!(catalog.change_stats(Path::new("/repo")), Some((3, 2)));
    assert_eq!(catalog.store.recent[0].stats, Some((3, 2)));
    assert_eq!(catalog.generation, 8);
    assert_eq!(catalog.active_refresh.as_ref().unwrap().generation, 8);
}

#[test]
fn persists_repository_identity_and_recent_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    for index in 0..12 {
        catalog
            .remember_workspace(
                Some(Path::new(&format!("/repo-{index}/.git"))),
                Path::new(&format!("/repo-{index}")),
            )
            .unwrap();
    }
    catalog
        .remember_workspace(Some(Path::new("/repo-5/.git")), Path::new("/repo-5-linked"))
        .unwrap();
    wait_for_persistence(&mut catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.recent.len(), 12);
    assert_eq!(
        restored.store.recent[0].common_dir.as_deref(),
        Some(Path::new("/repo-5/.git"))
    );
    assert_eq!(restored.store.recent[0].root, Path::new("/repo-5-linked"));
    assert_eq!(
        restored.store.recent[1].common_dir.as_deref(),
        Some(Path::new("/repo-11/.git"))
    );
}

#[test]
fn stale_stats_persistence_cannot_overwrite_newer_topology() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("repositories.json");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    catalog
        .remember_workspace(Some(Path::new("/first/.git")), Path::new("/first"))
        .unwrap();
    assert!(
        catalog
            .store
            .update_stats(&[(PathBuf::from("/first"), (14, 3))])
            .unwrap()
    );
    let stale_stats = catalog.store.persistence_request().unwrap().unwrap();

    catalog
        .remember_workspace(Some(Path::new("/second/.git")), Path::new("/second"))
        .unwrap();
    known_repositories::persist(stale_stats).unwrap();
    wait_for_persistence(&mut catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.recent[0].root, PathBuf::from("/second"));
    assert_eq!(restored.store.recent[1].root, PathBuf::from("/first"));
    assert_eq!(restored.store.recent[1].stats, Some((14, 3)));
}

#[test]
fn bounds_recent_history_and_moves_duplicates_to_the_front() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    for index in 0..known_repositories::MAX_RECENT_REPOSITORIES + 5 {
        catalog
            .remember_workspace(None, Path::new(&format!("/workspace-{index:03}")))
            .unwrap();
    }
    let repeated = Path::new("/workspace-010");
    catalog.remember_workspace(None, repeated).unwrap();
    wait_for_persistence(&mut catalog);

    assert_eq!(
        catalog.store.recent.len(),
        known_repositories::MAX_RECENT_REPOSITORIES
    );
    assert_eq!(catalog.store.recent[0].root, repeated);
    assert_eq!(
        catalog
            .store
            .recent
            .iter()
            .filter(|recent| recent.root == repeated)
            .count(),
        1
    );
    assert!(
        !catalog
            .store
            .recent
            .iter()
            .any(|recent| recent.root == Path::new("/workspace-000"))
    );

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.recent, catalog.store.recent);
}

#[test]
fn bounds_known_history_while_preserving_recent_and_relevant_repositories() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    for index in 0..known_repositories::MAX_KNOWN_REPOSITORIES + 10 {
        catalog
            .remember_workspace(
                Some(Path::new(&format!("/repo-{index:03}/.git"))),
                Path::new(&format!("/repo-{index:03}")),
            )
            .unwrap();
    }

    assert_eq!(
        catalog.store.repositories.len(),
        known_repositories::MAX_KNOWN_REPOSITORIES
    );
    for index in 0..10 {
        assert!(
            !catalog
                .store
                .repositories
                .contains(&PathBuf::from(format!("/repo-{index:03}/.git")))
        );
    }
    let oldest_retained = PathBuf::from("/repo-010/.git");
    let newest = PathBuf::from(format!(
        "/repo-{:03}/.git",
        known_repositories::MAX_KNOWN_REPOSITORIES + 9
    ));
    assert_eq!(catalog.store.repositories[0], oldest_retained);
    assert!(catalog.store.repositories.contains(&newest));
    for recent in &catalog.store.recent {
        assert!(
            catalog
                .store
                .repositories
                .contains(recent.common_dir.as_ref().unwrap())
        );
    }

    catalog
        .remember_workspace(Some(&oldest_retained), Path::new("/repo-010-linked"))
        .unwrap();
    assert_eq!(catalog.store.repositories[0], oldest_retained);
    assert_eq!(
        catalog
            .store
            .repositories
            .iter()
            .filter(|repository| *repository == &oldest_retained)
            .count(),
        1
    );
    assert_eq!(
        catalog.store.recent[0].common_dir.as_ref(),
        Some(&oldest_retained)
    );

    let relevant = PathBuf::from("/zzzz-current/.git");
    catalog
        .store
        .reconcile(vec![relevant.clone()], &[], std::slice::from_ref(&relevant))
        .unwrap();
    assert_eq!(
        catalog.store.repositories.len(),
        known_repositories::MAX_KNOWN_REPOSITORIES
    );
    assert!(catalog.store.repositories.contains(&relevant));
    assert_eq!(catalog.store.repositories.last(), Some(&relevant));
    assert!(
        !catalog
            .store
            .repositories
            .contains(&PathBuf::from("/repo-011/.git"))
    );
}

#[test]
fn accepts_oversized_persistence_and_rewrites_it_bounded_and_deduplicated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let mut repositories = (0..known_repositories::MAX_KNOWN_REPOSITORIES + 5)
        .map(|index| serde_json::json!({ "common_dir": format!("/repo-{index:03}/.git") }))
        .collect::<Vec<_>>();
    repositories.push(repositories[0].clone());
    let mut recent = (0..known_repositories::MAX_RECENT_REPOSITORIES + 5)
        .map(|index| {
            serde_json::json!({
                "common_dir": { "path": format!("/repo-{index:03}/.git") },
                "root": { "path": format!("/repo-{index:03}") },
                "stats": null,
            })
        })
        .collect::<Vec<_>>();
    recent.push(recent[0].clone());
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "repositories": repositories,
            "recent": recent,
        }))
        .unwrap(),
    )
    .unwrap();

    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    assert_eq!(
        catalog.store.repositories.len(),
        known_repositories::MAX_KNOWN_REPOSITORIES
    );
    assert_eq!(
        catalog.store.recent.len(),
        known_repositories::MAX_RECENT_REPOSITORIES
    );
    catalog
        .remember_workspace(None, Path::new("/local-workspace"))
        .unwrap();
    wait_for_persistence(&mut catalog);

    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(
        persisted["repositories"].as_array().unwrap().len(),
        known_repositories::MAX_KNOWN_REPOSITORIES
    );
    assert_eq!(
        persisted["recent"].as_array().unwrap().len(),
        known_repositories::MAX_RECENT_REPOSITORIES
    );
}

#[test]
fn missing_repository_pruning_overrides_history_protection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let missing = PathBuf::from("/missing/.git");
    let retained = PathBuf::from("/retained/.git");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    catalog
        .remember_workspace(Some(&missing), Path::new("/missing"))
        .unwrap();
    catalog
        .remember_workspace(Some(&retained), Path::new("/retained"))
        .unwrap();

    catalog
        .store
        .reconcile(
            Vec::new(),
            std::slice::from_ref(&missing),
            std::slice::from_ref(&missing),
        )
        .unwrap();
    persist_store(&catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.repositories, [retained]);
    assert!(
        restored
            .store
            .recent
            .iter()
            .all(|recent| recent.common_dir.as_ref() != Some(&missing))
    );
}

#[test]
fn persists_local_workspaces_in_recent_order_without_git_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    catalog
        .remember_workspace(Some(Path::new("/repo/.git")), Path::new("/repo"))
        .unwrap();
    catalog
        .remember_workspace(None, Path::new("/home/example"))
        .unwrap();
    wait_for_persistence(&mut catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    let recent = restored.recent_repository_picker_items();
    assert_eq!(recent[0].root, Path::new("/home/example"));
    assert_eq!(recent[0].label, "example");
    assert_eq!(restored.store.recent[0].common_dir, None);
    assert_eq!(restored.store.repositories, [PathBuf::from("/repo/.git")]);
}

#[test]
fn persists_repository_stats_for_an_instant_picker_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    catalog
        .remember_workspace(Some(Path::new("/repo/.git")), Path::new("/repo"))
        .unwrap();
    assert!(
        catalog
            .store
            .update_stats(&[(PathBuf::from("/repo"), (21, 8))])
            .unwrap()
    );
    persist_store(&catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(
        restored.recent_repository_picker_items()[0].stats,
        Some((21, 8))
    );
}

#[test]
fn repository_stats_deduplicate_repeated_roots_without_reordering() {
    let recent = vec![
        known_repositories::RecentRepository {
            common_dir: Some(PathBuf::from("/first/.git")),
            root: PathBuf::from("/shared"),
            stats: None,
        },
        known_repositories::RecentRepository {
            common_dir: Some(PathBuf::from("/second/.git")),
            root: PathBuf::from("/shared"),
            stats: None,
        },
        known_repositories::RecentRepository {
            common_dir: Some(PathBuf::from("/third/.git")),
            root: PathBuf::from("/other"),
            stats: None,
        },
    ];

    assert_eq!(
        recent_git_roots(recent),
        [PathBuf::from("/shared"), PathBuf::from("/other")]
    );
}

#[test]
fn malformed_inventory_is_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    fs::write(&path, b"not json").unwrap();
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));

    assert!(
        catalog
            .remember_workspace(Some(Path::new("/repo/.git")), Path::new("/repo"))
            .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), b"not json");
}

#[cfg(unix)]
#[test]
fn persists_non_utf8_repository_identity_without_loss() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known-repositories.json");
    let common_dir = PathBuf::from(std::ffi::OsString::from_vec(b"/repo/\xff/.git".to_vec()));
    let root = PathBuf::from(std::ffi::OsString::from_vec(b"/repo/\xff".to_vec()));
    let mut catalog = LinkedWorktreeCatalog::new(Some(path.clone()));
    catalog
        .remember_workspace(Some(&common_dir), &root)
        .unwrap();
    wait_for_persistence(&mut catalog);

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.repositories, [common_dir]);
    assert_eq!(restored.store.recent[0].root, root);
}

#[test]
fn orders_repositories_by_observed_workspace_order() {
    let directory = tempfile::tempdir().unwrap();
    let alpha = directory.path().join("alpha");
    let zulu = directory.path().join("zulu");
    for repository in [&alpha, &zulu] {
        fs::create_dir(repository).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repository)
                .status()
                .unwrap()
                .success()
        );
    }

    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.observe_herdr(LinkedWorktreeObservation {
        candidates: vec![
            LinkedWorktreeCandidate { path: zulu },
            LinkedWorktreeCandidate { path: alpha },
        ],
    });
    catalog.refresh();
    let deadline = Instant::now() + Duration::from_secs(2);
    while catalog.snapshot.loading && Instant::now() < deadline {
        catalog.poll();
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        catalog
            .snapshot
            .repositories
            .iter()
            .map(|repository| repository.label.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
}
