//! A one-line search field with a leading "Find:" label.
//!
//! Wraps a retrogui [`TextInput`] and draws a label to its left, so the
//! toolbar reads clearly without needing a horizontal layout container. The
//! app polls [`SearchBar::text`] after each event to drive live filtering.

use retrogui::{Color, Event, EventCtx, Painter, Rect, TextInput, Theme, Widget};

const LABEL_W: i32 = 44;
const PAD: i32 = 4;

pub struct SearchBar {
    bounds: Rect,
    input: TextInput,
    label: String,
}

impl SearchBar {
    pub fn new(rect: Rect) -> Self {
        let mut me = Self {
            bounds: rect,
            input: TextInput::new(Rect::new(0, 0, 0, 0)),
            label: "Find:".to_string(),
        };
        me.relayout();
        me
    }

    /// Current query text.
    pub fn text(&self) -> String {
        self.input.text()
    }

    fn relayout(&mut self) {
        let x = self.bounds.x + LABEL_W;
        let input_rect = Rect::new(
            x,
            self.bounds.y + PAD,
            (self.bounds.right() - x - PAD).max(0),
            (self.bounds.h - PAD * 2).max(0),
        );
        self.input.layout(input_rect);
    }
}

impl Widget for SearchBar {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.fill_rect(self.bounds, theme.face);
        let label_y = self.bounds.y + (self.bounds.h - theme.font_size as i32) / 2 - 1;
        painter.text(
            self.bounds.x + PAD,
            label_y,
            &self.label,
            theme.font_size,
            theme.text,
        );
        // A thin etched line under the bar separates it from the list below.
        painter.h_line(
            self.bounds.x,
            self.bounds.bottom() - 1,
            self.bounds.w,
            Color::MID_GRAY,
        );
        self.input.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.input.event(event, ctx);
    }

    fn captures_pointer(&self) -> bool {
        self.input.captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.input.focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }

    fn focus_first(&mut self) -> bool {
        self.input.focus_first()
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.relayout();
    }

    fn wants_ticks(&self) -> bool {
        self.input.wants_ticks()
    }
}
