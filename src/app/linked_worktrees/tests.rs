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
fn ignores_stale_inventory_completions() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.generation = 2;
    catalog.snapshot.repositories = vec![repository()];
    catalog
        .sender
        .send(InventoryCompletion {
            generation: 1,
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
fn superseded_completions_cannot_publish_topology_or_stats() {
    let mut catalog = LinkedWorktreeCatalog::new(None);
    catalog.snapshot.repositories = vec![repository()];
    catalog.store.recent = vec![known_repositories::RecentRepository {
        common_dir: Some(PathBuf::from("/repo/.git")),
        root: PathBuf::from("/repo"),
        stats: Some((3, 2)),
    }];
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
    catalog
        .sender
        .send(InventoryCompletion {
            generation: 7,
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
            stats: vec![(PathBuf::from("/repo"), (99, 88))],
        })
        .unwrap();

    let poll = catalog.poll();

    assert!(!poll.changed);
    assert_eq!(catalog.snapshot.repositories.len(), 1);
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
        .reconcile_and_save(vec![relevant.clone()], &[], std::slice::from_ref(&relevant))
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
        .reconcile_and_save(
            Vec::new(),
            std::slice::from_ref(&missing),
            std::slice::from_ref(&missing),
        )
        .unwrap();

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
            .update_stats_and_save(&[(PathBuf::from("/repo"), (21, 8))])
            .unwrap()
    );

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
