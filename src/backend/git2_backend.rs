//! The live [`RepoBackend`] backed by `git2` / libgit2.

use std::collections::HashMap;
use std::path::Path;

use git2::{Commit, Delta, DiffFormat, DiffLineType, DiffOptions, Oid, Repository, Sort};

use super::{
    BlobPair, ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RefKind,
    RefLabel, RepoBackend, WorkingStatus,
};

/// Cap on the rename/copy-detection workload, in candidate pairs. `find_similar`
/// builds an O(added × deleted) content-similarity matrix, so its cost tracks
/// that product — **not** the total number of changed files: a modification-only
/// commit is cheap at any size, while an add/delete-heavy one is costly even when
/// smaller (a 4240-file all-additions commit detects in ~0.1ms; an 8157-file
/// merge with ~4100 adds × ~3500 deletes took ~49s and froze the UI). Past this
/// many pairs we skip rename detection so a big merge can't hang the UI; ~100k
/// keeps the matching well under a second while still catching renames among a
/// few hundred files on each side. The cost of skipping is that renames in such
/// a diff show as add/delete pairs (git's CLI does the same past
/// `diff.renameLimit`).
const MAX_RENAME_PAIRS: usize = 100_000;

/// Cap on the number of lines [`render_diff`] emits for one diff. A pathological
/// commit (again, a big merge) can produce well over a million patch lines;
/// materializing them all freezes the UI for tens of seconds and wastes memory
/// on a diff no one scrolls through. Past the cap the diff is truncated with a
/// trailing marker; the per-file lists still show every changed file.
const MAX_DIFF_LINES: usize = 50_000;

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
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let mut opts = DiffOptions::new();
        opts.context_lines(3);
        if let Some(path) = path {
            opts.pathspec(path);
        }

        let mut diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&new_tree), Some(&mut opts))
            .ok()?;
        // Detect renames/copies so statuses and headers are accurate — but only
        // when the similarity matrix is small enough to stay cheap. Its cost is
        // ~O(added × deletes), so gate on that product rather than the total
        // file count (see [`MAX_RENAME_PAIRS`]); a huge merge would otherwise
        // hang the UI. Counts are taken before detection, so they're the raw
        // add/delete candidate pool.
        let (mut added, mut deleted) = (0usize, 0usize);
        for delta in diff.deltas() {
            match delta.status() {
                Delta::Added => added += 1,
                Delta::Deleted => deleted += 1,
                _ => {}
            }
        }
        if added.saturating_mul(deleted) <= MAX_RENAME_PAIRS {
            let _ = diff.find_similar(None);
        }
        Some(diff)
    }

    /// Read the bytes of `path`'s blob in `tree`, if it exists there and is a
    /// blob (not a submodule or directory).
    fn blob_in_tree(&self, tree: &git2::Tree, path: &str) -> Option<Vec<u8>> {
        let entry = tree.get_path(Path::new(path)).ok()?;
        let obj = entry.to_object(&self.repo).ok()?;
        obj.as_blob().map(|b| b.content().to_vec())
    }

    /// The bytes of `path`'s entry in the current index (the staged copy).
    fn blob_in_index(&self, path: &str) -> Option<Vec<u8>> {
        let index = self.repo.index().ok()?;
        let entry = index.get_path(Path::new(path), 0)?;
        let blob = self.repo.find_blob(entry.id).ok()?;
        Some(blob.content().to_vec())
    }

    /// The raw bytes of `path` in the working tree on disk.
    fn blob_in_workdir(&self, path: &str) -> Option<Vec<u8>> {
        let workdir = self.repo.workdir()?;
        std::fs::read(workdir.join(path)).ok()
    }

    /// The tree the staged side is diffed against: `HEAD`'s tree normally, or
    /// `HEAD`'s parent's tree when amending. `None` means "no base" (unborn
    /// `HEAD`, or amending the root commit) — the index is then diffed against
    /// the empty tree, so everything reads as additions.
    fn staged_base_tree(&self, amend: bool) -> Option<git2::Tree<'_>> {
        let head = self.repo.head().ok()?.peel_to_commit().ok()?;
        if amend {
            head.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            head.tree().ok()
        }
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
        diff.deltas()
            .map(|delta| file_change_from_delta(&delta))
            .collect()
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

    fn commit_file_blobs(&self, index: usize, path: &str) -> BlobPair {
        let Some(commit) = self.commit_at(index) else {
            return BlobPair::default();
        };
        let new = commit.tree().ok().and_then(|t| self.blob_in_tree(&t, path));
        // The version in the first parent (the diff base); absent for an added
        // file, or for a rename where the old content lives under a different
        // path (a rare case for images, left as a one-sided comparison).
        let old = commit
            .parent(0)
            .ok()
            .and_then(|p| p.tree().ok())
            .and_then(|t| self.blob_in_tree(&t, path));
        BlobPair { old, new }
    }

    fn working_file_blobs(&self, path: &str, staged: bool, amend: bool) -> BlobPair {
        if staged {
            // Staged base (HEAD / HEAD^) vs the index copy.
            let old = self
                .staged_base_tree(amend)
                .and_then(|t| self.blob_in_tree(&t, path));
            BlobPair {
                old,
                new: self.blob_in_index(path),
            }
        } else {
            // Index copy vs the working tree on disk.
            BlobPair {
                old: self.blob_in_index(path),
                new: self.blob_in_workdir(path),
            }
        }
    }

    fn working_status(&self, amend: bool) -> WorkingStatus {
        let base = self.staged_base_tree(amend);

        // Staged side: the index against the base tree (HEAD, or HEAD's parent
        // when amending). With no base (unborn / amending the root commit) the
        // whole index reads as additions.
        let mut staged_opts = DiffOptions::new();
        let mut staged = WorkingStatus::default();
        if let Ok(mut diff) =
            self.repo
                .diff_tree_to_index(base.as_ref(), None, Some(&mut staged_opts))
        {
            let _ = diff.find_similar(None);
            for delta in diff.deltas() {
                staged.staged.push(file_change_from_delta(&delta));
            }
        }

        // Unstaged side: the working tree against the index (independent of
        // the amend base), including untracked files.
        let mut wd_opts = DiffOptions::new();
        wd_opts.include_untracked(true).recurse_untracked_dirs(true);
        if let Ok(diff) = self.repo.diff_index_to_workdir(None, Some(&mut wd_opts)) {
            for delta in diff.deltas() {
                staged.unstaged.push(file_change_from_delta(&delta));
            }
        }

        staged
    }

    fn working_diff(&self, path: &str, staged: bool, amend: bool) -> Diff {
        let mut opts = DiffOptions::new();
        opts.context_lines(3).pathspec(path);
        let diff = if staged {
            let base = self.staged_base_tree(amend);
            self.repo
                .diff_tree_to_index(base.as_ref(), None, Some(&mut opts))
        } else {
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            self.repo.diff_index_to_workdir(None, Some(&mut opts))
        };
        diff.ok().map(render_diff).unwrap_or_default()
    }

    fn stage(&self, path: &str) -> Result<(), String> {
        let mut index = self.repo.index().map_err(err_msg)?;
        let p = Path::new(path);
        let in_workdir = self
            .repo
            .workdir()
            .map(|w| w.join(path).exists())
            .unwrap_or(false);
        if in_workdir {
            index.add_path(p).map_err(err_msg)?;
        } else {
            // The file is gone from the working tree — stage its removal.
            index.remove_path(p).map_err(err_msg)?;
        }
        index.write().map_err(err_msg)
    }

    fn unstage(&self, path: &str, amend: bool) -> Result<(), String> {
        // Reset the index entry to the staged base: HEAD normally, HEAD's
        // parent when amending (which drops the path from the amended commit).
        let head = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let target: Option<git2::Object> = match (amend, head) {
            (false, Some(commit)) => Some(commit.into_object()),
            (true, Some(commit)) => commit.parent(0).ok().map(|p| p.into_object()),
            (_, None) => None,
        };
        match target {
            Some(obj) => self.repo.reset_default(Some(&obj), [path]).map_err(err_msg),
            // No base commit (unborn HEAD, or amending the root commit):
            // unstaging just drops the path back out of the index.
            None => {
                let mut index = self.repo.index().map_err(err_msg)?;
                index.remove_path(Path::new(path)).map_err(err_msg)?;
                index.write().map_err(err_msg)
            }
        }
    }

    fn revert(&self, path: &str) -> Result<(), String> {
        // Rewrite the working-tree file from the index, overwriting any
        // unstaged edits. `update_index(false)` leaves the index untouched, so
        // a partially-staged file keeps its staged changes — only the
        // working-vs-index delta is discarded. An untracked path has no index
        // entry, so the checkout simply skips it.
        let mut opts = git2::build::CheckoutBuilder::new();
        opts.force().update_index(false).path(path);
        self.repo
            .checkout_index(None, Some(&mut opts))
            .map_err(err_msg)
    }

    fn delete_untracked(&self, path: &str) -> Result<(), String> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| "bare repository has no working tree".to_string())?;
        std::fs::remove_file(workdir.join(path)).map_err(|e| e.to_string())
    }

    fn apply_to_index(&self, patch: &str) -> Result<(), String> {
        let diff = git2::Diff::from_buffer(patch.as_bytes()).map_err(err_msg)?;
        self.repo
            .apply(&diff, git2::ApplyLocation::Index, None)
            .map_err(err_msg)
    }

    fn commit(&self, message: &str, amend: bool) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("Please enter a commit message.".into());
        }
        let mut index = self.repo.index().map_err(err_msg)?;
        let tree_oid = index.write_tree().map_err(err_msg)?;
        let tree = self.repo.find_tree(tree_oid).map_err(err_msg)?;

        if amend {
            let head = self
                .repo
                .head()
                .and_then(|h| h.peel_to_commit())
                .map_err(err_msg)?;
            // Keep the original author/committer; only the message and tree
            // change. (`None` tells libgit2 to reuse the existing values.)
            head.amend(Some("HEAD"), None, None, None, Some(message), Some(&tree))
                .map_err(err_msg)?;
        } else {
            let sig = self.repo.signature().map_err(|_| {
                "No git identity configured. Set user.name and user.email.".to_string()
            })?;
            let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&Commit> = parent.iter().collect();
            self.repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
                .map_err(err_msg)?;
        }
        Ok(())
    }

    fn head_message(&self) -> Option<String> {
        let commit = self.repo.head().ok()?.peel_to_commit().ok()?;
        Some(commit.message().unwrap_or("").to_string())
    }

    fn signature(&self) -> Option<(String, String)> {
        let sig = self.repo.signature().ok()?;
        Some((sig.name()?.to_string(), sig.email()?.to_string()))
    }
}

