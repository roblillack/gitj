//! The repository abstraction journey's UI talks to.
//!
//! The UI never touches `git2` directly — it goes through [`RepoBackend`].
//! That keeps the widget code testable: snapshot tests render the real UI
//! against a deterministic [`fixture::FixtureBackend`] instead of needing a
//! live repository with machine-dependent SHAs and timestamps.

mod git2_backend;
pub mod fixture;

pub use fixture::FixtureBackend;
pub use git2_backend::Git2Backend;

/// What a ref pointing at a commit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// The currently checked-out branch (drawn specially).
    Head,
    /// A detached `HEAD` sitting directly on the commit.
    DetachedHead,
    /// A local branch.
    LocalBranch,
    /// A remote-tracking branch (`origin/main`).
    RemoteBranch,
    /// A tag.
    Tag,
}

/// A branch / tag / HEAD label attached to a commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

/// How a file changed in a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Other,
}

impl ChangeStatus {
    /// Single-letter status badge as gitk / `git status --short` show it.
    pub fn badge(self) -> char {
        match self {
            ChangeStatus::Added => 'A',
            ChangeStatus::Modified => 'M',
            ChangeStatus::Deleted => 'D',
            ChangeStatus::Renamed => 'R',
            ChangeStatus::Copied => 'C',
            ChangeStatus::TypeChange => 'T',
            ChangeStatus::Other => '?',
        }
    }
}

/// One changed path in a commit's diff against its first parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Set for renames/copies: the path the file had before.
    pub old_path: Option<String>,
    pub status: ChangeStatus,
}

impl FileChange {
    /// Display form: `old -> new` for renames, otherwise just the path.
    pub fn display(&self) -> String {
        match (&self.old_path, self.status) {
            (Some(old), ChangeStatus::Renamed | ChangeStatus::Copied) if old != &self.path => {
                format!("{old} -> {}", self.path)
            }
            _ => self.path.clone(),
        }
    }
}

/// The semantic class of a single line in a unified diff. Drives coloring in
/// the [`DiffView`](crate::widgets::DiffView) widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `commit <sha>` / `Author:` / `Date:` metadata, as `git show` prints
    /// above the diff. Produced by the UI's commit-detail builder, never by
    /// the raw diff renderer.
    CommitHeader,
    /// `diff --git …`, `index …`, `--- a/…`, `+++ b/…` — file framing.
    FileHeader,
    /// `@@ -a,b +c,d @@` hunk header.
    HunkHeader,
    /// Unchanged context line.
    Context,
    /// `+` added line.
    Addition,
    /// `-` removed line.
    Deletion,
    /// Anything else git emits ("\ No newline at end of file", binary notes).
    Meta,
}

/// One rendered line of a unified diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

impl DiffLine {
    pub fn new(kind: DiffLineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// A whole diff, ready to render line-by-line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Everything the UI needs to show about a single commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Full 40-char hex SHA.
    pub id: String,
    /// Abbreviated SHA (first 8 hex chars) for compact display.
    pub short_id: String,
    /// First line of the message.
    pub summary: String,
    /// Full commit message.
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
    /// Author time, seconds since the Unix epoch.
    pub time_seconds: i64,
    /// Author timezone offset in minutes east of UTC.
    pub time_offset_minutes: i32,
    /// Parent SHAs (more than one for merge commits).
    pub parents: Vec<String>,
    /// Branch / tag / HEAD labels that point at this commit.
    pub refs: Vec<RefLabel>,
}

impl CommitInfo {
    /// `2026-05-29 23:10:42` in the commit's own timezone.
    pub fn date_string(&self) -> String {
        format_git_time(self.time_seconds, self.time_offset_minutes)
    }

    /// Short author date `2026-05-29 23:10` for the list row.
    pub fn short_date_string(&self) -> String {
        let full = self.date_string();
        full.get(..16).unwrap_or(&full).to_string()
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// The interface the UI layer depends on. Implemented by the live
/// [`Git2Backend`] and the in-memory [`FixtureBackend`].
pub trait RepoBackend {
    /// Human-readable path to the repository (shown in the title bar).
    fn path(&self) -> &str;

    /// All commits, newest first (reverse-topological, like `git log`).
    fn commits(&self) -> &[CommitInfo];

    /// Files changed by the commit at `index`, against its first parent.
    fn changed_files(&self, index: usize) -> Vec<FileChange>;

    /// Unified diff of the whole commit against its first parent.
    fn commit_diff(&self, index: usize) -> Diff;

    /// Unified diff for a single file within the commit.
    fn file_diff(&self, index: usize, path: &str) -> Diff;
}

/// Format a Unix timestamp (+ minute offset) as `YYYY-MM-DD HH:MM:SS ±HHMM`
/// in the given timezone, with no external date crate. Uses Howard Hinnant's
/// civil-from-days algorithm so it is correct for the full proleptic
/// Gregorian range.
pub fn format_git_time(seconds: i64, offset_minutes: i32) -> String {
    let local = seconds + offset_minutes as i64 * 60;
    let days = local.div_euclid(86_400);
    let secs_of_day = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let off = offset_minutes.abs();
    format!(
        "{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} {sign}{:02}{:02}",
        off / 60,
        off % 60,
    )
}

/// Convert a count of days since 1970-01-01 to a (year, month, day) triple.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod time_tests {
    use super::format_git_time;

    #[test]
    fn formats_known_timestamps() {
        // 2021-01-01 00:00:00 UTC
        assert_eq!(format_git_time(1_609_459_200, 0), "2021-01-01 00:00:00 +0000");
        // The Unix epoch itself.
        assert_eq!(format_git_time(0, 0), "1970-01-01 00:00:00 +0000");
        // With a +02:00 offset the wall clock advances two hours.
        assert_eq!(
            format_git_time(1_609_459_200, 120),
            "2021-01-01 02:00:00 +0200"
        );
        // Negative offset rolls the date back across midnight.
        assert_eq!(
            format_git_time(1_609_459_200, -120),
            "2020-12-31 22:00:00 -0200"
        );
    }
}
