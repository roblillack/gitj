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

    let ws = be.working_status();
    assert_eq!(ws.unstaged.len(), 2, "sample has two unstaged files");
    assert_eq!(ws.staged.len(), 2, "sample has two staged files");

    // Stage an unstaged file: it moves to the staged list.
    be.stage("src/ui.rs").unwrap();
    let ws = be.working_status();
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.staged.len(), 3);
    assert!(ws.staged.iter().any(|f| f.path == "src/ui.rs"));

    // Unstage a staged file: it moves back.
    be.unstage("Cargo.toml").unwrap();
    let ws = be.working_status();
    assert!(ws.unstaged.iter().any(|f| f.path == "Cargo.toml"));
    assert!(!ws.staged.iter().any(|f| f.path == "Cargo.toml"));

    // Commit clears the staged set and records the message.
    be.commit("wire up commit mode", false).unwrap();
    assert_eq!(
        be.last_commit(),
        Some(("wire up commit mode".to_string(), false))
    );
    assert!(be.working_status().staged.is_empty());

    // An empty message is rejected.
    assert!(be.commit("   ", false).is_err());
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
    let ws = backend.working_status();
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.unstaged[0].path, "a.txt");
    assert_eq!(ws.unstaged[0].status, ChangeStatus::Untracked);
    assert!(ws.staged.is_empty());

    // Staging moves it to the staged list (now an addition).
    backend.stage("a.txt").unwrap();
    let ws = backend.working_status();
    assert!(ws.unstaged.is_empty());
    assert_eq!(ws.staged.len(), 1);
    assert_eq!(ws.staged[0].status, ChangeStatus::Added);

    // Committing creates the root commit; re-open to see fresh history.
    backend.commit("first commit", false).unwrap();
    let reopened = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    assert_eq!(reopened.commits().len(), 1);
    assert_eq!(reopened.commits()[0].summary, "first commit");
    assert!(backend.working_status().is_clean());

    // Modify the file: it now diffs as a modification, and the staged diff
    // captures the new line.
    fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    let ws = backend.working_status();
    assert_eq!(ws.unstaged.len(), 1);
    assert_eq!(ws.unstaged[0].status, ChangeStatus::Modified);

    backend.stage("a.txt").unwrap();
    let staged_diff = backend.working_diff("a.txt", true);
    assert!(
        staged_diff.lines.iter().any(|l| l.text.contains("two")),
        "staged diff should add 'two', got {:?}",
        staged_diff.lines
    );
    backend.commit("second commit", false).unwrap();

    // Amend the second commit's message (no new commit is created).
    backend.commit("second commit, reworded", true).unwrap();
    let reopened = Git2Backend::open(dir.to_str().unwrap()).unwrap();
    assert_eq!(reopened.commits().len(), 2, "amend must not add a commit");
    assert_eq!(reopened.commits()[0].summary, "second commit, reworded");

    fs::remove_dir_all(&dir).ok();
}
