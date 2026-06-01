//! The top-level [`GitClient`] widget.
//!
//! `GitClient` drives two screens, each a flat [`Shell`] of panes:
//!
//! * **Browse** — the gitk-style history browser (commit list, diff, files).
//! * **Commit** — a `git gui`-style staging screen (unstaged / staged file
//!   lists, a per-file diff, a message editor and a commit button).
//!
//! saudade widgets are callback-free, so the cross-pane wiring is done here:
//! after each event the active screen's selections (and a small command queue
//! menus/buttons push into) are polled, and dependent panes are rebuilt from
//! the [`RepoBackend`].

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    Button, Checkbox, Dialog, Event, EventCtx, Key, List, ListItem, Menu, MenuBar, MenuItem,
    NamedKey, Painter, PopupRequest, Rect, TextEditor, Theme, Widget,
};

use crate::backend::{
    ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RefKind, RepoBackend,
    WorkingStatus,
};
use crate::widgets::{
    CommitList, CommitRow, DiffView, Heading, SearchBar, Shared, Shell, compute_graph, layout,
};

/// Direct-child index of the history list in the browse shell (focused first).
const BROWSE_HISTORY_IDX: usize = 2;
/// Direct-child index of the unstaged list in the commit shell.
const COMMIT_UNSTAGED_IDX: usize = 2;

/// Sentinel commit ids for the working-tree pseudo-rows in the log graph
/// (chosen so they never collide with a real 40-hex SHA).
const WIP_UNSTAGED_ID: &str = "\u{1}journey-wip-unstaged";
const WIP_STAGED_ID: &str = "\u{1}journey-wip-staged";

/// A closure that re-opens the repository (used by File ▸ Reload and after a
/// commit). `None` for fixture-backed clients in tests.
type ReopenFn = Box<dyn Fn() -> Option<Rc<dyn RepoBackend>>>;

/// Which screen is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Commit,
}

/// Which working-tree list a commit-mode selection came from, and which side
/// a working-tree pseudo-row in the log represents.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Unstaged,
    Staged,
}

/// What a row in the history log refers to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowRef {
    /// A working-tree pseudo-row ("Uncommitted changes" / "Staged changes").
    Wip(Side),
    /// A real commit, by backend index.
    Commit(usize),
}

/// Deferred actions menus / buttons request; drained by `GitClient` after
/// event dispatch so they can mutate state the callbacks can't reach.
#[derive(Clone, Copy)]
enum AppCommand {
    Reload,
    EnterCommitMode,
    EnterBrowseMode,
    Rescan,
    StageSelected,
    StageAll,
    UnstageSelected,
    /// Ask to revert the selected unstaged file (pops the confirm dialog).
    RevertSelected,
    /// The confirm dialog's affirmative button fired — carry out the armed
    /// `pending_discard`.
    PerformDiscard,
    SignOff,
    Commit,
}

/// A discard armed by [`GitClient::revert_selected`] and awaiting the user's
/// confirmation: revert a tracked file to its index copy, or delete an
/// untracked file outright (it has nothing to revert to).
enum PendingDiscard {
    Revert(String),
    Delete(String),
}

pub struct GitClient {
    backend: Rc<dyn RepoBackend>,
    mode: Mode,
    bounds: Rect,

    // ---- browse screen ----------------------------------------------------
    browse_root: Shell,
    search: Rc<RefCell<SearchBar>>,
    commit_list: Rc<RefCell<CommitList>>,
    file_list: Rc<RefCell<List>>,
    diff_view: Rc<RefCell<DiffView>>,

    // ---- commit screen ----------------------------------------------------
    commit_root: Shell,
    unstaged_list: Rc<RefCell<List>>,
    staged_list: Rc<RefCell<List>>,
    unstaged_heading: Rc<RefCell<Heading>>,
    staged_heading: Rc<RefCell<Heading>>,
    commit_diff_view: Rc<RefCell<DiffView>>,
    message_editor: Rc<RefCell<TextEditor>>,
    amend_check: Rc<RefCell<Checkbox>>,

    // ---- shared -----------------------------------------------------------
    dialog: Rc<RefCell<Dialog>>,
    commands: Rc<RefCell<Vec<AppCommand>>>,
    reopen: Option<ReopenFn>,

    // ---- browse sync state ------------------------------------------------
    /// Row references in display order: the working-tree pseudo-rows (when
    /// present) followed by the visible commits.
    rows: Vec<RowRef>,
    last_query: String,
    /// Working-tree status backing the log's pseudo-rows (and the file/diff
    /// panes when one is selected). Refreshed by `rebuild_commits`.
    log_working: WorkingStatus,
    current_files: Vec<FileChange>,
    /// The log row whose detail the file/diff panes currently show.
    shown: Option<RowRef>,
    shown_file: Option<usize>,

