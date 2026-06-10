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
use std::collections::BTreeSet;
use std::rc::Rc;

use saudade::{
    Button, Checkbox, Dialog, Event, EventCtx, List, ListItem, Menu, MenuBar, MenuItem, Painter,
    PopupRequest, Rect, SvgImage, TextEditor, Theme, Widget, include_svg,
};

use crate::backend::{
    BlobPair, ChangeStatus, CommitInfo, Diff, DiffLine, DiffLineKind, FileChange, PartialMode,
    RefKind, RepoBackend, WorkingStatus, build_partial_patch,
};
use crate::imagediff::{ImageComparison, is_image_path};
use crate::widgets::{
    CommitList, CommitRow, DiffMode, DiffPane, Heading, SearchBar, Shared, Shell, compute_graph,
    layout,
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
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Select the next / previous image file in the active list (View ▸ Next /
    /// Previous Image, Ctrl+N / Ctrl+P).
    NextImage,
    PrevImage,
    /// Cycle the shown image comparison mode (View ▸ Switch Mode, Ctrl+M).
    CycleImageMode,
    /// Show just the before / after side of the shown image (View ▸ Before /
    /// After Image, Ctrl+Left / Ctrl+Right).
    ShowImageBefore,
    ShowImageAfter,
}

/// Live state the View menu reads, refreshed after every event (see
/// [`GitClient::update_menu_nav`]) so the menu reflects the current screen: it
/// greys the image actions that don't apply — whether the active diff pane
/// shows an image (Switch Mode / Before / After) and whether the active file
/// list holds another image to jump to (Next / Previous) — and checkmarks the
/// active mode against the Browse History / Commit Changes entries.
#[derive(Default)]
struct MenuNav {
    mode: Mode,
    showing_image: bool,
    can_nav_images: bool,
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
    diff_pane: Rc<RefCell<DiffPane>>,

    // ---- commit screen ----------------------------------------------------
    commit_root: Shell,
    unstaged_list: Rc<RefCell<List>>,
    staged_list: Rc<RefCell<List>>,
    unstaged_heading: Rc<RefCell<Heading>>,
    staged_heading: Rc<RefCell<Heading>>,
    commit_diff_pane: Rc<RefCell<DiffPane>>,
    message_editor: Rc<RefCell<TextEditor>>,
    amend_check: Rc<RefCell<Checkbox>>,
    stage_btn: Rc<RefCell<Button>>,
    unstage_btn: Rc<RefCell<Button>>,
    rescan_btn: Rc<RefCell<Button>>,
    /// Whether the last layout was at a narrow width; the Stage/Unstage/Rescan
    /// buttons drop their text in narrow mode (see [`Self::apply_narrow`]).
    narrow: bool,

    // ---- shared -----------------------------------------------------------
    dialog: Rc<RefCell<Dialog>>,
    commands: Rc<RefCell<Vec<AppCommand>>>,
    reopen: Option<ReopenFn>,
    /// Enable state the View-menu image actions read; kept current by
    /// [`Self::update_menu_nav`].
    nav_state: Rc<RefCell<MenuNav>>,

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
        let nav_state: Rc<RefCell<MenuNav>> = Rc::new(RefCell::new(MenuNav::default()));

        // Browse-screen widgets.
        let search = Rc::new(RefCell::new(SearchBar::new(Rect::new(0, 0, 0, 0))));
        let commit_list = Rc::new(RefCell::new(CommitList::new(Rect::new(0, 0, 0, 0))));
        let file_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let diff_pane = Rc::new(RefCell::new(DiffPane::new(Rect::new(0, 0, 0, 0))));

        // Add order sets the Tab focus order: search → commits → files → diff
        // (the menu bar isn't focusable; it works via accelerators). The file
        // list follows the commit list so Tab walks the panes left-to-right.
        // No flat background fill: the panes float on the window's desktop
        // pattern, which shows through the padding around them.
        let browse_root = Shell::new()
            .no_background()
            .add(
                build_browse_menu(commands.clone(), dialog.clone(), nav_state.clone()),
                layout::browse_menu,
            )
            .add(Shared::new(search.clone()), layout::browse_toolbar)
            .add(Shared::new(commit_list.clone()), layout::browse_history)
            .add(Shared::new(file_list.clone()), layout::browse_files)
            .add(Shared::new(diff_pane.clone()), layout::browse_diff)
            .add_overlay(Shared::new(dialog.clone()));

        // Commit-screen widgets.
        let unstaged_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let staged_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));
        let unstaged_heading = Rc::new(RefCell::new(Heading::new("Unstaged Changes")));
        let staged_heading = Rc::new(RefCell::new(Heading::new("Staged Changes")));
        let commit_diff_pane = Rc::new(RefCell::new(DiffPane::new(Rect::new(0, 0, 0, 0))));
        let message_editor = Rc::new(RefCell::new(TextEditor::new(Rect::new(0, 0, 0, 0))));
        let amend_check = Rc::new(RefCell::new(Checkbox::new(
            Rect::new(0, 0, 0, 0),
            "Amend last commit",
        )));
        // Created with the wide (symbol + text) labels; `layout` swaps them for
        // symbol-only when the window is narrow.
        let [stage_lbl, unstage_lbl, rescan_lbl] = left_btn_labels(false);
        let stage_btn = Rc::new(RefCell::new(command_button(
            stage_lbl,
            &commands,
            AppCommand::StageSelected,
        )));
        let unstage_btn = Rc::new(RefCell::new(command_button(
            unstage_lbl,
            &commands,
            AppCommand::UnstageSelected,
        )));
        let rescan_btn = Rc::new(RefCell::new(command_button(
            rescan_lbl,
            &commands,
            AppCommand::Rescan,
        )));

        // No flat background fill: the staging panes float on the window's
        // desktop pattern (git-gui style), which shows through the gaps.
        let commit_root = Shell::new()
            .no_background()
            .add(
                build_commit_menu(commands.clone(), dialog.clone(), nav_state.clone()),
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
            .add(Shared::new(stage_btn.clone()), layout::commit_stage_btn)
            .add(Shared::new(unstage_btn.clone()), layout::commit_unstage_btn)
            .add(Shared::new(rescan_btn.clone()), layout::commit_rescan_btn)
            .add(Heading::new("Diff"), layout::commit_diff_label)
            .add(Shared::new(commit_diff_pane.clone()), layout::commit_diff)
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
            diff_pane,
            commit_root,
            unstaged_list,
            staged_list,
            unstaged_heading,
            staged_heading,
            commit_diff_pane,
            message_editor,
            amend_check,
            stage_btn,
            unstage_btn,
            rescan_btn,
            narrow: false,
            dialog,
            commands,
            reopen: None,
            nav_state,
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
        client.update_menu_nav();
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

    /// Apply width-only affordances: in narrow mode the Stage/Unstage/Rescan
    /// buttons shrink to share a third-width column (see `layout`), so they drop
    /// their text and keep just the symbol. Cheap no-op when the state is
    /// unchanged.
    fn apply_narrow(&mut self, narrow: bool) {
        if narrow == self.narrow {
            return;
        }
        self.narrow = narrow;
        let [stage, unstage, rescan] = left_btn_labels(narrow);
        self.stage_btn.borrow_mut().label = stage.to_string();
        self.unstage_btn.borrow_mut().label = unstage.to_string();
        self.rescan_btn.borrow_mut().label = rescan.to_string();
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
                AppCommand::NextImage => self.navigate_image(true),
                AppCommand::PrevImage => self.navigate_image(false),
                AppCommand::CycleImageMode => self.cycle_image_mode(),
                AppCommand::ShowImageBefore => self.show_image_side(true),
                AppCommand::ShowImageAfter => self.show_image_side(false),
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
            self.show_browse_diff(sel, None);
            changed = true;
        }

        // 3. Narrow the diff to a single file when one is selected.
        let file_sel = self.file_list.borrow().selected_index();
        if file_sel != self.shown_file {
            self.shown_file = file_sel;
            self.show_browse_diff(self.shown, file_sel);
            changed = true;
        }

        changed
    }

    /// Update the browse diff pane for a log selection: a graphical image
    /// comparison when a single image file is selected, otherwise the text diff
    /// (whole-commit overview when no single file is picked).
    fn show_browse_diff(&self, sel: Option<RowRef>, file_sel: Option<usize>) {
        if let Some(file) = file_sel.and_then(|f| self.current_files.get(f))
            && is_image_path(&file.path)
            && let Some(cmp) = self.browse_image(sel, &file.path)
        {
            self.diff_pane.borrow_mut().show_image(cmp);
            return;
        }
        let diff = self.selection_diff(sel, file_sel);
        self.diff_pane.borrow_mut().set_diff(diff);
    }

    /// Build the image comparison for `path` under the current log selection,
    /// pulling the two blobs from the commit or the working tree. `None` when
    /// neither side decodes (the caller then shows the text diff).
    fn browse_image(&self, sel: Option<RowRef>, path: &str) -> Option<ImageComparison> {
        let blobs: BlobPair = match sel? {
            RowRef::Commit(cidx) => self.backend.commit_file_blobs(cidx, path),
            RowRef::Wip(side) => {
                self.backend
                    .working_file_blobs(path, matches!(side, Side::Staged), false)
            }
        };
        ImageComparison::from_blobs(&blobs)
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
        self.rescan_selecting(None);
    }

    /// Re-read the working tree and rebuild the staged / unstaged lists. When
    /// `prefer` names a `(side, path)` that survives the rescan, that file stays
    /// selected (so partial staging keeps the same file focused in the diff);
    /// otherwise the selection defaults to the first file.
    fn rescan_selecting(&mut self, prefer: Option<(Side, String)>) {
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
        self.staged_heading
            .borrow_mut()
            .set_text(format!("Staged Changes ({})", self.working.staged.len()));

        self.prev_unstaged_sel = None;
        self.prev_staged_sel = None;
        {
            let mut view = self.commit_diff_pane.borrow_mut();
            view.set_mode(DiffMode::Plain);
            view.set_diff(Diff::default());
        }

        // Keep the preferred file selected when it's still present; otherwise
        // default to the first file so the diff pane isn't blank.
        let target = prefer.and_then(|(side, path)| {
            let files = match side {
                Side::Unstaged => &self.working.unstaged,
                Side::Staged => &self.working.staged,
            };
            files.iter().position(|f| f.path == path).map(|i| (side, i))
        });
        match target {
            Some((side, i)) => self.apply_commit_selection(side, i),
            None if !self.working.unstaged.is_empty() => {
                self.apply_commit_selection(Side::Unstaged, 0)
            }
            None if !self.working.staged.is_empty() => self.apply_commit_selection(Side::Staged, 0),
            None => {}
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
        // Unstaged files offer per-line staging; staged files, per-line
        // unstaging. (Only the commit screen's diff view is ever non-Plain.)
        let mode = match side {
            Side::Unstaged => DiffMode::Stage,
            Side::Staged => DiffMode::Unstage,
        };
        let mut view = self.commit_diff_pane.borrow_mut();
        view.set_mode(mode);
        // An image file shows the graphical comparison; line-range staging
        // doesn't apply to it (the image view has no selectable lines).
        if let Some(file) = files.get(i)
            && is_image_path(&file.path)
            && let Some(cmp) = ImageComparison::from_blobs(
                &self.backend.working_file_blobs(&file.path, staged, amend),
            )
        {
            view.show_image(cmp);
            return;
        }
        let diff = files
            .get(i)
            .map(|f| self.backend.working_diff(&f.path, staged, amend))
            .unwrap_or_default();
        view.set_diff(diff);
    }

    /// Poll the commit screen after an event: handle stage/unstage activations
    /// (double-click or Enter on a list), selection-driven diff updates, and
    /// the amend toggle.
    fn sync_commit(&mut self) -> bool {
        // The Stage/Unstage button floating over a highlighted diff range.
        let action = self.commit_diff_pane.borrow_mut().take_action();
        if let Some((lo, hi)) = action {
            return self.apply_partial(lo, hi);
        }

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

    /// The file whose diff the commit screen is currently showing, as
    /// `(side, index)` — whichever list holds the selection.
    fn current_commit_target(&self) -> Option<(Side, usize)> {
        if let Some(i) = self.unstaged_list.borrow().selected_index() {
            Some((Side::Unstaged, i))
        } else {
            self.staged_list
                .borrow()
                .selected_index()
                .map(|i| (Side::Staged, i))
        }
    }

    /// Stage (or unstage) just the highlighted rows `lo..=hi` of the current
    /// file's diff: rebuild a patch covering only those lines and apply it to
    /// the index. An unstaged file stages the selection; a staged one unstages
    /// it. A no-op when the range covers no actual change.
    fn apply_partial(&mut self, lo: usize, hi: usize) -> bool {
        let Some((side, i)) = self.current_commit_target() else {
            return false;
        };
        let staged = matches!(side, Side::Staged);
        let files = match side {
            Side::Unstaged => &self.working.unstaged,
            Side::Staged => &self.working.staged,
        };
        let Some(path) = files.get(i).map(|f| f.path.clone()) else {
            return false;
        };

        let amend = self.amend_check.borrow().is_checked();
        let diff = self.backend.working_diff(&path, staged, amend);
        let mode = if staged {
            PartialMode::Unstage
        } else {
            PartialMode::Stage
        };
        let selected: BTreeSet<usize> = (lo..=hi).collect();
        let Some(patch) = build_partial_patch(&diff, &selected, mode) else {
            return false;
        };

        if let Err(e) = self.backend.apply_to_index(&patch) {
            let title = if staged {
                "Unstage failed"
            } else {
                "Stage failed"
            };
            self.dialog.borrow_mut().show_error(title, &e);
        }
        // Keep the same file focused on the same side so the user can stage the
        // next chunk without the selection jumping back to the first file.
        self.rescan_selecting(Some((side, path)));
        true
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

    // ---- graphical image diff (both screens) ------------------------------

    /// The diff pane of the active screen.
    fn active_diff_pane(&self) -> Rc<RefCell<DiffPane>> {
        match self.mode {
            Mode::Browse => self.diff_pane.clone(),
            Mode::Commit => self.commit_diff_pane.clone(),
        }
    }

    /// Whether the active diff pane is currently showing a graphical image diff
    /// (vs. a text diff) — the enable state for Switch Mode / Before / After.
    fn active_showing_image(&self) -> bool {
        self.active_diff_pane().borrow().showing_image()
    }

    /// The commit-screen list a navigation acts on: whichever holds the
    /// selection, else the first non-empty list (unstaged preferred).
    fn active_commit_side(&self) -> Side {
        if self.unstaged_list.borrow().selected_index().is_some() {
            Side::Unstaged
        } else if self.staged_list.borrow().selected_index().is_some() {
            Side::Staged
        } else if !self.working.unstaged.is_empty() {
            Side::Unstaged
        } else {
            Side::Staged
        }
    }

    /// Whether the active file list holds an image file other than the current
    /// selection — the enable state for Next / Previous Image.
    fn has_other_image(&self) -> bool {
        match self.mode {
            Mode::Browse => {
                let sel = self.file_list.borrow().selected_index();
                other_image_exists(&self.current_files, sel)
            }
            Mode::Commit => match self.active_commit_side() {
                Side::Unstaged => other_image_exists(
                    &self.working.unstaged,
                    self.unstaged_list.borrow().selected_index(),
                ),
                Side::Staged => other_image_exists(
                    &self.working.staged,
                    self.staged_list.borrow().selected_index(),
                ),
            },
        }
    }

    /// Move the active list's selection to the next / previous image file. The
    /// normal selection-sync then shows it. Returns `true` when it moved.
    fn navigate_image(&mut self, forward: bool) -> bool {
        match self.mode {
            Mode::Browse => {
                let sel = self.file_list.borrow().selected_index();
                let Some(target) = next_image_index(&self.current_files, sel, forward) else {
                    return false;
                };
                self.file_list.borrow_mut().set_selected(Some(target));
                true
            }
            Mode::Commit => {
                let side = self.active_commit_side();
                let (target, list) = match side {
                    Side::Unstaged => (
                        next_image_index(
                            &self.working.unstaged,
                            self.unstaged_list.borrow().selected_index(),
                            forward,
                        ),
                        &self.unstaged_list,
                    ),
                    Side::Staged => (
                        next_image_index(
                            &self.working.staged,
                            self.staged_list.borrow().selected_index(),
                            forward,
                        ),
                        &self.staged_list,
                    ),
                };
                let Some(target) = target else { return false };
                list.borrow_mut().set_selected(Some(target));
                true
            }
        }
    }

    /// Cycle the shown image comparison mode (no-op unless an image is shown).
    fn cycle_image_mode(&mut self) -> bool {
        let pane = self.active_diff_pane();
        let mut pane = pane.borrow_mut();
        if !pane.showing_image() {
            return false;
        }
        pane.cycle_image_mode();
        true
    }

    /// Show just the before / after side of the shown image (no-op unless an
    /// image is shown).
    fn show_image_side(&mut self, before: bool) -> bool {
        let pane = self.active_diff_pane();
        let mut pane = pane.borrow_mut();
        if !pane.showing_image() {
            return false;
        }
        pane.show_image_side(before);
        true
    }

    /// Refresh the View-menu state from the active screen: the image actions'
    /// enable state (so the menu greys what doesn't currently apply) and the
    /// active mode (so the Browse / Commit entries show the checkmark).
    fn update_menu_nav(&self) {
        let showing_image = self.active_showing_image();
        let can_nav_images = self.has_other_image();
        let mut nav = self.nav_state.borrow_mut();
        nav.mode = self.mode;
        nav.showing_image = showing_image;
        nav.can_nav_images = can_nav_images;
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
        // Keyboard accelerators live on each screen's menu bar (`with_accel`):
        // the Shell's accelerator pass hands every key to the bar before the
        // focused pane, so e.g. Ctrl+Enter commits instead of inserting a
        // newline in the message editor. A chord whose item is disabled falls
        // through to the focused widget, and an open dialog overlay owns the
        // keyboard outright — both come with the routing, no gating here.
        self.active_mut().event(event, ctx);
        // After the tree processes the event, apply commands and sync the
        // active screen's dependent panes.
        let mut dirty = self.drain_commands();
        dirty |= match self.mode {
            Mode::Browse => self.sync_browse(false),
            Mode::Commit => self.sync_commit(),
        };
        // Keep the View-menu image actions' enable state current for the next
        // paint (e.g. when the menu is about to open).
        self.update_menu_nav();
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
        self.apply_narrow(bounds.w <= layout::NARROW_W);
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

/// Build the browse-screen menu bar: File ▸ Reload / Exit, View ▸ the
/// Browse / Commit mode switches + the image-diff actions, Help ▸ About.
fn build_browse_menu(
    commands: Rc<RefCell<Vec<AppCommand>>>,
    dialog: Rc<RefCell<Dialog>>,
    nav: Rc<RefCell<MenuNav>>,
) -> MenuBar {
    let mut view = mode_items(&commands, &nav);
    view.push(MenuItem::separator());
    view.extend(image_view_items(&commands, &nav));
    MenuBar::new(Rect::new(0, 0, 0, 0))
        .add_menu(Menu::new(
            "&File",
            vec![
                cmd_item("&Reload", &commands, AppCommand::Reload),
                MenuItem::separator(),
                MenuItem::action("E&xit", |cx| cx.close()).with_accel("Ctrl+Q"),
            ],
        ))
        .add_menu(Menu::new("&View", view))
        .add_menu(Menu::new("&Help", vec![about_item(&dialog)]))
}

/// Build the commit-screen menu bar: File, Commit (the staging actions), View
/// ▸ the Browse / Commit mode switches + the image-diff actions, Help.
fn build_commit_menu(
    commands: Rc<RefCell<Vec<AppCommand>>>,
    dialog: Rc<RefCell<Dialog>>,
    nav: Rc<RefCell<MenuNav>>,
) -> MenuBar {
    let mut view = mode_items(&commands, &nav);
    view.push(MenuItem::separator());
    view.extend(image_view_items(&commands, &nav));
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
                cmd_item("&Unstage Selected", &commands, AppCommand::UnstageSelected)
                    .with_accel("Ctrl+U"),
                cmd_item("Re&vert Changes", &commands, AppCommand::RevertSelected)
                    .with_accel("Ctrl+J"),
                MenuItem::separator(),
                cmd_item("Sign &Off", &commands, AppCommand::SignOff).with_accel("Ctrl+S"),
                cmd_item("&Commit", &commands, AppCommand::Commit).with_accel("Ctrl+Enter"),
            ],
        ))
        .add_menu(Menu::new("&View", view))
        .add_menu(Menu::new("&Help", vec![about_item(&dialog)]))
}

/// The View-menu mode switches, shared by both screens: Browse History and
/// Commit Changes, each checkmarked when its screen is the active one (read live
/// from `nav`). Picking the entry that's already checked is a harmless no-op, so
/// both stay enabled. The accelerators are Ctrl+1 / Ctrl+2 — view switching à la
/// GitHub Desktop — deliberately clear of every editing chord (Ctrl+C must stay
/// copy in the commit message editor), so no fall-through gating is needed.
fn mode_items(commands: &Rc<RefCell<Vec<AppCommand>>>, nav: &Rc<RefCell<MenuNav>>) -> Vec<MenuItem> {
    let is_mode = |mode: Mode| {
        let nav = nav.clone();
        move || nav.borrow().mode == mode
    };
    vec![
        cmd_item("&Browse History", commands, AppCommand::EnterBrowseMode)
            .with_accel("Ctrl+1")
            .with_checked(is_mode(Mode::Browse)),
        cmd_item("&Commit Changes", commands, AppCommand::EnterCommitMode)
            .with_accel("Ctrl+2")
            .with_checked(is_mode(Mode::Commit)),
    ]
}

/// The View-menu entries that drive the graphical image diff, shared by both
/// screens. Next / Previous Image walk the image files in the active list
/// (disabled when there's no other image to jump to); Switch Mode / Before /
/// After act on the shown comparison (disabled unless an image is on screen).
/// The enable predicates read live [`MenuNav`] state via the captured `nav`.
fn image_view_items(
    commands: &Rc<RefCell<Vec<AppCommand>>>,
    nav: &Rc<RefCell<MenuNav>>,
) -> Vec<MenuItem> {
    let can_nav = || {
        let nav = nav.clone();
        move || nav.borrow().can_nav_images
    };
    let showing = || {
        let nav = nav.clone();
        move || nav.borrow().showing_image
    };
    vec![
        cmd_item("&Next Image", commands, AppCommand::NextImage)
            .with_accel("Ctrl+N")
            .with_enabled(can_nav()),
        cmd_item("&Previous Image", commands, AppCommand::PrevImage)
            .with_accel("Ctrl+P")
            .with_enabled(can_nav()),
        MenuItem::separator(),
        cmd_item("Switch &Mode", commands, AppCommand::CycleImageMode)
            .with_accel("Ctrl+M")
            .with_enabled(showing()),
        cmd_item("Be&fore Image", commands, AppCommand::ShowImageBefore)
            .with_accel("Ctrl+Left")
            .with_enabled(showing()),
        cmd_item("&After Image", commands, AppCommand::ShowImageAfter)
            .with_accel("Ctrl+Right")
            .with_enabled(showing()),
    ]
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
            "About Git Journey",
            "Git Journey\n\nA gitk-style repository browser\nbuilt on the Saudade toolkit.",
        );
        cx.request_paint();
    })
}

