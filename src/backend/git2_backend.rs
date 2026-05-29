//! The live [`RepoBackend`] backed by `git2` / libgit2.

use std::collections::HashMap;

use git2::{Commit, Delta, DiffFormat, DiffLineType, DiffOptions, Oid, Repository, Sort};

use super::{
    ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RefKind, RefLabel,
    RepoBackend,
};

/// Opens a repository and reads commits/diffs through libgit2.
pub struct Git2Backend {
    path: String,
    repo: Repository,
    commits: Vec<CommitInfo>,
}

impl Git2Backend {
    /// Open the repository at (or above) `path` and load its commit history.
    pub fn open(path: impl AsRef<str>) -> Result<Self, git2::Error> {
        let path = path.as_ref().to_string();
        let repo = Repository::discover(&path)?;
        // Prefer the repository's working-directory path for display; fall
        // back to the .git dir for bare repos.
        let display_path = repo
            .workdir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| repo.path().display().to_string());

        let refs = collect_refs(&repo)?;
        let commits = load_commits(&repo, &refs)?;

        Ok(Self {
            path: display_path,
            repo,
            commits,
        })
    }

    fn commit_at(&self, index: usize) -> Option<Commit<'_>> {
        let info = self.commits.get(index)?;
        let oid = Oid::from_str(&info.id).ok()?;
        self.repo.find_commit(oid).ok()
    }

    /// Build a libgit2 diff for a commit against its first parent, optionally
    /// restricted to a single path. Renames are detected so the file list can
    /// show `old -> new`.
    fn build_diff(&self, index: usize, path: Option<&str>) -> Option<git2::Diff<'_>> {
        let commit = self.commit_at(index)?;
        let new_tree = commit.tree().ok()?;
        let parent_tree = commit
            .parent(0)
            .ok()
            .and_then(|p| p.tree().ok());

        let mut opts = DiffOptions::new();
        opts.context_lines(3);
        if let Some(path) = path {
            opts.pathspec(path);
        }

        let mut diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&new_tree), Some(&mut opts))
            .ok()?;
        // Detect renames/copies so statuses and headers are accurate.
        let _ = diff.find_similar(None);
        Some(diff)
    }
}

impl RepoBackend for Git2Backend {
    fn path(&self) -> &str {
        &self.path
    }

    fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    fn changed_files(&self, index: usize) -> Vec<FileChange> {
        let Some(diff) = self.build_diff(index, None) else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let new_path = delta
                .new_file()
                .path()
                .map(|p| p.display().to_string());
            let old_path = delta
                .old_file()
                .path()
                .map(|p| p.display().to_string());
            let status = status_from_delta(delta.status());
            let path = new_path
                .clone()
                .or_else(|| old_path.clone())
                .unwrap_or_default();
            files.push(FileChange {
                path,
                old_path: old_path.filter(|o| Some(o) != new_path.as_ref()),
                status,
            });
        }
        files
    }

    fn commit_diff(&self, index: usize) -> Diff {
        self.build_diff(index, None)
            .map(render_diff)
            .unwrap_or_default()
    }

    fn file_diff(&self, index: usize, path: &str) -> Diff {
        self.build_diff(index, Some(path))
            .map(render_diff)
            .unwrap_or_default()
    }
}

/// Walk all references once and group branch/tag labels by the commit they
/// resolve to. The currently checked-out branch is tagged [`RefKind::Head`];
/// a detached HEAD becomes a [`RefKind::DetachedHead`] label.
fn collect_refs(repo: &Repository) -> Result<HashMap<Oid, Vec<RefLabel>>, git2::Error> {
    let mut map: HashMap<Oid, Vec<RefLabel>> = HashMap::new();

    let head = repo.head().ok();
    let head_branch = head
        .as_ref()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand())
        .map(str::to_string);
    let detached = repo.head_detached().unwrap_or(false);

    if detached
        && let Some(oid) = head.as_ref().and_then(|h| h.target())
    {
        map.entry(oid).or_default().push(RefLabel {
            name: "HEAD".into(),
            kind: RefKind::DetachedHead,
        });
    }

    if let Ok(references) = repo.references() {
        for reference in references.flatten() {
            let Ok(commit) = reference.peel_to_commit() else {
                continue;
            };
            let oid = commit.id();
            let Some(name) = reference.shorthand().map(str::to_string) else {
                continue;
            };
            let kind = if reference.is_tag() {
                RefKind::Tag
            } else if reference.is_remote() {
                // Skip the synthetic origin/HEAD pointer; it's noise.
                if name.ends_with("/HEAD") {
                    continue;
                }
                RefKind::RemoteBranch
            } else if reference.is_branch() {
                if head_branch.as_deref() == Some(name.as_str()) {
                    RefKind::Head
                } else {
                    RefKind::LocalBranch
                }
            } else {
                continue;
            };
            map.entry(oid).or_default().push(RefLabel { name, kind });
        }
    }

    // Stable, readable ordering: HEAD/branch first, remotes, then tags.
    for labels in map.values_mut() {
        labels.sort_by_key(|l| match l.kind {
            RefKind::Head | RefKind::DetachedHead => 0,
            RefKind::LocalBranch => 1,
            RefKind::RemoteBranch => 2,
            RefKind::Tag => 3,
        });
    }

    Ok(map)
}