    // ---- commit sync state ------------------------------------------------
    working: WorkingStatus,
    prev_unstaged_sel: Option<usize>,
    prev_staged_sel: Option<usize>,
    last_amend: bool,
    /// The discard awaiting confirmation, set when the confirm dialog is shown
    /// and consumed when its affirmative button drives
    /// `AppCommand::PerformDiscard`.
    pending_discard: Option<PendingDiscard>,
}

impl GitClient {
    pub fn new(backend: Rc<dyn RepoBackend>) -> Self {
        let dialog = Rc::new(RefCell::new(Dialog::new()));
        let commands: Rc<RefCell<Vec<AppCommand>>> = Rc::new(RefCell::new(Vec::new()));

        // Browse-screen widgets.
        let search = Rc::new(RefCell::new(SearchBar::new(Rect::new(0, 0, 0, 0))));
        let commit_list = Rc::new(RefCell::new(CommitList::new(Rect::new(0, 0, 0, 0))));
        let file_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let diff_view = Rc::new(RefCell::new(DiffView::new(Rect::new(0, 0, 0, 0))));

        // Add order sets the Tab focus order: search → commits → files → diff
        // (the menu bar isn't focusable; it works via accelerators). The file
        // list follows the commit list so Tab walks the panes left-to-right.
        // No flat background fill: the panes float on the window's desktop
        // pattern, which shows through the padding around them.
        let browse_root = Shell::new()
            .no_background()
            .add(
                build_browse_menu(commands.clone(), dialog.clone()),
                layout::browse_menu,
            )
            .add(Shared::new(search.clone()), layout::browse_toolbar)
            .add(Shared::new(commit_list.clone()), layout::browse_history)
            .add(Shared::new(file_list.clone()), layout::browse_files)
            .add(Shared::new(diff_view.clone()), layout::browse_diff)
            .add_overlay(Shared::new(dialog.clone()));

        // Commit-screen widgets.
        let unstaged_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let staged_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let unstaged_heading = Rc::new(RefCell::new(Heading::new("Unstaged Changes")));
        let staged_heading = Rc::new(RefCell::new(Heading::new("Staged Changes")));
        let commit_diff_view = Rc::new(RefCell::new(DiffView::new(Rect::new(0, 0, 0, 0))));
        let message_editor = Rc::new(RefCell::new(TextEditor::new(Rect::new(0, 0, 0, 0))));
        let amend_check = Rc::new(RefCell::new(Checkbox::new(
            Rect::new(0, 0, 0, 0),
            "Amend last commit",
        )));

        // No flat background fill: the staging panes float on the window's
        // desktop pattern (git-gui style), which shows through the gaps.
        let commit_root = Shell::new()
            .no_background()
            .add(
                build_commit_menu(commands.clone(), dialog.clone()),
                layout::commit_menu,
            )
            .add(
                Shared::new(unstaged_heading.clone()),
                layout::commit_unstaged_label,
            )
            .add(
                Shared::new(unstaged_list.clone()),
                layout::commit_unstaged_list,
            )
            .add(
                Shared::new(staged_heading.clone()),
                layout::commit_staged_label,
            )
            .add(Shared::new(staged_list.clone()), layout::commit_staged_list)
            .add(
                command_button("Stage \u{2192}", &commands, AppCommand::StageSelected),
                layout::commit_stage_btn,
            )
            .add(
                command_button("\u{2190} Unstage", &commands, AppCommand::UnstageSelected),
                layout::commit_unstage_btn,
            )
            .add(
                command_button("Rescan", &commands, AppCommand::Rescan),
                layout::commit_rescan_btn,
            )
            .add(Shared::new(commit_diff_view.clone()), layout::commit_diff)
            .add(Heading::new("Commit Message"), layout::commit_msg_label)
            .add(Shared::new(message_editor.clone()), layout::commit_editor)
            .add(Shared::new(amend_check.clone()), layout::commit_amend)
            .add(
                command_button("Commit", &commands, AppCommand::Commit),
                layout::commit_commit_btn,
            )
            .add_overlay(Shared::new(dialog.clone()));

        let mut client = Self {
            backend,
            mode: Mode::Browse,
            bounds: Rect::new(0, 0, 0, 0),
            browse_root,
            search,
            commit_list,
            file_list,
            diff_view,
            commit_root,
            unstaged_list,
            staged_list,
            unstaged_heading,
            staged_heading,
            commit_diff_view,
            message_editor,
            amend_check,
            dialog,
            commands,
            reopen: None,
            rows: Vec::new(),
            last_query: String::new(),
            log_working: WorkingStatus::default(),
            current_files: Vec::new(),
            shown: None,
            shown_file: None,
            working: WorkingStatus::default(),
            prev_unstaged_sel: None,
            prev_staged_sel: None,
            last_amend: false,
            pending_discard: None,
        };
        client.sync_browse(true);
        client
    }

