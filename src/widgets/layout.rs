//! Pure geometry for journey's two screens.
//!
//! The generic [`Shell`](crate::widgets::Shell) places each child by calling
//! one of these functions with the container bounds, so the browse (gitk) and
//! commit (git-gui) layouts are simply two sets of rectangles.

use saudade::Rect;

// ---- browse (gitk) layout -------------------------------------------------

/// Height of the menu bar (shared by both screens).
pub const MENU_H: i32 = 20;
/// Height of the search toolbar below the menu.
pub const TOOLBAR_H: i32 = 26;
/// Width of the changed-files pane on the lower left.
pub const FILES_W: i32 = 300;
/// Fraction of the content height given to the history pane.
pub const HISTORY_FRAC: f32 = 0.46;
/// Slight breathing room around — and between — the three browse panes
/// (history, files, diff), so they don't run into each other or the window
/// edge.
pub const BROWSE_PAD: i32 = 4;
/// At or below this logical window width the left pane (the browse files list
/// and the commit file lists) is capped at a third of the width, so the diff
/// and the other right-hand panes keep the lion's share of a cramped window.
pub const NARROW_W: i32 = 800;

/// A third of the window width — the cap applied to the left pane on narrow
/// windows. `None` above [`NARROW_W`], where the fixed widths stand.
fn narrow_left_cap(b: Rect) -> Option<i32> {
    (b.w <= NARROW_W).then_some(b.w / 3)
}

pub fn browse_menu(b: Rect) -> Rect {
    Rect::new(b.x, b.y, b.w, MENU_H)
}

pub fn browse_toolbar(b: Rect) -> Rect {
    Rect::new(b.x, b.y + MENU_H, b.w, TOOLBAR_H)
}

/// Top of the padded three-pane content area, below the menu and toolbar.
fn content_y(b: Rect) -> i32 {
    b.y + MENU_H + TOOLBAR_H + BROWSE_PAD
}

/// Height available to the three panes, after top and bottom padding.
fn content_h(b: Rect) -> i32 {
    (b.h - MENU_H - TOOLBAR_H - 2 * BROWSE_PAD).max(0)
}

/// Height of the history pane: a fraction of the content area, leaving a gap
/// above the lower (files + diff) band.
fn history_h(b: Rect) -> i32 {
    let avail = (content_h(b) - BROWSE_PAD).max(0);
    (avail as f32 * HISTORY_FRAC).round() as i32
}

pub fn browse_history(b: Rect) -> Rect {
    Rect::new(
        b.x + BROWSE_PAD,
        content_y(b),
        (b.w - 2 * BROWSE_PAD).max(0),
        history_h(b),
    )
}

pub fn browse_files(b: Rect) -> Rect {
    let (lower_y, lower_h) = lower_band(b);
    Rect::new(b.x + BROWSE_PAD, lower_y, clamp_files_w(b), lower_h)
}

pub fn browse_diff(b: Rect) -> Rect {
    let (lower_y, lower_h) = lower_band(b);
    let files_w = clamp_files_w(b);
    let diff_x = b.x + 2 * BROWSE_PAD + files_w;
    let diff_w = (b.w - files_w - 3 * BROWSE_PAD).max(0);
    Rect::new(diff_x, lower_y, diff_w, lower_h)
}

/// The (top, height) of the lower band holding the files and diff panes, below
/// the history pane and the gap under it.
fn lower_band(b: Rect) -> (i32, i32) {
    let lower_y = content_y(b) + history_h(b) + BROWSE_PAD;
    let lower_h = (content_h(b) - history_h(b) - BROWSE_PAD).max(0);
    (lower_y, lower_h)
}

/// Width of the files pane, clamped so the diff pane keeps a usable minimum
/// once the inter-pane gap is accounted for, and capped to a third of the
/// window on narrow layouts (see [`narrow_left_cap`]).
fn clamp_files_w(b: Rect) -> i32 {
    let w = FILES_W.min((b.w - 3 * BROWSE_PAD - 80).max(0));
    match narrow_left_cap(b) {
        Some(cap) => w.min(cap),
        None => w,
    }
}

// ---- commit (git-gui) layout ----------------------------------------------
//
// Both columns share one vertical grid: a top section (heading over a pane) and
// a bottom section of the same shape, above a button band. So the left lists
// line up row-for-row with the right diff and editor — "Unstaged Changes" with
// "Diff", "Staged Changes" with "Commit Message" — and the action buttons share
// one baseline.

const LEFT_W: i32 = 320;
/// Margin between the panes and the window edges.
const PAD: i32 = 6;
/// Space between the two columns, split evenly across the `LEFT_W` divider.
/// Narrower than the outer `PAD` so the lists and the diff sit close together.
const GUTTER: i32 = 6;
const HEADING_H: i32 = 18;
/// Height of the bottom band reserved for the action buttons on both columns.
const BTN_BAND_H: i32 = 34;
const BTN_GAP: i32 = 4;
const LEFT_BTN_H: i32 = 24;
const AMEND_H: i32 = 24;
const COMMIT_BTN_H: i32 = 26;

pub fn commit_menu(b: Rect) -> Rect {
    Rect::new(b.x, b.y, b.w, MENU_H)
}

fn content_top(b: Rect) -> i32 {
    b.y + MENU_H
}
fn content_height(b: Rect) -> i32 {
    (b.h - MENU_H).max(0)
}

