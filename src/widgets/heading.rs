//! A tiny section heading for the commit screen.
//!
//! retrogui's `Label` is positioned absolutely at construction and ignores
//! `layout()`, so it can't reflow inside a resizable shell. `Heading` is a
//! minimal left-aligned, vertically-centered text widget that honors
//! `layout()` and whose text can be updated (e.g. to show a live file count).

use retrogui::{Painter, Rect, Theme, Widget};

pub struct Heading {
    rect: Rect,
    text: String,
    size: f32,
}

impl Heading {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            rect: Rect::new(0, 0, 0, 0),
            text: text.into(),
            size: 12.0,
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Widget for Heading {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let y = self.rect.y + (self.rect.h - self.size as i32).max(0) / 2;
        painter.text(self.rect.x, y, &self.text, self.size, theme.text);
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}
