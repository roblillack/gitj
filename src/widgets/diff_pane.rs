//! A diff pane that shows either a text diff or a graphical image diff.
//!
//! Most files diff as text in a [`DiffView`]; image files instead show an
//! [`ImageDiffView`] comparing the two versions visually. `DiffPane` owns both
//! and shows whichever the current selection calls for, so [`crate::ui`] can
//! treat the diff pane as a single widget (one Tab stop, one layout rect, one
//! handle) and just call [`set_diff`](Self::set_diff) for text or
//! [`show_image`](Self::show_image) for an image. Every [`Widget`] method
//! delegates to the active inner view.

use saudade::{Event, EventCtx, Painter, PopupRequest, Rect, Theme, Widget};

use crate::backend::Diff;
use crate::imagediff::ImageComparison;
use crate::widgets::{DiffMode, DiffView, ImageDiffView};

/// A diff pane backed by a text view and an image view, one shown at a time.
pub struct DiffPane {
    text: DiffView,
    image: ImageDiffView,
    showing_image: bool,
    focused: bool,
}

impl DiffPane {
    pub fn new(rect: Rect) -> Self {
        Self {
            text: DiffView::new(rect),
            image: ImageDiffView::new(rect),
            showing_image: false,
            focused: false,
        }
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.text = self.text.with_font_size(size);
        self.image = self.image.with_font_size(size);
        self
    }

    /// Show `diff` as a text diff, switching away from the image view.
    pub fn set_diff(&mut self, diff: Diff) {
        self.text.set_diff(diff);
        self.set_showing_image(false);
    }

    /// Show a graphical comparison of an image file's two versions.
    pub fn show_image(&mut self, comparison: ImageComparison) {
        self.image.set_comparison(Some(comparison));
        self.set_showing_image(true);
    }

    /// Whether the graphical image view is currently shown (vs. the text diff).
    pub fn showing_image(&self) -> bool {
        self.showing_image
    }

    /// Cycle the image comparison mode (View ▸ Switch Mode). No-op unless an
    /// image is currently shown.
    pub fn cycle_image_mode(&mut self) {
        if self.showing_image {
            self.image.cycle_mode();
        }
    }

    /// Show the "before" (old) or "after" (new) side of the image at full size
    /// (View ▸ Before / After Image). No-op unless an image is currently shown.
    pub fn show_image_side(&mut self, before: bool) {
        if self.showing_image {
            self.image.show_side(before);
        }
    }

    /// Set the text view's staging mode (no-op while an image is shown — the
    /// image view has no line-range selection).
    pub fn set_mode(&mut self, mode: DiffMode) {
        self.text.set_mode(mode);
    }

    /// Take a pending partial-stage request from the text view; always `None`
    /// while an image is shown.
    pub fn take_action(&mut self) -> Option<(usize, usize)> {
        if self.showing_image {
            None
        } else {
            self.text.take_action()
        }
    }

    pub fn is_empty(&self) -> bool {
        if self.showing_image {
            self.image.is_empty()
        } else {
            self.text.is_empty()
        }
    }

    /// Switch the active view, moving keyboard focus to the newly-shown one so
    /// it responds immediately without waiting for a re-focus.
    fn set_showing_image(&mut self, showing_image: bool) {
        if showing_image == self.showing_image {
            return;
        }
        self.showing_image = showing_image;
        if self.focused {
            self.text.set_focused(!showing_image);
            self.image.set_focused(showing_image);
        }
    }

    fn active(&self) -> &dyn Widget {
        if self.showing_image {
            &self.image
        } else {
            &self.text
        }
    }

    fn active_mut(&mut self) -> &mut dyn Widget {
        if self.showing_image {
            &mut self.image
        } else {
            &mut self.text
        }
    }
}

impl Widget for DiffPane {
    fn bounds(&self) -> Rect {
        self.active().bounds()
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.active_mut().paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.active_mut().event(event, ctx);
    }

    fn captures_pointer(&self) -> bool {
        self.active().captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.active().focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.active_mut().set_focused(focused);
    }

    fn focus_first(&mut self) -> bool {
        self.focused = true;
        self.active_mut().focus_first()
    }

    fn layout(&mut self, bounds: Rect) {
        // Lay out both, so the inactive view is correctly sized the instant it
        // is shown.
        self.text.layout(bounds);
        self.image.layout(bounds);
    }

    fn wants_ticks(&self) -> bool {
        self.active().wants_ticks()
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.active().popup_request()
    }
}
