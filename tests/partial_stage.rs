//! Integration test for partial (line-range) staging against the live
//! [`Git2Backend`]: build a patch covering a subset of a file's changes with
//! [`build_partial_patch`], apply it to the index via
//! [`RepoBackend::apply_to_index`], and confirm only the selected change moved.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Repository, Signature, Time};
use journey::backend::{
    Diff, DiffLineKind, Git2Backend, PartialMode, RepoBackend, build_partial_patch,
};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("journey-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Indices of the change (`+`/`-`) rows in `diff` whose text contains `needle`.
fn change_rows_matching(diff: &Diff, needle: &str) -> BTreeSet<usize> {
    diff.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            matches!(l.kind, DiffLineKind::Addition | DiffLineKind::Deletion)
                && l.text.contains(needle)
        })
        .map(|(i, _)| i)
        .collect()
}

fn has_change(diff: &Diff, needle: &str) -> bool {
    diff.lines.iter().any(|l| {
        matches!(l.kind, DiffLineKind::Addition | DiffLineKind::Deletion) && l.text.contains(needle)
    })
}

#[test]
fn stages_and_unstages_a_single_change_among_two() {
    let dir = scratch_dir("partial");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 0)).unwrap();

    // Commit a 20-line file so two edits land in two separate hunks.
    let base: String = (1..=20).map(|n| format!("l{n:02}\n")).collect();
    fs::write(dir.join("a.txt"), &base).unwrap();
    {
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "base\n", &tree, &[])
            .unwrap();
    }

    // Edit line 2 and line 18 in the working tree (two independent changes).
    let edited = base
        .replace("l02\n", "l02-edited\n")
        .replace("l18\n", "l18-edited\n");
    fs::write(dir.join("a.txt"), &edited).unwrap();

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");

    // Two distinct changes show up unstaged.
    let unstaged = backend.working_diff("a.txt", false, false);
    assert!(has_change(&unstaged, "l02-edited"));
    assert!(has_change(&unstaged, "l18-edited"));
    assert!(
        backend.working_diff("a.txt", true, false).is_empty(),
        "nothing staged yet"
    );

    // Stage only the line-2 change (both its `-` and `+` rows).
    let mut sel = change_rows_matching(&unstaged, "l02");
    sel.extend(change_rows_matching(&unstaged, "l02-edited"));
    let patch = build_partial_patch(&unstaged, &sel, PartialMode::Stage).expect("patch");
    backend.apply_to_index(&patch).expect("apply stage");

    // The line-2 change is now staged; line 18 stays unstaged.
    let staged = backend.working_diff("a.txt", true, false);
    assert!(has_change(&staged, "l02-edited"), "line 2 should be staged");
    assert!(
        !has_change(&staged, "l18-edited"),
        "line 18 must not be staged"
    );
    let still_unstaged = backend.working_diff("a.txt", false, false);
    assert!(
        has_change(&still_unstaged, "l18-edited"),
        "line 18 still unstaged"
    );
    assert!(
        !has_change(&still_unstaged, "l02-edited"),
        "line 2 no longer unstaged"
    );

    // Now unstage the line-2 change back out of the index.
    let mut sel = change_rows_matching(&staged, "l02");
    sel.extend(change_rows_matching(&staged, "l02-edited"));
    let patch = build_partial_patch(&staged, &sel, PartialMode::Unstage).expect("patch");
    backend.apply_to_index(&patch).expect("apply unstage");

    assert!(
        backend.working_diff("a.txt", true, false).is_empty(),
        "index back to clean after unstage"
    );
    assert!(
        has_change(&backend.working_diff("a.txt", false, false), "l02-edited"),
        "line 2 back to unstaged"
    );

    fs::remove_dir_all(&dir).ok();
}