    /// Install the repository re-open hook used by File ▸ Reload and refresh
    /// after a commit.
    pub fn with_reopen(mut self, reopen: ReopenFn) -> Self {
        self.reopen = Some(reopen);
        self
    }

    /// Switch to the commit screen. Exposed for tests; at runtime the View
    /// menu drives this through the command queue.
    pub fn enter_commit_mode(&mut self) {
        self.set_mode(Mode::Commit);
    }

    fn active(&self) -> &Shell {
        match self.mode {
            Mode::Browse => &self.browse_root,
            Mode::Commit => &self.commit_root,
        }
    }

    fn active_mut(&mut self) -> &mut Shell {
        match self.mode {
            Mode::Browse => &mut self.browse_root,
            Mode::Commit => &mut self.commit_root,
        }
    }

    fn set_mode(&mut self, mode: Mode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        match mode {
            Mode::Commit => {
                self.rescan();
                self.commit_root.layout(self.bounds);
                self.commit_root.focus_child(COMMIT_UNSTAGED_IDX);
            }
            Mode::Browse => {
                self.browse_root.layout(self.bounds);
                self.browse_root.focus_child(BROWSE_HISTORY_IDX);
            }
        }
        true
    }

    /// Apply queued menu / button commands. Returns `true` if state changed.
    fn drain_commands(&mut self) -> bool {
        let pending: Vec<AppCommand> = self.commands.borrow_mut().drain(..).collect();
        let mut changed = false;
        for command in pending {
            changed |= match command {
                AppCommand::Reload => self.reload(),
                AppCommand::EnterCommitMode => self.set_mode(Mode::Commit),
                AppCommand::EnterBrowseMode => self.set_mode(Mode::Browse),
                AppCommand::Rescan => {
                    self.rescan();
                    true
                }
                AppCommand::StageSelected => self.stage_selected(),
                AppCommand::StageAll => self.stage_all(),
                AppCommand::UnstageSelected => self.unstage_selected(),
                AppCommand::RevertSelected => self.revert_selected(),
                AppCommand::PerformDiscard => self.perform_discard(),
                AppCommand::SignOff => self.sign_off(),
                AppCommand::Commit => self.do_commit(),
            };
        }
        changed
    }

