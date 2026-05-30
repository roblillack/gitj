//! Helpers shared by journey's snapshot tests.
//!
//! Mirrors retrogui's harness: every widget tree is rendered against the
//! bundled DejaVu fonts (so glyph rasterization is bit-identical regardless of
//! the host's installed fonts) at four scales, and each render is compared to
//! a checked-in PNG baseline via `insta::assert_binary_snapshot!`. Review
//! diffs with `cargo insta review`.

use retrogui::mock::MockBackend;
use retrogui::{Event, Font, Widget};

pub fn sans_font() -> Font {
    Font::from_bytes(include_bytes!("../fonts/DejaVuSans.ttf").to_vec())
        .expect("bundled DejaVuSans.ttf failed to load")
}

pub fn mono_font() -> Font {
    Font::from_bytes(include_bytes!("../fonts/DejaVuSansMono.ttf").to_vec())
        .expect("bundled DejaVuSansMono.ttf failed to load")
}

/// The fractional and integer scales every widget should look correct at.
pub const SCALES: &[f32] = &[1.0, 1.25, 1.5, 2.0];

/// Render `build()` at each scale in [`SCALES`] and emit one binary insta
/// snapshot per scale. `name` is the snapshot's base name; each scale appends
/// its own suffix (`<name>_1_00.snap.png`, …).
pub fn snapshot_at_all_scales<F>(name: &str, width: i32, height: i32, mut build: F)
where
    F: FnMut() -> Box<dyn Widget>,
{
    for &scale in SCALES {
        snapshot_one(name, width, height, scale, build(), &[]);
    }
}

/// Like [`snapshot_at_all_scales`] but feeds a sequence of synthetic events
/// into the freshly-built widget (after a layout at the target size) before
/// rendering. Lets tests capture interaction states — a selected row, a typed
/// query, a scrolled diff — deterministically.
pub fn snapshot_at_all_scales_with_events<F, E>(
    name: &str,
    width: i32,
    height: i32,
    mut build: F,
    events: E,
) where
    F: FnMut() -> Box<dyn Widget>,
    E: Fn() -> Vec<Event>,
{
    for &scale in SCALES {
        snapshot_one(name, width, height, scale, build(), &events());
    }
}

fn snapshot_one(
    name: &str,
    width: i32,
    height: i32,
    scale: f32,
    mut widget: Box<dyn Widget>,
    events: &[Event],
) {
    let backend = MockBackend::new(width, height)
        .with_scale(scale)
        .with_font(sans_font())
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
    let snap_name = format!("{}_{}.png", name, scale_tag(scale));
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        insta::assert_binary_snapshot!(snap_name.as_str(), snap.to_png());
    });
}

fn scale_tag(scale: f32) -> String {
    let scaled = (scale * 100.0).round() as i32;
    format!("{}_{:02}", scaled / 100, scaled % 100)
}
