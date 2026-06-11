//! A graphical image-diff pane.
//!
//! Shown in place of [`DiffView`](crate::widgets::DiffView) when the selected
//! file is a raster image (see [`crate::imagediff::is_image_path`]). It decodes
//! the two sides and offers the same comparison modes as the author's `imgap`
//! CLI — 2-up, swipe, onion skin, difference, and single left/right — selected
//! from a row of buttons along the bottom, with a slider driving the swipe /
//! onion-skin position. The image floats on the same sunken white field the
//! text diff uses, over a transparency checkerboard.
//!
//! Like [`DiffView`] this is a self-contained widget: it embeds no child
//! widgets, hand-drawing its buttons and slider and tracking their hit-rects,
//! so the cross-pane wiring in [`crate::ui`] stays simple.

use saudade::{Color, Event, EventCtx, MouseButton, Painter, Point, Rect, Theme, Widget};

use crate::imagediff::{CompareMode, ImageComparison};

const PAD: i32 = 4;
/// Height of the metadata line along the top.
const META_H: i32 = 18;
/// Height of the mode-button row along the bottom.
const BTN_H: i32 = 22;
/// Height of the slider row above the buttons (present only for slider modes).
const SLIDER_H: i32 = 16;
/// Gap between adjacent mode buttons.
const BTN_GAP: i32 = 3;

/// Backdrop behind the centered image (the letterbox area). Matches the field
/// so an aspect-fit image just floats on white.
const LETTERBOX: Color = Color::WHITE;
const META_FG: Color = Color::rgb(0x40, 0x40, 0x40);
/// Filled portion of the slider trough.
const SLIDER_FILL: Color = Color::rgb(0x00, 0x00, 0x80);

/// A graphical comparison of the two versions of an image file.
pub struct ImageDiffView {
    rect: Rect,
    comparison: Option<ImageComparison>,
    mode: CompareMode,
    /// The comparison mode to return to from a single-image view (the `s` key).
    last_compare_mode: CompareMode,
    /// Swipe / onion-skin position, 0..1.
    slider: f32,
    font_size: f32,
    /// Mode-button hit-rects from the last paint, for event hit-testing.
    button_rects: Vec<(CompareMode, Rect)>,
    dragging_slider: bool,
    /// A drag on the image body is steering the slider (swipe / onion modes).
    dragging_image: bool,
}