    /// Re-open the repository and rebuild every pane. No-op (returns `false`)
    /// without a reopen hook, e.g. fixture-backed clients.
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
        self.shown = None;
        self.shown_file = None;
        self.last_query.clear();
        self.search.borrow_mut().clear();
        self.sync_browse(true);
        self.rescan();
        true
    }

    // ---- browse screen ----------------------------------------------------

    /// Reload browse panes from the current selection state. Double-clicking a
    /// working-tree pseudo-row jumps to the commit screen; otherwise, when the
    /// selected row changes, reload the file list and overview diff, and when
    /// the file selection changes, narrow the diff to that file.
    fn sync_browse(&mut self, force: bool) -> bool {
        let mut changed = false;

        // 1. Re-filter the commit list when the query changes.
        let query = self.search.borrow().text().trim().to_lowercase();
        if force || query != self.last_query {
            self.last_query = query.clone();
            self.rebuild_commits(&query);
            self.shown = None;
            changed = true;
        }

        // 1b. Double-clicking a working-tree row opens the staging view.
        let activated = self.commit_list.borrow_mut().take_activated();
        if let Some(pos) = activated
            && matches!(self.rows.get(pos), Some(RowRef::Wip(_)))
        {
            self.set_mode(Mode::Commit);
            return true;
        }

        // 2. Map the selection to a row reference; on change, reload the file
        //    list and the overview diff.
        let sel_pos = self.commit_list.borrow().selected_index();
        let sel = sel_pos.and_then(|p| self.rows.get(p).copied());
        if force || sel != self.shown {
            self.shown = sel;
            self.current_files = match sel {
                Some(RowRef::Commit(idx)) => self.backend.changed_files(idx),
                Some(RowRef::Wip(Side::Unstaged)) => self.log_working.unstaged.clone(),
                Some(RowRef::Wip(Side::Staged)) => self.log_working.staged.clone(),
                None => Vec::new(),
            };
            let items: Vec<ListItem> = self.current_files.iter().map(file_row).collect();
            self.file_list.borrow_mut().set_items(items);
            self.shown_file = None;
            let diff = self.selection_diff(sel, None);
            self.diff_view.borrow_mut().set_diff(diff);
            changed = true;
        }

        // 3. Narrow the diff to a single file when one is selected.
        let file_sel = self.file_list.borrow().selected_index();
        if file_sel != self.shown_file {
            self.shown_file = file_sel;
            let diff = self.selection_diff(self.shown, file_sel);
            self.diff_view.borrow_mut().set_diff(diff);
            changed = true;
        }

        changed
    }

    /// The diff to show for a log selection: a whole-commit / whole-working-set
    /// overview when `file_sel` is `None`, otherwise that single file's diff.
    fn selection_diff(&self, sel: Option<RowRef>, file_sel: Option<usize>) -> Diff {
        match sel {
            Some(RowRef::Commit(cidx)) => match file_sel.and_then(|f| self.current_files.get(f)) {
                Some(file) => self.backend.file_diff(cidx, &file.path),
                None => self.commit_detail(cidx),
            },
            Some(RowRef::Wip(side)) => {
                let staged = matches!(side, Side::Staged);
                match file_sel.and_then(|f| self.current_files.get(f)) {
                    Some(file) => self.backend.working_diff(&file.path, staged, false),
                    None => self.wip_overview_diff(staged),
                }
            }
            None => Diff::default(),
        }
    }

    /// Concatenate the per-file working diffs of the currently-shown files into
    /// one overview, the working-tree analogue of `commit_detail`.
    fn wip_overview_diff(&self, staged: bool) -> Diff {
        let mut lines = Vec::new();
        for file in &self.current_files {
            lines.extend(self.backend.working_diff(&file.path, staged, false).lines);
        }
        Diff { lines }
    }

    /// Recompute the visible rows for `query` (empty = all). On the unfiltered
    /// view, the working tree's "Uncommitted changes" / "Staged changes"
    /// pseudo-rows lead the list and the DAG graph includes them, chained into
    /// `HEAD`. The selection is preserved when it survives, else falls to the
    /// first real commit (so the log opens on `HEAD`, not a pseudo-row).
    fn rebuild_commits(&mut self, query: &str) {
        // Working-tree pseudo-rows only on the unfiltered view (which also
        // carries the graph); a filter is about commit content.
        self.log_working = if query.is_empty() {
            self.backend.working_status(false)
        } else {
            WorkingStatus::default()
        };
        let show_unstaged = !self.log_working.unstaged.is_empty();
        let show_staged = !self.log_working.staged.is_empty();

        let commits = self.backend.commits();
        let commit_rows: Vec<usize> = (0..commits.len())
            .filter(|&i| query.is_empty() || commit_matches(&commits[i], query))
            .collect();

        let mut row_refs: Vec<RowRef> = Vec::new();
        let mut display: Vec<CommitRow> = Vec::new();
        if show_unstaged {
            row_refs.push(RowRef::Wip(Side::Unstaged));
            display.push(wip_row(Side::Unstaged, self.log_working.unstaged.len()));
        }
        if show_staged {
            row_refs.push(RowRef::Wip(Side::Staged));
            display.push(wip_row(Side::Staged, self.log_working.staged.len()));
        }
        for &i in &commit_rows {
            row_refs.push(RowRef::Commit(i));
            display.push(commit_row(&commits[i]));
        }

        // The DAG graph needs the full parent chain, so it's shown only on the
        // unfiltered view; the pseudo-rows are chained into HEAD so the gutter
        // lines up with them.
        let graph = if query.is_empty() {
            let head_id = head_commit_id(commits);
            let mut dag: Vec<(String, Vec<String>)> = Vec::new();
            if show_unstaged {
                let parent = if show_staged {
                    vec![WIP_STAGED_ID.to_string()]
                } else {
                    head_id.clone().into_iter().collect()
                };
                dag.push((WIP_UNSTAGED_ID.to_string(), parent));
            }
            if show_staged {
                dag.push((WIP_STAGED_ID.to_string(), head_id.into_iter().collect()));
            }
            for &i in &commit_rows {
                dag.push((commits[i].id.clone(), commits[i].parents.clone()));
            }
            Some(compute_graph(&dag))
        } else {
            None
        };

        self.rows = row_refs;
        let new_pos = self
            .shown
            .and_then(|s| self.rows.iter().position(|&r| r == s))
            .or_else(|| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, RowRef::Commit(_)))
            })
            .or(if self.rows.is_empty() { None } else { Some(0) });

        let mut list = self.commit_list.borrow_mut();
        list.set_rows(display);
        list.set_graph(graph);
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
            let shorts: Vec<String> = commit.parents.iter().map(|p| short(p)).collect();
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

    // ---- commit screen ----------------------------------------------------

    /// Re-read the working tree and rebuild the staged / unstaged lists.
    fn rescan(&mut self) {
        let amend = self.amend_check.borrow().is_checked();
        self.working = self.backend.working_status(amend);

        let unstaged: Vec<ListItem> = self.working.unstaged.iter().map(file_row).collect();
        let staged: Vec<ListItem> = self.working.staged.iter().map(file_row).collect();
        self.unstaged_list.borrow_mut().set_items(unstaged);
        self.staged_list.borrow_mut().set_items(staged);
        self.unstaged_heading.borrow_mut().set_text(format!(
            "Unstaged Changes ({})",
            self.working.unstaged.len()
        ));
        self.staged_heading.borrow_mut().set_text(format!(
            "Staged Changes — will commit ({})",
            self.working.staged.len()
        ));

        self.prev_unstaged_sel = None;
        self.prev_staged_sel = None;
        self.commit_diff_view.borrow_mut().set_diff(Diff::default());

        // Default the selection to the first file so the diff pane isn't blank.
        if !self.working.unstaged.is_empty() {
            self.apply_commit_selection(Side::Unstaged, 0);
        } else if !self.working.staged.is_empty() {
            self.apply_commit_selection(Side::Staged, 0);
        }
    }

    /// Select file `i` in the `side` list, clear the other list's selection,
    /// and show that file's diff.
    fn apply_commit_selection(&mut self, side: Side, i: usize) {
        match side {
            Side::Unstaged => {
                self.unstaged_list.borrow_mut().set_selected(Some(i));
                self.staged_list.borrow_mut().set_selected(None);
            }
            Side::Staged => {
                self.staged_list.borrow_mut().set_selected(Some(i));
                self.unstaged_list.borrow_mut().set_selected(None);
            }
        }
        self.prev_unstaged_sel = self.unstaged_list.borrow().selected_index();
        self.prev_staged_sel = self.staged_list.borrow().selected_index();

        let staged = matches!(side, Side::Staged);
        let amend = self.amend_check.borrow().is_checked();
        let files = match side {
            Side::Unstaged => &self.working.unstaged,
            Side::Staged => &self.working.staged,
        };
        let diff = files
            .get(i)
            .map(|f| self.backend.working_diff(&f.path, staged, amend))
            .unwrap_or_default();
        self.commit_diff_view.borrow_mut().set_diff(diff);
    }

    /// Poll the commit screen after an event: handle stage/unstage activations
    /// (double-click or Enter on a list), selection-driven diff updates, and
    /// the amend toggle.
    fn sync_commit(&mut self) -> bool {
        let unstaged_activated = self.unstaged_list.borrow_mut().take_activated();
        if let Some(i) = unstaged_activated {
            self.stage_index(i);
            return true;
        }
        let staged_activated = self.staged_list.borrow_mut().take_activated();
        if let Some(i) = staged_activated {
            self.unstage_index(i);
            return true;
        }

        let u = self.unstaged_list.borrow().selected_index();
        let s = self.staged_list.borrow().selected_index();
        if let Some(i) = u
            && self.prev_unstaged_sel != Some(i)
        {
            self.apply_commit_selection(Side::Unstaged, i);
            return true;
        }
        if let Some(i) = s
            && self.prev_staged_sel != Some(i)
        {
            self.apply_commit_selection(Side::Staged, i);
            return true;
        }
        // A selection may have been cleared elsewhere — keep trackers honest.
        self.prev_unstaged_sel = u;
        self.prev_staged_sel = s;

        let amend = self.amend_check.borrow().is_checked();
        if amend != self.last_amend {
            self.last_amend = amend;
            if amend
                && self.message_editor.borrow().text().trim().is_empty()
                && let Some(msg) = self.backend.head_message()
            {
                self.message_editor.borrow_mut().set_text(msg.trim_end());
            }
            // Re-base the staging view on HEAD's parent (or back on HEAD), so
            // the already-committed changes appear in / leave the staged list.
            self.rescan();
            return true;
        }

        false
    }

    fn stage_selected(&mut self) -> bool {
        let sel = self.unstaged_list.borrow().selected_index();
        match sel {
            Some(i) => {
                self.stage_index(i);
                true
            }
            None => false,
        }
    }

    /// Stage every unstaged file (git gui's "Stage Changed Files To Commit").
    fn stage_all(&mut self) -> bool {
        if self.working.unstaged.is_empty() {
            return false;
        }
        let paths: Vec<String> = self
            .working
            .unstaged
            .iter()
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            if let Err(e) = self.backend.stage(&path) {
                self.dialog.borrow_mut().show_error("Stage failed", &e);
                break;
            }
        }
        self.rescan();
        true
    }

    /// Append a `Signed-off-by` trailer for the configured identity to the
    /// message editor (git gui's "Sign Off").
    fn sign_off(&mut self) -> bool {
        let Some((name, email)) = self.backend.signature() else {
            self.dialog.borrow_mut().show_error(
                "Sign off",
                "No git identity configured. Set user.name and user.email.",
            );
            return true;
        };
        let body = self.message_editor.borrow().text();
        match with_signoff(&body, &name, &email) {
            Some(text) => {
                self.message_editor.borrow_mut().set_text(&text);
                true
            }
            // Already signed off — nothing to change.
            None => false,
        }
    }

    fn unstage_selected(&mut self) -> bool {
        let sel = self.staged_list.borrow().selected_index();
        match sel {
            Some(i) => {
                self.unstage_index(i);
                true
            }
            None => false,
        }
    }

    fn stage_index(&mut self, i: usize) {
        if let Some(file) = self.working.unstaged.get(i) {
            let path = file.path.clone();
            if let Err(e) = self.backend.stage(&path) {
                self.dialog.borrow_mut().show_error("Stage failed", &e);
            }
        }
        self.rescan();
    }

    fn unstage_index(&mut self, i: usize) {
        if let Some(file) = self.working.staged.get(i) {
            let path = file.path.clone();
            let amend = self.amend_check.borrow().is_checked();
            if let Err(e) = self.backend.unstage(&path, amend) {
                self.dialog.borrow_mut().show_error("Unstage failed", &e);
            }
        }
        self.rescan();
    }

    /// `git gui`'s "Revert Changes" (Ctrl+J): discard the working-tree changes
    /// to the selected *unstaged* file. Because the change can't be undone, this
    /// only arms the operation — it stashes what to do and pops a confirm dialog
    /// whose affirmative button drives [`AppCommand::PerformDiscard`]. A tracked
    /// file is reverted to its index copy; an untracked file (no committed or
    /// staged version to fall back to) is instead offered up for deletion.
    fn revert_selected(&mut self) -> bool {
        let Some(i) = self.unstaged_list.borrow().selected_index() else {
            return false;
        };
        let Some(file) = self.working.unstaged.get(i) else {
            return false;
        };
        let display = file.display();
        let path = file.path.clone();
        let (title, message, affirm) = if file.status == ChangeStatus::Untracked {
            self.pending_discard = Some(PendingDiscard::Delete(path));
            (
                "Delete File",
                format!(
                    "Delete untracked file\n{display}?\n\nIt is not tracked by git and cannot be recovered."
                ),
                "Delete File",
            )
        } else {
            self.pending_discard = Some(PendingDiscard::Revert(path));
            (
                "Revert Changes",
                format!(
                    "Revert unstaged changes in\n{display}?\n\nThese changes will be permanently lost."
                ),
                "Revert Changes",
            )
        };

        let commands = self.commands.clone();
        self.dialog
            .borrow_mut()
            .show_confirm(title, message, affirm, move |cx| {
                commands.borrow_mut().push(AppCommand::PerformDiscard);
                cx.request_paint();
            });
        true
    }

    /// Carry out the revert / delete the user confirmed in
    /// [`Self::revert_selected`].
    fn perform_discard(&mut self) -> bool {
        let (failure, result) = match self.pending_discard.take() {
            Some(PendingDiscard::Revert(path)) => ("Revert failed", self.backend.revert(&path)),
            Some(PendingDiscard::Delete(path)) => {
                ("Delete failed", self.backend.delete_untracked(&path))
            }
            None => return false,
        };
        if let Err(e) = result {
            self.dialog.borrow_mut().show_error(failure, &e);
        }
        self.rescan();
        true
    }

    fn do_commit(&mut self) -> bool {
        let amend = self.amend_check.borrow().is_checked();
        let message = self.message_editor.borrow().text();

        if self.working.staged.is_empty() && !amend {
            self.dialog.borrow_mut().show_error(
                "Nothing to commit",
                "Stage some changes first, or enable \u{201C}Amend last commit\u{201D}.",
            );
            return true;
        }

        match self.backend.commit(&message, amend) {
            Ok(()) => {
                self.message_editor.borrow_mut().set_text("");
                self.amend_check.borrow_mut().set_checked(false);
                self.last_amend = false;
                // Refresh history + working tree. Re-open when we can (so the
                // new commit shows in the log); otherwise refresh in place.
                if !self.reload() {
                    self.shown = None;
                    self.sync_browse(true);
                    self.rescan();
                }
                // Return to the log view, now showing the new commit.
                self.set_mode(Mode::Browse);
            }
            Err(e) => {
                self.dialog.borrow_mut().show_error("Commit failed", &e);
            }
        }
        true
    }

    /// `git gui`-style keyboard accelerators, handled before the active screen
    /// sees the event so they fire regardless of which pane holds focus — in
    /// particular Ctrl+Enter commits instead of inserting a newline in the
    /// message editor. Returns `true` when the keystroke was consumed.
    fn handle_shortcut(&mut self, event: &Event, ctx: &mut EventCtx) -> bool {
        // While a modal dialog is up it owns the keyboard.
        if self.dialog.borrow().is_open() {
            return false;
        }
        let Event::KeyDown { key, modifiers } = event else {
            return false;
        };
        // Only plain Ctrl-chords; Alt / Logo combos belong to the menu bar / OS.
        if !modifiers.control || modifiers.alt || modifiers.logo {
            return false;
        }

        let letter = match key {
            Key::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };

        // Ctrl+Q quits from either screen (git gui binds quit globally).
        if letter == Some('q') {
            ctx.close();
            return true;
        }

        // The remaining accelerators drive the staging screen.
        if self.mode != Mode::Commit {
            return false;
        }
        let command = if matches!(key, Key::Named(NamedKey::Enter)) {
            AppCommand::Commit
        } else {
            match letter {
                Some('r') => AppCommand::Rescan,
                Some('t') => AppCommand::StageSelected,
                Some('i') => AppCommand::StageAll,
                Some('j') => AppCommand::RevertSelected,
                Some('s') => AppCommand::SignOff,
                _ => return false,
            }
        };
        self.commands.borrow_mut().push(command);
        true
    }
}

