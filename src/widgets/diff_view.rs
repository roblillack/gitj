//! A read-only, scrollable, syntax-colored unified-diff viewer.
//!
//! `DiffView` renders a [`Diff`] line-by-line in the monospace font, tinting
//! additions green, deletions red, hunk headers blue and file headers gray —
//! the standard gitk / `git diff --color` palette adapted to retrogui's Win
//! 3.1 chrome. It owns a vertical scrollbar pinned to the right edge and, like
//! retrogui's `List`, only measures and paints the rows currently on screen.

use retrogui::{
    Color, Event, EventCtx, Key, MouseButton, NamedKey, Painter, Rect, ScrollBar,
    SCROLLBAR_THICKNESS, Theme, Widget,
};

use crate::backend::{Diff, DiffLineKind};

const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;

// Diff palette — readable on the sunken white field.
const ADD_BG: Color = Color::rgb(0xDC, 0xFF, 0xDC);
const ADD_FG: Color = Color::rgb(0x00, 0x64, 0x00);
const DEL_BG: Color = Color::rgb(0xFF, 0xDC, 0xDC);
const DEL_FG: Color = Color::rgb(0x90, 0x00, 0x00);
const HUNK_BG: Color = Color::rgb(0xE2, 0xE8, 0xFF);
const HUNK_FG: Color = Color::rgb(0x00, 0x00, 0x80);
const COMMIT_BG: Color = Color::rgb(0xFF, 0xF6, 0xCC);
const COMMIT_FG: Color = Color::rgb(0x40, 0x30, 0x00);
const FILE_BG: Color = Color::rgb(0xE6, 0xE6, 0xE6);
const FILE_FG: Color = Color::rgb(0x00, 0x00, 0x00);
const META_FG: Color = Color::rgb(0x80, 0x80, 0x80);
const CONTEXT_FG: Color = Color::rgb(0x20, 0x20, 0x20);

/// A read-only diff pane.
pub struct DiffView {
    rect: Rect,
    diff: Diff,
    v_scrollbar: ScrollBar,
    focused: bool,
    font_size: f32,
}

impl DiffView {
    pub fn new(rect: Rect) -> Self {
        let mut me = Self {
            rect,
            diff: Diff::default(),
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            focused: false,
            font_size: 12.0,
        };
        me.relayout_scrollbar();
        me
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Replace the displayed diff and reset the scroll position to the top.
    pub fn set_diff(&mut self, diff: Diff) {
        self.diff = diff;
        self.v_scrollbar.set_value(0);
        self.sync_scrollbar();
    }

    pub fn is_empty(&self) -> bool {
        self.diff.is_empty()
    }

    fn line_height(&self) -> i32 {
        (self.font_size as i32 + 4).max(8)
    }

    fn text_area(&self) -> Rect {
        let sb_w = if self.v_scrollbar.rect().w > 0 {
            SCROLLBAR_THICKNESS
        } else {
            0
        };
        Rect::new(
            self.rect.x,
            self.rect.y,
            (self.rect.w - sb_w).max(0),
            self.rect.h,
        )
    }

    fn visible_rows(&self) -> i32 {
        ((self.text_area().h - TEXT_PAD_Y * 2) / self.line_height()).max(1)
    }

    fn scroll_top(&self) -> usize {
        self.v_scrollbar.value().max(0) as usize
    }

    fn sync_scrollbar(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = (self.diff.lines.len() as i32 - visible).max(0);
        self.v_scrollbar.set_range(visible, max_scroll);
        self.v_scrollbar.set_line_step(1);
    }

    fn relayout_scrollbar(&mut self) {
        let sb_rect = Rect::new(
            self.rect.right() - SCROLLBAR_THICKNESS,
            self.rect.y,
            SCROLLBAR_THICKNESS,
            self.rect.h,
        );
        self.v_scrollbar.set_rect(sb_rect);
        self.sync_scrollbar();
    }

    fn scroll_by(&mut self, delta: i32) {
        let v = self.v_scrollbar.value();
        self.v_scrollbar.set_value(v + delta);
    }
}

impl Widget for DiffView {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync_scrollbar();
        let text = self.text_area();
        painter.fill_rect(text, Color::WHITE);
        painter.sunken_bevel(text, theme.highlight, theme.shadow);
        painter.stroke_rect(text, theme.border);

        let line_h = self.line_height();
        let text_x = text.x + TEXT_PAD_X;
        let text_y0 = text.y + TEXT_PAD_Y;
        let row_w = (text.w - TEXT_PAD_X).max(0);
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();

        // Clip so long lines don't bleed across the scrollbar or the border.
        let saved = painter.push_clip(text.inset(1));
        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            let Some(line) = self.diff.lines.get(row) else {
                break;
            };
            let y = text_y0 + row_offset as i32 * line_h;
            let (fg, bg) = colors_for(line.kind);
            if let Some(bg) = bg {
                painter.fill_rect(Rect::new(text.x + 1, y, row_w, line_h), bg);
            }
            let label_y = y + (line_h - self.font_size as i32) / 2 - 1;
            painter.mono_text(text_x, label_y, &line.text, self.font_size, fg);
        }
        painter.restore_clip(saved);

        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Route to the scrollbar while it's dragging or being clicked.
        if self.v_scrollbar.captures_pointer() {
            self.v_scrollbar.event(event, ctx);
            return;
        }
        if let Some(pos) = event.position()
            && self.v_scrollbar.rect().contains(pos)
        {
            self.v_scrollbar.event(event, ctx);
            return;
        }

        match event {
            Event::PointerDown {
                button: MouseButton::Left,
                ..
            } => {
                ctx.request_focus();
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                let page = (self.visible_rows() - 1).max(1);
                let consumed = match key {
                    Key::Named(NamedKey::Up) => {
                        self.scroll_by(-1);
                        true
                    }
                    Key::Named(NamedKey::Down) => {
                        self.scroll_by(1);
                        true
                    }
                    Key::Named(NamedKey::PageUp) => {
                        self.scroll_by(-page);
                        true
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.scroll_by(page);
                        true
                    }
                    Key::Named(NamedKey::Home) => {
                        self.v_scrollbar.set_value(0);
                        true
                    }
                    Key::Named(NamedKey::End) => {
                        self.v_scrollbar.set_value(self.diff.lines.len() as i32);
                        true
                    }
                    _ => false,
                };
                if consumed {
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.v_scrollbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.relayout_scrollbar();
    }
}

/// Foreground color and optional row background tint for a diff line kind.
fn colors_for(kind: DiffLineKind) -> (Color, Option<Color>) {
    match kind {
        DiffLineKind::CommitHeader => (COMMIT_FG, Some(COMMIT_BG)),
        DiffLineKind::Addition => (ADD_FG, Some(ADD_BG)),
        DiffLineKind::Deletion => (DEL_FG, Some(DEL_BG)),
        DiffLineKind::HunkHeader => (HUNK_FG, Some(HUNK_BG)),
        DiffLineKind::FileHeader => (FILE_FG, Some(FILE_BG)),
        DiffLineKind::Meta => (META_FG, None),
        DiffLineKind::Context => (CONTEXT_FG, None),
    }
}
