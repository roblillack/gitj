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
    Color, Event, EventCtx, Key, Merged, MouseButton, NamedKey, Painter, Point, Rect,
    SCROLLBAR_THICKNESS, ScrollBar, Theme, Widget,
};

use crate::backend::{RefKind, RefLabel};
use crate::widgets::graph::{Graph, GraphRow};
use crate::widgets::palette::Palette;

const ROW_HEIGHT: i32 = 18;
const TEXT_PAD_X: i32 = 4;
const TEXT_PAD_Y: i32 = 2;
const COL_GAP: i32 = 12;
const AUTHOR_COL_W: i32 = 120;
const BADGE_GAP: i32 = 3;
const DOUBLE_CLICK_MS: u64 = 400;

/// Logical width of one graph lane.
const LANE_W: i32 = 14;

/// Logical radius of a commit dot, out to its edge: 7 pixels across at 1.0x.
const DOT_RADIUS: f32 = 3.5;

/// The gitk-style ink of graph lane `col` (the palette wraps).
fn lane_color(palette: &Palette, col: usize) -> Color {
    palette.lanes[col % palette.lanes.len()]
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
            font_size: 11.0,
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
        // The field's right edge sits on the scrollbar's own left border (see
        // `text_area`), so it is a merged edge: the border line lands on the
        // scrollbar's border pixels at every scale.
        let merged = if self.v_scrollbar.rect().w > 0 {
            Merged::RIGHT
        } else {
            Merged::NONE
        };
        painter.fill_rect(text, theme.background);
        painter.sunken_bevel_merged(text, merged, theme.highlight, theme.shadow);
        painter.stroke_rect_merged(text, merged, theme.border);

        let text_x = text.x + TEXT_PAD_X;
        let text_y0 = text.y + TEXT_PAD_Y;
        let visible = self.visible_rows() as usize;
        let scroll_top = self.scroll_top();
        let row_right = text.right() - TEXT_PAD_X;
        let graph_w = self.graph_width();
        let date_w = date_col_width(painter, self.font_size);

        // Clip everything to the frame's interior, so rows (and the selection
        // band, painted edge to edge) stop on exactly the border line's device
        // pixels instead of bleeding into it at fractional scales.
        let saved_clip = painter.push_clip_frame(text, 1, merged);
        let palette = Palette::for_theme(theme);

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
                // The band spans the full field width — gitk-style, border to
                // border — with the frame clip trimming it to the interior.
                painter.fill_rect(Rect::new(text.x, y, text.w, ROW_HEIGHT), bg);
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
                let gutter = Rect::new(text_x, y, graph_w, ROW_HEIGHT);
                draw_graph_row(painter, palette, grow, gutter);
            }

            // Fixed-width date column at the right, author column to its left;
            // both left-aligned so every row's columns start at the same x.
            let date_x = row_right - date_w;
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
                x +=
                    draw_badge(painter, palette, x, y, &r.name, r.kind, self.font_size) + BADGE_GAP;
            }
            let summary_right = author_x - COL_GAP;
            if summary_right > x {
                let saved = painter.push_clip(Rect::new(x, y, summary_right - x, ROW_HEIGHT));
                painter.text(x, label_y, &data.summary, self.font_size, fg);
                painter.restore_clip(saved);
            }
        }

        painter.restore_clip(saved_clip);

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

/// Width of the date column: the widest `YYYY-MM-DD HH:MM` the UI font can
/// draw. Its digits are proportional, so sizing to any one row's date would let
/// the column (and the author column left of it) drift from row to row.
fn date_col_width(painter: &Painter, font_size: f32) -> i32 {
    let widest_digit = ('0'..='9')
        .max_by_key(|d| painter.measure_text(&d.to_string(), font_size).w)
        .unwrap_or('0');
    let template: String = "0000-00-00 00:00"
        .chars()
        .map(|c| if c == '0' { widest_digit } else { c })
        .collect();
    painter.measure_text(&template, font_size).w
}

