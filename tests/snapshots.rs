//! Pixel-snapshot tests for journey's UI, rendered against the deterministic
//! [`FixtureBackend`] so they never depend on a real repository.

mod common;

use std::rc::Rc;

use common::{snapshot_at_all_scales, snapshot_at_all_scales_with_events};
use journey::backend::{Diff, DiffLine, DiffLineKind, FixtureBackend, RefKind, RefLabel};
use journey::ui::GitClient;
use journey::widgets::{CommitList, CommitRow, DiffView};
use retrogui::{
    Color, Container, Event, Key, Modifiers, MouseButton, NamedKey, Point, Rect, Widget,
};

const W: i32 = 760;
const H: i32 = 520;

fn sample_client() -> GitClient {
    GitClient::new(Rc::new(FixtureBackend::sample()))
}

fn key(k: NamedKey) -> Event {
    Event::KeyDown {
        key: Key::Named(k),
        modifiers: Modifiers::default(),
    }
}

fn click(x: i32, y: i32) -> Event {
    Event::PointerDown {
        pos: Point::new(x, y),
        button: MouseButton::Left,
    }
}

fn type_text(s: &str) -> Vec<Event> {
    s.chars()
        .map(|ch| Event::Char {
            ch,
            modifiers: Modifiers::default(),
        })
        .collect()
}

// ----------------------------------------------------------- whole-app screens

/// Default main screen: commit list focused on the newest commit, its single
/// changed file listed, and the whole-commit diff in the bottom pane.
#[test]
fn main_screen() {
    snapshot_at_all_scales("main_screen", W, H, || {
        let mut client = sample_client();
        client.focus_first();
        Box::new(client)
    });
}

/// Arrow-down once selects the second commit (two changed files); the diff
/// pane shows that commit's combined diff.
#[test]
fn main_screen_files_synced() {
    snapshot_at_all_scales_with_events(
        "main_screen_files_synced",
        W,
        H,
        || Box::new(sample_client()),
        || vec![key(NamedKey::Down)],
    );
}

/// Select the second commit, then click the second file row: the diff pane
/// narrows to just that file's diff.
#[test]
fn main_screen_file_diff() {
    snapshot_at_all_scales_with_events(
        "main_screen_file_diff",
        W,
        H,
        || Box::new(sample_client()),
        || {
            vec![
                key(NamedKey::Down), // commit list -> second commit (two files)
                click(30, 224),      // file list row 1 (src/main.rs)
            ]
        },
    );
}

/// Click into the search bar and type "rename": the commit list filters down
/// to the matching commit and the panes follow.
#[test]
fn main_screen_filtered() {
    snapshot_at_all_scales_with_events(
        "main_screen_filtered",
        W,
        H,
        || Box::new(sample_client()),
        || {
            let mut events = vec![click(200, 13)]; // focus the search field
            events.extend(type_text("rename"));
            events
        },
    );
}

// ----------------------------------------------------------- DiffView widget

/// A standalone diff pane showing one of every line kind, so the palette is
/// captured independently of the rest of the UI.
#[test]
fn diff_view_all_kinds() {
    snapshot_at_all_scales("diff_view_all_kinds", 460, 220, || {
        let mut view = DiffView::new(Rect::new(8, 8, 444, 204));
        view.set_diff(sample_diff());
        Box::new(
            Container::new(460, 220)
                .with_background(Color::LIGHT_GRAY)
                .add(SharedWidget(Box::new(view))),
        )
    });
}

/// A small DiffView showing more lines than fit, with the scroll moved down a
/// few rows via the keyboard.
#[test]
fn diff_view_scrolled() {
    snapshot_at_all_scales_with_events(
        "diff_view_scrolled",
        300,
        120,
        || {
            let mut view = DiffView::new(Rect::new(8, 8, 284, 104));
            view.set_diff(sample_diff());
            Box::new(
                Container::new(300, 120)
                    .with_background(Color::LIGHT_GRAY)
                    .add(SharedWidget(Box::new(view))),
            )
        },
        || {
            vec![
                click(20, 20), // focus the diff view
                key(NamedKey::Down),
                key(NamedKey::Down),
                key(NamedKey::Down),
            ]
        },
    );
}

// ----------------------------------------------------------- CommitList widget

