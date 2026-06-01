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
/// once the inter-pane gap is accounted for.
fn clamp_files_w(b: Rect) -> i32 {
    FILES_W.min((b.w - 3 * BROWSE_PAD - 80).max(0))
}

// ---- commit (git-gui) layout ----------------------------------------------

const LEFT_W: i32 = 320;
const PAD: i32 = 6;
const HEADING_H: i32 = 18;
const LEFT_BTN_H: i32 = 28;
const RIGHT_BTN_H: i32 = 34;
const DIFF_FRAC: f32 = 0.5;
const BTN_W: i32 = 96;
const BTN_GAP: i32 = 4;

pub fn commit_menu(b: Rect) -> Rect {
    Rect::new(b.x, b.y, b.w, MENU_H)
}

fn content_top(b: Rect) -> i32 {
    b.y + MENU_H
}
fn content_height(b: Rect) -> i32 {
    (b.h - MENU_H).max(0)
}
fn left_x(b: Rect) -> i32 {
    b.x + PAD
}
fn left_w() -> i32 {
    (LEFT_W - 2 * PAD).max(0)
}
fn left_area_h(b: Rect) -> i32 {
    (content_height(b) - LEFT_BTN_H).max(0)
}
fn section_h(b: Rect) -> i32 {
    left_area_h(b) / 2
}

pub fn commit_unstaged_label(b: Rect) -> Rect {
    Rect::new(left_x(b), content_top(b) + 2, left_w(), HEADING_H)
}

pub fn commit_unstaged_list(b: Rect) -> Rect {
    let y = content_top(b) + 2 + HEADING_H;
    Rect::new(
        left_x(b),
        y,
        left_w(),
        (section_h(b) - HEADING_H - 4).max(0),
    )
}

pub fn commit_staged_label(b: Rect) -> Rect {
    let y = content_top(b) + section_h(b) + 2;
    Rect::new(left_x(b), y, left_w(), HEADING_H)
}

pub fn commit_staged_list(b: Rect) -> Rect {
    let y = content_top(b) + section_h(b) + 2 + HEADING_H;
    let h = (left_area_h(b) - section_h(b) - HEADING_H - 4).max(0);
    Rect::new(left_x(b), y, left_w(), h)
}

fn left_btn_y(b: Rect) -> i32 {
    content_top(b) + left_area_h(b) + 2
}

pub fn commit_stage_btn(b: Rect) -> Rect {
    Rect::new(left_x(b), left_btn_y(b), BTN_W, 24)
}

pub fn commit_unstage_btn(b: Rect) -> Rect {
    Rect::new(left_x(b) + BTN_W + BTN_GAP, left_btn_y(b), BTN_W, 24)
}

pub fn commit_rescan_btn(b: Rect) -> Rect {
    Rect::new(left_x(b) + 2 * (BTN_W + BTN_GAP), left_btn_y(b), BTN_W, 24)
}

fn right_x(b: Rect) -> i32 {
    b.x + LEFT_W
}
fn right_w(b: Rect) -> i32 {
    (b.w - LEFT_W).max(0)
}
fn right_inner_x(b: Rect) -> i32 {
    right_x(b) + PAD
}
fn right_inner_w(b: Rect) -> i32 {
    (right_w(b) - 2 * PAD).max(0)
}
fn diff_h(b: Rect) -> i32 {
    (content_height(b) as f32 * DIFF_FRAC) as i32
}
fn right_btn_y(b: Rect) -> i32 {
    content_top(b) + content_height(b) - RIGHT_BTN_H + 4
}

pub fn commit_diff(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        content_top(b) + 2,
        right_inner_w(b),
        (diff_h(b) - 4).max(0),
    )
}

pub fn commit_msg_label(b: Rect) -> Rect {
    Rect::new(
        right_inner_x(b),
        content_top(b) + diff_h(b),
        right_inner_w(b),
        HEADING_H,
    )
}

pub fn commit_editor(b: Rect) -> Rect {
    let top = content_top(b) + diff_h(b) + HEADING_H;
    let h = (right_btn_y(b) - top - 4).max(0);
    Rect::new(right_inner_x(b), top, right_inner_w(b), h)
}

pub fn commit_amend(b: Rect) -> Rect {
    Rect::new(right_inner_x(b), right_btn_y(b), 180, 24)
}

pub fn commit_commit_btn(b: Rect) -> Rect {
    let w = 110;
    Rect::new(right_x(b) + right_w(b) - PAD - w, right_btn_y(b), w, 26)
}