/// Paint one row of the commit graph into `gutter`, the row's slice of the
/// left-hand graph column.
///
/// The graph is drawn in device pixels, not logical ones. Built out of logical
/// pixels it was the 1.0x art blown up: at 2x every diagonal and every dot
/// stair-stepped in 2×2 blocks, and at fractional
/// scales a line came out one device pixel wide in one place and two in the
/// next, depending on where its logical pixels happened to snap. Here every
/// line is a whole [`Painter::chrome_unit`] thick — the same width as the
/// field's border and the UI's other thin lines — and steps at the display's
/// own resolution, and the dot is a circle rasterized at its scaled size.
///
/// Each point still sits where the logical pixel it replaces would have
/// snapped, measured from the gutter's snapped corner, and the row's bottom is
/// the next row's top, so lanes run on unbroken from row to row.
fn draw_graph_row(painter: &mut Painter, palette: &Palette, row: &GraphRow, gutter: Rect) {
    let scale = painter.scale();
    let unit = painter.chrome_unit();
    let radius = DOT_RADIUS * scale;
    // A logical length in device pixels, rounded the way the painter snaps.
    let device = |logical: i32| (logical as f32 * scale).round() as i32;

    painter.physical(gutter, |p, gutter| {
        let lane_x = |col: usize| gutter.x + device(col as i32 * LANE_W + LANE_W / 2);
        let top = gutter.y;
        let center = gutter.y + device(ROW_HEIGHT / 2);
        let bottom = gutter.bottom();

        // Top half: a lane at the top edge curving in to the row center. Color
        // by the upper lane so a line keeps its color along its length.
        for &(from, to) in &row.top {
            let color = lane_color(palette, from);
            draw_line(p, (lane_x(from), top), (lane_x(to), center), unit, color);
        }
        // Bottom half: from the center down to a lane at the bottom edge. Color
        // by the lower lane (the lane the segment becomes).
        for &(from, to) in &row.bottom {
            let color = lane_color(palette, to);
            draw_line(p, (lane_x(from), center), (lane_x(to), bottom), unit, color);
        }
        let color = lane_color(palette, row.node_col);
        draw_dot(p, (lane_x(row.node_col), center), unit, radius, color);
    });
}

