//! An in-memory [`RepoBackend`] for tests and demos.
//!
//! [`FixtureBackend`] holds a hand-built commit history with deterministic
//! SHAs, timestamps, refs, file lists and diffs, so snapshot tests render
//! identical pixels on every machine without touching a real repository.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::{
    BlobPair, BranchInfo, ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange,
    RefKind, RefLabel, RepoBackend, WorkingStatus,
};

/// A file changed by a commit, paired with the diff that produced it.
pub struct FileEntry {
    pub change: FileChange,
    pub diff: Diff,
}

/// One path in the simulated working tree. Real git tracks separate
/// working-vs-index and index-vs-HEAD diffs; the fixture keeps a single diff
/// per file and a `staged` flag, which is enough to drive and snapshot the
/// commit UI deterministically.
struct WorkingEntry {
    change: FileChange,
    diff: Diff,
    staged: bool,
    /// Raw image bytes for the two sides, when this entry is an image whose
    /// graphical diff a test wants to drive. Empty for ordinary text entries.
    blobs: BlobPair,
}

pub struct FixtureBackend {
    path: String,
    commits: Vec<CommitInfo>,
    files: HashMap<usize, Vec<FileEntry>>,
    /// Branches for review mode, each carrying the aggregated files its
    /// review shows, in the order they were added.
    branches: Vec<(BranchInfo, Vec<FileEntry>)>,
    /// The simulated working tree, mutated by stage/unstage/commit.
    working: RefCell<Vec<WorkingEntry>>,
    /// Paths from the HEAD commit the user has pulled out of an in-progress
    /// amend (so they show as unstaged instead of staged while amending).
    amend_removed: RefCell<HashSet<String>>,
    /// The last commit performed, recorded for test assertions: (message, amend).
    last_commit: RefCell<Option<(String, bool)>>,
    /// The simulated commit identity returned by [`RepoBackend::signature`].
    signature: Option<(String, String)>,
}

