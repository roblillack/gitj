//! The commit history list — a gitk-style scrollable list whose rows carry
//! colored ref badges (branches, tags, HEAD) on the left and author / date
//! columns on the right.
//!
//! It's a specialization of saudade's `List`: the selection, scrolling and
//! keyboard behavior are the same, but each row is custom-painted so the refs
//! render as little colored boxes and a graph gutter can be slotted in front
//! later (Phase 6). Kept in journey rather than saudade because the row
//! content is git-specific.

use std::time::{Duration, Instant};

use saudade::{
    Color, Event, EventCtx, Key, MouseButton, NamedKey, Painter, Point, Rect, SCROLLBAR_THICKNESS,
    ScrollBar, Theme, Widget,
};

use crate::backend::{RefKind, RefLabel};
use crate::widgets::graph::{Graph, GraphRow};

const ROW_HEIGHT: i32 = 18;
const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;
const COL_GAP: i32 = 12;
const AUTHOR_COL_W: i32 = 120;
const BADGE_GAP: i32 = 3;
const DOUBLE_CLICK_MS: u64 = 400;

/// Logical width of one graph lane.
const LANE_W: i32 = 14;

/// gitk-style lane palette, indexed by lane column (wraps).
const LANE_COLORS: [Color; 7] = [
    Color::rgb(0x00, 0x80, 0x00),
    Color::rgb(0xC0, 0x00, 0x00),
    Color::rgb(0x00, 0x00, 0xC0),
    Color::rgb(0xA0, 0x00, 0xA0),
    Color::rgb(0x00, 0x80, 0x80),
    Color::rgb(0xB0, 0x60, 0x00),
    Color::rgb(0x50, 0x50, 0x50),
];

fn lane_color(col: usize) -> Color {
    LANE_COLORS[col % LANE_COLORS.len()]
}

/// One commit's worth of row content.
#[derive(Clone, Default)]
pub struct CommitRow {
    pub id: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub refs: Vec<RefLabel>,
    pub author: String,
    pub date: String,
}