/// A line from `from` to `to`, both ends included, drawn `unit` device pixels
/// thick by stamping a `unit`-square pen along a Bresenham path. Coordinates
/// are device pixels, for a painter inside a [`Painter::physical`] pass; the
/// pen hangs right of and below each point, as a logical pixel does from the
/// device pixel its corner snaps to. Straight verticals are a single fill.
fn draw_line(painter: &mut Painter, from: (i32, i32), to: (i32, i32), unit: i32, color: Color) {
    let ((x0, y0), (x1, y1)) = (from, to);
    if x0 == x1 {
        let (a, b) = (y0.min(y1), y0.max(y1));
        painter.fill_rect(Rect::new(x0, a, unit, b - a + unit), color);
        return;
    }
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        painter.fill_rect(Rect::new(x, y, unit, unit), color);
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

/// A filled commit dot of `radius` device pixels, centered on the `unit`-thick
/// pen [`draw_line`] puts down at device point `at` — so it sits squarely on
/// the lane line whether that is an odd number of device pixels wide (centered
/// on a pixel) or an even one (centered between two). A pixel is in when its
/// center is inside the circle.
fn draw_dot(painter: &mut Painter, at: (i32, i32), unit: i32, radius: f32, color: Color) {
    let half_unit = unit as f32 / 2.0;
    let (cx, cy) = (at.0 as f32 + half_unit, at.1 as f32 + half_unit);
    let first_row = (cy - radius).floor() as i32;
    let last_row = (cy + radius).ceil() as i32;
    for y in first_row..=last_row {
        let dy = y as f32 + 0.5 - cy;
        let reach_sq = radius * radius - dy * dy;
        if reach_sq < 0.0 {
            continue;
        }
        // The run of pixels whose centers lie within `reach` of the center.
        let reach = reach_sq.sqrt();
        let x0 = (cx - reach - 0.5).ceil() as i32;
        let x1 = (cx + reach - 0.5).floor() as i32;
        if x1 >= x0 {
            painter.fill_rect(Rect::new(x0, y, x1 - x0 + 1, 1), color);
        }
    }
}

/// Fill color for a ref badge by kind.
fn badge_color(palette: &Palette, kind: RefKind) -> Color {
    match kind {
        RefKind::Head => palette.badge_head,
        RefKind::LocalBranch => palette.badge_local,
        RefKind::RemoteBranch => palette.badge_remote,
        RefKind::Tag => palette.badge_tag,
        RefKind::DetachedHead => palette.badge_detached,
    }
}

/// Draw one ref badge and return its drawn width.
fn draw_badge(
    painter: &mut Painter,
    palette: &Palette,
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
    painter.fill_rect(rect, badge_color(palette, kind));
    painter.stroke_rect(rect, palette.badge_border);
    let label_y = row_y + (ROW_HEIGHT - font_size as i32) / 2 - 1;
    painter.text(x + 4, label_y, label, font_size, palette.badge_text);
    bw
}

#[cfg(test)]
mod tests {
    use super::*;
    use saudade::Container;
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

    /// Paint a straight three-commit history, graph attached, at `scale` with
    /// the list `x` logical pixels in from the window's left edge, and hand back
    /// the device rows of the middle commit's row band (the one with a lane both
    /// above and below its dot) along with the lane's color.
    fn middle_graph_row(scale: f32, x: i32) -> (Vec<Vec<u32>>, Color) {
        let ids = ["c", "b", "a"];
        let rows: Vec<CommitRow> = ids
            .iter()
            .zip(["b", "a", ""])
            .map(|(id, parent)| CommitRow {
                id: id.to_string(),
                parents: (!parent.is_empty())
                    .then(|| parent.to_string())
                    .into_iter()
                    .collect(),
                ..CommitRow::default()
            })
            .collect();
        let dag: Vec<_> = rows
            .iter()
            .map(|r| (r.id.clone(), r.parents.clone()))
            .collect();
        let theme = Theme::windows_31();
        let be = MockBackend::new(x + W, H)
            .with_theme(theme.clone())
            .with_scale(scale);
        let bounds = Rect::new(x, 0, W, H);
        let mut list = CommitList::new(bounds).with_rows(rows);
        list.set_graph(Some(crate::widgets::graph::compute_graph(&dag)));
        list.layout(bounds);
        // The backend lays its root out at the window's origin; a container
        // keeps the list at `x`.
        let snap = be.render(&mut Container::new(x + W, H).add(list));

        let device = |logical: i32| (logical as f32 * scale).round() as i32;
        let top = TEXT_PAD_Y + ROW_HEIGHT;
        let width = snap.width() as usize;
        let band = (device(top)..device(top + ROW_HEIGHT))
            .map(|y| snap.pixels()[y as usize * width..][..width].to_vec())
            .collect();
        (band, Palette::for_theme(&theme).lanes[0])
    }

    /// How many of a device row's pixels are `color`.
    fn ink(row: &[u32], color: Color) -> usize {
        row.iter().filter(|&&p| p == color.0).count()
    }

    #[test]
    fn graph_lines_are_a_whole_chrome_unit_thick_at_any_scale() {
        // Painted out of logical pixels, a lane was one device pixel wide or
        // two at 1.5x (two or three at 2.5x), depending on where in the window
        // its logical pixel happened to snap — so the list is moved across a
        // few positions too.
        for (scale, unit) in [
            (1.0, 1),
            (1.25, 1),
            (1.5, 1),
            (1.75, 2),
            (2.0, 2),
            (2.25, 2),
            (2.5, 2),
            (3.0, 3),
        ] {
            for x in 0..4 {
                let (band, lane) = middle_graph_row(scale, x);
                // Away from the dot, which spans the middle 7 logical rows,
                // the device rows hold nothing but the lane line.
                let device = |logical: f32| (logical * scale).round() as usize;
                let line_rows = (0..device(4.0)).chain(device(14.0)..band.len());
                for y in line_rows {
                    let n = ink(&band[y], lane);
                    assert_eq!(n, unit, "{scale}x, list at x={x}: device row {y}");
                }
            }
        }
    }

    #[test]
    fn commit_dots_are_round_and_scale_with_the_display() {
        // The dot used to be the 1.0x pixel art blown up: 1-pixel nubs on its
        // top and bottom rows made it taller than its round middle was wide,
        // and at 2x it was 14 device pixels across but only 10 rows of that.
        for (scale, unit) in [(1.0, 1), (1.25, 1), (1.5, 1), (2.0, 2), (2.5, 2), (3.0, 3)] {
            let (band, lane) = middle_graph_row(scale, 0);
            let dot_rows: Vec<usize> = band
                .iter()
                .map(|row| ink(row, lane))
                .filter(|&n| n > unit)
                .collect();
            let across = *dot_rows.iter().max().unwrap();
            assert_eq!(across, dot_rows.len(), "{scale}x: as tall as it is wide");
            let design = 2.0 * DOT_RADIUS * scale;
            assert!(
                (across as f32 - design).abs() <= 1.0,
                "{scale}x: {across} device pixels across, want ~{design}",
            );
        }
    }
}
