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

use crate::backend::{CommitInfo, FileChange, RepoBackend};
use crate::widgets::Shared;

/// Height of the changed-files pane at the bottom of the window.
const FILE_PANE_H: i32 = 160;

pub struct GitClient {
    /// retrogui layout/event engine; owns the actual widget tree.
    root: Column,
    backend: Rc<dyn RepoBackend>,
    /// Shared handle to the commit list (also owned by `root` via `Shared`).
    commit_list: Rc<RefCell<List>>,
    /// Shared handle to the changed-files list.
    file_list: Rc<RefCell<List>>,
    /// Last commit index we loaded files for, so we only reload on change.
    shown_commit: Option<usize>,
}

impl GitClient {
    pub fn new(backend: Rc<dyn RepoBackend>) -> Self {
        let commit_items: Vec<ListItem> =
            backend.commits().iter().map(commit_row).collect();
        let mut commits = List::new(Rect::new(0, 0, 0, 0)).with_items(commit_items);
        if !backend.commits().is_empty() {
            commits.set_selected(Some(0));
        }
        let commit_list = Rc::new(RefCell::new(commits));
        let file_list = Rc::new(RefCell::new(List::new(Rect::new(0, 0, 0, 0))));

        let root = Column::new()
            .with_background(Color::LIGHT_GRAY)
            .add_fill(Shared::new(commit_list.clone()))
            .add_fixed(Shared::new(file_list.clone()), FILE_PANE_H);

        let mut client = Self {
            root,
            backend,
            commit_list,
            file_list,
            shown_commit: None,
        };
        client.refresh_files(true);
        client
    }

    /// Reload the changed-files pane for the currently-selected commit.
    /// Returns `true` if anything changed (so the caller can request a paint).
    fn refresh_files(&mut self, force: bool) -> bool {
        let selected = self.commit_list.borrow().selected_index();
        if !force && selected == self.shown_commit {
            return false;
        }
        self.shown_commit = selected;
        let items: Vec<ListItem> = match selected {
            Some(idx) => self
                .backend
                .changed_files(idx)
                .iter()
                .map(file_row)
                .collect(),
            None => Vec::new(),
        };
        let mut files = self.file_list.borrow_mut();
        files.set_items(items);
        true
    }
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
        if self.refresh_files(false) {
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
        self.root.focus_first()
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.root.popup_request()
    }

    fn wants_ticks(&self) -> bool {
        self.root.wants_ticks()
    }
}

/// Format a commit as a list row: ref decorations followed by the summary.
pub fn commit_row(commit: &CommitInfo) -> ListItem {
    let mut label = String::new();
    for r in &commit.refs {
        label.push('[');
        label.push_str(&r.name);
        label.push_str("] ");
    }
    label.push_str(&commit.summary);
    ListItem::new(label)
}

/// Format a changed file as a list row: status badge + path.
pub fn file_row(file: &FileChange) -> ListItem {
    ListItem::new(format!("{}  {}", file.status.badge(), file.display()))
}