impl ImageDiffView {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            comparison: None,
            mode: CompareMode::TwoUp,
            last_compare_mode: CompareMode::TwoUp,
            slider: 0.5,
            font_size: 12.0,
            button_rects: Vec::new(),
            dragging_slider: false,
            dragging_image: false,
        }
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Show `comparison` (or clear the pane with `None`). The chosen mode and
    /// slider position persist across files, matching the comparison the user
    /// last picked.
    pub fn set_comparison(&mut self, comparison: Option<ImageComparison>) {
        self.comparison = comparison;
        self.dragging_slider = false;
        self.dragging_image = false;
    }

    pub fn is_empty(&self) -> bool {
        self.comparison.is_none()
    }

    /// The currently selected comparison mode (exposed for tests).
    #[cfg(test)]
    pub fn mode(&self) -> CompareMode {
        self.mode
    }

    /// The current slider position (exposed for tests).
    #[cfg(test)]
    pub fn slider(&self) -> f32 {
        self.slider
    }

    /// The sunken field, inset one pixel inside the border.
    fn field(&self) -> Rect {
        self.rect
    }

    fn inner(&self) -> Rect {
        self.field().inset(PAD)
    }

    /// Height the control strip occupies (buttons, plus the slider row for
    /// slider modes).
    fn controls_h(&self) -> i32 {
        if self.mode.uses_slider() {
            BTN_H + SLIDER_H + PAD
        } else {
            BTN_H
        }
    }

    /// The metadata line along the top.
    fn meta_rect(&self) -> Rect {
        let inner = self.inner();
        Rect::new(inner.x, inner.y, inner.w, META_H)
    }

    /// The area the composed image is centered in.
    fn image_area(&self) -> Rect {
        let inner = self.inner();
        let top = inner.y + META_H + PAD;
        let bottom = inner.bottom() - self.controls_h() - PAD;
        Rect::new(inner.x, top, inner.w, (bottom - top).max(0))
    }

    /// The slider trough rect (valid only in slider modes).
    fn slider_track(&self) -> Rect {
        let inner = self.inner();
        let y = inner.bottom() - BTN_H - PAD - SLIDER_H;
        // Leave room for a trailing percentage readout.
        let pct_w = 40;
        Rect::new(
            inner.x,
            y + (SLIDER_H - 6) / 2,
            (inner.w - pct_w).max(10),
            6,
        )
    }

    /// A generous hit band around the slider trough (covering the thumb, which
    /// overhangs it), so the slider is easy to grab. Shares the trough's `x` and
    /// width, so mapping an x coordinate over it matches the drawn thumb.
    fn slider_hit(&self) -> Rect {
        let t = self.slider_track();
        Rect::new(t.x, t.y - 6, t.w, t.h + 12)
    }

    /// The mode-button row along the bottom.
    fn button_row(&self) -> Rect {
        let inner = self.inner();
        Rect::new(inner.x, inner.bottom() - BTN_H, inner.w, BTN_H)
    }

    /// Map an x coordinate within `track` to a 0..1 slider value.
    fn value_at(track: Rect, x: i32) -> f32 {
        if track.w <= 1 {
            return 0.0;
        }
        ((x - track.x) as f32 / track.w as f32).clamp(0.0, 1.0)
    }

    /// Switch to a comparison mode, remembering it as the one to restore from a
    /// single-image view.
    fn select_mode(&mut self, mode: CompareMode) {
        self.mode = mode;
        if !mode.is_single() {
            self.last_compare_mode = mode;
        }
    }

    /// Cycle to the next comparison mode — the View ▸ Switch Mode action
    /// (Ctrl+M). The single-image views cycle back to 2-Up.
    pub fn cycle_mode(&mut self) {
        self.select_mode(self.mode.next());
    }

    /// Show just the "before" (old) or "after" (new) image at full size — the
    /// View ▸ Before / After Image actions (Ctrl+Left / Ctrl+Right).
    pub fn show_side(&mut self, before: bool) {
        self.select_mode(if before {
            CompareMode::Left
        } else {
            CompareMode::Right
        });
    }

    /// Handle a press at `pos`; returns whether it was consumed.
    fn press(&mut self, pos: Point) -> bool {
        if let Some((mode, _)) = self
            .button_rects
            .iter()
            .find(|(_, r)| r.contains(pos))
            .copied()
        {
            self.select_mode(mode);
            return true;
        }
        if self.mode.uses_slider() {
            let track = self.slider_hit();
            if track.contains(pos) {
                self.slider = Self::value_at(track, pos.x);
                self.dragging_slider = true;
                return true;
            }
            let area = self.image_area();
            if area.contains(pos) {
                self.slider = Self::value_at(area, pos.x);
                self.dragging_image = true;
                return true;
            }
        }
        false
    }

    fn paint_meta(&self, painter: &mut Painter) {
        let Some(cmp) = &self.comparison else {
            return;
        };
        let meta = cmp.meta();
        if meta.is_empty() {
            return;
        }
        let r = self.meta_rect();
        let y = r.y + (r.h - self.font_size as i32) / 2 - 1;
        painter.text(r.x, y, meta, self.font_size, META_FG);
    }

    /// Blit the composed comparison, centered in the image area.
    fn paint_image(&mut self, painter: &mut Painter) {
        let area = self.image_area();
        painter.fill_rect(area, LETTERBOX);
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let Some(cmp) = &mut self.comparison else {
            return;
        };
        let canvas = cmp.render(self.mode, self.slider, area.w as u32, area.h as u32);
        if canvas.w == 0 || canvas.h == 0 {
            return;
        }
        let ox = area.x + (area.w - canvas.w as i32) / 2;
        let oy = area.y + (area.h - canvas.h as i32) / 2;
        // One bulk blit rather than a `pixel()` call per pixel: the composed
        // canvas is opaque, so it goes straight into the framebuffer with the
        // logical→physical snap done once per row/column instead of per pixel.
        let saved = painter.push_clip(area);
        painter.blit_argb(ox, oy, canvas.w, canvas.h, &canvas.argb);
        painter.restore_clip(saved);
    }

    fn paint_slider(&self, painter: &mut Painter, theme: &Theme) {
        if !self.mode.uses_slider() {
            return;
        }
        let track = self.slider_track();
        painter.fill_rect(track, Color::WHITE);
        painter.sunken_bevel(track, theme.highlight, theme.shadow);
        painter.stroke_rect(track, theme.border);
        // Filled portion up to the thumb.
        let fill_w = ((track.w - 2) as f32 * self.slider) as i32;
        if fill_w > 0 {
            painter.fill_rect(
                Rect::new(track.x + 1, track.y + 1, fill_w, track.h - 2),
                SLIDER_FILL,
            );
        }
        // Thumb.
        let thumb_w = 8;
        let tx = track.x + ((track.w - thumb_w) as f32 * self.slider) as i32;
        let thumb = Rect::new(tx, track.y - 4, thumb_w, track.h + 8);
        painter.button(thumb, theme, false, false);
        // Percentage readout.
        let pct = format!("{:>3}%", (self.slider * 100.0).round() as i32);
        let pr = Rect::new(track.right() + PAD, track.y - 5, 36, 16);
        painter.text_centered(pr, &pct, self.font_size - 1.0, META_FG);
    }

    fn paint_buttons(&mut self, painter: &mut Painter, theme: &Theme) {
        self.button_rects.clear();
        let row = self.button_row();
        let mut x = row.x;
        for mode in CompareMode::ALL {
            let label = mode.label();
            let w = painter.measure_text(label, self.font_size).w + 14;
            let brect = Rect::new(x, row.y, w, BTN_H);
            let active = mode == self.mode;
            painter.button(brect, theme, active, false);
            let label_rect = if active {
                Rect::new(brect.x + 1, brect.y + 1, brect.w, brect.h)
            } else {
                brect
            };
            painter.text_centered(label_rect, label, self.font_size, theme.text);
            self.button_rects.push((mode, brect));
            x += w + BTN_GAP;
        }
    }
}

