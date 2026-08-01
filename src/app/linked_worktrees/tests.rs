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
        group: None,
        worktrees: vec![linked("/repo", true), linked("/repo-feature", false)],
        error: None,
    }
}

#[test]
fn routes_removal_from_current_authority() {
    let repositories = vec![repository()];
    let path = Path::new("/repo-feature");

    let native = LinkedWorktreeCatalogSnapshot::for_test(
        repositories.clone(),
        Some(PathBuf::from("/repo")),
        HerdrOwnership::Disabled,
    );
    assert_eq!(
        native.removal_plan(path),
        Ok(LinkedWorktreeRemovalPlan::Native {
            common_dir: PathBuf::from("/repo/.git"),
            path: path.to_owned(),
        })
    );

    let unverified = LinkedWorktreeCatalogSnapshot::for_test(
        repositories.clone(),
        Some(PathBuf::from("/repo")),
        HerdrOwnership::Unverified,
    );
    assert!(
        unverified
            .removal_plan(path)
            .unwrap_err()
            .contains("Waiting for Herdr")
    );
    assert_eq!(unverified.repositories.len(), 1);

    let owned = LinkedWorktreeCatalogSnapshot::for_test(
        repositories.clone(),
        Some(PathBuf::from("/repo")),
        HerdrOwnership::Verified(vec![HerdrOwnedWorktree {
            path: path.to_owned(),
            workspace_id: "workspace-2".to_owned(),
        }]),
    );
    assert_eq!(
        owned.removal_plan(path),
        Ok(LinkedWorktreeRemovalPlan::Herdr {
            workspace_id: "workspace-2".to_owned(),
            path: path.to_owned(),
        })
    );

    let unowned = LinkedWorktreeCatalogSnapshot::for_test(
        repositories,
        Some(PathBuf::from("/repo")),
        HerdrOwnership::Verified(Vec::new()),
    );
    assert!(matches!(
        unowned.removal_plan(path),
        Ok(LinkedWorktreeRemovalPlan::Native { .. })
    ));
}

#[test]
fn protects_git_topology_and_active_worktree() {
    let snapshot = LinkedWorktreeCatalogSnapshot::for_test(
        vec![repository()],
        Some(PathBuf::from("/repo-feature")),
        HerdrOwnership::Disabled,
    );
    assert!(
        snapshot
            .removal_plan(Path::new("/repo"))
            .unwrap_err()
            .contains("primary")
    );
    assert!(
        snapshot
            .removal_plan(Path::new("/repo-feature"))
            .unwrap_err()
            .contains("current")
    );
}

#[test]
fn herdr_only_paths_do_not_become_catalog_entries() {
    let snapshot = LinkedWorktreeCatalogSnapshot::for_test(
        vec![repository()],
        None,
        HerdrOwnership::Verified(vec![HerdrOwnedWorktree {
            path: PathBuf::from("/not-in-git"),
            workspace_id: "workspace-3".to_owned(),
        }]),
    );
    assert!(snapshot.removal_plan(Path::new("/not-in-git")).is_err());
    assert!(
        !snapshot
            .repositories
            .iter()
            .flat_map(|repository| &repository.worktrees)
            .any(|worktree| worktree.path == Path::new("/not-in-git"))
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
            .remember_repository(
                Path::new(&format!("/repo-{index}/.git")),
                Path::new(&format!("/repo-{index}")),
            )
            .unwrap();
    }
    catalog
        .remember_repository(Path::new("/repo-5/.git"), Path::new("/repo-5-linked"))
        .unwrap();

    let restored = LinkedWorktreeCatalog::new(Some(path));
    assert_eq!(restored.store.recent.len(), 10);
    assert_eq!(
        restored.store.recent[0].common_dir,
        Path::new("/repo-5/.git")
    );
    assert_eq!(restored.store.recent[0].root, Path::new("/repo-5-linked"));
    assert_eq!(
        restored.store.recent[1].common_dir,
        Path::new("/repo-11/.git")
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
            .remember_repository(Path::new("/repo/.git"), Path::new("/repo"))
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
    catalog.remember_repository(&common_dir, &root).unwrap();

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
            LinkedWorktreeCandidate {
                path: zulu,
                group: Some("First".to_owned()),
            },
            LinkedWorktreeCandidate {
                path: alpha,
                group: Some("Second".to_owned()),
            },
        ],
        ownership: HerdrOwnership::Disabled,
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
    assert_eq!(
        catalog.snapshot.repositories[0].group.as_deref(),
        Some("First")
    );
    assert_eq!(
        catalog.snapshot.repositories[1].group.as_deref(),
        Some("Second")
    );
}
