//! Renders the README screenshots — the gitk-style **browse** (history) screen
//! and the `git gui`-style **commit** (staging) screen — straight from the real
//! [`GitClient`].
//!
//! It drives the deterministic [`FixtureBackend`] through saudade's offscreen
//! [`MockBackend`], exactly like the snapshot tests, then wraps each window in
//! Canoe-style chrome (a title bar, frame and drop shadow) with
//! [`MockBackend::render_framed`] so the images look the way a user sees the
//! app on the desktop. No live repository (or windowing system) is needed and
//! the output is byte-stable across machines. The shots are captured at 2× for
//! crisp hi-DPI rendering. Run it from the crate root to refresh the images
//! under `docs/`:
//!
//! ```sh
//! cargo run --example screenshots
//! ```

use std::path::Path;
use std::rc::Rc;

use journey::backend::{FixtureBackend, RepoBackend};
use journey::ui::GitClient;
use saudade::mock::MockBackend;
use saudade::{Event, Font, Modifiers, MouseButton, Point, Widget, WindowChrome};

// Both screens share one logical window size, so the two framed screenshots
// line up. The fixture repository is small, so this is enough to show every
// pane without acres of empty chrome.
const WINDOW_W: i32 = 600;
const WINDOW_H: i32 = 400;

// Capture the windows at 2× so the README images stay crisp on hi-DPI displays.
const SCALE: f32 = 2.0;

fn main() {
    // Mirror the live binary's window title (`Journey — <repo path>`); the app
    // is a resizable top-level window, so use the resizable Canoe frame.
    let title = format!("Journey — {}", FixtureBackend::sample().path());

    // Browse screen: the default history view with the newest commit selected,
    // its files listed and the whole-commit diff shown.
    let mut browse = sample_client();
    browse.focus_first();
    shoot("screenshot-browse.png", WINDOW_W, WINDOW_H, &title, Box::new(browse), &[]);

    // Commit screen: the staging view with a message typed into the editor.
    let mut commit = sample_client();
    commit.enter_commit_mode();
    // A full click (down + up) focuses the editor and clears the click's
    // selection anchor before typing — see the commit-mode snapshot tests. The
    // point lands inside the message editor at this window size.
    let mut events = vec![click(460, 290), release(460, 290)];
    events.extend(type_text("Add git-gui commit mode"));
    shoot("screenshot-commit.png", WINDOW_W, WINDOW_H, &title, Box::new(commit), &events);
}

fn sample_client() -> GitClient {
    GitClient::new(Rc::new(FixtureBackend::sample()))
}

/// Render `widget` at `w × h` (logical), dispatching `events` first (with the
/// same warm-up render the live app does before its first input), then wrap it
/// in Canoe window chrome titled `title` and write the framed PNG into
/// `docs/<name>`. Everything is captured at [`SCALE`].
fn shoot(name: &str, w: i32, h: i32, title: &str, mut widget: Box<dyn Widget>, events: &[Event]) {
    let backend = MockBackend::new(w, h)
        .with_scale(SCALE)
        .with_font(sans_font())
        .with_mono_font(mono_font());

    if !events.is_empty() {
        // Warm-up render so widgets that cache geometry during paint (menu-bar
        // label rects, the editor's bounds) are ready for hit-testing. Layout
        // is in logical pixels regardless of scale, so the click coordinates
        // below stay valid at 2×.
        let _ = backend.render(widget.as_mut());
        widget.focus_first();
        for event in events {
            backend.dispatch(widget.as_mut(), event);
        }
    }

    let chrome = WindowChrome::resizable(title);
    let png = backend.render_framed(widget.as_mut(), &chrome).to_png();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create docs/");
    std::fs::write(&path, png).expect("write screenshot");
    println!("wrote {}", path.display());
}

// The mock backend needs explicit fonts so glyph rasterization is identical
// regardless of the host's installed fonts; reuse the bundled DejaVu faces the
// snapshot tests ship.
fn sans_font() -> Font {
    Font::from_bytes(include_bytes!("../tests/fonts/DejaVuSans.ttf").to_vec())
        .expect("bundled DejaVuSans.ttf failed to load")
}

fn mono_font() -> Font {
    Font::from_bytes(include_bytes!("../tests/fonts/DejaVuSansMono.ttf").to_vec())
        .expect("bundled DejaVuSansMono.ttf failed to load")
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