pub struct CommitList {
    rect: Rect,
    rows: Vec<CommitRow>,
    /// Optional precomputed DAG layout, aligned 1:1 with `rows`. Present only
    /// when showing full (unfiltered) history.
    graph: Option<Graph>,
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
            graph: None,
            selected: None,
            focused: false,
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            activated: None,
            last_click: None,
            font_size: 12.0,
        }
    }

    /// Attach (or clear) the DAG graph. Must be aligned with the current rows.
    pub fn set_graph(&mut self, graph: Option<Graph>) {
        self.graph = graph;
    }

    /// Logical width reserved for the graph gutter (0 when no graph).
    fn graph_width(&self) -> i32 {
        match &self.graph {
            Some(g) => g.lane_count as i32 * LANE_W,
            None => 0,
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
        // When the scrollbar is present the field overlaps it by 1px so the
        // field's right border lands on the scrollbar's own left-border column,
        // collapsing the divider to a single 1px line instead of stacking the
        // two 1px borders into a 2px band. The scrollbar is painted last, on
        // top, so that shared column reads as the scrollbar's edge — exactly
        // how saudade's `List` does it.
        let (sb_w, overlap) = if self.v_scrollbar.rect().w > 0 {
            (SCROLLBAR_THICKNESS, 1)
        } else {
            (0, 0)
        };
        Rect::new(
            self.rect.x,
            self.rect.y,
            (self.rect.w - sb_w + overlap).max(0),
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
        let graph_w = self.graph_width();

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
            let fg = if active {
                theme.highlight_text
            } else {
                theme.text
            };

            // Graph gutter, if present, in its own column at the far left.
            if let Some(graph) = &self.graph
                && let Some(grow) = graph.rows.get(row)
            {
                draw_graph_row(painter, grow, text_x, y);
            }

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
            let mut x = text_x + graph_w + 2;
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
        // The wheel scrolls the list whenever the pointer is anywhere over
        // it — not just over the scrollbar gutter — without disturbing the
        // selection, matching native list boxes.
        if let Event::Scroll { pos, .. } = event {
            if self.rect.contains(*pos) {
                self.v_scrollbar.event(event, ctx);
            }
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
                ..
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

/// Paint one row of the commit graph in the left gutter starting at `gutter_x`.
fn draw_graph_row(painter: &mut Painter, row: &GraphRow, gutter_x: i32, y: i32) {
    let lane_x = |col: usize| gutter_x + col as i32 * LANE_W + LANE_W / 2;
    let top = y;
    let center = y + ROW_HEIGHT / 2;
    let bottom = y + ROW_HEIGHT;

    // Top half: a lane at the top edge curving in to the row center. Color by
    // the upper lane so a line keeps its color along its length.
    for &(from, to) in &row.top {
        draw_line(
            painter,
            lane_x(from),
            top,
            lane_x(to),
            center,
            lane_color(from),
        );
    }
    // Bottom half: from the center down to a lane at the bottom edge. Color by
    // the lower lane (the lane the segment becomes).
    for &(from, to) in &row.bottom {
        draw_line(
            painter,
            lane_x(from),
            center,
            lane_x(to),
            bottom,
            lane_color(to),
        );
    }
    draw_dot(
        painter,
        lane_x(row.node_col),
        center,
        lane_color(row.node_col),
    );
}

/// Bresenham line via single logical pixels (crisp at any DPI). Straight
/// verticals use the faster `v_line`.
fn draw_line(painter: &mut Painter, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
    if x0 == x1 {
        let (a, b) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        painter.v_line(x0, a, b - a + 1, color);
        return;
    }
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        painter.pixel(x, y, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// A small filled commit dot.
fn draw_dot(painter: &mut Painter, cx: i32, cy: i32, color: Color) {
    let r = 3;
    for dy in -r..=r {
        // Half-width of the disc at this row (circle of radius r).
        let hw = ((r * r - dy * dy) as f32).sqrt().round() as i32;
        painter.h_line(cx - hw, cy + dy, hw * 2 + 1, color);
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

#[cfg(test)]
mod tests {
    use super::*;
    use saudade::mock::MockBackend;

    const W: i32 = 320;
    const H: i32 = 200;

    fn scroll(x: i32, y: i32, delta_y: f32) -> Event {
        Event::Scroll {
            pos: Point::new(x, y),
            delta_x: 0.0,
            delta_y,
        }
    }

    /// A list with more rows than fit, so it can actually scroll.
    fn long_list() -> (MockBackend, CommitList) {
        let rows = (0..40)
            .map(|i| CommitRow {
                id: format!("{i:040x}"),
                summary: format!("commit {i}"),
                ..CommitRow::default()
            })
            .collect();
        let be = MockBackend::new(W, H).with_scale(1.0);
        let mut list = CommitList::new(Rect::new(0, 0, W, H)).with_rows(rows);
        list.set_selected(Some(0));
        list.layout(Rect::new(0, 0, W, H));
        let _ = be.render(&mut list);
        (be, list)
    }

    #[test]
    fn the_wheel_scrolls_the_list_without_touching_the_selection() {
        let (be, mut list) = long_list();
        assert_eq!(list.scroll_top(), 0);

        be.dispatch(&mut list, &scroll(W / 2, H / 2, 3.0));
        assert_eq!(list.scroll_top(), 3, "one notch scrolls three rows down");
        assert_eq!(list.selected_index(), Some(0), "selection is untouched");

        be.dispatch(&mut list, &scroll(W / 2, H / 2, -3.0));
        assert_eq!(list.scroll_top(), 0, "scrolling back returns to the top");
    }

    #[test]
    fn a_wheel_event_outside_the_list_is_ignored() {
        let (be, mut list) = long_list();
        be.dispatch(&mut list, &scroll(W + 10, H + 10, 3.0));
        assert_eq!(list.scroll_top(), 0);
    }
}
