use super::*;

#[test]
fn atomically_replaces_file_contents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state");
    fs::write(&path, "old").unwrap();

    atomic_write(&path, b"new").unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[test]
fn path_identity_uses_lexical_and_canonical_equality() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    fs::create_dir(&child).unwrap();

    assert!(same_path(&child, &child));
    assert!(same_path(&child, &directory.path().join("./child")));
    assert!(!same_path(&child, &directory.path().join("missing")));
}

#[test]
fn reads_only_immediate_workspace_children() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::create_dir(root.join("empty")).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join("README.md"), "readme").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    let root_entries = read_workspace_directory(root, &RepoPath::default()).unwrap();
    assert_eq!(
        root_entries,
        vec![
            WorkspaceEntry {
                path: "empty".into(),
                is_directory: true,
            },
            WorkspaceEntry {
                path: "src".into(),
                is_directory: true,
            },
            WorkspaceEntry {
                path: "README.md".into(),
                is_directory: false,
            },
        ]
    );
    assert_eq!(
        read_workspace_directory(root, &"src".into()).unwrap(),
        vec![
            WorkspaceEntry {
                path: "src/nested".into(),
                is_directory: true,
            },
            WorkspaceEntry {
                path: "src/main.rs".into(),
                is_directory: false,
            },
        ]
    );
}

#[test]
fn creates_moves_renames_and_deletes_workspace_entries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();

    perform(
        root,
        &FileOperation::CreateDirectory {
            path: "docs".into(),
        },
    )
    .unwrap();
    perform(
        root,
        &FileOperation::CreateFile {
            path: "readme.md".into(),
        },
    )
    .unwrap();
    perform(
        root,
        &FileOperation::Move {
            from: "readme.md".into(),
            to: "docs/readme.md".into(),
        },
    )
    .unwrap();
    perform(
        root,
        &FileOperation::Rename {
            from: "docs/readme.md".into(),
            to: "docs/guide.md".into(),
        },
    )
    .unwrap();
    assert!(root.join("docs/guide.md").is_file());

    perform(
        root,
        &FileOperation::Delete {
            path: "docs".into(),
        },
    )
    .unwrap();
    assert!(!root.join("docs").exists());
}

#[test]
fn rejects_traversal_overwrites_and_descendant_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file"), "content").unwrap();
    fs::write(root.join("existing"), "content").unwrap();
    fs::create_dir_all(root.join("nested-repository/.git")).unwrap();

    assert!(
        perform(
            root,
            &FileOperation::Delete {
                path: "../outside".into()
            }
        )
        .is_err()
    );
    assert!(
        perform(
            root,
            &FileOperation::Move {
                from: "nested-repository".into(),
                to: "moved-repository".into(),
            },
        )
        .is_err()
    );
    assert!(
        perform(
            root,
            &FileOperation::Delete {
                path: ".git/config".into()
            }
        )
        .is_err()
    );
    assert!(
        perform(
            root,
            &FileOperation::Rename {
                from: "source/file".into(),
                to: "existing".into(),
            },
        )
        .is_err()
    );
    assert!(
        perform(
            root,
            &FileOperation::Move {
                from: "source".into(),
                to: "source/nested/source".into(),
            },
        )
        .is_err()
    );
}

#[test]
fn refuses_to_delete_nested_repositories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("parent/nested/.git")).unwrap();
    fs::write(root.join("parent/file"), "content").unwrap();

    let error = perform(
        root,
        &FileOperation::Delete {
            path: "parent".into(),
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("nested Git repository"));
    assert!(root.join("parent/file").is_file());
}

#[test]
fn validates_regular_files_inside_the_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_eq!(
        safe_regular_file(root, &RepoPath::from("src/main.rs")).unwrap(),
        root.join("src/main.rs")
    );
    assert!(safe_regular_file(root, &RepoPath::from("../outside")).is_err());
    assert!(safe_regular_file(root, &RepoPath::from("src")).is_err());
}

#[cfg(unix)]
#[test]
fn deletes_a_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("target")).unwrap();
    fs::write(root.join("target/keep"), "content").unwrap();
    symlink(root.join("target"), root.join("link")).unwrap();

    perform(
        root,
        &FileOperation::Delete {
            path: "link".into(),
        },
    )
    .unwrap();
    assert!(root.join("target/keep").exists());
    assert!(!root.join("link").exists());
}

#[cfg(unix)]
#[test]
fn file_operations_preserve_invalid_utf8_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = OsString::from_vec(b"entry-\x80".to_vec());
    let second = OsString::from_vec(b"entry-\x81".to_vec());
    let renamed = OsString::from_vec(b"renamed-\x80".to_vec());
    fs::write(root.join(&first), "first").unwrap();
    fs::write(root.join(&second), "second").unwrap();

    perform(
        root,
        &FileOperation::Rename {
            from: RepoPath::from(PathBuf::from(&first)),
            to: RepoPath::from(PathBuf::from(&renamed)),
        },
    )
    .unwrap();

    assert!(!root.join(&first).exists());
    assert_eq!(fs::read(root.join(&renamed)).unwrap(), b"first");
    assert_eq!(fs::read(root.join(&second)).unwrap(), b"second");
}

#[cfg(unix)]
#[test]
fn rejects_operations_through_a_symlinked_ancestor() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), "content").unwrap();
    symlink(outside.path(), workspace.path().join("link")).unwrap();

    assert!(
        perform(
            workspace.path(),
            &FileOperation::Delete {
                path: "link/keep".into(),
            },
        )
        .is_err()
    );
    assert!(
        perform(
            workspace.path(),
            &FileOperation::CreateFile {
                path: "link/new".into(),
            },
        )
        .is_err()
    );
    assert!(outside.path().join("keep").exists());
    assert!(!outside.path().join("new").exists());

    let container = tempfile::tempdir().unwrap();
    let root = container.path().join("workspace");
    let original = container.path().join("original");
    fs::create_dir(&root).unwrap();
    fs::rename(&root, &original).unwrap();
    symlink(outside.path(), &root).unwrap();
    assert!(
        perform(
            &root,
            &FileOperation::Delete {
                path: "keep".into(),
            },
        )
        .is_err()
    );
    assert!(outside.path().join("keep").exists());

    assert!(safe_regular_file(workspace.path(), &RepoPath::from("link/keep")).is_err());
}
