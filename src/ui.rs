//! The top-level [`GitClient`] widget.
//!
//! `GitClient` wraps a [`Panes`] shell (the gitk-style flat layout) as its
//! layout / focus / event engine and keeps shared handles to the panes it
//! needs to coordinate. After each event it polls selections and, when they
//! change, reloads dependent panes from the backend — the cross-pane wiring
//! retrogui's callback-free widgets don't provide on their own.

use std::cell::RefCell;
use std::rc::Rc;

use retrogui::{
    Dialog, Event, EventCtx, List, ListItem, Menu, MenuBar, MenuItem, Painter, PopupRequest, Rect,
    Theme, Widget,
};

use crate::backend::{CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RepoBackend};
use crate::widgets::{CommitList, CommitRow, DiffView, Pane, Panes, SearchBar, Shared};

/// Height of the menu bar.
const MENU_H: i32 = 20;
/// Height of the search bar below the menu.
const SEARCH_H: i32 = 26;
/// Width of the changed-files pane on the lower right.
const FILES_W: i32 = 300;

/// A closure that re-opens the repository (used by File ▸ Reload). `None` for
/// fixture-backed clients in tests.
type ReopenFn = Box<dyn Fn() -> Option<Rc<dyn RepoBackend>>>;

/// Deferred actions menu callbacks request; drained by `GitClient` after
/// event dispatch so they can mutate state the callbacks can't reach.
#[derive(Clone, Copy)]
enum AppCommand {
    Reload,
}

pub struct GitClient {
    /// Flat layout/event engine; owns the actual widget tree.
    root: Panes,
    backend: Rc<dyn RepoBackend>,
    /// Shared handle to the search/filter bar.
    search: Rc<RefCell<SearchBar>>,
    /// Shared handle to the commit list.
    commit_list: Rc<RefCell<CommitList>>,
    /// Shared handle to the changed-files list.
    file_list: Rc<RefCell<List>>,
    /// Shared handle to the diff pane.
    diff_view: Rc<RefCell<DiffView>>,
    /// Shared handle to the modal dialog overlay (About, errors).
    dialog: Rc<RefCell<Dialog>>,
    /// Queue menu callbacks push into; drained after each event.
    commands: Rc<RefCell<Vec<AppCommand>>>,
    /// Re-open hook for File ▸ Reload.
    reopen: Option<ReopenFn>,
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
        let dialog = Rc::new(RefCell::new(Dialog::new()));
        let commands: Rc<RefCell<Vec<AppCommand>>> = Rc::new(RefCell::new(Vec::new()));

        let menu_bar = build_menu_bar(commands.clone(), dialog.clone());

        // Add order sets keyboard focus order: search → commits → diff →
        // files (the menu bar isn't focusable, it works via accelerators).
        let root = Panes::new()
            .menu_height(MENU_H)
            .toolbar_height(SEARCH_H)
            .files_width(FILES_W)
            .add(Pane::Menu, menu_bar)
            .add(Pane::Toolbar, Shared::new(search.clone()))
            .add(Pane::History, Shared::new(commit_list.clone()))
            .add(Pane::Diff, Shared::new(diff_view.clone()))
            .add(Pane::Files, Shared::new(file_list.clone()))
            .add_overlay(Shared::new(dialog.clone()));

        let mut client = Self {
            root,
            backend,
            search,
            commit_list,
            file_list,
            diff_view,
            dialog,
            commands,
            reopen: None,
            visible: Vec::new(),
            last_query: String::new(),
            current_files: Vec::new(),
            shown_commit: None,
            shown_file: None,
        };
        client.sync(true);
        client
    }

    /// Install the repository re-open hook used by File ▸ Reload.
    pub fn with_reopen(mut self, reopen: ReopenFn) -> Self {
        self.reopen = Some(reopen);
        self
    }

    /// Apply any queued menu commands. Returns `true` if state changed.
    fn drain_commands(&mut self) -> bool {
        let pending: Vec<AppCommand> = self.commands.borrow_mut().drain(..).collect();
        let mut changed = false;
        for command in pending {
            match command {
                AppCommand::Reload => changed |= self.reload(),
            }
        }
        changed
    }

    /// Re-open the repository and rebuild every pane. No-op without a reopen
    /// hook (e.g. fixture-backed clients).
    fn reload(&mut self) -> bool {
        let Some(reopen) = &self.reopen else {
            return false;
        };
        let Some(backend) = reopen() else {
            self.dialog
                .borrow_mut()
                .show_error("Reload failed", "Could not re-open the repository.");
            return true;
        };
        self.backend = backend;
        self.shown_commit = None;
        self.shown_file = None;
        self.last_query.clear();
        self.search.borrow_mut().clear();
        self.sync(true);
        true
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
        // After the tree has processed the event, apply menu commands and
        // sync dependent panes.
        let mut dirty = self.drain_commands();
        dirty |= self.sync(false);
        if dirty {
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

    fn layout(&mut self, bounds: Rect) {
        self.root.layout(bounds);
    }

    fn focus_first(&mut self) -> bool {
        // Start on the commit list rather than the leading search field, so
        // arrow keys navigate history immediately (gitk behavior).
        self.root.focus_pane(Pane::History)
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.root.popup_request()
    }

    fn wants_ticks(&self) -> bool {
        self.root.wants_ticks()
    }
}

/// Build the File / View / Help menu bar. Reload is deferred to the command
/// queue (it must rebuild panes the callback can't reach); Exit closes the
/// window directly; About pops the shared dialog.
fn build_menu_bar(
    commands: Rc<RefCell<Vec<AppCommand>>>,
    dialog: Rc<RefCell<Dialog>>,
) -> MenuBar {
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                MenuItem::action("&Reload", {
                    let commands = commands.clone();
                    move |cx| {
                        commands.borrow_mut().push(AppCommand::Reload);
                        cx.request_paint();
                    }
                }),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ))
        .add_menu(Menu::new(
            "&Help",
            vec![MenuItem::action("&About", {
                let dialog = dialog.clone();
                move |cx| {
                    dialog.borrow_mut().show_info(
                        "About Journey",
                        "Journey\n\nA gitk-style repository browser\nbuilt on the retrogui toolkit.",
                    );
                    cx.request_paint();
                }
            })],
        ))
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
