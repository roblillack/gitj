//! Rasterizing SVG blobs for the graphical diff.
//!
//! SVG is text, so its line-by-line diff is meaningful — but for *snapshot*
//! comparison (a chart or icon that changed) seeing the two renders side by side
//! is far more useful. This module is the vector half of [`crate::imagediff`]:
//! it sniffs a blob for SVG and rasterizes it to an [`RgbaImage`] so the two
//! sides flow through the exact same compose pipeline as any PNG. It mirrors the
//! SVG support in the author's `imgap` CLI, adapted to work on the in-memory
//! blobs git hands over rather than files on disk.
//!
//! The whole module is gated behind the `svg` Cargo feature; with it off no
//! vector backend is linked and SVG files keep their text diff (see
//! [`crate::imagediff::is_image_path`]).

use image::RgbaImage;
use resvg::{tiny_skia, usvg};

/// Embedded fallback fonts. resvg ships no font of its own, so text in an SVG
/// (axis labels, titles — the common case for committed chart snapshots) would
/// render as blank boxes. Bundling our own faces also keeps rendering identical
/// regardless of the host's fonts, which is what lets the snapshot tests stay
/// deterministic.
///
/// Two faces, because a proportional default mangles SVG snapshots of *text
/// interfaces* (terminals, code, TUIs): a monospace request gets the monospace
/// face, everything else the sans-serif one (see [`is_monospace_family`]). IBM
/// Plex Mono is the monospace face — small (~130 KB) yet covers the box-drawing
/// and block-element glyphs those interfaces draw with.
const SANS_FONT: &[u8] = include_bytes!("../assets/fonts/PublicSans-Regular.ttf");
const MONO_FONT: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf");
/// Default family name handed to usvg for text that names no family at all.
const DEFAULT_FONT_FAMILY: &str = "Public Sans";

/// Hard ceiling on either edge of a rasterized SVG. SVG is resolution
/// independent, so [`rasterize_fit`] renders it at whatever size the pane needs;
/// this just caps that so a stray huge `viewBox` (or an enormous pane) can't
/// allocate a giant buffer.
const MAX_DIM: f32 = 2048.0;

/// Whether `bytes` looks like an SVG document worth handing to the rasterizer.
/// Cheap structural sniffing — the bytes only ever reach here for a path that
/// already passed [`crate::imagediff::is_image_path`], so this just keeps
/// non-SVG blobs (and `image`-decodable rasters) off the vector path.
pub fn looks_like_svg(bytes: &[u8]) -> bool {
    // `.svgz` is a gzip stream; let the rasterizer's usvg decompress it.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    // Plain SVG: find the opening `<svg` tag within the leading window (past any
    // XML declaration, doctype, comments or BOM/whitespace).
    let head = &bytes[..bytes.len().min(1024)];
    head.to_ascii_lowercase()
        .windows(b"<svg".len())
        .any(|w| w == b"<svg")
}

/// Rasterize an SVG blob at its intrinsic size (capped at [`MAX_DIM`]),
/// preserving aspect ratio, or `None` when the bytes aren't a parseable SVG.
/// This is what feeds the metadata line its `WxH`; the diff itself renders the
/// SVG to fit the pane via [`rasterize_fit`].
pub fn rasterize(bytes: &[u8]) -> Option<RgbaImage> {
    let tree = parse(bytes)?;
    let (svg_w, svg_h) = dimensions(&tree)?;
    // Only ever shrink an oversized `viewBox` here; intrinsic size otherwise.
    let scale = (MAX_DIM / svg_w.max(svg_h)).min(1.0);
    render_scaled(&tree, svg_w, svg_h, scale)
}

/// Rasterize an SVG blob scaled to *fit* `max_w` × `max_h` — scaling up or down
/// as needed, by one uniform factor so it's never skewed. Because SVG is
/// resolution independent this renders crisply at whatever size the diff pane
/// gives it, so the image fills the pane instead of floating at its intrinsic
/// size. `None` when the bytes aren't a parseable SVG.
pub fn rasterize_fit(bytes: &[u8], max_w: u32, max_h: u32) -> Option<RgbaImage> {
    if max_w == 0 || max_h == 0 {
        return None;
    }
    let tree = parse(bytes)?;
    let (svg_w, svg_h) = dimensions(&tree)?;
    // Fit within the box, then clamp so a tiny box paired with a giant `viewBox`
    // (or vice versa) can't blow past MAX_DIM.
    let scale = (max_w as f32 / svg_w)
        .min(max_h as f32 / svg_h)
        .min(MAX_DIM / svg_w.max(svg_h));
    render_scaled(&tree, svg_w, svg_h, scale)
}

