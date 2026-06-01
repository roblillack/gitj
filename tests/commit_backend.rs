//! Integration tests for commit mode's backend operations: working-tree
//! status, staging / unstaging, committing and amending. The git2 test builds
//! a throwaway repository with a local identity so `commit` can sign.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::Repository;
use journey::backend::{ChangeStatus, FixtureBackend, Git2Backend, RepoBackend};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("journey-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fixture_stage_unstage_commit() {
    let be = FixtureBackend::sample();

    let ws = be.working_status(false);
    assert_eq!(ws.unstaged.len(), 2, "sample has two unstaged files");
    assert_eq!(ws.staged.len(), 2, "sample has two staged files");

    // Stage an unstaged file: it moves to the staged list.
    be.stage("src/ui.rs").unwrap();
    let ws = be.working_status(false);
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.staged.len(), 3);
    assert!(ws.staged.iter().any(|f| f.path == "src/ui.rs"));

    // Unstage a staged file: it moves back.
    be.unstage("Cargo.toml", false).unwrap();
    let ws = be.working_status(false);
    assert!(ws.unstaged.iter().any(|f| f.path == "Cargo.toml"));
    assert!(!ws.staged.iter().any(|f| f.path == "Cargo.toml"));

    // Commit clears the staged set and records the message.
    be.commit("wire up commit mode", false).unwrap();
    assert_eq!(
        be.last_commit(),
        Some(("wire up commit mode".to_string(), false))
    );
    assert!(be.working_status(false).staged.is_empty());

    // Amend view: the HEAD commit's files show up as staged (they'd be
    // re-committed), and can be pulled back out to the unstaged side.
    let amend = be.working_status(true);
    assert!(
        amend
            .staged
            .iter()
            .any(|f| f.path == "src/widgets/graph.rs"),
        "amend should stage HEAD's files, got {:?}",
        amend.staged
    );
    be.unstage("src/widgets/graph.rs", true).unwrap();
    let amend = be.working_status(true);
    assert!(
        !amend
            .staged
            .iter()
            .any(|f| f.path == "src/widgets/graph.rs")
    );
    assert!(
        amend
            .unstaged
            .iter()
            .any(|f| f.path == "src/widgets/graph.rs")
    );
    // Normal (non-amend) view never shows HEAD's files.
    assert!(
        !be.working_status(false)
            .staged
            .iter()
            .any(|f| f.path == "src/widgets/graph.rs")
    );

    // An empty message is rejected.
    assert!(be.commit("   ", false).is_err());
}

#[test]
fn fixture_revert_discards_unstaged_only() {
    let be = FixtureBackend::sample();
    // sample() seeds src/ui.rs (unstaged, modified), notes.md (unstaged,
    // untracked) and Cargo.toml (staged), among others.
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs")
    );

    // Reverting a tracked unstaged file drops it from the working set.
    be.revert("src/ui.rs").unwrap();
    let ws = be.working_status(false);
    assert!(
        !ws.unstaged.iter().any(|f| f.path == "src/ui.rs"),
        "revert should discard the unstaged change, got {ws:?}"
    );

    // Untracked files have no index version to restore, so revert leaves them.
    be.revert("notes.md").unwrap();
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "notes.md"),
        "revert must not delete untracked files"
    );

    // Staged changes are never touched by a revert.
    be.revert("Cargo.toml").unwrap();
    assert!(
        be.working_status(false)
            .staged
            .iter()
            .any(|f| f.path == "Cargo.toml")
    );
}

#[test]
fn fixture_delete_untracked_removes_the_file() {
    let be = FixtureBackend::sample();
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "notes.md")
    );

    // Deleting the untracked file takes it out of the working set.
    be.delete_untracked("notes.md").unwrap();
    assert!(
        !be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "notes.md")
    );

    // It only removes untracked files — a tracked path is left in place.
    be.delete_untracked("src/ui.rs").unwrap();
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs")
    );
}

