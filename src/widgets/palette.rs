//! gitj's own colors, in a light and a dark variant.
//!
//! Saudade paints with `Theme::windows_31()`, or `Theme::windows_31_dark()`
//! while the system is in dark mode, and hands every widget whichever one it
//! is. The chrome — field fills, bevels, outlines, text — comes straight off
//! that theme, and neutral shades in between are mixed from it with
//! [`Color::mix`]. What can't be derived from it are the colors that *mean*
//! something: the diff's green additions and red deletions, the graph's lane
//! inks, the ref badges. Those get a hand-picked palette per appearance,
//! picked by [`Palette::for_theme`].

use saudade::{Color, Theme};

/// The semantic colors for one appearance.
pub struct Palette {
    /// Text and band of a diff's added lines.
    pub add_fg: Color,
    pub add_bg: Color,
    /// Text and band of a diff's deleted lines.
    pub del_fg: Color,
    pub del_bg: Color,
    /// Text and band of a `@@ … @@` hunk header.
    pub hunk_fg: Color,
    pub hunk_bg: Color,
    /// Text and band of the commit header atop a whole-commit diff.
    pub commit_fg: Color,
    pub commit_bg: Color,
    /// gitk-style lane inks for the history graph, indexed by lane column.
    pub lanes: [Color; 7],
    /// Ref badge fills, by kind.
    pub badge_head: Color,
    pub badge_local: Color,
    pub badge_remote: Color,
    pub badge_tag: Color,
    pub badge_detached: Color,
    /// Outline of a ref badge.
    pub badge_border: Color,
    /// Label of a ref badge.
    pub badge_text: Color,
}

impl Palette {
    /// The palette that suits `theme`: [`DARK`] on a theme whose text is
    /// lighter than its background, [`LIGHT`] otherwise.
    pub fn for_theme(theme: &Theme) -> &'static Palette {
        if is_dark(theme) { &DARK } else { &LIGHT }
    }
}

/// Whether `theme` is a dark one — light text on a dark background.
pub fn is_dark(theme: &Theme) -> bool {
    luma(theme.text) > luma(theme.background)
}

fn luma(c: Color) -> u32 {
    // Rec. 601 weights, scaled to integers.
    299 * c.red() as u32 + 587 * c.green() as u32 + 114 * c.blue() as u32
}

/// The classic gitk / `git diff --color` palette, readable on a white field.
pub static LIGHT: Palette = Palette {
    add_fg: Color::rgb(0x00, 0x64, 0x00),
    add_bg: Color::rgb(0xDC, 0xFF, 0xDC),
    del_fg: Color::rgb(0x90, 0x00, 0x00),
    del_bg: Color::rgb(0xFF, 0xDC, 0xDC),
    hunk_fg: Color::rgb(0x00, 0x00, 0x80),
    hunk_bg: Color::rgb(0xE2, 0xE8, 0xFF),
    commit_fg: Color::rgb(0x40, 0x30, 0x00),
    commit_bg: Color::rgb(0xFF, 0xF6, 0xCC),
    lanes: [
        Color::rgb(0x00, 0x80, 0x00),
        Color::rgb(0xC0, 0x00, 0x00),
        Color::rgb(0x00, 0x00, 0xC0),
        Color::rgb(0xA0, 0x00, 0xA0),
        Color::rgb(0x00, 0x80, 0x80),
        Color::rgb(0xB0, 0x60, 0x00),
        Color::rgb(0x50, 0x50, 0x50),
    ],
    badge_head: Color::rgb(0x7C, 0xE0, 0x7C),
    badge_local: Color::rgb(0xC4, 0xF0, 0xC4),
    badge_remote: Color::rgb(0xF0, 0xCF, 0x9C),
    badge_tag: Color::rgb(0xF2, 0xEA, 0x9C),
    badge_detached: Color::rgb(0xBE, 0xDE, 0xF2),
    badge_border: Color::BLACK,
    badge_text: Color::BLACK,
};

/// The same hues after dark, for the charcoal field of the dark theme: the
/// bands sink to deep tints of their hue, the inks rise to pastels, so text
/// keeps its contrast on either.
pub static DARK: Palette = Palette {
    add_fg: Color::rgb(0x8C, 0xEC, 0x8C),
    add_bg: Color::rgb(0x23, 0x48, 0x23),
    del_fg: Color::rgb(0xFF, 0x8C, 0x8C),
    del_bg: Color::rgb(0x52, 0x24, 0x24),
    hunk_fg: Color::rgb(0x9C, 0xB4, 0xFF),
    hunk_bg: Color::rgb(0x26, 0x30, 0x55),
    commit_fg: Color::rgb(0xF4, 0xE2, 0xA8),
    commit_bg: Color::rgb(0x4C, 0x42, 0x1C),
    lanes: [
        Color::rgb(0x50, 0xC8, 0x50),
        Color::rgb(0xF0, 0x58, 0x58),
        Color::rgb(0x70, 0x90, 0xFF),
        Color::rgb(0xD8, 0x68, 0xD8),
        Color::rgb(0x38, 0xC8, 0xC8),
        Color::rgb(0xF0, 0x98, 0x38),
        Color::rgb(0xA8, 0xA8, 0xA8),
    ],
    badge_head: Color::rgb(0x2E, 0x8A, 0x2E),
    badge_local: Color::rgb(0x2C, 0x5E, 0x2C),
    badge_remote: Color::rgb(0x7A, 0x52, 0x1E),
    badge_tag: Color::rgb(0x6E, 0x64, 0x18),
    badge_detached: Color::rgb(0x2A, 0x58, 0x7A),
    // Black outlines, as on the dark theme's own controls.
    badge_border: Color::BLACK,
    badge_text: Color::rgb(0xF0, 0xF0, 0xF0),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stock_themes_get_their_palettes() {
        assert!(!is_dark(&Theme::windows_31()));
        assert!(is_dark(&Theme::windows_31_dark()));
        assert!(std::ptr::eq(
            Palette::for_theme(&Theme::windows_31()),
            &LIGHT
        ));
        assert!(std::ptr::eq(
            Palette::for_theme(&Theme::windows_31_dark()),
            &DARK
        ));
    }
}
