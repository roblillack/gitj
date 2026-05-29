//! Integration test for the live [`Git2Backend`].
//!
//! Builds a throwaway repository with fixed author signatures and timestamps
//! so every assertion is deterministic and machine-independent, then reads it
//! back through the public [`RepoBackend`] interface.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Repository, Signature, Time};
use journey::backend::{ChangeStatus, DiffLineKind, Git2Backend, RefKind, RepoBackend};

/// Create a unique scratch directory under the system temp dir.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("journey-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Commit `path`'s current contents on top of `parents`, returning the new
/// commit id.
fn commit_file(
    repo: &Repository,
    path: &str,
    sig: &Signature,
    message: &str,
    parents: &[git2::Oid],
) -> git2::Oid {
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(path)).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent_commits: Vec<_> = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect();
    let parent_refs: Vec<_> = parent_commits.iter().collect();
    repo.commit(Some("HEAD"), sig, sig, message, &tree, &parent_refs)
        .unwrap()
}

#[test]
fn reads_history_refs_and_diffs() {
    let dir = scratch_dir("git2");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 60)).unwrap();

    fs::write(dir.join("a.txt"), "hello\n").unwrap();
    let c1 = commit_file(&repo, "a.txt", &sig, "first commit\n", &[]);

    fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
    let c2 = commit_file(&repo, "a.txt", &sig, "second commit\n", &[c1]);

    // A lightweight tag on the first commit and the default branch on the
    // second (whatever `init` named it — we assert on kind, not name).
    repo.tag_lightweight("v1", &repo.find_object(c1, None).unwrap(), false)
        .unwrap();

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");

    // Newest first.
    let commits = backend.commits();
    assert_eq!(commits.len(), 2, "expected two commits");
    assert_eq!(commits[0].summary, "second commit");
    assert_eq!(commits[1].summary, "first commit");
    assert_eq!(commits[0].id, c2.to_string());
    assert_eq!(commits[1].id, c1.to_string());

    // Parent links.
    assert_eq!(commits[0].parents, vec![c1.to_string()]);
    assert!(commits[1].parents.is_empty());

    // Deterministic author date (offset +0100).
    assert_eq!(commits[0].date_string(), "2023-11-14 23:13:20 +0100");

    // The checked-out branch decorates the tip commit.
    assert!(
        commits[0].refs.iter().any(|r| r.kind == RefKind::Head),
        "tip commit should carry the HEAD branch label, got {:?}",
        commits[0].refs
    );
    // The tag decorates the first commit.
    assert!(
        commits[1]
            .refs
            .iter()
            .any(|r| r.kind == RefKind::Tag && r.name == "v1"),
        "first commit should carry tag v1, got {:?}",
        commits[1].refs
    );

    // File statuses: modified in c2, added in c1.
    let f2 = backend.changed_files(0);
    assert_eq!(f2.len(), 1);
    assert_eq!(f2[0].path, "a.txt");
    assert_eq!(f2[0].status, ChangeStatus::Modified);

    let f1 = backend.changed_files(1);
    assert_eq!(f1.len(), 1);
    assert_eq!(f1[0].status, ChangeStatus::Added);

    // The diff of c2 adds the "world" line.
    let diff = backend.commit_diff(0);
    assert!(
        diff.lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Addition && l.text.contains("world")),
        "diff should add 'world', got {:?}",
        diff.lines
    );
    // Single-file diff agrees with the whole-commit diff for this one file.
    let file_diff = backend.file_diff(0, "a.txt");
    assert!(
        file_diff
            .lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Addition && l.text.contains("world"))
    );

    fs::remove_dir_all(&dir).ok();
}
