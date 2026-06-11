//! A scrollable, syntax-colored unified-diff viewer.
//!
//! `DiffView` renders a [`Diff`] line-by-line in the monospace font, tinting
//! additions green, deletions red, hunk headers blue and file headers gray —
//! the standard gitk / `git diff --color` palette adapted to saudade's Win
//! 3.1 chrome. It owns a vertical scrollbar pinned to the right edge and, like
//! saudade's `List`, only measures and paints the rows currently on screen.
//!
//! In the commit screen it also gains a *line-range selection*: the user
//! click-drags (or clicks one line and Shift-clicks another) to highlight an
//! adjacent block of lines, which gets a translucent overlay and an animated
//! "marching ants" border, and a small **Stage** / **Unstage** button floats in
//! the selection's bottom-right corner so part of a file's diff can be staged
//! without staging the whole file. This is enabled only by [`set_mode`] with a
//! non-[`DiffMode::Plain`] mode, which the browse view never does.
//!
//! [`set_mode`]: DiffView::set_mode

use saudade::{
    Color, Event, EventCtx, FontFamily, FontStyle, Key, MouseButton, NamedKey, Painter, Point,
    Rect, SCROLLBAR_THICKNESS, ScrollBar, Theme, Widget,
};

use crate::backend::{Diff, DiffLineKind, is_change_line};

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

// Selection chrome. The overlay is a 50%-coverage checkerboard of `SEL_OVERLAY`
// stippled over the already-painted diff — the toolkit has no alpha-blended
// fill, and a stipple is the authentic Win 3.1 way to read as ~50% opacity
// while letting the text show through. The border is animated marching ants
// alternating between `ANT_LIGHT` and `ANT_DARK`.
const SEL_OVERLAY: Color = Color::rgb(0x33, 0x66, 0xCC);
const ANT_LIGHT: Color = Color::WHITE;
const ANT_DARK: Color = Color::rgb(0x00, 0x33, 0x99);
/// Run length (logical px) of one marching-ant dash.
const ANT_DASH: i32 = 3;
/// Advance the ant phase once every N ticks (~60 Hz), throttling the animation
/// — and the repaints it drives — to a calm march rather than a 60 fps blur.
const ANT_TICK_DIV: u32 = 3;

/// Whether the diff view offers line-range staging, and which way. The browse
/// view stays [`DiffMode::Plain`]; the commit view sets [`DiffMode::Stage`] for
/// an unstaged file and [`DiffMode::Unstage`] for a staged one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    /// Read-only: no selection, no staging affordance.
    Plain,
    /// Selecting lines offers to stage them (unstaged-file diff).
    Stage,
    /// Selecting lines offers to unstage them (staged-file diff).
    Unstage,
}

/// A diff pane, read-only in browse mode and line-stageable in commit mode.
pub struct DiffView {
    rect: Rect,
    diff: Diff,
    v_scrollbar: ScrollBar,
    focused: bool,
    font_size: f32,
    mode: DiffMode,
    /// The fixed end of a range selection (the row where it was anchored).
    anchor: Option<usize>,
    /// The moving end of a range selection (the last row touched).
    lead: Option<usize>,
    /// A press-drag selection is in progress.
    dragging: bool,
    /// Marching-ants animation phase, advanced on `Tick`.
    ant_phase: u32,
    /// Tick counter used to throttle phase advances to [`ANT_TICK_DIV`].
    tick_accum: u32,
    /// Set when the Stage/Unstage button is clicked: the inclusive selected row
    /// range, drained by the UI via [`take_action`](Self::take_action).
    pending_action: Option<(usize, usize)>,
    /// The Stage/Unstage button's bounds from the last paint, for hit-testing.
    button_rect: Option<Rect>,
    /// A press on the Stage/Unstage button is in progress (mouse went down on
    /// it and hasn't been released yet). The action fires only on release *over*
    /// the button — like a real push button.
    button_pressed: bool,
    /// While a press is in progress, whether the cursor is currently over the
    /// button (so it draws sunken, and pops back up if the user drags off).
    button_hot: bool,
}