/// Run a reverse-topological, newest-first revwalk from every ref tip and
/// build a [`CommitInfo`] per commit.
fn load_commits(
    repo: &Repository,
    refs: &HashMap<Oid, Vec<RefLabel>>,
) -> Result<Vec<CommitInfo>, git2::Error> {
    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TIME | Sort::TOPOLOGICAL)?;
    // Show history reachable from all branches/tags, not just HEAD, so the
    // browser behaves like `gitk --all`.
    if revwalk.push_glob("refs/heads/*").is_err() {
        let _ = revwalk.push_head();
    }
    let _ = revwalk.push_glob("refs/remotes/*");
    let _ = revwalk.push_glob("refs/tags/*");
    let _ = revwalk.push_head();

    let mut commits = Vec::new();
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        commits.push(commit_info(&commit, refs));
    }
    Ok(commits)
}

fn commit_info(commit: &Commit, refs: &HashMap<Oid, Vec<RefLabel>>) -> CommitInfo {
    let id = commit.id().to_string();
    let short_id = id.chars().take(8).collect();
    let message = commit.message().unwrap_or("").to_string();
    let summary = commit
        .summary()
        .map(str::to_string)
        .unwrap_or_else(|| message.lines().next().unwrap_or("").to_string());
    let author = commit.author();
    let committer = commit.committer();
    let time = author.when();

    CommitInfo {
        short_id,
        summary,
        message,
        author_name: author.name().unwrap_or("").to_string(),
        author_email: author.email().unwrap_or("").to_string(),
        committer_name: committer.name().unwrap_or("").to_string(),
        committer_email: committer.email().unwrap_or("").to_string(),
        time_seconds: time.seconds(),
        time_offset_minutes: time.offset_minutes(),
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        refs: refs.get(&commit.id()).cloned().unwrap_or_default(),
        id,
    }
}

fn status_from_delta(delta: Delta) -> ChangeStatus {
    match delta {
        Delta::Added => ChangeStatus::Added,
        Delta::Deleted => ChangeStatus::Deleted,
        Delta::Modified => ChangeStatus::Modified,
        Delta::Renamed => ChangeStatus::Renamed,
        Delta::Copied => ChangeStatus::Copied,
        Delta::Typechange => ChangeStatus::TypeChange,
        _ => ChangeStatus::Other,
    }
}

/// Drive libgit2's patch printer and translate each emitted line into a typed
/// [`DiffLine`]. Content/hunk/file-header lines keep their text; +/-/context
/// lines get their origin character prepended so the monospace view reads like
/// a real unified diff even before color is applied.
fn render_diff(diff: git2::Diff) -> Diff {
    let mut lines = Vec::new();
    let _ = diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        let content = String::from_utf8_lossy(line.content());
        let content = content.trim_end_matches('\n');
        match line.origin_value() {
            DiffLineType::FileHeader => push_multiline(&mut lines, DiffLineKind::FileHeader, content),
            DiffLineType::HunkHeader => push_multiline(&mut lines, DiffLineKind::HunkHeader, content),
            DiffLineType::Context => {
                lines.push(DiffLine::new(DiffLineKind::Context, format!(" {content}")))
            }
            DiffLineType::Addition => {
                lines.push(DiffLine::new(DiffLineKind::Addition, format!("+{content}")))
            }
            DiffLineType::Deletion => {
                lines.push(DiffLine::new(DiffLineKind::Deletion, format!("-{content}")))
            }
            DiffLineType::ContextEOFNL
            | DiffLineType::AddEOFNL
            | DiffLineType::DeleteEOFNL => {
                lines.push(DiffLine::new(DiffLineKind::Meta, content.to_string()))
            }
            _ => push_multiline(&mut lines, DiffLineKind::Meta, content),
        }
        true
    });
    Diff { lines }
}

fn push_multiline(out: &mut Vec<DiffLine>, kind: DiffLineKind, content: &str) {
    for line in content.split('\n') {
        out.push(DiffLine::new(kind, line.to_string()));
    }
}