/// Parse SVG bytes into a render tree, with the bundled faces wired in so text
/// renders identically everywhere: a monospace request gets the monospace face,
/// everything else the sans-serif one. `None` for bytes that don't look like —
/// or don't parse as — SVG.
fn parse(bytes: &[u8]) -> Option<usvg::Tree> {
    if !looks_like_svg(bytes) {
        return None;
    }
    usvg::Tree::from_data(bytes, &font_options()).ok()
}

/// Build the usvg options with both bundled faces loaded and a resolver that
/// maps monospace requests to the monospace face and everything else to the
/// sans-serif one — always returning *some* face, so text is never dropped.
fn font_options() -> usvg::Options<'static> {
    let mut opt = usvg::Options {
        font_family: DEFAULT_FONT_FAMILY.to_string(),
        ..usvg::Options::default()
    };
    let db = opt.fontdb_mut();
    // Each TTF is a single face, so the just-loaded id is the last one.
    db.load_font_data(SANS_FONT.to_vec());
    let sans = db.faces().last().map(|f| (f.id, family_name(&f.families)));
    db.load_font_data(MONO_FONT.to_vec());
    let mono = db.faces().last().map(|f| (f.id, family_name(&f.families)));

    // Point the generic-family aliases at the right face, so the default
    // per-glyph fallback resolver picks sanely too.
    if let Some((_, name)) = &sans {
        db.set_serif_family(name);
        db.set_sans_serif_family(name);
        db.set_cursive_family(name);
        db.set_fantasy_family(name);
    }
    if let Some((_, name)) = &mono {
        db.set_monospace_family(name);
    }

    if let Some((sans_id, _)) = sans {
        let mono_id = mono.map(|(id, _)| id).unwrap_or(sans_id);
        opt.font_resolver = usvg::FontResolver {
            select_font: Box::new(move |font, _| {
                let mono = font.families().iter().any(is_monospace_family);
                Some(if mono { mono_id } else { sans_id })
            }),
            select_fallback: usvg::FontResolver::default().select_fallback,
        };
    }
    opt
}

/// The primary family name of a fontdb face (empty string if it somehow has
/// none — only used to alias generic families, never to fail a render).
fn family_name(families: &[(String, usvg::fontdb::Language)]) -> String {
    families
        .first()
        .map(|(name, _)| name.clone())
        .unwrap_or_default()
}

/// Whether a requested family should resolve to the monospace face: the generic
/// `monospace` keyword, or a named family that reads like a fixed-width font
/// (the usual suspects in text-interface snapshots — Courier, Consolas, Menlo,
/// Source Code Pro, …).
fn is_monospace_family(family: &usvg::FontFamily) -> bool {
    match family {
        usvg::FontFamily::Monospace => true,
        usvg::FontFamily::Named(name) => {
            let name = name.to_ascii_lowercase();
            [
                "mono", "courier", "consol", "menlo", "monaco", "code", "terminal", "fixed",
            ]
            .iter()
            .any(|kw| name.contains(kw))
        }
        _ => false,
    }
}

