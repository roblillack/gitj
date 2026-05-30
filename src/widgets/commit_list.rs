//! The commit history list — a gitk-style scrollable list whose rows carry
//! colored ref badges (branches, tags, HEAD) on the left and author / date
//! columns on the right.
//!
//! It's a specialization of retrogui's `List`: the selection, scrolling and
//! keyboard behavior are the same, but each row is custom-painted so the refs
//! render as little colored boxes and a graph gutter can be slotted in front
//! later (Phase 6). Kept in journey rather than retrogui because the row
//! content is git-specific.

use std::time::{Duration, Instant};

use retrogui::{
    Color, Event, EventCtx, Key, MouseButton, NamedKey, Painter, Point, Rect, ScrollBar,
    SCROLLBAR_THICKNESS, Theme, Widget,
};

use crate::backend::{RefKind, RefLabel};

const ROW_HEIGHT: i32 = 18;
const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;
const COL_GAP: i32 = 12;
const AUTHOR_COL_W: i32 = 120;
const BADGE_GAP: i32 = 3;
const DOUBLE_CLICK_MS: u64 = 400;

/// Reserved width for the graph gutter at the left of each row. Zero until the
/// DAG graph lands; kept as a named constant so the column math is ready.
const GRAPH_W: i32 = 0;

/// One commit's worth of row content.
#[derive(Clone)]
pub struct CommitRow {
    pub summary: String,
    pub refs: Vec<RefLabel>,
    pub author: String,
    pub date: String,
}

pub struct CommitList {
    rect: Rect,
    rows: Vec<CommitRow>,
    selected: Option<usize>,
    focused: bool,
    v_scrollbar: ScrollBar,
    activated: Option<usize>,
    last_click: Option<(usize, Instant)>,
    font_size: f32,
}

impl CommitList {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            rows: Vec::new(),
            selected: None,
            focused: false,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            activated: None,
            last_click: None,
            font_size: 12.0,
        }
    }

    pub fn with_rows(mut self, rows: Vec<CommitRow>) -> Self {
        self.set_rows(rows);
        self
    }

    pub fn set_rows(&mut self, rows: Vec<CommitRow>) {
        self.rows = rows;
        if let Some(idx) = self.selected
            && idx >= self.rows.len()
        {
            self.selected = None;
        }
        self.activated = None;
        self.last_click = None;
        self.v_scrollbar.set_value(0);
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_selected(&mut self, idx: Option<usize>) {
        self.selected = idx.filter(|&i| i < self.rows.len());
        self.ensure_selection_visible();
    }

    pub fn take_activated(&mut self) -> Option<usize> {
        self.activated.take()
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
        ((self.text_area().h - TEXT_PAD_Y * 2) / ROW_HEIGHT).max(1)
    }

    fn scroll_top(&self) -> usize {
        self.v_scrollbar.value().max(0) as usize
    }

    fn set_scroll_top(&mut self, top: usize) {
        self.v_scrollbar.set_value(top as i32);
    }

    fn sync_scrollbar(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = (self.rows.len() as i32 - visible).max(0);
        self.v_scrollbar.set_range(visible, max_scroll);
    }

    fn ensure_selection_visible(&mut self) {
        self.sync_scrollbar();
        let Some(idx) = self.selected else { return };
        let visible = self.visible_rows() as usize;
        let mut top = self.scroll_top();
        if idx < top {
            top = idx;
        } else if idx >= top + visible {
            top = idx + 1 - visible;
        }
        self.set_scroll_top(top);
    }

    fn row_at(&self, pos: Point) -> Option<usize> {
        let text = self.text_area();
        if !text.contains(pos) {
            return None;
        }
        let local_y = pos.y - text.y - TEXT_PAD_Y;
        if local_y < 0 {
            return None;
        }
        let row = self.scroll_top() + (local_y / ROW_HEIGHT) as usize;
        if row < self.rows.len() {
            Some(row)
        } else {
            None
        }
    }

    fn select_and_show(&mut self, idx: usize) {
        self.selected = Some(idx);
        self.ensure_selection_visible();
    }

    fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.selected.unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, self.rows.len() as i32 - 1);
        self.select_and_show(next as usize);
    }

    fn move_page(&mut self, pages: i32) {
        let step = (self.visible_rows() - 1).max(1);
        self.move_selection(pages * step);
    }

    fn handle_click(&mut self, idx: usize) {
        let now = Instant::now();
        let threshold = Duration::from_millis(DOUBLE_CLICK_MS);
        let double = self
            .last_click
            .map(|(prev_idx, prev_time)| {
                prev_idx == idx && now.duration_since(prev_time) <= threshold
            })
            .unwrap_or(false);
        self.select_and_show(idx);
        if double {
            self.activated = Some(idx);
            self.last_click = None;
        } else {
            self.last_click = Some((idx, now));
        }
    }
}