impl FixtureBackend {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            commits: Vec::new(),
            files: HashMap::new(),
            branches: Vec::new(),
            working: RefCell::new(Vec::new()),
            amend_removed: RefCell::new(HashSet::new()),
            last_commit: RefCell::new(None),
            signature: Some(("Robert Lillack".to_string(), "rob@example.com".to_string())),
        }
    }

    /// Override the simulated commit identity (or clear it with `None`).
    pub fn with_signature(mut self, signature: Option<(String, String)>) -> Self {
        self.signature = signature;
        self
    }

    /// The files belonging to the `HEAD` commit (index 0, newest first) — the
    /// changes that an amend would re-commit.
    fn head_files(&self) -> &[FileEntry] {
        self.files.get(&0).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The aggregated file entries of the branch named like `branch`.
    fn branch_entries(&self, branch: &BranchInfo) -> Option<&[FileEntry]> {
        self.branches
            .iter()
            .find(|(info, _)| info.name == branch.name)
            .map(|(_, entries)| entries.as_slice())
    }

    /// Append a commit and the files it touched.
    pub fn add_commit(&mut self, info: CommitInfo, files: Vec<FileEntry>) -> &mut Self {
        let idx = self.commits.len();
        self.commits.push(info);
        self.files.insert(idx, files);
        self
    }

    /// Add a path to the simulated working tree (for commit-mode tests/demos).
    pub fn add_working(
        &mut self,
        path: &str,
        status: ChangeStatus,
        staged: bool,
        diff_lines: &[(DiffLineKind, &str)],
    ) -> &mut Self {
        self.working.borrow_mut().push(WorkingEntry {
            change: FileChange {
                path: path.to_string(),
                old_path: None,
                status,
            },
            diff: diff(diff_lines),
            staged,
            blobs: BlobPair::default(),
        });
        self
    }

    /// Add an *image* path to the simulated working tree. The `old`/`new` bytes
    /// are what [`RepoBackend::working_file_blobs`] returns for it, so the
    /// graphical diff can be snapshot-tested deterministically. The text diff is
    /// the usual "binary files differ" stub the image view replaces.
    pub fn add_working_image(
        &mut self,
        path: &str,
        status: ChangeStatus,
        staged: bool,
        old: Option<Vec<u8>>,
        new: Option<Vec<u8>>,
    ) -> &mut Self {
        self.working.borrow_mut().push(WorkingEntry {
            change: FileChange {
                path: path.to_string(),
                old_path: None,
                status,
            },
            diff: Diff {
                lines: vec![DiffLine::new(
                    DiffLineKind::Meta,
                    format!("Binary files a/{path} and b/{path} differ"),
                )],
            },
            staged,
            blobs: BlobPair { old, new },
        });
        self
    }

    /// Add a branch and the aggregated files its review-mode diff shows
    /// (for review-mode tests/demos).
    pub fn add_branch(&mut self, info: BranchInfo, files: Vec<FileEntry>) -> &mut Self {
        self.branches.push((info, files));
        self
    }

    /// The most recent commit recorded via [`RepoBackend::commit`], as
    /// `(message, amend)` — exposed for tests.
    pub fn last_commit(&self) -> Option<(String, bool)> {
        self.last_commit.borrow().clone()
    }

    /// A small, realistic history used by snapshot tests and as a demo when no
    /// real repository is available. Five commits, two refs, varied statuses.
    pub fn sample() -> Self {
        let mut be = FixtureBackend::new("/home/rob/dev/journey");

        be.add_commit(
            commit(
                "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
                "Add commit DAG graph view",
                "Robert Lillack",
                "rob@example.com",
                1_716_500_000,
                120,
                &["b2c3d4e5f60718293a4b5c6d7e8f90123456789a"],
                &[("main", RefKind::Head)],
            ),
            vec![file_entry(
                "src/widgets/graph.rs",
                None,
                ChangeStatus::Added,
                &[
                    (
                        DiffLineKind::FileHeader,
                        "diff --git a/src/widgets/graph.rs b/src/widgets/graph.rs",
                    ),
                    (DiffLineKind::FileHeader, "new file mode 100644"),
                    (DiffLineKind::HunkHeader, "@@ -0,0 +1,3 @@"),
                    (DiffLineKind::Addition, "+pub struct Graph {"),
                    (DiffLineKind::Addition, "+    lanes: Vec<Lane>,"),
                    (DiffLineKind::Addition, "+}"),
                ],
            )],
        );

        be.add_commit(
            commit(
                "b2c3d4e5f60718293a4b5c6d7e8f90123456789a",
                "Build basic file list per commit",
                "Robert Lillack",
                "rob@example.com",
                1_716_400_000,
                120,
                &["c3d4e5f60718293a4b5c6d7e8f90123456789ab2"],
                &[("v0.2", RefKind::Tag)],
            ),
            vec![
                file_entry(
                    "src/backend.rs",
                    None,
                    ChangeStatus::Modified,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/src/backend.rs b/src/backend.rs",
                        ),
                        (
                            DiffLineKind::HunkHeader,
                            "@@ -10,6 +10,10 @@ impl Backend {",
                        ),
                        (
                            DiffLineKind::Context,
                            "     pub fn log(&self) -> Vec<Commit> {",
                        ),
                        (
                            DiffLineKind::Addition,
                            "+        // collect changed files too",
                        ),
                        (DiffLineKind::Addition, "+        self.changed_files();"),
                        (DiffLineKind::Context, "         self.commits.clone()"),
                        (DiffLineKind::Context, "     }"),
                    ],
                ),
                file_entry(
                    "src/main.rs",
                    None,
                    ChangeStatus::Modified,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/src/main.rs b/src/main.rs",
                        ),
                        (DiffLineKind::HunkHeader, "@@ -42,7 +42,7 @@"),
                        (DiffLineKind::Deletion, "-    let files = vec![];"),
                        (
                            DiffLineKind::Addition,
                            "+    let files = backend.changed_files(idx);",
                        ),
                    ],
                ),
            ],
        );

        be.add_commit(
            commit(
                "c3d4e5f60718293a4b5c6d7e8f90123456789ab2",
                "Show path in title",
                "Robert Lillack",
                "rob@example.com",
                1_716_300_000,
                120,
                &["d4e5f60718293a4b5c6d7e8f90123456789ab2c3"],
                &[],
            ),
            vec![file_entry(
                "src/main.rs",
                None,
                ChangeStatus::Modified,
                &[
                    (
                        DiffLineKind::FileHeader,
                        "diff --git a/src/main.rs b/src/main.rs",
                    ),
                    (DiffLineKind::HunkHeader, "@@ -20,1 +20,1 @@ fn title()"),
                    (DiffLineKind::Deletion, "-        String::from(\"Journey\")"),
                    (
                        DiffLineKind::Addition,
                        "+        format!(\"Journey: {}\", path)",
                    ),
                ],
            )],
        );

        be.add_commit(
            commit(
                "d4e5f60718293a4b5c6d7e8f90123456789ab2c3",
                "Rename boldFont() -> bold_font()",
                "Robert Lillack",
                "rob@example.com",
                1_716_200_000,
                120,
                &["e5f60718293a4b5c6d7e8f90123456789ab2c3d4"],
                &[("origin/main", RefKind::RemoteBranch)],
            ),
            vec![file_entry(
                "src/style.rs",
                None,
                ChangeStatus::Modified,
                &[
                    (
                        DiffLineKind::FileHeader,
                        "diff --git a/src/style.rs b/src/style.rs",
                    ),
                    (DiffLineKind::HunkHeader, "@@ -80,4 +80,4 @@"),
                    (DiffLineKind::Deletion, "-pub fn boldFont() -> Font {"),
                    (DiffLineKind::Addition, "+pub fn bold_font() -> Font {"),
                ],
            )],
        );

        be.add_commit(
            commit(
                "e5f60718293a4b5c6d7e8f90123456789ab2c3d4",
                "Initial import",
                "Robert Lillack",
                "rob@example.com",
                1_716_100_000,
                120,
                &[],
                &[],
            ),
            vec![
                file_entry(
                    "Cargo.toml",
                    None,
                    ChangeStatus::Added,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/Cargo.toml b/Cargo.toml",
                        ),
                        (DiffLineKind::FileHeader, "new file mode 100644"),
                        (DiffLineKind::HunkHeader, "@@ -0,0 +1,2 @@"),
                        (DiffLineKind::Addition, "+[package]"),
                        (DiffLineKind::Addition, "+name = \"journey\""),
                    ],
                ),
                file_entry(
                    "src/main.rs",
                    None,
                    ChangeStatus::Added,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/src/main.rs b/src/main.rs",
                        ),
                        (DiffLineKind::FileHeader, "new file mode 100644"),
                        (DiffLineKind::HunkHeader, "@@ -0,0 +1,1 @@"),
                        (DiffLineKind::Addition, "+fn main() {}"),
                    ],
                ),
            ],
        );

        // A working tree with a realistic mix of staged and unstaged changes,
        // so commit mode has something to show.
        be.add_working(
            "src/ui.rs",
            ChangeStatus::Modified,
            false,
            &[
                (
                    DiffLineKind::FileHeader,
                    "diff --git a/src/ui.rs b/src/ui.rs",
                ),
                (
                    DiffLineKind::HunkHeader,
                    "@@ -40,6 +40,9 @@ impl GitClient {",
                ),
                (DiffLineKind::Context, "     fn sync(&mut self) {"),
                (
                    DiffLineKind::Addition,
                    "+        // refresh the working-tree panes",
                ),
                (DiffLineKind::Addition, "+        self.rescan();"),
                (DiffLineKind::Context, "         self.repaint();"),
                (DiffLineKind::Context, "     }"),
            ],
        );
        be.add_working(
            "notes.md",
            ChangeStatus::Untracked,
            false,
            &[
                (DiffLineKind::FileHeader, "diff --git a/notes.md b/notes.md"),
                (DiffLineKind::FileHeader, "new file mode 100644"),
                (DiffLineKind::HunkHeader, "@@ -0,0 +1,2 @@"),
                (DiffLineKind::Addition, "+# Notes"),
                (DiffLineKind::Addition, "+- wire up commit mode"),
            ],
        );
        be.add_working(
            "src/widgets/commit_panel.rs",
            ChangeStatus::Added,
            true,
            &[
                (
                    DiffLineKind::FileHeader,
                    "diff --git a/src/widgets/commit_panel.rs b/src/widgets/commit_panel.rs",
                ),
                (DiffLineKind::FileHeader, "new file mode 100644"),
                (DiffLineKind::HunkHeader, "@@ -0,0 +1,3 @@"),
                (DiffLineKind::Addition, "+pub struct CommitPanel {"),
                (DiffLineKind::Addition, "+    message: String,"),
                (DiffLineKind::Addition, "+}"),
            ],
        );
        be.add_working(
            "Cargo.toml",
            ChangeStatus::Modified,
            true,
            &[
                (
                    DiffLineKind::FileHeader,
                    "diff --git a/Cargo.toml b/Cargo.toml",
                ),
                (
                    DiffLineKind::HunkHeader,
                    "@@ -8,3 +8,4 @@ edition = \"2024\"",
                ),
                (DiffLineKind::Context, " [dependencies]"),
                (
                    DiffLineKind::Addition,
                    "+git2 = { version = \"0.18\", default-features = false }",
                ),
                (
                    DiffLineKind::Context,
                    " saudade = { path = \"../saudade\" }",
                ),
            ],
        );

        // Branches for review mode. `main` is in sync with its tracked
        // `origin/main`, so the two fold into one row; the feature branch
        // carries an aggregated diff of everything it contains; the
        // remote-only branch (an already-merged ancestor of main) reviews as
        // empty.
        be.add_branch(
            BranchInfo {
                name: "main".into(),
                kind: RefKind::Head,
                tip_id: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".into(),
                summary: "Add commit DAG graph view".into(),
                author: "Robert Lillack".into(),
                time_seconds: 1_716_500_000,
                time_offset_minutes: 120,
                upstream: Some("origin/main".into()),
                base_name: "main".into(),
                base_id: Some("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".into()),
            },
            vec![],
        );
        be.add_branch(
            BranchInfo {
                name: "feature/list-icons".into(),
                kind: RefKind::LocalBranch,
                tip_id: "f60718293a4b5c6d7e8f90123456789ab2c3d4e5".into(),
                summary: "Bake SVG status markers".into(),
                author: "Robert Lillack".into(),
                time_seconds: 1_716_450_000,
                time_offset_minutes: 120,
                upstream: None,
                base_name: "main".into(),
                base_id: Some("c3d4e5f60718293a4b5c6d7e8f90123456789ab2".into()),
            },
            vec![
                file_entry(
                    "assets/status/added.svg",
                    None,
                    ChangeStatus::Added,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/assets/status/added.svg b/assets/status/added.svg",
                        ),
                        (DiffLineKind::FileHeader, "new file mode 100644"),
                        (DiffLineKind::HunkHeader, "@@ -0,0 +1,2 @@"),
                        (DiffLineKind::Addition, "+<svg viewBox=\"0 0 12 12\">"),
                        (DiffLineKind::Addition, "+  <rect rx=\"2\" fill=\"#2A2\"/>"),
                    ],
                ),
                file_entry(
                    "src/widgets/list.rs",
                    None,
                    ChangeStatus::Modified,
                    &[
                        (
                            DiffLineKind::FileHeader,
                            "diff --git a/src/widgets/list.rs b/src/widgets/list.rs",
                        ),
                        (
                            DiffLineKind::HunkHeader,
                            "@@ -12,5 +12,6 @@ impl ListItem {",
                        ),
                        (
                            DiffLineKind::Context,
                            "     pub fn new(text: &str) -> Self {",
                        ),
                        (
                            DiffLineKind::Deletion,
                            "-        Self { text: text.into() }",
                        ),
                        (
                            DiffLineKind::Addition,
                            "+        Self { text: text.into(), icon: None }",
                        ),
                        (DiffLineKind::Context, "     }"),
                        (DiffLineKind::Context, " }"),
                    ],
                ),
            ],
        );
        be.add_branch(
            BranchInfo {
                name: "origin/font-rename".into(),
                kind: RefKind::RemoteBranch,
                tip_id: "d4e5f60718293a4b5c6d7e8f90123456789ab2c3".into(),
                summary: "Rename boldFont() -> bold_font()".into(),
                author: "Robert Lillack".into(),
                time_seconds: 1_716_200_000,
                time_offset_minutes: 120,
                upstream: None,
                base_name: "main".into(),
                base_id: Some("d4e5f60718293a4b5c6d7e8f90123456789ab2c3".into()),
            },
            vec![],
        );

        be
    }
}