/// The tree's intrinsic pixel size, or `None` when it's degenerate.
fn dimensions(tree: &usvg::Tree) -> Option<(f32, f32)> {
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// Render `tree` at a uniform `scale` into an RGBA image of the scaled size.
fn render_scaled(tree: &usvg::Tree, svg_w: f32, svg_h: f32, scale: f32) -> Option<RgbaImage> {
    let out_w = (svg_w * scale).round().max(1.0) as u32;
    let out_h = (svg_h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(out_w, out_h)?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny_skia stores premultiplied RGBA; `image` (and our compositor) expect
    // straight alpha, so undo the premultiplication.
    let rgba = demultiply(pixmap.data());
    RgbaImage::from_raw(out_w, out_h, rgba)
}

/// Convert premultiplied RGBA (tiny_skia's storage) to straight alpha.
fn demultiply(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        let (ur, ug, ub) = match a {
            0 => (0, 0, 0),
            255 => (r, g, b),
            _ => {
                let a32 = a as u32;
                let half = a32 / 2;
                (
                    ((r as u32 * 255 + half) / a32).min(255) as u8,
                    ((g as u32 * 255 + half) / a32).min(255) as u8,
                    ((b as u32 * 255 + half) / a32).min(255) as u8,
                )
            }
        };
        out.extend_from_slice(&[ur, ug, ub, a]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT_SVG: &[u8] =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;

    #[test]
    fn sniffs_svg_and_rejects_others() {
        assert!(looks_like_svg(RECT_SVG));
        assert!(looks_like_svg(
            br#"<?xml version="1.0"?>
            <!-- a comment --> <svg xmlns="http://www.w3.org/2000/svg"/>"#
        ));
        assert!(looks_like_svg(&[0x1f, 0x8b, 0x08, 0x00])); // gzip → maybe .svgz
        assert!(!looks_like_svg(b"\x89PNG\r\n\x1a\n")); // PNG magic
        assert!(!looks_like_svg(b"just some text"));
    }

    #[test]
    fn rasterizes_at_intrinsic_size_with_expected_pixels() {
        let img = rasterize(RECT_SVG).expect("rect svg rasterizes");
        assert_eq!(img.dimensions(), (40, 20));
        // The whole canvas is the red rect, fully opaque.
        let px = img.get_pixel(20, 10).0;
        assert_eq!(px, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn caps_a_huge_viewbox_at_max_dim() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="5000"><rect width="10000" height="5000" fill="#00ff00"/></svg>"##;
        let img = rasterize(svg).expect("rasterizes");
        // Longest edge clamped to MAX_DIM, aspect ratio preserved (2:1).
        assert_eq!(img.width(), MAX_DIM as u32);
        assert_eq!(img.height(), MAX_DIM as u32 / 2);
    }

    #[test]
    fn fit_scales_up_to_fill_the_box_without_skew() {
        // A 40×20 SVG asked to fill a 400×400 box: it scales up by the limiting
        // (width) factor of 10 — one uniform factor, so the 2:1 aspect holds and
        // it isn't stretched to the square box.
        let img = rasterize_fit(RECT_SVG, 400, 400).expect("rasterizes");
        assert_eq!(img.dimensions(), (400, 200));
        assert_eq!(img.get_pixel(200, 100).0, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn fit_scales_down_to_fit_the_box() {
        let big = br##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="400"><rect width="800" height="400" fill="#00ff00"/></svg>"##;
        let img = rasterize_fit(big, 200, 200).expect("rasterizes");
        // Limited by width (200/800 = 0.25), aspect preserved.
        assert_eq!(img.dimensions(), (200, 100));
    }

    #[test]
    fn renders_text_via_the_bundled_font() {
        // No system fonts are consulted, yet the glyphs must paint *something*:
        // the bundled face is what makes this deterministic.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">
            <text x="2" y="28" font-size="28" fill="#000000">Hi</text></svg>"##;
        let img = rasterize(svg).expect("text svg rasterizes");
        let inked = img.pixels().filter(|p| p.0[3] > 0).count();
        assert!(inked > 0, "bundled font drew no glyphs");
    }

    #[test]
    fn monospace_request_selects_a_different_face_than_sans() {
        let svg = |family: &str| {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"><text x="2" y="28" font-size="24" font-family="{family}" fill="#000000">Illimi</text></svg>"##
            )
            .into_bytes()
        };
        let sans = rasterize(&svg("sans-serif")).expect("sans rasterizes");
        let mono = rasterize(&svg("monospace")).expect("mono rasterizes");
        assert_eq!(sans.dimensions(), mono.dimensions());
        // Proportional vs fixed-width lay "Illimi" out differently, so the two
        // rasters can't be identical — proof the monospace face was chosen
        // rather than everything collapsing onto the one proportional face.
        assert_ne!(
            sans.as_raw(),
            mono.as_raw(),
            "monospace request rendered with the proportional face"
        );
    }

    #[test]
    fn recognizes_monospace_families() {
        use usvg::FontFamily::{Monospace, Named, SansSerif};
        assert!(is_monospace_family(&Monospace));
        assert!(is_monospace_family(&Named("Courier New".into())));
        assert!(is_monospace_family(&Named("Consolas".into())));
        assert!(is_monospace_family(&Named("Source Code Pro".into())));
        assert!(!is_monospace_family(&SansSerif));
        assert!(!is_monospace_family(&Named("Helvetica".into())));
    }

    #[test]
    fn renders_box_drawing_and_block_glyphs() {
        // The point of a monospace face with box-drawing coverage: these glyphs
        // must actually paint (not fall through to a blank/tofu), so text-
        // interface snapshots render. A normal string literal because the glyphs
        // are multi-byte UTF-8 (byte-string literals are ASCII-only).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="40"><text x="2" y="28" font-family="monospace" font-size="24" fill="#000000">┌─┐│└┘█░</text></svg>"##;
        let img = rasterize(svg.as_bytes()).expect("box-drawing svg rasterizes");
        let inked = img.pixels().filter(|p| p.0[3] > 0).count();
        assert!(inked > 0, "box-drawing/block glyphs drew nothing");
    }

    #[test]
    fn non_svg_bytes_do_not_rasterize() {
        assert!(rasterize(b"\x89PNG\r\n\x1a\n not an svg").is_none());
    }
}