impl Widget for CommitList {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync_scrollbar();
        let text = self.text_area();
        painter.fill_rect(text, Color::WHITE);
        painter.sunken_bevel(text, theme.highlight, theme.shadow);
        painter.stroke_rect(text, theme.border);

        let text_x = text.x + TEXT_PAD_X;
        let text_y0 = text.y + TEXT_PAD_Y;
        let row_w = (text.w - TEXT_PAD_X * 2).max(0);
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();
        let row_right = text.right() - TEXT_PAD_X;

        for row_offset in 0..visible {
            let row = scroll_top + row_offset;
            let Some(data) = self.rows.get(row) else {
                break;
            };
            let y = text_y0 + row_offset as i32 * ROW_HEIGHT;
            let selected = self.selected == Some(row);
            let active = selected && self.focused;
            if selected {
                let bg = if self.focused {
                    theme.highlight_bg
                } else {
                    theme.face
                };
                painter.fill_rect(Rect::new(text_x, y, row_w, ROW_HEIGHT), bg);
            }
            let fg = if active { theme.highlight_text } else { theme.text };

            // Right-aligned date, then author column to its left.
            let date_size = painter.measure_text(&data.date, self.font_size);
            let date_x = row_right - date_size.w;
            let author_x = date_x - COL_GAP - AUTHOR_COL_W;
            let label_y = y + (ROW_HEIGHT - self.font_size as i32) / 2 - 1;

            painter.text(date_x, label_y, &data.date, self.font_size, fg);

            let author_clip = Rect::new(author_x, y, AUTHOR_COL_W, ROW_HEIGHT);
            let saved = painter.push_clip(author_clip);
            painter.text(author_x, label_y, &data.author, self.font_size, fg);
            painter.restore_clip(saved);

            // Left side: graph gutter (reserved), ref badges, then summary.
            let mut x = text_x + GRAPH_W + 2;
            for r in &data.refs {
                x += draw_badge(painter, x, y, &r.name, r.kind, self.font_size) + BADGE_GAP;
            }
            let summary_right = author_x - COL_GAP;
            if summary_right > x {
                let saved = painter.push_clip(Rect::new(x, y, summary_right - x, ROW_HEIGHT));
                painter.text(x, label_y, &data.summary, self.font_size, fg);
                painter.restore_clip(saved);
            }
        }

        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
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
                pos,
                button: MouseButton::Left,
            } => {
                ctx.request_focus();
                if let Some(row) = self.row_at(*pos) {
                    self.handle_click(row);
                }
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                let consumed = match key {
                    Key::Named(NamedKey::Up) => {
                        self.move_selection(-1);
                        true
                    }
                    Key::Named(NamedKey::Down) => {
                        self.move_selection(1);
                        true
                    }
                    Key::Named(NamedKey::Home) => {
                        if !self.rows.is_empty() {
                            self.select_and_show(0);
                        }
                        true
                    }
                    Key::Named(NamedKey::End) => {
                        if let Some(last) = self.rows.len().checked_sub(1) {
                            self.select_and_show(last);
                        }
                        true
                    }
                    Key::Named(NamedKey::PageUp) => {
                        self.move_page(-1);
                        true
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.move_page(1);
                        true
                    }
                    Key::Named(NamedKey::Enter) => {
                        if let Some(idx) = self.selected {
                            self.activated = Some(idx);
                        }
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
        self.v_scrollbar.set_rect(Rect::new(
            bounds.right() - SCROLLBAR_THICKNESS,
            bounds.y,
            SCROLLBAR_THICKNESS,
            bounds.h,
        ));
        self.ensure_selection_visible();
    }
}

/// Background color for a ref badge by kind.
fn badge_color(kind: RefKind) -> Color {
    match kind {
        RefKind::Head => Color::rgb(0x7C, 0xE0, 0x7C),
        RefKind::LocalBranch => Color::rgb(0xC4, 0xF0, 0xC4),
        RefKind::RemoteBranch => Color::rgb(0xF0, 0xCF, 0x9C),
        RefKind::Tag => Color::rgb(0xF2, 0xEA, 0x9C),
        RefKind::DetachedHead => Color::rgb(0xBE, 0xDE, 0xF2),
    }
}

/// Draw one ref badge and return its drawn width.
fn draw_badge(
    painter: &mut Painter,
    x: i32,
    row_y: i32,
    label: &str,
    kind: RefKind,
    font_size: f32,
) -> i32 {
    let tw = painter.measure_text(label, font_size).w;
    let bw = tw + 8;
    let bh = font_size as i32 + 3;
    let by = row_y + (ROW_HEIGHT - bh) / 2;
    let rect = Rect::new(x, by, bw, bh);
    painter.fill_rect(rect, badge_color(kind));
    painter.stroke_rect(rect, Color::BLACK);
    let label_y = row_y + (ROW_HEIGHT - font_size as i32) / 2 - 1;
    painter.text(x + 4, label_y, label, font_size, Color::BLACK);
    bw
}