impl RepoBackend for FixtureBackend {
    fn path(&self) -> &str {
        &self.path
    }

    fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    fn changed_files(&self, index: usize) -> Vec<FileChange> {
        self.files
            .get(&index)
            .map(|entries| entries.iter().map(|e| e.change.clone()).collect())
            .unwrap_or_default()
    }

    fn commit_diff(&self, index: usize) -> Diff {
        let mut lines = Vec::new();
        if let Some(entries) = self.files.get(&index) {
            for entry in entries {
                lines.extend(entry.diff.lines.iter().cloned());
            }
        }
        Diff { lines }
    }

    fn file_diff(&self, index: usize, path: &str) -> Diff {
        self.files
            .get(&index)
            .and_then(|entries| entries.iter().find(|e| e.change.path == path))
            .map(|e| e.diff.clone())
            .unwrap_or_default()
    }

    fn branches(&self) -> Vec<BranchInfo> {
        self.branches.iter().map(|(info, _)| info.clone()).collect()
    }

    fn branch_files(&self, branch: &BranchInfo) -> Vec<FileChange> {
        self.branch_entries(branch)
            .map(|entries| entries.iter().map(|e| e.change.clone()).collect())
            .unwrap_or_default()
    }

    fn branch_diff(&self, branch: &BranchInfo) -> Diff {
        let mut lines = Vec::new();
        if let Some(entries) = self.branch_entries(branch) {
            for entry in entries {
                lines.extend(entry.diff.lines.iter().cloned());
            }
        }
        Diff { lines }
    }

