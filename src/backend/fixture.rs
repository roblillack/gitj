//! An in-memory [`RepoBackend`] for tests and demos.
//!
//! [`FixtureBackend`] holds a hand-built commit history with deterministic
//! SHAs, timestamps, refs, file lists and diffs, so snapshot tests render
//! identical pixels on every machine without touching a real repository.

use std::collections::HashMap;

use super::{
    ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RefKind, RefLabel,
    RepoBackend,
};

/// A file changed by a commit, paired with the diff that produced it.
pub struct FileEntry {
    pub change: FileChange,
    pub diff: Diff,
}

pub struct FixtureBackend {
    path: String,
    commits: Vec<CommitInfo>,
    files: HashMap<usize, Vec<FileEntry>>,
}

impl FixtureBackend {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            commits: Vec::new(),
            files: HashMap::new(),
        }
    }

    /// Append a commit and the files it touched.
    pub fn add_commit(&mut self, info: CommitInfo, files: Vec<FileEntry>) -> &mut Self {
        let idx = self.commits.len();
        self.commits.push(info);
        self.files.insert(idx, files);
        self
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
            vec![
                file_entry(
                    "src/widgets/graph.rs",
                    None,
                    ChangeStatus::Added,
                    &[
                        (DiffLineKind::FileHeader, "diff --git a/src/widgets/graph.rs b/src/widgets/graph.rs"),
                        (DiffLineKind::FileHeader, "new file mode 100644"),
                        (DiffLineKind::HunkHeader, "@@ -0,0 +1,3 @@"),
                        (DiffLineKind::Addition, "+pub struct Graph {"),
                        (DiffLineKind::Addition, "+    lanes: Vec<Lane>,"),
                        (DiffLineKind::Addition, "+}"),
                    ],
                ),
            ],
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
                        (DiffLineKind::FileHeader, "diff --git a/src/backend.rs b/src/backend.rs"),
                        (DiffLineKind::HunkHeader, "@@ -10,6 +10,10 @@ impl Backend {"),
                        (DiffLineKind::Context, "     pub fn log(&self) -> Vec<Commit> {"),
                        (DiffLineKind::Addition, "+        // collect changed files too"),
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
                        (DiffLineKind::FileHeader, "diff --git a/src/main.rs b/src/main.rs"),
                        (DiffLineKind::HunkHeader, "@@ -42,7 +42,7 @@"),
                        (DiffLineKind::Deletion, "-    let files = vec![];"),
                        (DiffLineKind::Addition, "+    let files = backend.changed_files(idx);"),
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
                    (DiffLineKind::FileHeader, "diff --git a/src/main.rs b/src/main.rs"),
                    (DiffLineKind::HunkHeader, "@@ -20,1 +20,1 @@ fn title()"),
                    (DiffLineKind::Deletion, "-        String::from(\"Journey\")"),
                    (DiffLineKind::Addition, "+        format!(\"Journey: {}\", path)"),
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
                    (DiffLineKind::FileHeader, "diff --git a/src/style.rs b/src/style.rs"),
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
                        (DiffLineKind::FileHeader, "diff --git a/Cargo.toml b/Cargo.toml"),
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
                        (DiffLineKind::FileHeader, "diff --git a/src/main.rs b/src/main.rs"),
                        (DiffLineKind::FileHeader, "new file mode 100644"),
                        (DiffLineKind::HunkHeader, "@@ -0,0 +1,1 @@"),
                        (DiffLineKind::Addition, "+fn main() {}"),
                    ],
                ),
            ],
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
        diff: Diff {
            lines: diff_lines
                .iter()
                .map(|(kind, text)| DiffLine::new(*kind, text.to_string()))
                .collect(),
        },
    }
}
