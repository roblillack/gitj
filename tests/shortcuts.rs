//! Behavioral tests for the `git gui`-style keyboard accelerators on the commit
//! screen. Each test drives a real [`GitClient`] through synthetic events with
//! saudade's [`MockBackend`] dispatcher and asserts the resulting working-tree
//! / commit state — the same path the live runtime takes, minus the windowing.

mod common;

use std::rc::Rc;

use journey::backend::{FixtureBackend, RepoBackend};
use journey::ui::GitClient;
use saudade::mock::MockBackend;
use saudade::{Event, Key, Modifiers, MouseButton, NamedKey, Point, Widget};

// The commit screen uses the same window size as its snapshots.
const CW: i32 = 820;
const CH: i32 = 560;

fn ctrl(key: Key) -> Event {
    Event::KeyDown {
        key,
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
    }
}

fn ctrl_char(c: char) -> Event {
    ctrl(Key::Char(c))
}

fn click(x: i32, y: i32) -> Event {
    Event::PointerDown {
        pos: Point::new(x, y),
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
    }
}

fn release(x: i32, y: i32) -> Event {
    Event::PointerUp {
        pos: Point::new(x, y),
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
    }
}

fn type_text(w: &mut dyn Widget, backend: &MockBackend, s: &str) {
    for ch in s.chars() {
        backend.dispatch(
            w,
            &Event::Char {
                ch,
                modifiers: Modifiers::default(),
            },
        );
    }
}

/// A commit-mode client, laid out and focused exactly as the live app would be
/// after its first frame, plus a handle on its backing fixture for assertions.
fn commit_client() -> (Rc<FixtureBackend>, MockBackend, Box<dyn Widget>) {
    let be = Rc::new(FixtureBackend::sample());
    let mut client = GitClient::new(be.clone());
    client.enter_commit_mode();
    let backend = MockBackend::new(CW, CH)
        .with_scale(1.0)
        .with_sans_font(common::sans_font())
        .with_mono_font(common::mono_font());
    let mut widget: Box<dyn Widget> = Box::new(client);
    // Warm-up render lays the tree out (and primes geometry caches), matching
    // the state the live app is in before the user's first keystroke.
    let _ = backend.render(widget.as_mut());
    widget.focus_first();
    (be, backend, widget)
}

#[test]
fn ctrl_t_stages_the_selected_unstaged_file() {
    let (be, backend, mut w) = commit_client();
    // The first unstaged file (src/ui.rs) is auto-selected on entry.
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs")
    );

    backend.dispatch(w.as_mut(), &ctrl_char('t'));

    let ws = be.working_status(false);
    assert!(
        ws.staged.iter().any(|f| f.path == "src/ui.rs"),
        "Ctrl+T should stage the selected file, got {ws:?}"
    );
    assert!(!ws.unstaged.iter().any(|f| f.path == "src/ui.rs"));
}

#[test]
fn ctrl_i_stages_every_unstaged_file() {
    let (be, backend, mut w) = commit_client();
    assert_eq!(be.working_status(false).unstaged.len(), 2);

    backend.dispatch(w.as_mut(), &ctrl_char('i'));

    assert!(
        be.working_status(false).unstaged.is_empty(),
        "Ctrl+I should stage all changed files"
    );
}

#[test]
fn ctrl_u_unstages_the_selected_staged_file() {
    let (be, backend, mut w) = commit_client();
    // src/widgets/commit_panel.rs is staged in the sample fixture. Select it in
    // the lower-left staged list (the same row the staged-file snapshot clicks).
    assert!(
        be.working_status(false)
            .staged
            .iter()
            .any(|f| f.path == "src/widgets/commit_panel.rs")
    );
    backend.dispatch(w.as_mut(), &click(60, 304));
    backend.dispatch(w.as_mut(), &release(60, 304));

    backend.dispatch(w.as_mut(), &ctrl_char('u'));

    let ws = be.working_status(false);
    assert!(
        !ws.staged
            .iter()
            .any(|f| f.path == "src/widgets/commit_panel.rs"),
        "Ctrl+U should unstage the selected file, got {ws:?}"
    );
    assert!(
        ws.unstaged
            .iter()
            .any(|f| f.path == "src/widgets/commit_panel.rs"),
        "the file should reappear in the unstaged list"
    );
}