impl Widget for GitClient {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.active_mut().paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.active_mut().paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Application accelerators take precedence over the focused pane.
        if !self.handle_shortcut(event, ctx) {
            self.active_mut().event(event, ctx);
        }
        // After the tree processes the event, apply commands and sync the
        // active screen's dependent panes.
        let mut dirty = self.drain_commands();
        dirty |= match self.mode {
            Mode::Browse => self.sync_browse(false),
            Mode::Commit => self.sync_commit(),
        };
        if dirty {
            ctx.request_paint();
        }
    }

    fn captures_pointer(&self) -> bool {
        self.active().captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.active().focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.active_mut().set_focused(focused);
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.browse_root.layout(bounds);
        self.commit_root.layout(bounds);
    }

    fn focus_first(&mut self) -> bool {
        match self.mode {
            // Start on the commit list rather than the leading search field,
            // so arrow keys navigate history immediately (gitk behavior).
            Mode::Browse => self.browse_root.focus_child(BROWSE_HISTORY_IDX),
            Mode::Commit => self.commit_root.focus_child(COMMIT_UNSTAGED_IDX),
        }
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.active().popup_request()
    }

    fn wants_ticks(&self) -> bool {
        self.active().wants_ticks()
    }
}

/// Build the browse-screen menu bar: File ▸ Reload / Exit, View ▸ Commit
/// Changes (switch screens), Help ▸ About.
fn build_browse_menu(
    commands: Rc<RefCell<Vec<AppCommand>>>,
    dialog: Rc<RefCell<Dialog>>,
) -> MenuBar {
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                cmd_item("&Reload", &commands, AppCommand::Reload),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()).with_accel("Ctrl+Q"),
            ],
        ))
        .add_menu(Menu::new(
            "&View",
            vec![cmd_item(
                "&Commit Changes",
                &commands,
                AppCommand::EnterCommitMode,
            )],
        ))
        .add_menu(Menu::new("&Help", vec![about_item(&dialog)]))
}