    fn branch_file_diff(&self, branch: &BranchInfo, path: &str) -> Diff {
        self.branch_entries(branch)
            .and_then(|entries| entries.iter().find(|e| e.change.path == path))
            .map(|e| e.diff.clone())
            .unwrap_or_default()
    }

    fn working_status(&self, amend: bool) -> WorkingStatus {
        let mut status = WorkingStatus::default();
        for entry in self.working.borrow().iter() {
            if entry.staged {
                status.staged.push(entry.change.clone());
            } else {
                status.unstaged.push(entry.change.clone());
            }
        }
        // When amending, the HEAD commit's files join the staged side (they'll
        // be re-committed) unless the user has pulled them back out.
        if amend {
            let removed = self.amend_removed.borrow();
            for fe in self.head_files() {
                if removed.contains(&fe.change.path) {
                    status.unstaged.push(fe.change.clone());
                } else {
                    status.staged.push(fe.change.clone());
                }
            }
        }
        status
    }

    fn working_diff(&self, path: &str, _staged: bool, amend: bool) -> Diff {
        // The simulation keeps a single diff per path, so the staged/unstaged
        // side doesn't change which diff we show.
        if let Some(diff) = self
            .working
            .borrow()
            .iter()
            .find(|e| e.change.path == path)
            .map(|e| e.diff.clone())
        {
            return diff;
        }
        if amend
            && let Some(diff) = self
                .head_files()
                .iter()
                .find(|fe| fe.change.path == path)
                .map(|fe| fe.diff.clone())
        {
            return diff;
        }
        Diff::default()
    }

