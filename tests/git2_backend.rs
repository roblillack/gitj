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

/// A repository for the branch-review tests: `main` holds a base commit plus
/// one of its own, `feature` branches at the base and adds three commits (one
/// of them touching a binary file). Both locals track an `origin` upstream:
/// `origin/main` sits at main's tip (in sync), `origin/feature` one commit
/// behind feature's (diverged). `main` is checked out. Returns the scratch
/// dir, the repo, and the (base, feature-tip) commit ids.
fn branchy_repo() -> (std::path::PathBuf, Repository, git2::Oid, git2::Oid) {
    let dir = scratch_dir("branches");
    let repo = Repository::init(&dir).unwrap();
    // Pin the default branch name before the first commit, so the review base
    // is host-independent (init.defaultBranch varies).
    repo.set_head("refs/heads/main").unwrap();
    let sig = Signature::new(
        "Tester",
        "tester@example.com",
        &Time::new(1_700_000_000, 60),
    )
    .unwrap();

    fs::write(dir.join("a.txt"), "one\n").unwrap();
    let base = commit_file(&repo, "a.txt", &sig, "base commit\n", &[]);
    fs::write(dir.join("data.bin"), [0u8, 1, 2]).unwrap();
    let base2 = commit_file(&repo, "data.bin", &sig, "add binary\n", &[base]);

    // main moves on after the branch point — these changes must NOT show up
    // in the feature branch's review.
    fs::write(dir.join("c.txt"), "main side\n").unwrap();
    let main_tip = commit_file(&repo, "c.txt", &sig, "main moves on\n", &[base2]);

    // feature branches at base2 and adds two commits. The checkout resets the
    // index (which `commit_file` reuses) to the branch point, so main's c.txt
    // doesn't leak into the feature commits' trees.
    repo.branch("feature", &repo.find_commit(base2).unwrap(), false)
        .unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout)).unwrap();
    fs::write(dir.join("b.txt"), "feature side\n").unwrap();
    let f1 = commit_file(&repo, "b.txt", &sig, "feature: add b\n", &[base2]);
    fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    let f2 = commit_file(&repo, "a.txt", &sig, "feature: extend a\n", &[f1]);
    fs::write(dir.join("data.bin"), [9u8, 8]).unwrap();
    let f3 = commit_file(&repo, "data.bin", &sig, "feature: change binary\n", &[f2]);

    // Remote-tracking branches under a configured `origin` (set_upstream needs
    // the remote to resolve the tracking refspec): origin/main in sync with
    // main, origin/feature one commit behind feature.
    repo.remote("origin", "https://example.com/repo.git")
        .unwrap();
    repo.reference("refs/remotes/origin/main", main_tip, true, "test")
        .unwrap();
    repo.reference("refs/remotes/origin/feature", f2, true, "test")
        .unwrap();
    repo.find_branch("main", git2::BranchType::Local)
        .unwrap()
        .set_upstream(Some("origin/main"))
        .unwrap();
    repo.find_branch("feature", git2::BranchType::Local)
        .unwrap()
        .set_upstream(Some("origin/feature"))
        .unwrap();

    // Check main back out so it is the HEAD branch.
    repo.set_head("refs/heads/main").unwrap();
    (dir, repo, base2, f3)
}

#[test]
fn branch_review_diffs_against_the_merge_base() {
    let (dir, _repo, base, tip) = branchy_repo();
    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");

    // Checked-out branch first, then locals, then remotes. origin/main sits
    // at main's tip and is tracked by it, so it folds into main's row instead
    // of being listed; origin/feature has diverged from feature, so it keeps
    // its own row.
    let branches = backend.branches();
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["main", "feature", "origin/feature"]);
    assert_eq!(branches[0].kind, RefKind::Head);
    assert_eq!(branches[1].kind, RefKind::LocalBranch);
    assert_eq!(branches[2].kind, RefKind::RemoteBranch);
    assert_eq!(branches[0].upstream.as_deref(), Some("origin/main"));
    assert_eq!(
        branches[1].upstream, None,
        "a diverged upstream must not fold into the local's row"
    );

    // The feature branch reviews against its merge base with main: the
    // branch point, not main's tip.
    let feature = &branches[1];
    assert_eq!(feature.base_name, "main");
    assert_eq!(feature.base_id.as_deref(), Some(base.to_string().as_str()));
    assert_eq!(feature.tip_id, tip.to_string());
    assert_eq!(feature.summary, "feature: change binary");

    // The aggregated file list spans all three feature commits — and does NOT
    // contain main's own post-branch file (a tip-vs-tip diff would have shown
    // c.txt as deleted).
    let files = backend.branch_files(feature);
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["a.txt", "b.txt", "data.bin"]);
    assert_eq!(
        files.iter().find(|f| f.path == "a.txt").unwrap().status,
        ChangeStatus::Modified
    );
    assert_eq!(
        files.iter().find(|f| f.path == "b.txt").unwrap().status,
        ChangeStatus::Added
    );

    // The aggregated diff carries both text changes, nothing from main's side.
    let diff = backend.branch_diff(feature);
    let has_addition = |needle: &str| {
        diff.lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Addition && l.text.contains(needle))
    };
    assert!(has_addition("two"), "a.txt's change is part of the review");
    assert!(
        has_addition("feature side"),
        "b.txt's change is part of the review"
    );
    assert!(
        !diff.lines.iter().any(|l| l.text.contains("main side")),
        "main's own commit must not leak into the branch review"
    );

    // A single-file diff narrows to that file.
    let file_diff = backend.branch_file_diff(feature, "b.txt");
    assert!(
        file_diff
            .lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Addition && l.text.contains("feature side"))
    );
    assert!(!file_diff.lines.iter().any(|l| l.text.contains("two")));

    // main itself is the review base — its review is empty.
    assert!(backend.branch_files(&branches[0]).is_empty());
    assert!(backend.branch_diff(&branches[0]).is_empty());

    // The diverged remote branch reviews at its own tip: only the two changes
    // that were "pushed" (f3's binary change is missing from origin/feature).
    let remote_files = backend.branch_files(&branches[2]);
    let mut remote_paths: Vec<&str> = remote_files.iter().map(|f| f.path.as_str()).collect();
    remote_paths.sort_unstable();
    assert_eq!(remote_paths, ["a.txt", "b.txt"]);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn branch_file_blobs_compare_base_and_tip() {
    let (dir, _repo, _base, _tip) = branchy_repo();
    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");
    let branches = backend.branches();
    let feature = branches.iter().find(|b| b.name == "feature").unwrap();

    // A file changed on the branch: base bytes vs tip bytes.
    let blobs = backend.branch_file_blobs(feature, "data.bin");
    assert_eq!(blobs.old.as_deref(), Some(&[0u8, 1, 2][..]));
    assert_eq!(blobs.new.as_deref(), Some(&[9u8, 8][..]));

    // A file added on the branch has no base side.
    let added = backend.branch_file_blobs(feature, "b.txt");
    assert_eq!(added.old, None);
    assert_eq!(added.new.as_deref(), Some("feature side\n".as_bytes()));

    fs::remove_dir_all(&dir).ok();
}