#[test]
fn ctrl_enter_commits_with_the_typed_message() {
    let (be, backend, mut w) = commit_client();
    // Focus the message editor, type a message, then commit with Ctrl+Enter.
    backend.dispatch(w.as_mut(), &click(420, 360));
    backend.dispatch(w.as_mut(), &release(420, 360));
    type_text(w.as_mut(), &backend, "Wire up commit shortcuts");

    backend.dispatch(w.as_mut(), &ctrl(Key::Named(NamedKey::Enter)));

    // Ctrl+Enter commits rather than inserting a newline, so the recorded
    // message is exactly what was typed (single line, no trailing newline).
    assert_eq!(
        be.last_commit(),
        Some(("Wire up commit shortcuts".to_string(), false))
    );
    assert!(be.working_status(false).staged.is_empty());
}

#[test]
fn ctrl_j_arms_revert_without_discarding() {
    let (be, backend, mut w) = commit_client();
    // src/ui.rs is the auto-selected unstaged file on entry.
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs")
    );

    // Ctrl+J only opens the confirm dialog — nothing is discarded until the
    // user explicitly confirms.
    backend.dispatch(w.as_mut(), &ctrl_char('j'));

    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs"),
        "Ctrl+J must not revert before the user confirms"
    );
}

#[test]
fn ctrl_j_then_confirm_reverts_the_selected_file() {
    let (be, backend, mut w) = commit_client();
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "src/ui.rs")
    );

    backend.dispatch(w.as_mut(), &ctrl_char('j'));
    // Paint once so the confirm dialog lays out its buttons for hit-testing
    // (button rects are computed during paint, as in the live runtime).
    let _ = backend.render(w.as_mut());
    // Click the affirmative "Revert Changes" button (the left of the two).
    backend.dispatch(w.as_mut(), &click(369, 320));
    backend.dispatch(w.as_mut(), &release(369, 320));

    let ws = be.working_status(false);
    assert!(
        !ws.unstaged.iter().any(|f| f.path == "src/ui.rs"),
        "confirming the dialog should revert src/ui.rs, got {ws:?}"
    );
}

#[test]
fn ctrl_j_on_untracked_then_confirm_deletes_the_file() {
    let (be, backend, mut w) = commit_client();
    // Move the selection off src/ui.rs onto the untracked notes.md.
    backend.dispatch(
        w.as_mut(),
        &Event::KeyDown {
            key: Key::Named(NamedKey::Down),
            modifiers: Modifiers::default(),
        },
    );
    assert!(
        be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "notes.md")
    );

    backend.dispatch(w.as_mut(), &ctrl_char('j'));
    let _ = backend.render(w.as_mut());
    // Click the affirmative "Delete File" button (the left of the two).
    backend.dispatch(w.as_mut(), &click(367, 320));
    backend.dispatch(w.as_mut(), &release(367, 320));

    assert!(
        !be.working_status(false)
            .unstaged
            .iter()
            .any(|f| f.path == "notes.md"),
        "confirming should delete the untracked notes.md"
    );
}

#[test]
fn ctrl_1_and_2_switch_between_browse_and_commit() {
    let be = Rc::new(FixtureBackend::sample());
    let client = GitClient::new(be.clone());
    let backend = MockBackend::new(CW, CH)
        .with_scale(1.0)
        .with_sans_font(common::sans_font())
        .with_mono_font(common::mono_font());
    let mut w: Box<dyn Widget> = Box::new(client);
    let _ = backend.render(w.as_mut());
    w.focus_first();

    // The app starts on the browse screen, whose menu bar carries no staging
    // accelerators — Ctrl+T is inert there.
    backend.dispatch(w.as_mut(), &ctrl_char('t'));
    assert!(
        !be.working_status(false)
            .staged
            .iter()
            .any(|f| f.path == "src/ui.rs"),
        "Ctrl+T must do nothing on the browse screen"
    );

    // Ctrl+2 switches to the commit screen; the staging accelerators are live
    // now, so Ctrl+T stages the auto-selected first unstaged file.
    backend.dispatch(w.as_mut(), &ctrl_char('2'));
    backend.dispatch(w.as_mut(), &ctrl_char('t'));
    assert!(
        be.working_status(false)
            .staged
            .iter()
            .any(|f| f.path == "src/ui.rs"),
        "Ctrl+2 should land on the commit screen, where Ctrl+T stages"
    );

    // Ctrl+1 returns to the browse screen, deactivating them again.
    backend.dispatch(w.as_mut(), &ctrl_char('1'));
    backend.dispatch(w.as_mut(), &ctrl_char('i'));
    assert_eq!(
        be.working_status(false).unstaged.len(),
        1,
        "back on the browse screen, Ctrl+I must not stage the remaining file"
    );
}

#[test]
fn ctrl_q_requests_window_close() {
    let (_be, backend, mut w) = commit_client();
    let outcome = backend.dispatch(w.as_mut(), &ctrl_char('q'));
    assert!(
        outcome.close_requested,
        "Ctrl+Q should ask the window to close"
    );
}
