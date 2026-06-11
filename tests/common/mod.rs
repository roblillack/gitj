//! Helpers shared by journey's integration tests.
//!
//! Mirrors saudade's harness: every widget tree is rendered against the
//! bundled DejaVu fonts (so glyph rasterization is bit-identical regardless of
//! the host's installed fonts) and compared to a checked-in PNG baseline via
//! `insta::assert_binary_snapshot!`. Review diffs with `cargo insta review`.

// Each integration-test binary is its own crate and pulls in only the helpers
// it needs (the snapshot tests use the renderers; the shortcut tests only the
// fonts), so unused-helper warnings here are expected.
#![allow(dead_code)]

use saudade::mock::MockBackend;
use saudade::{Event, Font, Widget};

pub fn sans_font() -> Font {
    Font::from_sans_bytes(include_bytes!("../fonts/DejaVuSans.ttf").to_vec())
        .expect("bundled DejaVuSans.ttf failed to load")
}

pub fn mono_font() -> Font {
    Font::from_sans_bytes(include_bytes!("../fonts/DejaVuSansMono.ttf").to_vec())
        .expect("bundled DejaVuSansMono.ttf failed to load")
}

/// The single scale every snapshot is captured at. We only keep 1.0x baselines:
/// fractional/integer scaling is exercised by saudade's own harness, so storing
/// per-resolution copies here just multiplies the checked-in PNGs for no gain.
const SCALE: f32 = 1.0;

/// Render `build()` and emit one binary insta snapshot named `<name>.png`.
pub fn snapshot<F>(name: &str, width: i32, height: i32, mut build: F)
where
    F: FnMut() -> Box<dyn Widget>,
{
    snapshot_one(name, width, height, build(), &[]);
}

/// Like [`snapshot`] but feeds a sequence of synthetic events into the freshly-
/// built widget (after a layout at the target size) before rendering. Lets
/// tests capture interaction states — a selected row, a typed query, a scrolled
/// diff — deterministically.
pub fn snapshot_with_events<F, E>(name: &str, width: i32, height: i32, mut build: F, events: E)
where
    F: FnMut() -> Box<dyn Widget>,
    E: Fn() -> Vec<Event>,
{
    snapshot_one(name, width, height, build(), &events());
}

fn snapshot_one(
    name: &str,
    width: i32,
    height: i32,
    mut widget: Box<dyn Widget>,
    events: &[Event],
) {
    let backend = MockBackend::new(width, height)
        .with_scale(SCALE)
        .with_sans_font(sans_font())
        .with_mono_font(mono_font());

    if !events.is_empty() {
        // Warm-up render: lays out at the logical size and paints once, so
        // widgets that cache geometry during paint (e.g. a MenuBar's label
        // rects) are ready for hit-testing — exactly the state the live app
        // is in before the user's first input. Then focus and dispatch.
        let _ = backend.render(widget.as_mut());
        widget.focus_first();
        for event in events {
            backend.dispatch(widget.as_mut(), event);
        }
    }

    let snap = backend.render(widget.as_mut());
    let snap_name = format!("{name}.png");
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        insta::assert_binary_snapshot!(snap_name.as_str(), snap.to_png());
    });
}