/// A push button that pushes `command` onto the deferred-command queue.
/// Labels for the Stage / Unstage / Rescan buttons. The arrow-from-bar and
/// refresh symbols always lead; in narrow mode that's all there's room for, so
/// the words are dropped.
fn left_btn_labels(narrow: bool) -> [&'static str; 3] {
    if narrow {
        ["\u{21A7}", "\u{21A5}", "\u{21BB}"]
    } else {
        ["\u{21A7} Stage", "\u{21A5} Unstage", "\u{21BB} Rescan"]
    }
}

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

/// Index of the next (`forward`) or previous image file in `files` relative to
/// the selection `cur`. Navigation does not wrap: `None` once there is no further
/// image in the chosen direction. When `cur` isn't itself an image, the nearest
/// image in the chosen direction is picked.
fn next_image_index(files: &[FileChange], cur: Option<usize>, forward: bool) -> Option<usize> {
    let images: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| is_image_path(&f.path))
        .map(|(i, _)| i)
        .collect();
    let cur = cur.map(|c| c as i32);
    if forward {
        match cur {
            Some(c) => images.iter().copied().find(|&i| i as i32 > c),
            None => images.first().copied(),
        }
    } else {
        match cur {
            Some(c) => images.iter().rev().copied().find(|&i| (i as i32) < c),
            None => images.last().copied(),
        }
    }
}