/// Build the commit-screen menu bar: File, Commit (the staging actions), View
/// ▸ Browse History, Help.
fn build_commit_menu(
    commands: Rc<RefCell<Vec<AppCommand>>>,
    dialog: Rc<RefCell<Dialog>>,
) -> MenuBar {
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                cmd_item("&Reload", &commands, AppCommand::Reload),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()).with_accel("Ctrl+Q"),
            ],
        ))
        .add_menu(Menu::new(
            "&Commit",
            vec![
                cmd_item("&Rescan", &commands, AppCommand::Rescan).with_accel("Ctrl+R"),
                MenuItem::separator(),
                cmd_item("&Stage Selected", &commands, AppCommand::StageSelected)
                    .with_accel("Ctrl+T"),
                cmd_item("Stage &All", &commands, AppCommand::StageAll).with_accel("Ctrl+I"),
                cmd_item("&Unstage Selected", &commands, AppCommand::UnstageSelected),
                cmd_item("Re&vert Changes", &commands, AppCommand::RevertSelected)
                    .with_accel("Ctrl+J"),
                MenuItem::separator(),
                cmd_item("Sign &Off", &commands, AppCommand::SignOff).with_accel("Ctrl+S"),
                cmd_item("&Commit", &commands, AppCommand::Commit).with_accel("Ctrl+Enter"),
            ],
        ))
        .add_menu(Menu::new(
            "&View",
            vec![cmd_item(
                "&Browse History",
                &commands,
                AppCommand::EnterBrowseMode,
            )],
        ))
        .add_menu(Menu::new("&Help", vec![about_item(&dialog)]))
}