    fn working_file_blobs(&self, path: &str, _staged: bool, _amend: bool) -> BlobPair {
        // Like `working_diff`, the simulation keeps one set of bytes per path
        // regardless of the staged/unstaged side.
        self.working
            .borrow()
            .iter()
            .find(|e| e.change.path == path)
            .map(|e| e.blobs.clone())
            .unwrap_or_default()
    }

    fn stage(&self, path: &str) -> Result<(), String> {
        let mut found = false;
        for entry in self.working.borrow_mut().iter_mut() {
            if entry.change.path == path {
                entry.staged = true;
                found = true;
            }
        }
        // Re-staging a HEAD file that had been pulled out of an amend.
        if !found {
            self.amend_removed.borrow_mut().remove(path);
        }
        Ok(())
    }

    fn unstage(&self, path: &str, amend: bool) -> Result<(), String> {
        let mut found = false;
        for entry in self.working.borrow_mut().iter_mut() {
            if entry.change.path == path {
                entry.staged = false;
                found = true;
            }
        }
        // Dropping a HEAD file from the commit being amended.
        if !found && amend {
            self.amend_removed.borrow_mut().insert(path.to_string());
        }
        Ok(())
    }

    fn revert(&self, path: &str) -> Result<(), String> {
        // Drop the unstaged (working-vs-index) entry for this path, the
        // simulation's stand-in for restoring it from the index. Staged entries
        // and untracked files are left alone, matching the live backend.
        self.working.borrow_mut().retain(|e| {
            !(e.change.path == path && !e.staged && e.change.status != ChangeStatus::Untracked)
        });
        Ok(())
    }

