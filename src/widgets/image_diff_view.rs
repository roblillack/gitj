//! A graphical image-diff pane.
//!
//! Shown in place of [`DiffView`](crate::widgets::DiffView) when the selected
//! file is a raster image (see [`crate::imagediff::is_image_path`]). It decodes
//! the two sides and offers the same comparison modes as the author's `imgap`
//! CLI — 2-up, swipe, onion skin, difference, and single left/right — picked
//! from a [`ButtonGroup`] along the bottom, with a slider driving the swipe /
//! onion-skin position. The image floats on the same sunken field the text
//! diff uses, over a transparency checkerboard.
//!
//! Like [`DiffView`] this is one widget to the cross-pane wiring in
//! [`crate::ui`]: it owns the mode buttons itself, forwarding pointer events to
//! them, and hand-draws its slider.

use saudade::{
    ButtonGroup, Color, Event, EventCtx, FontFamily, Merged, MouseButton, Painter, Point, Rect,
    Theme, Widget,
};

use crate::imagediff::{CompareMode, ImageComparison};

const PAD: i32 = 4;
/// Height of the metadata line along the top.
const META_H: i32 = 18;
/// Height of the mode-button row along the bottom.
const BTN_H: i32 = 22;
/// Height of the slider row above the buttons (present only for slider modes).
const SLIDER_H: i32 = 16;
/// Room around a mode button's label, on top of the widest label's width.
const BTN_LABEL_PAD: i32 = 14;

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
    /// One button per [`CompareMode::ALL`] entry, the current mode selected.
    modes: ButtonGroup,
    /// Width of one mode button, sized to the widest label at the last paint.
    mode_btn_w: i32,
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
            font_size: 11.0,
            modes: ButtonGroup::horizontal(Rect::new(0, 0, 0, 0))
                .with_buttons(CompareMode::ALL.map(CompareMode::label)),
            mode_btn_w: 0,
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

    /// The mode buttons, along the bottom-left: each as wide as the widest
    /// label needs, neighbours sharing their border, cut to the pane's width.
    fn modes_rect(&self) -> Rect {
        let inner = self.inner();
        let n = CompareMode::ALL.len() as i32;
        let w = (n * (self.mode_btn_w - 1) + 1).min(inner.w);
        Rect::new(inner.x, inner.bottom() - BTN_H, w, BTN_H)
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
        self.modes
            .set_selected(CompareMode::ALL.iter().position(|&m| m == mode));
    }

    /// Hand `event` to the mode buttons, and switch to the mode they now have
    /// selected if a click picked another one.
    fn forward_to_modes(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.modes.event(event, ctx);
        if let Some(mode) = self.modes.selected().map(|i| CompareMode::ALL[i])
            && mode != self.mode
        {
            self.select_mode(mode);
            ctx.request_paint();
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

    /// Handle a press at `pos` on the slider or the image; returns whether it
    /// was consumed.
    fn press(&mut self, pos: Point) -> bool {
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

    fn paint_meta(&self, painter: &mut Painter, theme: &Theme) {
        let Some(cmp) = &self.comparison else {
            return;
        };
        let meta = cmp.meta();
        if meta.is_empty() {
            return;
        }
        let r = self.meta_rect();
        let y = r.y + (r.h - self.font_size as i32) / 2 - 1;
        painter.text(r.x, y, meta, self.font_size, meta_color(theme));
    }

    /// Blit the composed comparison, centered in the image area.
    fn paint_image(&mut self, painter: &mut Painter, theme: &Theme) {
        let area = self.image_area();
        // The letterbox matches the field, so an aspect-fit image just floats
        // on it.
        painter.fill_rect(area, theme.background);
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
        painter.fill_rect(track, theme.background);
        painter.sunken_bevel(track, theme.highlight, theme.shadow);
        painter.stroke_rect(track, theme.border);
        // Filled portion up to the thumb.
        let fill_w = ((track.w - 2) as f32 * self.slider) as i32;
        if fill_w > 0 {
            painter.fill_rect(
                Rect::new(track.x + 1, track.y + 1, fill_w, track.h - 2),
                theme.highlight_bg,
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
        painter.text_centered(pr, &pct, self.font_size - 1.0, meta_color(theme));
    }

    /// Size the mode buttons to the widest label, set in the face a button
    /// draws it in, and place them.
    fn paint_modes(&mut self, painter: &mut Painter, theme: &Theme) {
        let label_w = CompareMode::ALL
            .iter()
            .map(|m| {
                painter
                    .measure_text_styled(
                        m.label(),
                        theme.font_size,
                        FontFamily::Sans,
                        theme.button_style,
                    )
                    .w
            })
            .max()
            .unwrap_or(0);
        self.mode_btn_w = label_w + BTN_LABEL_PAD;
        self.modes.layout(self.modes_rect());
        self.modes.paint(painter, theme);
    }
}

/// The muted text of the metadata line and the slider's readout: a quarter of
/// the way from the theme's text toward its background.
fn meta_color(theme: &Theme) -> Color {
    theme.text.mix(theme.background, 0.25)
}

impl Widget for ImageDiffView {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let field = self.field();
        painter.fill_rect(field, theme.background);
        painter.sunken_bevel(field, theme.highlight, theme.shadow);
        painter.stroke_rect(field, theme.border);

        // Clip to the frame's interior so content stops on exactly the border
        // line's device pixels at fractional scales.
        let saved = painter.push_clip_frame(field, 1, Merged::NONE);
        self.paint_meta(painter, theme);
        self.paint_image(painter, theme);
        self.paint_slider(painter, theme);
        self.paint_modes(painter, theme);
        painter.restore_clip(saved);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // A press on a mode button is the group's until it is released.
        if self.modes.captures_pointer() {
            self.forward_to_modes(event, ctx);
            return;
        }
        match event {
            Event::PointerDown { pos, .. } if self.modes.bounds().contains(*pos) => {
                ctx.request_focus();
                self.forward_to_modes(event, ctx);
            }
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
        self.dragging_slider || self.dragging_image || self.modes.captures_pointer()
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

    /// The center of `mode`'s button, as the group lays it out: the buttons
    /// split its width evenly.
    fn mode_button_center(v: &ImageDiffView, mode: CompareMode) -> Point {
        let r = v.modes.bounds();
        let n = CompareMode::ALL.len() as i32;
        let i = CompareMode::ALL.iter().position(|&m| m == mode).unwrap() as i32;
        let edge = |i: i32| r.x + i * (r.w - 1) / n;
        Point::new((edge(i) + edge(i + 1)) / 2, r.y + r.h / 2)
    }

    #[test]
    fn clicking_a_mode_button_selects_it() {
        let (be, mut v) = view();
        let center = mode_button_center(&v, CompareMode::Difference);
        be.dispatch(
            &mut v,
            &Event::PointerDown {
                pos: center,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert!(v.captures_pointer(), "the press belongs to the buttons");
        assert_eq!(v.mode(), CompareMode::TwoUp, "not yet, on the press");
        be.dispatch(
            &mut v,
            &Event::PointerUp {
                pos: center,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(v.mode(), CompareMode::Difference);
        assert!(!v.captures_pointer());
    }

    #[test]
    fn the_mode_buttons_follow_the_view_menu() {
        let (_be, mut v) = view();
        v.cycle_mode();
        assert_eq!(v.modes.selected(), Some(1), "Swipe");
        v.show_side(false);
        assert_eq!(v.modes.selected(), Some(5), "Right");
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