impl DiffView {
    pub fn new(rect: Rect) -> Self {
        let mut me = Self {
            rect,
            diff: Diff::default(),
            v_scrollbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            focused: false,
            font_size: 12.0,
            mode: DiffMode::Plain,
            anchor: None,
            lead: None,
            dragging: false,
            ant_phase: 0,
            tick_accum: 0,
            pending_action: None,
            button_rect: None,
            button_pressed: false,
            button_hot: false,
        };
        me.relayout_scrollbar();
        me
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Replace the displayed diff and reset the scroll position and selection.
    pub fn set_diff(&mut self, diff: Diff) {
        self.diff = diff;
        self.v_scrollbar.set_value(0);
        self.clear_selection();
        self.pending_action = None;
        self.sync_scrollbar();
    }

    /// Set whether (and how) line-range staging is offered. Switching to
    /// [`DiffMode::Plain`] clears any selection in progress.
    pub fn set_mode(&mut self, mode: DiffMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        if mode == DiffMode::Plain {
            self.clear_selection();
        }
    }

    /// Take the pending Stage/Unstage request (the selected inclusive row
    /// range), if the button was clicked since the last poll.
    pub fn take_action(&mut self) -> Option<(usize, usize)> {
        self.pending_action.take()
    }

    pub fn is_empty(&self) -> bool {
        self.diff.is_empty()
    }

    fn clear_selection(&mut self) {
        self.anchor = None;
        self.lead = None;
        self.dragging = false;
        self.button_pressed = false;
        self.button_hot = false;
    }

    /// The raw selected row span `(lo, hi)` from the gesture's two endpoints.
    fn selection_span(&self) -> Option<(usize, usize)> {
        match (self.anchor, self.lead) {
            (Some(a), Some(l)) => Some((a.min(l), a.max(l))),
            _ => None,
        }
    }

    /// The selection clamped to actual *body* rows: the first and last
    /// selectable (non-header) row within the span. File and hunk headers are
    /// never part of a selection, so a span that begins or ends on one snaps
    /// inward to the content. `None` when the span holds no selectable row.
    fn body_bounds(&self) -> Option<(usize, usize)> {
        let (lo, hi) = self.selection_span()?;
        let mut first = None;
        let mut last = None;
        for r in lo..=hi {
            if self
                .diff
                .lines
                .get(r)
                .is_some_and(|l| is_selectable(l.kind))
            {
                first.get_or_insert(r);
                last = Some(r);
            }
        }
        Some((first?, last?))
    }

    /// Does the selection cover at least one `+`/`-` row — the only kind a
    /// partial stage/unstage can act on?
    fn selection_has_change(&self) -> bool {
        self.body_bounds().is_some_and(|(lo, hi)| {
            (lo..=hi).any(|r| {
                self.diff
                    .lines
                    .get(r)
                    .is_some_and(|l| is_change_line(l.kind))
            })
        })
    }

    /// The body-row range a click on `row` selects: the row itself for a content
    /// line, the whole hunk for a hunk header. `None` for a file/commit header
    /// (clicking those clears the selection), an empty hunk, or an out-of-range
    /// row.
    fn click_target_range(&self, row: usize) -> Option<(usize, usize)> {
        match self.diff.lines.get(row)?.kind {
            DiffLineKind::HunkHeader => self.hunk_body_bounds(row),
            DiffLineKind::FileHeader | DiffLineKind::CommitHeader => None,
            _ => Some((row, row)),
        }
    }

    /// First/last body row of the hunk introduced by the header at `header_row`.
    fn hunk_body_bounds(&self, header_row: usize) -> Option<(usize, usize)> {
        let lines = &self.diff.lines;
        let start = header_row + 1;
        if lines.get(start).is_none_or(|l| !is_selectable(l.kind)) {
            return None;
        }
        let mut end = start;
        while lines.get(end + 1).is_some_and(|l| is_selectable(l.kind)) {
            end += 1;
        }
        Some((start, end))
    }

    fn line_height(&self) -> i32 {
        (self.font_size as i32 + 4).max(8)
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
        ((self.text_area().h - TEXT_PAD_Y * 2) / self.line_height()).max(1)
    }

    fn scroll_top(&self) -> usize {
        self.v_scrollbar.value().max(0) as usize
    }

    /// The diff row under `pos`, or `None` when the click is past the last line
    /// (so a click in the empty area below the diff clears the selection).
    fn row_at(&self, pos: Point) -> Option<usize> {
        let text = self.text_area();
        if !text.inset(1).contains(pos) {
            return None;
        }
        let text_y0 = text.y + TEXT_PAD_Y;
        let offset = ((pos.y - text_y0).max(0)) / self.line_height();
        let row = self.scroll_top() + offset as usize;
        (row < self.diff.lines.len()).then_some(row)
    }

    /// Like [`row_at`](Self::row_at) but clamped into the content range, used
    /// while dragging so the selection can extend past the visible edge.
    fn row_at_clamped(&self, pos: Point) -> Option<usize> {
        if self.diff.lines.is_empty() {
            return None;
        }
        let text = self.text_area();
        let rel = pos.y - (text.y + TEXT_PAD_Y);
        let offset = if rel < 0 { 0 } else { rel / self.line_height() };
        let row = (self.scroll_top() as i32 + offset).clamp(0, self.diff.lines.len() as i32 - 1);
        Some(row as usize)
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

    /// Draw the selection overlay, marching-ants border and Stage/Unstage
    /// button over the already-painted diff, and cache the button's rect for
    /// hit-testing. `text` is the text field rect, `row_w` the row fill width.
    fn paint_selection(&mut self, painter: &mut Painter, theme: &Theme, text: Rect, row_w: i32) {
        self.button_rect = None;
        if self.mode == DiffMode::Plain {
            return;
        }
        let Some((lo, hi)) = self.body_bounds() else {
            return;
        };

        let line_h = self.line_height();
        let visible = self.visible_rows() as usize;
        let top = self.scroll_top();
        let vis_lo = lo.max(top);
        let vis_hi = hi.min(top + visible.saturating_sub(1));
        if vis_lo > vis_hi {
            return; // selection scrolled out of view
        }

        let text_y0 = text.y + TEXT_PAD_Y;
        let row_band = |r: usize| {
            Rect::new(
                text.x + 1,
                text_y0 + (r - top) as i32 * line_h,
                row_w,
                line_h,
            )
        };
        let y0 = text_y0 + (vis_lo - top) as i32 * line_h;
        let y1 = text_y0 + (vis_hi - top + 1) as i32 * line_h;
        let sel = Rect::new(text.x + 1, y0, row_w, y1 - y0);

        let saved = painter.push_clip(text.inset(1));
        // Stipple each selected content row; a header caught inside a cross-hunk
        // span stays clean (it is never part of the selection).
        for r in vis_lo..=vis_hi {
            if self
                .diff
                .lines
                .get(r)
                .is_some_and(|l| is_selectable(l.kind))
            {
                stipple_rect(painter, row_band(r), SEL_OVERLAY);
            }
        }
        marching_ants(painter, sel, self.ant_phase, ANT_LIGHT, ANT_DARK);

        if self.selection_has_change() {
            let label = match self.mode {
                DiffMode::Stage => "Stage",
                DiffMode::Unstage => "Unstage",
                DiffMode::Plain => unreachable!(),
            };
            let bh = (self.font_size as i32 + 10).max(18);
            let bw = painter.measure_text(label, self.font_size).w + 16;
            // Bottom-right of the selection, clamped inside the text field so it
            // stays fully visible even for a one-line or edge selection.
            let inner = text.inset(2);
            let bx = (sel.right() - bw - 4).min(inner.right() - bw).max(inner.x);
            let by = (sel.bottom() - bh - 4).clamp(inner.y, (inner.bottom() - bh).max(inner.y));
            let brect = Rect::new(bx, by, bw, bh);
            let pressed = self.button_pressed && self.button_hot;
            painter.button(brect, theme, pressed, false);
            // Nudge the label down-right a pixel while held, the usual pressed
            // affordance.
            let label_rect = if pressed {
                Rect::new(brect.x + 1, brect.y + 1, brect.w, brect.h)
            } else {
                brect
            };
            painter.text_centered(label_rect, label, self.font_size, theme.text);
            self.button_rect = Some(brect);
        }
        painter.restore_clip(saved);
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
            painter.text_styled(
                text_x,
                label_y,
                &line.text,
                self.font_size,
                fg,
                FontFamily::Mono,
                FontStyle::Regular,
            );
        }
        painter.restore_clip(saved);

        // Selection overlay + Stage/Unstage button float over the diff text but
        // under the scrollbar.
        self.paint_selection(painter, theme, text, row_w);

        self.v_scrollbar.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Route to the scrollbar while it's dragging or being clicked.
        if self.v_scrollbar.captures_pointer() {
            self.v_scrollbar.event(event, ctx);
            return;
        }
        // The wheel scrolls the diff whenever the pointer is anywhere over
        // it — not just over the scrollbar gutter — without disturbing any
        // line selection, matching native scrolled views.
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
                modifiers,
            } => {
                // Press on the floating button: arm it, but don't act until the
                // user releases over it (a real push button — releasing off it
                // cancels). Capturing the pointer keeps the release coming here.
                if self.button_rect.is_some_and(|r| r.contains(*pos)) {
                    self.button_pressed = true;
                    self.button_hot = true;
                    ctx.request_paint();
                    return;
                }
                ctx.request_focus();
                if self.mode != DiffMode::Plain {
                    // A click selects a content line; clicking a hunk header
                    // selects the whole hunk. File/commit headers aren't
                    // selectable, so clicking one clears the selection.
                    match self
                        .row_at(*pos)
                        .and_then(|row| self.click_target_range(row))
                    {
                        Some((s, e)) if modifiers.shift && self.anchor.is_some() => {
                            // Shift-click extends the existing selection to cover
                            // the clicked line (or whole hunk), keeping the anchor
                            // end fixed.
                            let anchor = self.anchor.unwrap();
                            self.lead = Some(if anchor <= s { e } else { s });
                        }
                        Some((s, e)) => {
                            self.anchor = Some(s);
                            self.lead = Some(e);
                            self.dragging = true;
                        }
                        None => self.clear_selection(),
                    }
                }
                ctx.request_paint();
            }
            // While the button is held, track whether the cursor is still over
            // it so it draws sunken / pops back up as the user drags on and off.
            Event::PointerMove { pos } if self.button_pressed => {
                let hot = self.button_rect.is_some_and(|r| r.contains(*pos));
                if hot != self.button_hot {
                    self.button_hot = hot;
                    ctx.request_paint();
                }
            }
            Event::PointerMove { pos } if self.dragging => {
                if let Some(row) = self.row_at_clamped(*pos) {
                    self.lead = Some(row);
                    ctx.request_paint();
                }
            }
            // Releasing over the button fires the action; releasing off it just
            // cancels the press, leaving the selection untouched.
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                ..
            } if self.button_pressed => {
                if self.button_rect.is_some_and(|r| r.contains(*pos)) {
                    self.pending_action = self.body_bounds();
                }
                self.button_pressed = false;
                self.button_hot = false;
                ctx.request_paint();
            }
            Event::PointerUp {
                button: MouseButton::Left,
                ..
            } if self.dragging => {
                self.dragging = false;
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                if self.mode != DiffMode::Plain
                    && matches!(key, Key::Named(NamedKey::Escape))
                    && self.selection_span().is_some()
                {
                    self.clear_selection();
                    ctx.request_paint();
                    return;
                }
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
            Event::Tick if self.mode != DiffMode::Plain && self.body_bounds().is_some() => {
                self.tick_accum = self.tick_accum.wrapping_add(1);
                if self.tick_accum.is_multiple_of(ANT_TICK_DIV) {
                    self.ant_phase = self.ant_phase.wrapping_add(1);
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.dragging || self.button_pressed || self.v_scrollbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn wants_ticks(&self) -> bool {
        self.mode != DiffMode::Plain && self.body_bounds().is_some()
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        self.relayout_scrollbar();
    }
}

/// 50%-coverage checkerboard stipple of `color` over `rect`, anchored to
/// absolute coordinates so it reads as a stable translucent screen rather than
/// shifting with the selection. The toolkit has no alpha-blended fill, so this
/// is how the overlay lets the diff text show through at ~50% opacity.
fn stipple_rect(painter: &mut Painter, rect: Rect, color: Color) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    for dy in 0..rect.h {
        let y = rect.y + dy;
        let mut dx = (rect.x + y).rem_euclid(2);
        while dx < rect.w {
            painter.pixel(rect.x + dx, y, color);
            dx += 2;
        }
    }
}

/// A 1px "marching ants" border around `rect`: each perimeter pixel alternates
/// between `light` and `dark` in runs of [`ANT_DASH`], shifted by `phase` so the
/// dashes appear to crawl around the selection.
fn marching_ants(painter: &mut Painter, rect: Rect, phase: u32, light: Color, dark: Color) {
    if rect.w <= 1 || rect.h <= 1 {
        return;
    }
    let p = phase as i32;
    let dash = ANT_DASH.max(1);
    let pick = |coord: i32| {
        if (coord + p).rem_euclid(dash * 2) < dash {
            light
        } else {
            dark
        }
    };
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    let mut x = rect.x;
    while x <= right {
        painter.pixel(x, rect.y, pick(x));
        painter.pixel(x, bottom, pick(x));
        x += 1;
    }
    let mut y = rect.y;
    while y <= bottom {
        painter.pixel(rect.x, y, pick(y));
        painter.pixel(right, y, pick(y));
        y += 1;
    }
}

/// Whether a diff row can be part of a line selection — every content row, but
/// not the file / hunk / commit header rows (clicking a hunk header selects the
/// whole hunk, a file header clears the selection; the headers themselves never
/// highlight).
fn is_selectable(kind: DiffLineKind) -> bool {
    !matches!(
        kind,
        DiffLineKind::FileHeader | DiffLineKind::HunkHeader | DiffLineKind::CommitHeader
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::DiffLine;
    use saudade::mock::MockBackend;
    use saudade::{Event, Modifiers, Point};

    const W: i32 = 320;
    const H: i32 = 200;

    fn down(x: i32, y: i32) -> Event {
        Event::PointerDown {
            pos: Point::new(x, y),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        }
    }
    fn up(x: i32, y: i32) -> Event {
        Event::PointerUp {
            pos: Point::new(x, y),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        }
    }

    /// rows: 0 file header, 1 hunk header, 2 context, 3 add, 4 add, 5 context.
    fn sample() -> Diff {
        use DiffLineKind::*;
        Diff {
            lines: [
                (FileHeader, "diff --git a/f b/f"),
                (HunkHeader, "@@ -1,2 +1,4 @@"),
                (Context, " ctx"),
                (Addition, "+one"),
                (Addition, "+two"),
                (Context, " ctx2"),
            ]
            .iter()
            .map(|(k, t)| DiffLine::new(*k, t.to_string()))
            .collect(),
        }
    }

    /// Center y of diff row `r` for a widget anchored at (0,0): rows start at
    /// `TEXT_PAD_Y` and are `line_height` (font 12 + 4 = 16) tall.
    fn row_y(r: i32) -> i32 {
        TEXT_PAD_Y + r * 16 + 8
    }

    fn staged_view() -> (MockBackend, DiffView) {
        let be = MockBackend::new(W, H).with_scale(1.0);
        let mut dv = DiffView::new(Rect::new(0, 0, W, H));
        dv.set_mode(DiffMode::Stage);
        dv.set_diff(sample());
        dv.layout(Rect::new(0, 0, W, H));
        let _ = be.render(&mut dv);
        (be, dv)
    }

    fn scroll(x: i32, y: i32, delta_y: f32) -> Event {
        Event::Scroll {
            pos: Point::new(x, y),
            delta_x: 0.0,
            delta_y,
        }
    }

    /// Like [`staged_view`] but with the sample diff padded by enough trailing
    /// context lines that the view can actually scroll.
    fn long_staged_view() -> (MockBackend, DiffView) {
        let mut diff = sample();
        diff.lines.extend(
            (0..40).map(|i| DiffLine::new(DiffLineKind::Context, format!(" pad {i}"))),
        );
        let be = MockBackend::new(W, H).with_scale(1.0);
        let mut dv = DiffView::new(Rect::new(0, 0, W, H));
        dv.set_mode(DiffMode::Stage);
        dv.set_diff(diff);
        dv.layout(Rect::new(0, 0, W, H));
        let _ = be.render(&mut dv);
        (be, dv)
    }

    #[test]
    fn the_wheel_scrolls_the_diff_without_touching_the_selection() {
        let (be, mut dv) = long_staged_view();
        // Select an addition first, so we can check the wheel leaves it alone.
        be.dispatch(&mut dv, &down(10, row_y(3)));
        be.dispatch(&mut dv, &up(10, row_y(3)));
        assert_eq!(dv.body_bounds(), Some((3, 3)));
        assert_eq!(dv.scroll_top(), 0);

        be.dispatch(&mut dv, &scroll(W / 2, H / 2, 3.0));
        assert_eq!(dv.scroll_top(), 3, "one notch scrolls three lines down");
        assert_eq!(dv.body_bounds(), Some((3, 3)), "selection is untouched");

        be.dispatch(&mut dv, &scroll(W / 2, H / 2, -3.0));
        assert_eq!(dv.scroll_top(), 0, "scrolling back returns to the top");
    }

    #[test]
    fn a_wheel_event_outside_the_diff_is_ignored() {
        let (be, mut dv) = long_staged_view();
        be.dispatch(&mut dv, &scroll(W + 10, H + 10, 3.0));
        assert_eq!(dv.scroll_top(), 0);
    }

    #[test]
    fn clicking_a_hunk_header_selects_the_whole_hunk() {
        let (be, mut dv) = staged_view();
        be.dispatch(&mut dv, &down(10, row_y(1))); // the @@ hunk header
        be.dispatch(&mut dv, &up(10, row_y(1)));
        // The hunk's body is rows 2..=5; the header itself is excluded.
        assert_eq!(dv.body_bounds(), Some((2, 5)));
    }

    #[test]
    fn clicking_a_file_header_clears_the_selection() {
        let (be, mut dv) = staged_view();
        be.dispatch(&mut dv, &down(10, row_y(3))); // select an addition
        be.dispatch(&mut dv, &up(10, row_y(3)));
        assert_eq!(dv.body_bounds(), Some((3, 3)));
        be.dispatch(&mut dv, &down(10, row_y(0))); // click the file header
        be.dispatch(&mut dv, &up(10, row_y(0)));
        assert_eq!(dv.body_bounds(), None, "file-header click deselects");
        assert!(dv.anchor.is_none());
    }

    #[test]
    fn button_fires_only_on_release_over_it() {
        let (be, mut dv) = staged_view();
        // Select an addition so the Stage button shows, then re-render to place
        // (and cache) the button rect.
        be.dispatch(&mut dv, &down(10, row_y(3)));
        be.dispatch(&mut dv, &up(10, row_y(3)));
        let _ = be.render(&mut dv);
        let b = dv.button_rect.expect("button shows for a change selection");
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);

        // Press on the button but release away from it: nothing happens, and the
        // selection is untouched.
        be.dispatch(&mut dv, &down(cx, cy));
        be.dispatch(&mut dv, &up(2, row_y(2)));
        assert!(dv.take_action().is_none(), "release off the button cancels");
        assert_eq!(
            dv.body_bounds(),
            Some((3, 3)),
            "selection survives a cancel"
        );
        assert!(!dv.button_pressed);

        // Press and release over the button: the action fires with the range.
        be.dispatch(&mut dv, &down(cx, cy));
        be.dispatch(&mut dv, &up(cx, cy));
        assert_eq!(
            dv.take_action(),
            Some((3, 3)),
            "release over the button fires"
        );
    }
}