    fn delete_untracked(&self, path: &str) -> Result<(), String> {
        // Removing the untracked file takes it out of the simulated working
        // tree entirely.
        self.working.borrow_mut().retain(|e| {
            !(e.change.path == path && !e.staged && e.change.status == ChangeStatus::Untracked)
        });
        Ok(())
    }

    fn apply_to_index(&self, _patch: &str) -> Result<(), String> {
        // The simulation keeps only a whole-file `staged` flag and no file
        // contents, so it can't model staging a subset of lines. Accept the
        // patch as a no-op so commit-mode UI tests can exercise the Stage/
        // Unstage button without a spurious error dialog; partial-staging
        // correctness is covered against the live backend instead.
        Ok(())
    }

    fn commit(&self, message: &str, amend: bool) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("Please enter a commit message.".into());
        }
        // The staged changes are now part of HEAD; drop them from the
        // working set so the panes clear after committing.
        self.working.borrow_mut().retain(|e| !e.staged);
        self.amend_removed.borrow_mut().clear();
        *self.last_commit.borrow_mut() = Some((message.to_string(), amend));
        Ok(())
    }

    fn head_message(&self) -> Option<String> {
        self.commits.first().map(|c| c.message.clone())
    }

    fn signature(&self) -> Option<(String, String)> {
        self.signature.clone()
    }
}

/// Build a [`CommitInfo`] without the ceremony of naming every field.
#[allow(clippy::too_many_arguments)]
pub fn commit(
    id: &str,
    summary: &str,
    author: &str,
    email: &str,
    time_seconds: i64,
    time_offset_minutes: i32,
    parents: &[&str],
    refs: &[(&str, RefKind)],
) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.chars().take(8).collect(),
        summary: summary.to_string(),
        message: format!("{summary}\n"),
        author_name: author.to_string(),
        author_email: email.to_string(),
        committer_name: author.to_string(),
        committer_email: email.to_string(),
        time_seconds,
        time_offset_minutes,
        parents: parents.iter().map(|p| p.to_string()).collect(),
        refs: refs
            .iter()
            .map(|(name, kind)| RefLabel {
                name: name.to_string(),
                kind: *kind,
            })
            .collect(),
    }
}

fn file_entry(
    path: &str,
    old_path: Option<&str>,
    status: ChangeStatus,
    diff_lines: &[(DiffLineKind, &str)],
) -> FileEntry {
    FileEntry {
        change: FileChange {
            path: path.to_string(),
            old_path: old_path.map(str::to_string),
            status,
        },
        diff: diff(diff_lines),
    }
}

/// Build a [`Diff`] from `(kind, text)` pairs.
fn diff(lines: &[(DiffLineKind, &str)]) -> Diff {
    Diff {
        lines: lines
            .iter()
            .map(|(kind, text)| DiffLine::new(*kind, text.to_string()))
            .collect(),
    }
}