/// A menu item that pushes `command` onto the deferred-command queue.
fn cmd_item(label: &str, commands: &Rc<RefCell<Vec<AppCommand>>>, command: AppCommand) -> MenuItem {
    let commands = commands.clone();
    MenuItem::action(label, move |cx| {
        commands.borrow_mut().push(command);
        cx.request_paint();
    })
}

/// The shared Help ▸ About item.
fn about_item(dialog: &Rc<RefCell<Dialog>>) -> MenuItem {
    let dialog = dialog.clone();
    MenuItem::action("&About", move |cx| {
        dialog.borrow_mut().show_info(
            "About Journey",
            "Journey\n\nA gitk-style repository browser\nbuilt on the saudade toolkit.",
        );
        cx.request_paint();
    })
}

/// A push button that pushes `command` onto the deferred-command queue.
fn command_button(
    label: &str,
    commands: &Rc<RefCell<Vec<AppCommand>>>,
    command: AppCommand,
) -> Button {
    let commands = commands.clone();
    Button::new(Rect::new(0, 0, 0, 0), label).on_click(move |cx| {
        commands.borrow_mut().push(command);
        cx.request_paint();
    })
}

/// First 8 hex chars of a SHA, for compact parent display.
fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// The message text after appending a `Signed-off-by` trailer for `name` /
/// `email`, or `None` when that exact trailer is already the last line. A prose
/// body is separated from the trailer by a blank line; an existing trailer
/// block keeps the sign-off tight against it (no blank line), matching git gui.
fn with_signoff(body: &str, name: &str, email: &str) -> Option<String> {
    let trailer = format!("Signed-off-by: {name} <{email}>");
    let last_line = body.lines().next_back().unwrap_or("").trim_end();
    if last_line.eq_ignore_ascii_case(&trailer) {
        return None;
    }
    let trimmed = body.trim_end();
    Some(if trimmed.is_empty() {
        trailer
    } else if is_trailer_line(last_line) {
        format!("{trimmed}\n{trailer}")
    } else {
        format!("{trimmed}\n\n{trailer}")
    })
}

