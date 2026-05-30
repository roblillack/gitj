//! The top-level [`GitClient`] widget.
//!
//! `GitClient` wraps a retrogui [`Column`] as its layout / focus / event
//! engine and keeps shared handles to the panes it needs to coordinate. After
//! each event it polls the commit list's selection and, when it changes,
//! reloads the file list from the backend — the cross-pane wiring retrogui's
//! callback-free widgets don't provide on their own.

use std::cell::RefCell;
use std::rc::Rc;

use retrogui::{
    Color, Column, Event, EventCtx, List, ListItem, Painter, PopupRequest, Rect, Theme, Widget,
};

use crate::backend::{CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RepoBackend};
use crate::widgets::{CommitList, CommitRow, DiffView, SearchBar, Shared};

/// Height of the search bar at the top of the window.
const SEARCH_H: i32 = 26;
/// Height of the changed-files pane between the commit list and the diff.
const FILE_PANE_H: i32 = 130;
/// Index of the commit list among the column's children (search bar is 0).
const COMMIT_CHILD: usize = 1;

pub struct GitClient {
    /// retrogui layout/event engine; owns the actual widget tree.
    root: Column,
    backend: Rc<dyn RepoBackend>,
    /// Shared handle to the search/filter bar.
    search: Rc<RefCell<SearchBar>>,
    /// Shared handle to the commit list (also owned by `root` via `Shared`).
    commit_list: Rc<RefCell<CommitList>>,
    /// Shared handle to the changed-files list.
    file_list: Rc<RefCell<List>>,
    /// Shared handle to the diff pane.
    diff_view: Rc<RefCell<DiffView>>,
    /// Backend commit indices currently shown in the list, in row order. The
    /// search filter narrows this; an empty query shows every commit.
    visible: Vec<usize>,
    /// Last search query we filtered on, so we only re-filter on change.
    last_query: String,
    /// Changed files for the currently-shown commit (index-aligned with the
    /// file list rows), so a file selection can be mapped back to a path.
    current_files: Vec<FileChange>,
    /// Backend index of the commit we last loaded files/diff for.
    shown_commit: Option<usize>,
    /// Last file index we loaded a diff for.
    shown_file: Option<usize>,
}

impl GitClient {
    pub fn new(backend: Rc<dyn RepoBackend>) -> Self {
        let search = Rc::new(RefCell::new(SearchBar::new(Rect::new(0, 0, 0, 0))));
        let commit_list = Rc::new(RefCell::new(CommitList::new(Rect::new(0, 0, 0, 0))));
        let file_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let diff_view = Rc::new(RefCell::new(DiffView::new(Rect::new(0, 0, 0, 0))));

        // Child order must match the COMMIT_CHILD index above.
        let root = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_fixed(Shared::new(search.clone()), SEARCH_H)
            .add_fill(Shared::new(commit_list.clone()))
            .add_fixed(Shared::new(file_list.clone()), FILE_PANE_H)
            .add_fill(Shared::new(diff_view.clone()));

        let mut client = Self {
            root,
            backend,
            search,
            commit_list,
            file_list,
            diff_view,
            visible: Vec::new(),
            last_query: String::new(),
            current_files: Vec::new(),
            shown_commit: None,
            shown_file: None,
        };
        client.sync(true);
        client
    }

    /// Reload dependent panes from the current selection state. When the
    /// commit selection changes we reload the file list and show the whole
    /// commit's diff; when the file selection changes we narrow the diff to
    /// that file. Returns `true` if anything changed.
    fn sync(&mut self, force: bool) -> bool {
        let mut changed = false;

        // 1. Re-filter the commit list when the query changes.
        let query = self.search.borrow().text().trim().to_lowercase();
        if force || query != self.last_query {
            self.last_query = query.clone();
            self.rebuild_commits(&query);
            // Force the file/diff panes to recompute against the (possibly
            // new) selection below.
            self.shown_commit = None;
            changed = true;
        }

        // 2. Map the selected row back to a backend commit index and, when it
        //    changes, reload the file list and show the commit's detail.
        let sel_pos = self.commit_list.borrow().selected_index();
        let commit_idx = sel_pos.and_then(|p| self.visible.get(p).copied());
        if force || commit_idx != self.shown_commit {
            self.shown_commit = commit_idx;
            self.current_files = match commit_idx {
                Some(idx) => self.backend.changed_files(idx),
                None => Vec::new(),
            };
            let items: Vec<ListItem> = self.current_files.iter().map(file_row).collect();
            self.file_list.borrow_mut().set_items(items);
            // Re-selecting items above resets the file selection to None, so
            // the whole-commit diff is what we want to show now.
            self.shown_file = None;
            let diff = match commit_idx {
                Some(idx) => self.commit_detail(idx),
                None => Default::default(),
            };
            self.diff_view.borrow_mut().set_diff(diff);
            changed = true;
        }

        // 3. Narrow the diff to a single file when one is selected.
        let file_sel = self.file_list.borrow().selected_index();
        if file_sel != self.shown_file {
            self.shown_file = file_sel;
            let diff = match (self.shown_commit, file_sel) {
                (Some(cidx), Some(fidx)) => match self.current_files.get(fidx) {
                    Some(file) => self.backend.file_diff(cidx, &file.path),
                    None => self.commit_detail(cidx),
                },
                (Some(cidx), None) => self.commit_detail(cidx),
                _ => Default::default(),
            };
            self.diff_view.borrow_mut().set_diff(diff);
            changed = true;
        }

        changed
    }