/// Whether `files` holds an image file other than the one already at `cur` — the
/// condition that enables the Next / Previous Image actions. Unlike
/// [`next_image_index`] this is direction-agnostic, so the actions stay enabled
/// at either end of the list (where one direction has nowhere to go).
fn other_image_exists(files: &[FileChange], cur: Option<usize>) -> bool {
    files
        .iter()
        .enumerate()
        .any(|(i, f)| is_image_path(&f.path) && Some(i) != cur)
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

/// Format a changed file as a list row: a colored status marker in the icon
/// gutter (see [`status_icon`]) followed by the path. The marker replaces the
/// old single-letter text badge — the same A/M/D/… letters, but baked from SVG
/// so they stay crisp at any DPI and legible on the navy selection band.
pub fn file_row(file: &FileChange) -> ListItem {
    ListItem::new(file.display()).with_svg_icon(status_icon(file.status))
}

/// The compile-time-baked status marker for a [`ChangeStatus`] — a small chip
/// carrying the letter [`ChangeStatus::badge`] would print. Each SVG lives in
/// `assets/status/`; `include_svg!` flattens it to polygons at build time, so no
/// SVG parser ships in the binary (see saudade's `include_svg!`).
fn status_icon(status: ChangeStatus) -> SvgImage {
    const ADDED: SvgImage = include_svg!("assets/status/added.svg");
    const MODIFIED: SvgImage = include_svg!("assets/status/modified.svg");
    const DELETED: SvgImage = include_svg!("assets/status/deleted.svg");
    const RENAMED: SvgImage = include_svg!("assets/status/renamed.svg");
    const COPIED: SvgImage = include_svg!("assets/status/copied.svg");
    const TYPECHANGE: SvgImage = include_svg!("assets/status/typechange.svg");
    const UNKNOWN: SvgImage = include_svg!("assets/status/unknown.svg");

    match status {
        ChangeStatus::Added => ADDED,
        ChangeStatus::Modified => MODIFIED,
        ChangeStatus::Deleted => DELETED,
        ChangeStatus::Renamed => RENAMED,
        ChangeStatus::Copied => COPIED,
        ChangeStatus::TypeChange => TYPECHANGE,
        ChangeStatus::Untracked | ChangeStatus::Other => UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_trailer_line, next_image_index, other_image_exists, with_signoff};
    use crate::backend::{ChangeStatus, FileChange};

    fn files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange {
                path: (*p).to_string(),
                old_path: None,
                status: ChangeStatus::Modified,
            })
            .collect()
    }

    #[test]
    fn next_image_navigates_only_images_without_wrapping() {
        // Images sit at indices 1, 3, 4; text files at 0 and 2.
        let f = files(&["a.txt", "logo.png", "notes.md", "icon.gif", "photo.jpeg"]);
        // Forward / backward step image-to-image.
        assert_eq!(next_image_index(&f, Some(1), true), Some(3));
        assert_eq!(next_image_index(&f, Some(3), false), Some(1));
        // …but stop at the ends rather than wrapping.
        assert_eq!(next_image_index(&f, Some(4), true), None);
        assert_eq!(next_image_index(&f, Some(1), false), None);
        // From a non-image row, jump to the nearest image in that direction.
        assert_eq!(next_image_index(&f, Some(2), true), Some(3));
        assert_eq!(next_image_index(&f, Some(2), false), Some(1));
        // No selection starts at the first / last image.
        assert_eq!(next_image_index(&f, None, true), Some(1));
        assert_eq!(next_image_index(&f, None, false), Some(4));
    }

    #[test]
    fn other_image_exists_stays_true_at_the_ends() {
        // Three images: navigation can't wrap, but the Next / Previous actions
        // stay enabled at either end because the *other* direction still moves.
        let f = files(&["a.txt", "logo.png", "notes.md", "icon.gif", "photo.jpeg"]);
        assert!(other_image_exists(&f, Some(4)));
        assert!(other_image_exists(&f, Some(1)));
        // A lone image already selected leaves nowhere to go.
        let one = files(&["a.txt", "logo.png"]);
        assert!(!other_image_exists(&one, Some(1)));
        // …but it's reachable from a non-image row, and from no selection.
        assert!(other_image_exists(&one, Some(0)));
        assert!(other_image_exists(&one, None));
        // No images at all.
        assert!(!other_image_exists(&files(&["a.txt", "b.rs"]), None));
    }

    #[test]
    fn next_image_is_none_when_no_other_image() {
        // No images at all.
        assert_eq!(
            next_image_index(&files(&["a.txt", "b.rs"]), Some(0), true),
            None
        );
        // The only image is already selected — nowhere to go either way.
        let one = files(&["a.txt", "logo.png"]);
        assert_eq!(next_image_index(&one, Some(1), true), None);
        assert_eq!(next_image_index(&one, Some(1), false), None);
        // …but from a non-image row that single image is reachable.
        assert_eq!(next_image_index(&one, Some(0), true), Some(1));
    }

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