fn badge_rows() -> Vec<CommitRow> {
    let r = |name: &str, kind| RefLabel {
        name: name.to_string(),
        kind,
    };
    vec![
        CommitRow {
            summary: "Implement commit graph rendering with colored lanes and refs".into(),
            refs: vec![r("main", RefKind::Head), r("v1.0", RefKind::Tag)],
            author: "Robert Lillack".into(),
            date: "2026-05-29 23:10".into(),
        },
        CommitRow {
            summary: "Fix scrollbar thumb minimum size".into(),
            refs: vec![r("feature/scroll", RefKind::LocalBranch)],
            author: "A. Hacker".into(),
            date: "2026-05-28 10:00".into(),
        },
        CommitRow {
            summary: "Merge remote-tracking branch".into(),
            refs: vec![
                r("origin/main", RefKind::RemoteBranch),
                r("HEAD", RefKind::DetachedHead),
            ],
            author: "Build Bot".into(),
            date: "2026-05-27 09:00".into(),
        },
        CommitRow {
            summary: "Plain commit with no refs".into(),
            refs: vec![],
            author: "Robert Lillack".into(),
            date: "2026-05-26 08:00".into(),
        },
    ]
}

/// Every ref-badge kind, a long summary that must clip before the author
/// column, and a focused selection on the first row.
#[test]
fn commit_list_focused() {
    snapshot_at_all_scales("commit_list_focused", 620, 90, || {
        let mut list = CommitList::new(Rect::new(8, 8, 604, 74)).with_rows(badge_rows());
        list.set_selected(Some(0));
        list.set_focused(true);
        Box::new(
            Container::new(620, 90)
                .with_background(Color::LIGHT_GRAY)
                .add(SharedWidget(Box::new(list))),
        )
    });
}

/// Same list with the selection present but focus elsewhere (muted gray band).
#[test]
fn commit_list_unfocused() {
    snapshot_at_all_scales("commit_list_unfocused", 620, 90, || {
        let mut list = CommitList::new(Rect::new(8, 8, 604, 74)).with_rows(badge_rows());
        list.set_selected(Some(0));
        Box::new(
            Container::new(620, 90)
                .with_background(Color::LIGHT_GRAY)
                .add(SharedWidget(Box::new(list))),
        )
    });
}

fn sample_diff() -> Diff {
    let lines = [
        (DiffLineKind::FileHeader, "diff --git a/src/main.rs b/src/main.rs"),
        (DiffLineKind::FileHeader, "index 1a2b3c4..5d6e7f8 100644"),
        (DiffLineKind::FileHeader, "--- a/src/main.rs"),
        (DiffLineKind::FileHeader, "+++ b/src/main.rs"),
        (DiffLineKind::HunkHeader, "@@ -1,7 +1,8 @@ fn main() {"),
        (DiffLineKind::Context, " use std::process;"),
        (DiffLineKind::Context, " "),
        (DiffLineKind::Deletion, "-    let n = 1;"),
        (DiffLineKind::Addition, "+    let n = 2;"),
        (DiffLineKind::Addition, "+    let m = n * 2;"),
        (DiffLineKind::Context, "     println!(\"{n}\");"),
        (DiffLineKind::Meta, "\\ No newline at end of file"),
    ];
    Diff {
        lines: lines
            .iter()
            .map(|(k, t)| DiffLine::new(*k, t.to_string()))
            .collect(),
    }
}

/// Minimal owning adapter so a bare widget can be dropped into a `Container`
/// for widget-level snapshots, forwarding the full `Widget` contract.
struct SharedWidget(Box<dyn Widget>);

impl Widget for SharedWidget {
    fn bounds(&self) -> Rect {
        self.0.bounds()
    }
    fn paint(&mut self, p: &mut retrogui::Painter, t: &retrogui::Theme) {
        self.0.paint(p, t);
    }
    fn paint_overlay(&mut self, p: &mut retrogui::Painter, t: &retrogui::Theme) {
        self.0.paint_overlay(p, t);
    }
    fn event(&mut self, e: &Event, c: &mut retrogui::EventCtx) {
        self.0.event(e, c);
    }
    fn captures_pointer(&self) -> bool {
        self.0.captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.0.focusable()
    }
    fn set_focused(&mut self, f: bool) {
        self.0.set_focused(f);
    }
    fn layout(&mut self, b: Rect) {
        self.0.layout(b);
    }
}
