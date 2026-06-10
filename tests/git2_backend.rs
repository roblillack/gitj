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
    let sig = Signature::new(
        "Tester",
        "tester@example.com",
        &Time::new(1_700_000_000, 60),
    )
    .unwrap();

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

/// A commit with a diff far larger than the render cap is **truncated** rather
/// than materialized in full — the regression guard for the merge-commit hang,
/// where a 1.2M-line diff froze the UI for tens of seconds when selected.
#[test]
fn huge_diff_is_truncated() {
    let dir = scratch_dir("git2-huge");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 0)).unwrap();

    // A file with many more lines than the cap, committed from empty so the diff
    // is one huge block of additions.
    let big: String = (0..60_000).map(|n| format!("line {n}\n")).collect();
    fs::write(dir.join("big.txt"), &big).unwrap();
    commit_file(&repo, "big.txt", &sig, "add a huge file\n", &[]);

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");
    let diff = backend.commit_diff(0);

    // Bounded (the cap is 50_000 lines + a trailing marker), not ~60_000.
    assert!(
        diff.lines.len() <= 50_001,
        "diff should be truncated, got {} lines",
        diff.lines.len()
    );
    assert!(diff.lines.len() > 1000, "but a real chunk is still shown");
    assert!(
        diff.lines.iter().any(|l| l.text.contains("truncated")),
        "a truncation marker should be appended"
    );

    fs::remove_dir_all(&dir).ok();
}

/// Rename detection still runs for an ordinary (small) diff under the
/// `added × deleted` gate: moving a file's content to a new path is reported as
/// one Renamed change, not a separate add + delete.
#[test]
fn detects_a_rename_in_a_small_diff() {
    let dir = scratch_dir("git2-rename");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 0)).unwrap();

    let body: String = (0..40)
        .map(|n| format!("the quick brown fox {n}\n"))
        .collect();
    fs::write(dir.join("old.txt"), &body).unwrap();
    commit_file(&repo, "old.txt", &sig, "add old.txt\n", &[]);

    // Rename: drop the old path, add the identical content under a new one.
    fs::remove_file(dir.join("old.txt")).unwrap();
    fs::write(dir.join("new.txt"), &body).unwrap();
    {
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("old.txt")).unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();
    }
    let head = repo.head().unwrap().peel_to_commit().unwrap().id();
    commit_file(
        &repo,
        "new.txt",
        &sig,
        "rename old.txt -> new.txt\n",
        &[head],
    );

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");
    let files = backend.changed_files(0);
    assert_eq!(files.len(), 1, "a rename is one change, got {files:?}");
    assert_eq!(files[0].status, ChangeStatus::Renamed);
    assert_eq!(files[0].path, "new.txt");
    assert_eq!(files[0].old_path.as_deref(), Some("old.txt"));

    fs::remove_dir_all(&dir).ok();
}

/// An ordinary small diff is shown in full, with no truncation marker.
#[test]
fn small_diff_is_not_truncated() {
    let dir = scratch_dir("git2-small");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 0)).unwrap();
    fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    commit_file(&repo, "a.txt", &sig, "add a.txt\n", &[]);

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");
    let diff = backend.commit_diff(0);
    assert!(
        !diff.lines.iter().any(|l| l.text.contains("truncated")),
        "a small diff must not be truncated"
    );

    fs::remove_dir_all(&dir).ok();
}