#[cfg(test)]
mod commit_focus_tests {
    use super::*;
    use crate::backend::{Git2Backend, is_change_line};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throwaway repo with two committed files, each then given two unstaged
    /// edits far enough apart to land in separate hunks. Returns the scratch dir
    /// (delete when done) and an opened backend.
    fn two_dirty_files() -> (std::path::PathBuf, Git2Backend) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("journey-focus-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let repo = git2::Repository::init(&dir).unwrap();
        let sig =
            git2::Signature::new("T", "t@example.com", &git2::Time::new(1_700_000_000, 0)).unwrap();

        let base: String = (1..=20).map(|n| format!("l{n:02}\n")).collect();
        for name in ["a.txt", "b.txt"] {
            std::fs::write(dir.join(name), &base).unwrap();
        }
        {
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("a.txt")).unwrap();
            index.add_path(std::path::Path::new("b.txt")).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "base\n", &tree, &[])
                .unwrap();
        }
        let edited = base
            .replace("l02\n", "l02-edited\n")
            .replace("l18\n", "l18-edited\n");
        for name in ["a.txt", "b.txt"] {
            std::fs::write(dir.join(name), &edited).unwrap();
        }

        let backend = Git2Backend::open(dir.to_str().unwrap()).unwrap();
        (dir, backend)
    }

    /// Partially staging a file keeps that same file selected and shown in the
    /// diff, rather than snapping the selection back to the first file.
    #[test]
    fn partial_stage_keeps_the_same_file_focused() {
        let (dir, backend) = two_dirty_files();
        let mut client = GitClient::new(Rc::new(backend));
        client.enter_commit_mode();

        // Select the *second* unstaged file, so a jump-to-first would be visible.
        let b = client
            .working
            .unstaged
            .iter()
            .position(|f| f.path == "b.txt")
            .expect("b.txt is unstaged");
        assert_ne!(b, 0, "b.txt must not already be the first row");
        client.apply_commit_selection(Side::Unstaged, b);

        // Stage only b.txt's first change (its two `l02` rows), leaving the
        // line-18 change unstaged so the file stays in the unstaged list.
        let diff = client.backend.working_diff("b.txt", false, false);
        let rows: Vec<usize> = diff
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_change_line(l.kind) && l.text.contains("l02"))
            .map(|(i, _)| i)
            .collect();
        let (lo, hi) = (rows[0], *rows.last().unwrap());
        assert!(client.apply_partial(lo, hi));

        // The change moved to the index but b.txt is still dirty…
        assert!(client.working.staged.iter().any(|f| f.path == "b.txt"));
        let still = client
            .working
            .unstaged
            .iter()
            .position(|f| f.path == "b.txt")
            .expect("b.txt still has unstaged changes");
        // …and it is still the selected/shown file, not the first one.
        assert_eq!(
            client.unstaged_list.borrow().selected_index(),
            Some(still),
            "the partially-staged file stays focused"
        );
        assert_eq!(client.staged_list.borrow().selected_index(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A tiny solid-color PNG, just enough that `ImageComparison` decodes it.
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// Next Image (Ctrl+N) jumps the commit-screen selection from a text file to
    /// the next image and the diff pane switches to the graphical comparison;
    /// stepping again advances to the following image.
    fn nav_client() -> GitClient {
        let mut be = crate::backend::FixtureBackend::new("/tmp/journey-nav");
        be.add_working(
            "notes.md",
            ChangeStatus::Modified,
            false,
            &[(DiffLineKind::Addition, "+note")],
        );
        be.add_working_image(
            "a.png",
            ChangeStatus::Modified,
            false,
            Some(tiny_png()),
            Some(tiny_png()),
        );
        be.add_working_image(
            "b.png",
            ChangeStatus::Modified,
            false,
            Some(tiny_png()),
            Some(tiny_png()),
        );
        GitClient::new(Rc::new(be))
    }

    #[test]
    fn next_image_jumps_to_images_and_shows_the_comparison() {
        let mut client = nav_client();
        client.enter_commit_mode();
        // The first unstaged row (the text file) is auto-selected; no image yet.
        assert_eq!(client.unstaged_list.borrow().selected_index(), Some(0));
        assert!(!client.commit_diff_pane.borrow().showing_image());
        // The menu enable state reflects that: there *is* an image to jump to.
        assert!(client.has_other_image());

        // Ctrl+N → first image (a.png, row 1); the sync then shows it.
        assert!(client.navigate_image(true));
        assert_eq!(client.unstaged_list.borrow().selected_index(), Some(1));
        client.sync_commit();
        assert!(client.commit_diff_pane.borrow().showing_image());

        // Ctrl+N again → the next image (b.png, row 2).
        assert!(client.navigate_image(true));
        assert_eq!(client.unstaged_list.borrow().selected_index(), Some(2));
    }

    #[test]
    fn switch_mode_only_applies_while_an_image_is_shown() {
        let mut client = nav_client();
        client.enter_commit_mode();
        // On the text file no image is shown, so Switch Mode is a no-op.
        assert!(!client.cycle_image_mode());
        // Move onto an image; now it applies.
        client.navigate_image(true);
        client.sync_commit();
        assert!(client.cycle_image_mode());
        assert!(client.show_image_side(true));
    }

}