// Shared vertical grid -------------------------------------------------------

/// Combined height of the two stacked sections, above the button band.
fn sections_h(b: Rect) -> i32 {
    (content_height(b) - BTN_BAND_H).max(0)
}
/// Height of a single section (heading + pane).
fn section_h(b: Rect) -> i32 {
    sections_h(b) / 2
}
fn top_label_y(b: Rect) -> i32 {
    content_top(b) + 2
}
fn top_pane_y(b: Rect) -> i32 {
    top_label_y(b) + HEADING_H
}
fn top_pane_h(b: Rect) -> i32 {
    (section_h(b) - HEADING_H - 4).max(0)
}
fn bottom_label_y(b: Rect) -> i32 {
    content_top(b) + section_h(b) + 2
}
fn bottom_pane_y(b: Rect) -> i32 {
    bottom_label_y(b) + HEADING_H
}
fn bottom_pane_h(b: Rect) -> i32 {
    (sections_h(b) - section_h(b) - HEADING_H - 4).max(0)
}
/// Top of a button of height `bh`, vertically centered in the bottom band so
/// the left and right rows line up regardless of each button's height.
fn btn_y(b: Rect, bh: i32) -> i32 {
    content_top(b) + sections_h(b) + ((BTN_BAND_H - bh) / 2).max(0)
}

// Left column (unstaged / staged file lists) ---------------------------------

fn left_x(b: Rect) -> i32 {
    b.x + PAD
}
/// Width of the left column (the file lists). Fixed at [`LEFT_W`], but capped
/// to a third of the window on narrow layouts (see [`narrow_left_cap`]) so the
/// diff and message editor keep the majority of the space.
fn left_col_w(b: Rect) -> i32 {
    match narrow_left_cap(b) {
        Some(cap) => LEFT_W.min(cap),
        None => LEFT_W,
    }
}
fn left_w(b: Rect) -> i32 {
    (left_col_w(b) - PAD - GUTTER / 2).max(0)
}

pub fn commit_unstaged_label(b: Rect) -> Rect {
    Rect::new(left_x(b), top_label_y(b), left_w(b), HEADING_H)
}

pub fn commit_unstaged_list(b: Rect) -> Rect {
    Rect::new(left_x(b), top_pane_y(b), left_w(b), top_pane_h(b))
}

pub fn commit_staged_label(b: Rect) -> Rect {
    Rect::new(left_x(b), bottom_label_y(b), left_w(b), HEADING_H)
}

pub fn commit_staged_list(b: Rect) -> Rect {
    Rect::new(left_x(b), bottom_pane_y(b), left_w(b), bottom_pane_h(b))
}

/// Width of each of the three left-column action buttons: they split the column
/// width evenly (minus the two gaps) so the row spans the full list width above
/// it at any window size.
fn left_btn_w(b: Rect) -> i32 {
    ((left_w(b) - 2 * BTN_GAP) / 3).max(0)
}

/// Left edge of the left-column button in `slot` (0..3), packed left-to-right.
fn left_btn_x(b: Rect, slot: i32) -> i32 {
    left_x(b) + slot * (left_btn_w(b) + BTN_GAP)
}

pub fn commit_stage_btn(b: Rect) -> Rect {
    Rect::new(left_btn_x(b, 0), btn_y(b, LEFT_BTN_H), left_btn_w(b), LEFT_BTN_H)
}

pub fn commit_unstage_btn(b: Rect) -> Rect {
    Rect::new(left_btn_x(b, 1), btn_y(b, LEFT_BTN_H), left_btn_w(b), LEFT_BTN_H)
}

pub fn commit_rescan_btn(b: Rect) -> Rect {
    Rect::new(left_btn_x(b, 2), btn_y(b, LEFT_BTN_H), left_btn_w(b), LEFT_BTN_H)
}

// Right column (diff view / commit message editor) ---------------------------

/// Left edge of the right column: a half-gutter past the divider, which moves
/// in with the left column on narrow layouts.
fn right_inner_x(b: Rect) -> i32 {
    b.x + left_col_w(b) + GUTTER / 2
}
/// Width of the right column: from `right_inner_x` to a `PAD` window margin.
fn right_inner_w(b: Rect) -> i32 {
    (b.x + b.w - PAD - right_inner_x(b)).max(0)
}

pub fn commit_diff_label(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        top_label_y(b),
        right_inner_w(b),
        HEADING_H,
    )
}

pub fn commit_diff(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        top_pane_y(b),
        right_inner_w(b),
        top_pane_h(b),
    )
}

pub fn commit_msg_label(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        bottom_label_y(b),
        right_inner_w(b),
        HEADING_H,
    )
}

pub fn commit_editor(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        bottom_pane_y(b),
        right_inner_w(b),
        bottom_pane_h(b),
    )
}

pub fn commit_amend(b: Rect) -> Rect {
    Rect::new(right_inner_x(b), btn_y(b, AMEND_H), 180, AMEND_H)
}

pub fn commit_commit_btn(b: Rect) -> Rect {
    let w = 110;
    Rect::new(
        right_inner_x(b) + right_inner_w(b) - w,
        btn_y(b, COMMIT_BTN_H),
        w,
        COMMIT_BTN_H,
    )
}