/// Does `line` look like an RFC-822-style commit trailer (`Signed-off-by:`,
/// `Acked-by:`, …)? Used so a fresh sign-off stays tight against an existing
/// trailer block rather than getting an extra blank line before it.
fn is_trailer_line(line: &str) -> bool {
    let Some((key, _)) = line.split_once(':') else {
        return false;
    };
    let key = key.to_ascii_lowercase();
    key.ends_with("-by") && key.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// Build the display row for a working-tree pseudo-entry in the log.
fn wip_row(side: Side, count: usize) -> CommitRow {
    let summary = match side {
        Side::Unstaged => format!("Uncommitted changes ({count})"),
        Side::Staged => format!("Staged changes ({count})"),
    };
    CommitRow {
        summary,
        ..Default::default()
    }
}

/// The id of the current `HEAD` commit (the one the working tree sits on), so
/// the working-tree pseudo-rows can chain into it in the graph. Falls back to
/// the newest commit, or `None` for an empty history.
fn head_commit_id(commits: &[CommitInfo]) -> Option<String> {
    commits
        .iter()
        .find(|c| {
            c.refs
                .iter()
                .any(|r| matches!(r.kind, RefKind::Head | RefKind::DetachedHead))
        })
        .or_else(|| commits.first())
        .map(|c| c.id.clone())
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

/// Build a commit-list row from a commit: ref badges + summary on the left,
/// author and short date in the right-hand columns.
pub fn commit_row(commit: &CommitInfo) -> CommitRow {
    CommitRow {
        id: commit.id.clone(),
        parents: commit.parents.clone(),
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

#[cfg(test)]
mod tests {
    use super::{is_trailer_line, with_signoff};

    const NAME: &str = "Ada Lovelace";
    const EMAIL: &str = "ada@example.com";
    const SOB: &str = "Signed-off-by: Ada Lovelace <ada@example.com>";

    #[test]
    fn signoff_into_empty_message_is_just_the_trailer() {
        assert_eq!(with_signoff("", NAME, EMAIL).as_deref(), Some(SOB));
        assert_eq!(with_signoff("   \n", NAME, EMAIL).as_deref(), Some(SOB));
    }

    #[test]
    fn signoff_after_prose_gets_a_blank_separator_line() {
        assert_eq!(
            with_signoff("Fix the thing", NAME, EMAIL).as_deref(),
            Some(format!("Fix the thing\n\n{SOB}").as_str())
        );
    }

    #[test]
    fn signoff_after_a_trailer_block_stays_tight() {
        let body = "Fix the thing\n\nReviewed-by: B <b@example.com>";
        assert_eq!(
            with_signoff(body, NAME, EMAIL).as_deref(),
            Some(format!("{body}\n{SOB}").as_str())
        );
    }

    #[test]
    fn signoff_is_idempotent_when_already_last_line() {
        let body = format!("Fix the thing\n\n{SOB}");
        assert_eq!(with_signoff(&body, NAME, EMAIL), None);
    }

    #[test]
    fn trailer_lines_are_recognized() {
        assert!(is_trailer_line("Signed-off-by: A <a@x>"));
        assert!(is_trailer_line("Reviewed-by: B <b@x>"));
        assert!(is_trailer_line("Co-authored-by: C <c@x>"));
        assert!(!is_trailer_line("Just a normal sentence."));
        assert!(!is_trailer_line("Fixes: #123"));
        assert!(!is_trailer_line(""));
    }
}