/// Map a libgit2 diff delta to our [`FileChange`], collapsing the old path for
/// non-rename changes.
fn file_change_from_delta(delta: &git2::DiffDelta) -> FileChange {
    let new_path = delta.new_file().path().map(|p| p.display().to_string());
    let old_path = delta.old_file().path().map(|p| p.display().to_string());
    let status = status_from_delta(delta.status());
    let path = new_path
        .clone()
        .or_else(|| old_path.clone())
        .unwrap_or_default();
    FileChange {
        path,
        old_path: old_path.filter(|o| Some(o) != new_path.as_ref()),
        status,
    }
}

/// Render a libgit2 error as a short message for the UI's dialog.
fn err_msg(e: git2::Error) -> String {
    e.message().to_string()
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

    if detached && let Some(oid) = head.as_ref().and_then(|h| h.target()) {
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
        Delta::Untracked => ChangeStatus::Untracked,
        _ => ChangeStatus::Other,
    }
}

/// Drive libgit2's patch printer and translate each emitted line into a typed
/// [`DiffLine`]. Content/hunk/file-header lines keep their text; +/-/context
/// lines get their origin character prepended so the monospace view reads like
/// a real unified diff even before color is applied.
fn render_diff(diff: git2::Diff) -> Diff {
    let mut lines = Vec::new();
    let mut truncated = false;
    let _ = diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        // Stop once the cap is hit: returning `false` aborts libgit2's patch
        // generation, so the remaining (huge) diff is never materialized.
        if lines.len() >= MAX_DIFF_LINES {
            truncated = true;
            return false;
        }
        let content = String::from_utf8_lossy(line.content());
        let content = content.trim_end_matches('\n');
        match line.origin_value() {
            DiffLineType::FileHeader => {
                push_multiline(&mut lines, DiffLineKind::FileHeader, content)
            }
            DiffLineType::HunkHeader => {
                push_multiline(&mut lines, DiffLineKind::HunkHeader, content)
            }
            DiffLineType::Context => {
                lines.push(DiffLine::new(DiffLineKind::Context, format!(" {content}")))
            }
            DiffLineType::Addition => {
                lines.push(DiffLine::new(DiffLineKind::Addition, format!("+{content}")))
            }
            DiffLineType::Deletion => {
                lines.push(DiffLine::new(DiffLineKind::Deletion, format!("-{content}")))
            }
            DiffLineType::ContextEOFNL | DiffLineType::AddEOFNL | DiffLineType::DeleteEOFNL => {
                lines.push(DiffLine::new(DiffLineKind::Meta, content.to_string()))
            }
            _ => push_multiline(&mut lines, DiffLineKind::Meta, content),
        }
        true
    });
    if truncated {
        lines.push(DiffLine::new(
            DiffLineKind::Meta,
            format!(
                "\u{2026} diff truncated at {MAX_DIFF_LINES} lines — too large to display in full"
            ),
        ));
    }
    Diff { lines }
}

fn push_multiline(out: &mut Vec<DiffLine>, kind: DiffLineKind, content: &str) {
    for line in content.split('\n') {
        out.push(DiffLine::new(kind, line.to_string()));
    }
}