#[test]
fn git2_delete_untracked_removes_file_from_workdir() {
    let dir = scratch_dir("delete");
    Repository::init(&dir).unwrap();
    fs::write(dir.join("new.txt"), "fresh\n").unwrap();
    let backend = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    assert!(
        backend
            .working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "new.txt" && f.status == ChangeStatus::Untracked)
    );

    backend.delete_untracked("new.txt").unwrap();
    assert!(
        !dir.join("new.txt").exists(),
        "the untracked file should be removed from disk"
    );
    assert!(backend.working_status(false).is_clean());

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn git2_revert_discards_working_changes() {
    let dir = scratch_dir("revert");
    let repo = Repository::init(&dir).unwrap();
    {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Tester").unwrap();
        cfg.set_str("user.email", "tester@example.com").unwrap();
    }
    fs::write(dir.join("a.txt"), "one\n").unwrap();
    let backend = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    backend.stage("a.txt").unwrap();
    backend.commit("seed", false).unwrap();

    // Stage a first edit, then make a *further* unstaged edit on top of it, and
    // drop an untracked file alongside.
    fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    backend.stage("a.txt").unwrap();
    fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(dir.join("b.txt"), "scratch\n").unwrap();

    let ws = backend.working_status(false);
    assert!(
        ws.unstaged.iter().any(|f| f.path == "a.txt"),
        "the working edit is unstaged"
    );
    assert!(
        ws.staged.iter().any(|f| f.path == "a.txt"),
        "the first edit is staged"
    );
    assert!(
        ws.unstaged
            .iter()
            .any(|f| f.path == "b.txt" && f.status == ChangeStatus::Untracked)
    );

    backend.revert("a.txt").unwrap();

    // The working file rewinds to the *index* (the staged "two" version), not
    // all the way to HEAD: only the unstaged "three" line is discarded.
    assert_eq!(
        fs::read_to_string(dir.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );
    let ws = backend.working_status(false);
    assert!(
        !ws.unstaged.iter().any(|f| f.path == "a.txt"),
        "no unstaged change should remain, got {ws:?}"
    );
    assert!(
        ws.staged.iter().any(|f| f.path == "a.txt"),
        "the staged change must be preserved"
    );

    // The untracked file is left untouched — revert has nothing to restore.
    backend.revert("b.txt").unwrap();
    assert!(
        dir.join("b.txt").exists(),
        "revert must not delete untracked files"
    );
    assert!(
        backend
            .working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "b.txt")
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn git2_stage_commit_amend() {
    let dir = scratch_dir("commit");
    let repo = Repository::init(&dir).unwrap();
    // A repo-local identity so `Git2Backend::commit` can sign without relying
    // on the machine's global git config.
    {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Tester").unwrap();
        cfg.set_str("user.email", "tester@example.com").unwrap();
    }

    fs::write(dir.join("a.txt"), "one\n").unwrap();

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");

    // A brand-new file shows up as untracked and unstaged.
    let ws = backend.working_status(false);
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.unstaged[0].path, "a.txt");
    assert_eq!(ws.unstaged[0].status, ChangeStatus::Untracked);
    assert!(ws.staged.is_empty());

    // Staging moves it to the staged list (now an addition).
    backend.stage("a.txt").unwrap();
    let ws = backend.working_status(false);
    assert!(ws.unstaged.is_empty());
    assert_eq!(ws.staged.len(), 1);
    assert_eq!(ws.staged[0].status, ChangeStatus::Added);

    // Committing creates the root commit; re-open to see fresh history.
    backend.commit("first commit", false).unwrap();
    let reopened = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    assert_eq!(reopened.commits().len(), 1);
    assert_eq!(reopened.commits()[0].summary, "first commit");
    assert!(backend.working_status(false).is_clean());

    // Modify the file: it now diffs as a modification, and the staged diff
    // captures the new line.
    fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    let ws = backend.working_status(false);
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.unstaged[0].status, ChangeStatus::Modified);

    backend.stage("a.txt").unwrap();
    let staged_diff = backend.working_diff("a.txt", true, false);
    assert!(
        staged_diff.lines.iter().any(|l| l.text.contains("two")),
        "staged diff should add 'two', got {:?}",
        staged_diff.lines
    );
    backend.commit("second commit", false).unwrap();

    // With nothing staged, the normal view is clean...
    assert!(backend.working_status(false).is_clean());
    // ...but the amend view re-bases on HEAD's parent, so the second commit's
    // own change to a.txt shows up as staged.
    let amend = backend.working_status(true);
    assert!(
        amend.staged.iter().any(|f| f.path == "a.txt"),
        "amend view should stage HEAD's file, got {:?}",
        amend
    );
    let amend_diff = backend.working_diff("a.txt", true, true);
    assert!(
        amend_diff.lines.iter().any(|l| l.text.contains("two")),
        "amend staged diff should show the committed change, got {:?}",
        amend_diff.lines
    );

    // Pull a.txt out of the amend: it leaves the staged side (reset to HEAD^).
    backend.unstage("a.txt", true).unwrap();
    let amend = backend.working_status(true);
    assert!(
        amend.staged.is_empty(),
        "after unstage, got {:?}",
        amend.staged
    );
    assert!(amend.unstaged.iter().any(|f| f.path == "a.txt"));

    // Re-stage and amend the message (no new commit is created).
    backend.stage("a.txt").unwrap();
    backend.commit("second commit, reworded", true).unwrap();
    let reopened = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    assert_eq!(reopened.commits().len(), 2, "amend must not add a commit");
    assert_eq!(reopened.commits()[0].summary, "second commit, reworded");

    fs::remove_dir_all(&dir).ok();
}
