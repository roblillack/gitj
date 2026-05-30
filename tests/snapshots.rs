//! Pixel-snapshot tests for journey's UI, rendered against the deterministic
//! [`FixtureBackend`] so they never depend on a real repository.

mod common;

use std::rc::Rc;

use common::{snapshot, snapshot_with_events};
use journey::backend::{Diff, DiffLine, DiffLineKind, FixtureBackend, RefKind, RefLabel};
use journey::ui::GitClient;
use journey::widgets::{compute_graph, CommitList, CommitRow, DiffView};
use retrogui::{
    Color, Container, Event, Key, Modifiers, MouseButton, NamedKey, Point, Rect, Widget,
};

const W: i32 = 760;
const H: i32 = 520;

// The commit screen carries more chrome (two lists, a diff, an editor and a
// button row), so its snapshots use a slightly larger window.
const CW: i32 = 820;
const CH: i32 = 560;

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

fn release(x: i32, y: i32) -> Event {
    Event::PointerUp {
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
    snapshot("main_screen", W, H, || {
        let mut client = sample_client();
        client.focus_first();
        Box::new(client)
    });
}

/// Arrow-down once selects the second commit (two changed files); the diff
/// pane shows that commit's combined diff.
#[test]
fn main_screen_files_synced() {
    snapshot_with_events(
        "main_screen_files_synced",
        W,
        H,
        || Box::new(sample_client()),
        || vec![key(NamedKey::Down)],
    );
}

/// Select the second commit, then click the second file row (the files pane
/// is bottom-right): the diff pane narrows to just that file's diff.
#[test]
fn main_screen_file_diff() {
    snapshot_with_events(
        "main_screen_file_diff",
        W,
        H,
        || Box::new(sample_client()),
        || {
            vec![
                key(NamedKey::Down), // commit list -> second commit (two files)
                click(500, 293),     // files pane row 1 (src/main.rs)
            ]
        },
    );
}

/// Click the "File" label in the menu bar to drop its menu (the first item is
/// pre-highlighted), exercising menu-bar + popup compositing in the shell.
#[test]
fn main_screen_menu_open() {
    snapshot_with_events(
        "main_screen_menu_open",
        W,
        H,
        || Box::new(sample_client()),
        || vec![click(16, 10)],
    );
}

/// Click into the search bar and type "rename": the commit list filters down
/// to the matching commit and the panes follow.
#[test]
fn main_screen_filtered() {
    snapshot_with_events(
        "main_screen_filtered",
        W,
        H,
        || Box::new(sample_client()),
        || {
            let mut events = vec![click(200, 33)]; // focus the search field (below the menu)
            events.extend(type_text("rename"));
            events
        },
    );
}

/// Open the Help menu with Alt+H, highlight and fire About with Down+Enter:
/// the modal About dialog floats over the shell. Exercises menu accelerators,
/// the command-free dialog callback, and the overlay compositing path.
#[test]
fn main_screen_about_dialog() {
    snapshot_with_events(
        "main_screen_about_dialog",
        W,
        H,
        || Box::new(sample_client()),
        || {
            vec![
                Event::KeyDown {
                    key: Key::Char('h'),
                    modifiers: Modifiers {
                        alt: true,
                        ..Modifiers::default()
                    },
                },
                key(NamedKey::Down),
                key(NamedKey::Enter),
            ]
        },
    );
}

// ----------------------------------------------------------- commit screen

/// The git-gui-style commit screen: unstaged and staged file lists on the
/// left, a diff of the (auto-selected first) unstaged file on the right, an
/// empty message editor and the staging button row.
#[test]
fn commit_mode() {
    snapshot("commit_mode", CW, CH, || {
        let mut client = sample_client();
        client.enter_commit_mode();
        client.focus_first();
        Box::new(client)
    });
}

/// Click a file in the *staged* list: the diff pane switches to that file's
/// staged (`index` vs `HEAD`) diff and the unstaged selection clears.
#[test]
fn commit_mode_staged_file() {
    snapshot_with_events(
        "commit_mode_staged_file",
        CW,
        CH,
        || {
            let mut client = sample_client();
            client.enter_commit_mode();
            Box::new(client)
        },
        || vec![click(60, 304)], // first row of the lower-left (staged) list
    );
}

/// Click into the message editor and type a commit message.
#[test]
fn commit_mode_message() {
    snapshot_with_events(
        "commit_mode_message",
        CW,
        CH,
        || {
            let mut client = sample_client();
            client.enter_commit_mode();
            Box::new(client)
        },
        || {
            // A full click (down + up) focuses the editor and clears the
            // click's selection anchor before typing.
            let mut events = vec![click(420, 360), release(420, 360)];
            events.extend(type_text("Add git-gui commit mode"));
            events
        },
    );
}

/// Tick the "Amend last commit" checkbox: the editor pre-fills with the
/// current HEAD commit's message.
#[test]
fn commit_mode_amend() {
    snapshot_with_events(
        "commit_mode_amend",
        CW,
        CH,
        || {
            let mut client = sample_client();
            client.enter_commit_mode();
            Box::new(client)
        },
        // The checkbox toggles on release, so send a full click.
        || vec![click(340, 542), release(340, 542)],
    );
}

/// Amend, then pull an already-committed file back *out* of the commit:
/// after ticking Amend the HEAD file (`src/widgets/graph.rs`) shows up
/// staged; selecting it and clicking Unstage moves it to the unstaged list,
/// dropping it from the amended commit.
#[test]
fn commit_mode_amend_unstage() {
    snapshot_with_events(
        "commit_mode_amend_unstage",
        CW,
        CH,
        || {
            let mut client = sample_client();
            client.enter_commit_mode();
            Box::new(client)
        },
        || {
            vec![
                click(340, 542),  // tick "Amend last commit"
                release(340, 542),
                click(50, 340),   // select the HEAD file (3rd staged row)
                click(150, 545),  // press the "Unstage" button...
                release(150, 545),
            ]
        },
    );
}

// --------------------------------------------------- log ↔ commit navigation

/// Double-clicking the "Uncommitted changes" pseudo-row at the top of the log
/// jumps straight to the staging view.
#[test]
fn log_double_click_opens_commit() {
    snapshot_with_events(
        "log_double_click_opens_commit",
        W,
        H,
        || Box::new(sample_client()),
        // Two clicks on the first log row (the working-tree pseudo-row) within
        // the double-click window open the commit screen.
        || vec![click(200, 55), click(200, 55)],
    );
}

/// Committing from the staging view drops back to the log automatically: the
/// staged entries are gone and the log is in front again.
#[test]
fn commit_returns_to_log() {
    snapshot_with_events(
        "commit_returns_to_log",
        CW,
        CH,
        || {
            let mut client = sample_client();
            client.enter_commit_mode();
            Box::new(client)
        },
        || {
            let mut events = vec![click(420, 360), release(420, 360)]; // focus editor
            events.extend(type_text("Ship the commit view"));
            events.push(click(755, 543)); // the Commit button
            events.push(release(755, 543));
            events
        },
    );
}

// ----------------------------------------------------------- DiffView widget

/// A standalone diff pane showing one of every line kind, so the palette is
/// captured independently of the rest of the UI.
#[test]
fn diff_view_all_kinds() {
    snapshot("diff_view_all_kinds", 460, 220, || {
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
    snapshot_with_events(
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
            ..Default::default()
        },
        CommitRow {
            summary: "Fix scrollbar thumb minimum size".into(),
            refs: vec![r("feature/scroll", RefKind::LocalBranch)],
            author: "A. Hacker".into(),
            date: "2026-05-28 10:00".into(),
            ..Default::default()
        },
        CommitRow {
            summary: "Merge remote-tracking branch".into(),
            refs: vec![
                r("origin/main", RefKind::RemoteBranch),
                r("HEAD", RefKind::DetachedHead),
            ],
            author: "Build Bot".into(),
            date: "2026-05-27 09:00".into(),
            ..Default::default()
        },
        CommitRow {
            summary: "Plain commit with no refs".into(),
            refs: vec![],
            author: "Robert Lillack".into(),
            date: "2026-05-26 08:00".into(),
            ..Default::default()
        },
    ]
}

/// Every ref-badge kind, a long summary that must clip before the author
/// column, and a focused selection on the first row.
#[test]
fn commit_list_focused() {
    snapshot("commit_list_focused", 620, 90, || {
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
    snapshot("commit_list_unfocused", 620, 90, || {
        let mut list = CommitList::new(Rect::new(8, 8, 604, 74)).with_rows(badge_rows());
        list.set_selected(Some(0));
        Box::new(
            Container::new(620, 90)
                .with_background(Color::LIGHT_GRAY)
                .add(SharedWidget(Box::new(list))),
        )
    });
}

/// A branchy DAG (fork + merge) with the graph gutter drawn: two lanes, a
/// merge dot fanning out and a feature lane merging back.
#[test]
fn commit_list_graph() {
    snapshot("commit_list_graph", 560, 130, || {
        let row = |id: &str, parents: &[&str], summary: &str, refs: Vec<RefLabel>| CommitRow {
            id: id.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            summary: summary.into(),
            refs,
            author: "Robert Lillack".into(),
            date: "2026-05-29 23:10".into(),
        };
        let head = |name: &str, kind| RefLabel {
            name: name.to_string(),
            kind,
        };
        let rows = vec![
            row("m", &["e", "d"], "Merge feature into main", vec![head("main", RefKind::Head)]),
            row("e", &["c"], "Main-line work", vec![]),
            row("d", &["c"], "Feature tweak", vec![head("feature", RefKind::LocalBranch)]),
            row("c", &["b"], "Shared base", vec![]),
            row("b", &["a"], "Earlier change", vec![]),
            row("a", &[], "Initial commit", vec![]),
        ];
        let dag: Vec<(String, Vec<String>)> =
            rows.iter().map(|r| (r.id.clone(), r.parents.clone())).collect();

        let mut list = CommitList::new(Rect::new(8, 8, 544, 114)).with_rows(rows);
        list.set_graph(Some(compute_graph(&dag)));
        list.set_selected(Some(0));
        list.set_focused(true);
        Box::new(
            Container::new(560, 130)
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
