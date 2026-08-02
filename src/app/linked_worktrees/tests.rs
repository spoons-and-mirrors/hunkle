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
        })
        .unwrap();

    assert!(!catalog.poll().changed);
    assert_eq!(catalog.snapshot.repositories.len(), 1);
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
