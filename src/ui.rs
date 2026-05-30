//! The top-level [`GitClient`] widget.
//!
//! `GitClient` drives two screens, each a flat [`Shell`] of panes:
//!
//! * **Browse** — the gitk-style history browser (commit list, diff, files).
//! * **Commit** — a `git gui`-style staging screen (unstaged / staged file
//!   lists, a per-file diff, a message editor and a commit button).
//!
//! retrogui widgets are callback-free, so the cross-pane wiring is done here:
//! after each event the active screen's selections (and a small command queue
//! menus/buttons push into) are polled, and dependent panes are rebuilt from
//! the [`RepoBackend`].

use std::cell::RefCell;
use std::rc::Rc;

use retrogui::{
    Button, Checkbox, Dialog, Event, EventCtx, List, ListItem, Menu, MenuBar, MenuItem, Painter,
    PopupRequest, Rect, TextEditor, Theme, Widget,
};

use crate::backend::{
    CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, RepoBackend, WorkingStatus,
};
use crate::widgets::{
    compute_graph, layout, CommitList, CommitRow, DiffView, Heading, SearchBar, Shared, Shell,
};

/// Direct-child index of the history list in the browse shell (focused first).
const BROWSE_HISTORY_IDX: usize = 2;
/// Direct-child index of the unstaged list in the commit shell.
const COMMIT_UNSTAGED_IDX: usize = 2;

/// A closure that re-opens the repository (used by File ▸ Reload and after a
/// commit). `None` for fixture-backed clients in tests.
type ReopenFn = Box<dyn Fn() -> Option<Rc<dyn RepoBackend>>>;

/// Which screen is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Commit,
}

/// Which working-tree list a commit-mode selection came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Unstaged,
    Staged,
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
    UnstageSelected,
    Commit,
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
    visible: Vec<usize>,
    last_query: String,
    current_files: Vec<FileChange>,
    shown_commit: Option<usize>,
    shown_file: Option<usize>,

    // ---- commit sync state ------------------------------------------------
    working: WorkingStatus,
    prev_unstaged_sel: Option<usize>,
    prev_staged_sel: Option<usize>,
    last_amend: bool,
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

        // Add order sets the Tab focus order: search → commits → diff → files
        // (the menu bar isn't focusable; it works via accelerators).
        let browse_root = Shell::new()
            .add(build_browse_menu(commands.clone(), dialog.clone()), layout::browse_menu)
            .add(Shared::new(search.clone()), layout::browse_toolbar)
            .add(Shared::new(commit_list.clone()), layout::browse_history)
            .add(Shared::new(diff_view.clone()), layout::browse_diff)
            .add(Shared::new(file_list.clone()), layout::browse_files)
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

        let commit_root = Shell::new()
            .add(build_commit_menu(commands.clone(), dialog.clone()), layout::commit_menu)
            .add(Shared::new(unstaged_heading.clone()), layout::commit_unstaged_label)
            .add(Shared::new(unstaged_list.clone()), layout::commit_unstaged_list)
            .add(Shared::new(staged_heading.clone()), layout::commit_staged_label)
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
            visible: Vec::new(),
            last_query: String::new(),
            current_files: Vec::new(),
            shown_commit: None,
            shown_file: None,
            working: WorkingStatus::default(),
            prev_unstaged_sel: None,
            prev_staged_sel: None,
            last_amend: false,
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
                AppCommand::UnstageSelected => self.unstage_selected(),
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
        self.shown_commit = None;
        self.shown_file = None;
        self.last_query.clear();
        self.search.borrow_mut().clear();
        self.sync_browse(true);
        self.rescan();
        true
    }

    // ---- browse screen ----------------------------------------------------

    /// Reload browse panes from the current selection state. When the commit
    /// selection changes, reload the file list and show the whole commit's
    /// diff; when the file selection changes, narrow the diff to that file.
    fn sync_browse(&mut self, force: bool) -> bool {
        let mut changed = false;

        // 1. Re-filter the commit list when the query changes.
        let query = self.search.borrow().text().trim().to_lowercase();
        if force || query != self.last_query {
            self.last_query = query.clone();
            self.rebuild_commits(&query);
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
            self.shown_file = None;
            let diff = match commit_idx {
                Some(idx) => self.commit_detail(idx),
                None => Diff::default(),
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
                _ => Diff::default(),
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
        let rows: Vec<CommitRow> = self.visible.iter().map(|&i| commit_row(&commits[i])).collect();

        // The DAG graph needs the full parent chain, which only holds for the
        // complete, unfiltered history; hide it while a filter is active.
        let graph = if query.is_empty() {
            let dag: Vec<(String, Vec<String>)> = commits
                .iter()
                .map(|c| (c.id.clone(), c.parents.clone()))
                .collect();
            Some(compute_graph(&dag))
        } else {
            None
        };

        let mut list = self.commit_list.borrow_mut();
        list.set_rows(rows);
        list.set_graph(graph);
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
        self.unstaged_heading
            .borrow_mut()
            .set_text(format!("Unstaged Changes ({})", self.working.unstaged.len()));
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
                // new commit appears in the browser); otherwise just rescan.
                if !self.reload() {
                    self.rescan();
                }
            }
            Err(e) => {
                self.dialog.borrow_mut().show_error("Commit failed", &e);
            }
        }
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
        self.active_mut().event(event, ctx);
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
fn build_browse_menu(commands: Rc<RefCell<Vec<AppCommand>>>, dialog: Rc<RefCell<Dialog>>) -> MenuBar {
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                cmd_item("&Reload", &commands, AppCommand::Reload),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ))
        .add_menu(Menu::new(
            "&View",
            vec![cmd_item("&Commit Changes", &commands, AppCommand::EnterCommitMode)],
        ))
        .add_menu(Menu::new("&Help", vec![about_item(&dialog)]))
}

/// Build the commit-screen menu bar: File, Commit (the staging actions), View
/// ▸ Browse History, Help.
fn build_commit_menu(commands: Rc<RefCell<Vec<AppCommand>>>, dialog: Rc<RefCell<Dialog>>) -> MenuBar {
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                cmd_item("&Reload", &commands, AppCommand::Reload),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()),
            ],
        ))
        .add_menu(Menu::new(
            "&Commit",
            vec![
                cmd_item("&Rescan", &commands, AppCommand::Rescan),
                MenuItem::separator(),
                cmd_item("&Stage Selected", &commands, AppCommand::StageSelected),
                cmd_item("&Unstage Selected", &commands, AppCommand::UnstageSelected),
                MenuItem::separator(),
                cmd_item("&Commit", &commands, AppCommand::Commit),
            ],
        ))
        .add_menu(Menu::new(
            "&View",
            vec![cmd_item("&Browse History", &commands, AppCommand::EnterBrowseMode)],
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
            "Journey\n\nA gitk-style repository browser\nbuilt on the retrogui toolkit.",
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