    /// Recompute the visible commit set for `query` (empty = all), rebuild the
    /// commit list rows, and preserve the selected commit when it survives the
    /// filter (otherwise select the first visible row).
    fn rebuild_commits(&mut self, query: &str) {
        let commits = self.backend.commits();
        self.visible = (0..commits.len())
            .filter(|&i| query.is_empty() || commit_matches(&commits[i], query))
            .collect();
        let rows: Vec<CommitRow> =
            self.visible.iter().map(|&i| commit_row(&commits[i])).collect();

        let mut list = self.commit_list.borrow_mut();
        list.set_rows(rows);
        let new_pos = self
            .shown_commit
            .and_then(|c| self.visible.iter().position(|&i| i == c))
            .or(if self.visible.is_empty() { None } else { Some(0) });
        list.set_selected(new_pos);
    }

    /// Build a `git show`-style view of a commit: a metadata header (SHA,
    /// refs, author, date, parents), the message, then the full diff.
    fn commit_detail(&self, idx: usize) -> Diff {
        let Some(commit) = self.backend.commits().get(idx) else {
            return Diff::default();
        };

        let mut lines = Vec::new();
        let header = |lines: &mut Vec<DiffLine>, text: String| {
            lines.push(DiffLine::new(DiffLineKind::CommitHeader, text));
        };
        let blank = |lines: &mut Vec<DiffLine>| {
            lines.push(DiffLine::new(DiffLineKind::Context, String::new()));
        };

        header(&mut lines, format!("commit {}", commit.id));
        if !commit.refs.is_empty() {
            let names: Vec<&str> = commit.refs.iter().map(|r| r.name.as_str()).collect();
            header(&mut lines, format!("Refs:   {}", names.join(", ")));
        }
        header(
            &mut lines,
            format!("Author: {} <{}>", commit.author_name, commit.author_email),
        );
        header(&mut lines, format!("Date:   {}", commit.date_string()));
        if commit.is_merge() {
            let shorts: Vec<String> =
                commit.parents.iter().map(|p| short(p)).collect();
            header(&mut lines, format!("Merge:  {}", shorts.join(" ")));
        }

        blank(&mut lines);
        for line in commit.message.trim_end().lines() {
            lines.push(DiffLine::new(DiffLineKind::Context, format!("    {line}")));
        }
        blank(&mut lines);

        lines.extend(self.backend.commit_diff(idx).lines);
        Diff { lines }
    }
}

/// First 8 hex chars of a SHA, for compact parent display.
fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Does a commit match a (already-lowercased) search query? Matches against
/// the summary, message, author name/email, ref names and the full SHA.
fn commit_matches(commit: &CommitInfo, query: &str) -> bool {
    commit.summary.to_lowercase().contains(query)
        || commit.message.to_lowercase().contains(query)
        || commit.author_name.to_lowercase().contains(query)
        || commit.author_email.to_lowercase().contains(query)
        || commit.id.contains(query)
        || commit
            .refs
            .iter()
            .any(|r| r.name.to_lowercase().contains(query))
}

impl Widget for GitClient {
    fn bounds(&self) -> Rect {
        self.root.bounds()
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.root.paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.root.paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.root.event(event, ctx);
        // After the tree has processed the event, sync dependent panes.
        if self.sync(false) {
            ctx.request_paint();
        }
    }

    fn captures_pointer(&self) -> bool {
        self.root.captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.root.focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.root.set_focused(focused);
    }

    fn accepts_accelerators(&self) -> bool {
        self.root.accepts_accelerators()
    }

    fn layout(&mut self, bounds: Rect) {
        self.root.layout(bounds);
    }

    fn focus_first(&mut self) -> bool {
        // Start on the commit list rather than the leading search field, so
        // arrow keys navigate history immediately (gitk behavior).
        self.root.focus_child(COMMIT_CHILD)
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.root.popup_request()
    }

    fn wants_ticks(&self) -> bool {
        self.root.wants_ticks()
    }
}

/// Build a commit-list row from a commit: ref badges + summary on the left,
/// author and short date in the right-hand columns.
pub fn commit_row(commit: &CommitInfo) -> CommitRow {
    CommitRow {
        summary: commit.summary.clone(),
        refs: commit.refs.clone(),
        author: commit.author_name.clone(),
        date: commit.short_date_string(),
    }
}

/// Format a changed file as a list row: status badge + path.
pub fn file_row(file: &FileChange) -> ListItem {
    ListItem::new(format!("{}  {}", file.status.badge(), file.display()))
}