impl Widget for ImageDiffView {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let field = self.field();
        painter.fill_rect(field, Color::WHITE);
        painter.sunken_bevel(field, theme.highlight, theme.shadow);
        painter.stroke_rect(field, theme.border);

        let saved = painter.push_clip(field.inset(1));
        self.paint_meta(painter);
        self.paint_image(painter);
        self.paint_slider(painter, theme);
        self.paint_buttons(painter, theme);
        painter.restore_clip(saved);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                ctx.request_focus();
                if self.press(*pos) {
                    ctx.request_paint();
                }
            }
            Event::PointerMove { pos } if self.dragging_slider => {
                self.slider = Self::value_at(self.slider_hit(), pos.x);
                ctx.request_paint();
            }
            Event::PointerMove { pos } if self.dragging_image => {
                self.slider = Self::value_at(self.image_area(), pos.x);
                ctx.request_paint();
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.dragging_slider || self.dragging_image => {
                self.dragging_slider = false;
                self.dragging_image = false;
                ctx.request_paint();
            }
            // Keyboard control is driven from the View menu's accelerators
            // (Ctrl+M / Ctrl+Left / Ctrl+Right) via [`Self::cycle_mode`] /
            // [`Self::show_side`], handled application-side in [`crate::ui`], not
            // here — so the bare m/s/arrow keys are intentionally not bound.
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.dragging_slider || self.dragging_image
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, _focused: bool) {
        // No focus-dependent state: keyboard control comes from the View menu's
        // accelerators (see [`crate::ui`]), not from this widget owning focus.
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BlobPair;
    use crate::imagediff::ImageComparison;
    use saudade::mock::MockBackend;
    use saudade::{Modifiers, Point};

    const W: i32 = 480;
    const H: i32 = 320;

    fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    fn view() -> (MockBackend, ImageDiffView) {
        let be = MockBackend::new(W, H).with_scale(1.0);
        let mut v = ImageDiffView::new(Rect::new(0, 0, W, H));
        v.set_focused(true);
        let cmp = ImageComparison::from_blobs(&BlobPair {
            old: Some(png(16, 16, [255, 0, 0, 255])),
            new: Some(png(16, 16, [0, 0, 255, 255])),
        });
        v.set_comparison(cmp);
        v.layout(Rect::new(0, 0, W, H));
        let _ = be.render(&mut v);
        (be, v)
    }

    #[test]
    fn clicking_a_mode_button_selects_it() {
        let (be, mut v) = view();
        // Find the "Diff" (difference) button rect placed at paint.
        let (_, rect) = v
            .button_rects
            .iter()
            .find(|(m, _)| *m == CompareMode::Difference)
            .copied()
            .expect("difference button laid out");
        let center = Point::new(rect.x + rect.w / 2, rect.y + rect.h / 2);
        be.dispatch(
            &mut v,
            &Event::PointerDown {
                pos: center,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(v.mode(), CompareMode::Difference);
    }

    #[test]
    fn cycle_mode_and_show_side() {
        let (_be, mut v) = view();
        assert_eq!(v.mode(), CompareMode::TwoUp);
        v.cycle_mode();
        assert_eq!(v.mode(), CompareMode::Swipe);
        v.cycle_mode();
        assert_eq!(v.mode(), CompareMode::Onion);
        // Before / after jump straight to the single-image views…
        v.show_side(true);
        assert_eq!(v.mode(), CompareMode::Left);
        v.show_side(false);
        assert_eq!(v.mode(), CompareMode::Right);
        // …and cycling out of a single view returns to the comparison set.
        v.cycle_mode();
        assert_eq!(v.mode(), CompareMode::TwoUp);
    }

    #[test]
    fn dragging_the_slider_moves_it() {
        let (be, mut v) = view();
        v.cycle_mode(); // -> Swipe, which uses the slider
        assert_eq!(v.mode(), CompareMode::Swipe);

        let hit = v.slider_hit();
        let cy = hit.y + hit.h / 2;
        let press = |x: i32| Event::PointerDown {
            pos: Point::new(x, cy),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };

        // Press near the right end: the value jumps high and the drag captures.
        be.dispatch(&mut v, &press(hit.x + hit.w - 1));
        assert!(v.slider() > 0.9, "press near the right sets a high value");
        assert!(v.captures_pointer(), "the slider drag captures the pointer");

        // Drag back to the left: the value follows the pointer down.
        be.dispatch(
            &mut v,
            &Event::PointerMove {
                pos: Point::new(hit.x + 1, cy),
            },
        );
        assert!(v.slider() < 0.1, "dragging left lowers the value");

        // Releasing ends the drag.
        be.dispatch(
            &mut v,
            &Event::PointerUp {
                pos: Point::new(hit.x + 1, cy),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert!(!v.captures_pointer());
    }
}
