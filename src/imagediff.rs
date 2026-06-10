//! Graphical diffing of image blobs.
//!
//! When a changed file is an image, a unified text diff is useless ("Binary
//! files differ"). Instead journey decodes the two sides and shows them
//! visually. This module is the pure, toolkit-independent half: it decodes the
//! [`BlobPair`](crate::backend::BlobPair) the backend hands over and composes
//! the two images into a single ARGB [`Canvas`] for one of several comparison
//! modes — the same set the author's `imgap` CLI offers interactively:
//!
//! * **2-up** — the before/after images side by side.
//! * **Swipe** — a single frame split left-from-`old`, right-from-`new`, the
//!   split following a 0..1 slider.
//! * **Onion skin** — the two cross-faded by the slider.
//! * **Difference** — a heatmap of per-pixel difference (black = identical,
//!   through blue/green/yellow to red = maximal; magenta where the images
//!   differ in size, so don't overlap).
//! * **Left** / **Right** — just one side, full size.
//!
//! [`ImageComparison`] owns the decoded images, a cache of the fit-scaled
//! buffers, and the last fully-composed [`Canvas`] keyed by what produced it —
//! so [`ImageComparison::render`] is cheap to call every frame: an unchanged
//! repaint (a caret blink, a scroll in another pane) returns the cached canvas
//! without re-scaling or re-compositing. The widget that drives it lives in
//! [`crate::widgets::ImageDiffView`].

use image::{Rgba, RgbaImage};

use crate::backend::BlobPair;

/// Edge of one transparency-checker square, in canvas pixels.
const CHECK_SIZE: u32 = 8;
const CHECK_LIGHT: [u8; 3] = [0xFF, 0xFF, 0xFF];
const CHECK_DARK: [u8; 3] = [0xCC, 0xCC, 0xCC];
/// Gap (and its color) between the two images in 2-up mode.
const TWO_UP_SEP: u32 = 4;
const SEP_COLOR: Rgba<u8> = Rgba([0x80, 0x80, 0x80, 0xFF]);
/// The 1px divider drawn at the swipe split.
const SWIPE_DIVIDER: Rgba<u8> = Rgba([0xFF, 0xDC, 0x00, 0xFF]);

/// File extensions journey treats as raster images for the graphical diff.
/// SVG is deliberately excluded — it's text, so its normal diff is meaningful.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tiff", "tif", "ico", "qoi", "tga", "pnm", "ppm",
    "pgm", "pbm",
];

/// Whether `path` looks like a raster image worth showing graphically.
pub fn is_image_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .filter(|_| path.contains('.'))
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// One of the ways the two images can be compared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareMode {
    TwoUp,
    Swipe,
    Onion,
    Difference,
    Left,
    Right,
}

impl CompareMode {
    /// The comparison modes offered in the control bar, left to right. The
    /// single-image views (`Left`/`Right`) come last.
    pub const ALL: [CompareMode; 6] = [
        CompareMode::TwoUp,
        CompareMode::Swipe,
        CompareMode::Onion,
        CompareMode::Difference,
        CompareMode::Left,
        CompareMode::Right,
    ];

    /// Short label for the mode button.
    pub fn label(self) -> &'static str {
        match self {
            CompareMode::TwoUp => "2-Up",
            CompareMode::Swipe => "Swipe",
            CompareMode::Onion => "Onion",
            CompareMode::Difference => "Diff",
            CompareMode::Left => "Left",
            CompareMode::Right => "Right",
        }
    }

    /// Cycle to the next comparison mode (the View ▸ Switch Mode action). The
    /// single-image views are skipped — they cycle back to 2-Up.
    pub fn next(self) -> Self {
        match self {
            CompareMode::TwoUp => CompareMode::Swipe,
            CompareMode::Swipe => CompareMode::Onion,
            CompareMode::Onion => CompareMode::Difference,
            CompareMode::Difference => CompareMode::TwoUp,
            CompareMode::Left | CompareMode::Right => CompareMode::TwoUp,
        }
    }

    /// Whether this mode is steered by the 0..1 slider.
    pub fn uses_slider(self) -> bool {
        matches!(self, CompareMode::Swipe | CompareMode::Onion)
    }

    /// Whether this mode shows just one side.
    pub fn is_single(self) -> bool {
        matches!(self, CompareMode::Left | CompareMode::Right)
    }
}

/// A composed, fully-opaque ARGB image ready to blit, sized to the actual
/// content (≤ the box passed to [`ImageComparison::render`], which centers it).
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    /// `0xAARRGGBB` per pixel, row-major, alpha always `0xFF`.
    pub argb: Vec<u32>,
}

impl Canvas {
    fn empty() -> Self {
        Canvas {
            w: 0,
            h: 0,
            argb: Vec::new(),
        }
    }
}

/// How finely the slider position is quantized for the [`RenderCache`] key.
/// 1000 steps is finer than a pixel on any real pane, so the cache only misses
/// when the composited image would actually change.
const SLIDER_STEPS: u32 = 1000;

/// The last fully-composed [`Canvas`] and the key it was built for. A repaint
/// that changes none of `(mode, slider, box)` re-blits these pixels instead of
/// re-running the scale + composite + flatten pipeline — which matters because
/// the toolkit repaints the whole tree on any event (a caret blink, a scroll
/// elsewhere), each of which would otherwise recompose the image from scratch.
struct RenderCache {
    mode: CompareMode,
    /// Slider position quantized to [`SLIDER_STEPS`]; held at 0 for modes that
    /// ignore the slider, so their cache survives an unrelated slider change.
    slider_key: u32,
    box_w: u32,
    box_h: u32,
    canvas: Canvas,
}

/// Fit-scaled buffers cached for a given render box, so the slider modes don't
/// re-scale the originals every frame.
struct FitCache {
    box_w: u32,
    box_h: u32,
    /// Both sides laid out on a common canvas and scaled by one shared factor,
    /// so identical coordinates line up for swipe / onion.
    norm_old: RgbaImage,
    norm_new: RgbaImage,
    /// The difference heatmap, scaled to the same box.
    diff: RgbaImage,
}

/// The decoded two sides of an image change, plus a metadata summary line.
pub struct ImageComparison {
    old: Option<RgbaImage>,
    new: Option<RgbaImage>,
    meta: String,
    cache: Option<FitCache>,
    render_cache: Option<RenderCache>,
}

impl ImageComparison {
    /// Decode both sides of `blobs`. Returns `None` when neither side decodes to
    /// an image (so the caller falls back to the text diff).
    pub fn from_blobs(blobs: &BlobPair) -> Option<Self> {
        let old = blobs.old.as_deref().and_then(decode);
        let new = blobs.new.as_deref().and_then(decode);
        if old.is_none() && new.is_none() {
            return None;
        }
        let meta = meta_line(blobs, old.as_ref(), new.as_ref());
        Some(ImageComparison {
            old,
            new,
            meta,
            cache: None,
            render_cache: None,
        })
    }

    /// The `PNG 64x64 1.2KiB→1.4KiB`-style summary shown above the image.
    pub fn meta(&self) -> &str {
        &self.meta
    }

    /// The opaque canvas for `mode` at slider position `slider` (0..1), fitting
    /// within `box_w` × `box_h`. Returns a cached canvas when nothing that
    /// affects the pixels has changed since the last call, so repaints driven by
    /// unrelated UI activity don't recompose the image (see [`RenderCache`]).
    pub fn render(&mut self, mode: CompareMode, slider: f32, box_w: u32, box_h: u32) -> &Canvas {
        let slider_key = if mode.uses_slider() {
            (slider.clamp(0.0, 1.0) * SLIDER_STEPS as f32).round() as u32
        } else {
            0
        };
        let hit = self.render_cache.as_ref().is_some_and(|c| {
            c.mode == mode && c.slider_key == slider_key && c.box_w == box_w && c.box_h == box_h
        });
        if !hit {
            let canvas = self.compose(mode, slider, box_w, box_h);
            self.render_cache = Some(RenderCache {
                mode,
                slider_key,
                box_w,
                box_h,
                canvas,
            });
        }
        &self
            .render_cache
            .as_ref()
            .expect("cache just populated")
            .canvas
    }

    /// Run the scale + composite + flatten pipeline for `mode`, with no caching.
    fn compose(&mut self, mode: CompareMode, slider: f32, box_w: u32, box_h: u32) -> Canvas {
        if box_w == 0 || box_h == 0 {
            return Canvas::empty();
        }
        let composed = match mode {
            CompareMode::TwoUp => self.two_up(box_w, box_h),
            CompareMode::Left => single(self.old.as_ref(), box_w, box_h),
            CompareMode::Right => single(self.new.as_ref(), box_w, box_h),
            CompareMode::Swipe | CompareMode::Onion | CompareMode::Difference => {
                self.ensure_cache(box_w, box_h);
                let c = self.cache.as_ref().expect("cache just built");
                match mode {
                    CompareMode::Swipe => compose_swipe(&c.norm_old, &c.norm_new, slider),
                    CompareMode::Onion => compose_onion(&c.norm_old, &c.norm_new, slider),
                    CompareMode::Difference => c.diff.clone(),
                    _ => unreachable!(),
                }
            }
        };
        flatten_to_canvas(&composed)
    }

    /// The before/after images side by side, each in half the width.
    fn two_up(&self, box_w: u32, box_h: u32) -> RgbaImage {
        let half = box_w.saturating_sub(TWO_UP_SEP) / 2;
        let left = single(self.old.as_ref(), half.max(1), box_h);
        let right = single(self.new.as_ref(), half.max(1), box_h);
        let h = left.height().max(right.height());
        let w = left.width() + TWO_UP_SEP + right.width();
        let mut canvas = RgbaImage::new(w, h);
        // Vertical separator bar between the two panels.
        for y in 0..h {
            for x in left.width()..(left.width() + TWO_UP_SEP) {
                canvas.put_pixel(x, y, SEP_COLOR);
            }
        }
        let lw = left.width();
        image::imageops::overlay(&mut canvas, &left, 0, 0);
        image::imageops::overlay(&mut canvas, &right, (lw + TWO_UP_SEP) as i64, 0);
        canvas
    }

    /// Build (or reuse) the fit-scaled buffers for `box_w` × `box_h`.
    fn ensure_cache(&mut self, box_w: u32, box_h: u32) {
        if self
            .cache
            .as_ref()
            .is_some_and(|c| c.box_w == box_w && c.box_h == box_h)
        {
            return;
        }
        let (na, nb) = normalize_pair(self.old.as_ref(), self.new.as_ref());
        let (norm_old, norm_new) = scale_pair(&na, &nb, box_w, box_h);
        let diff = scale_fit(
            &build_diff_heatmap(self.old.as_ref(), self.new.as_ref()),
            box_w,
            box_h,
        );
        self.cache = Some(FitCache {
            box_w,
            box_h,
            norm_old,
            norm_new,
            diff,
        });
    }
}

/// Decode raw image bytes to RGBA, or `None` if the format isn't recognized.
fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|img| img.to_rgba8())
}

/// Scale one image to fit within `max_w` × `max_h`, preserving aspect ratio.
/// Returned unchanged when it already fits.
fn scale_fit(img: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || (w <= max_w && h <= max_h) {
        return img.clone();
    }
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let nw = ((w as f64 * scale) as u32).max(1);
    let nh = ((h as f64 * scale) as u32).max(1);
    image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle)
}

/// A single side scaled to fit the box, or a transparent placeholder (which
/// flattens to the checkerboard) when that side is absent.
fn single(img: Option<&RgbaImage>, box_w: u32, box_h: u32) -> RgbaImage {
    match img {
        Some(img) => scale_fit(img, box_w, box_h),
        None => RgbaImage::new(box_w.max(1), box_h.max(1)),
    }
}

/// Lay both sides on a common canvas (the max of the two sizes) so identical
/// coordinates line up. A missing side becomes a transparent canvas.
fn normalize_pair(a: Option<&RgbaImage>, b: Option<&RgbaImage>) -> (RgbaImage, RgbaImage) {
    let w = a
        .map_or(0, |i| i.width())
        .max(b.map_or(0, |i| i.width()))
        .max(1);
    let h = a
        .map_or(0, |i| i.height())
        .max(b.map_or(0, |i| i.height()))
        .max(1);
    let mut out_a = RgbaImage::new(w, h);
    let mut out_b = RgbaImage::new(w, h);
    if let Some(a) = a {
        image::imageops::overlay(&mut out_a, a, 0, 0);
    }
    if let Some(b) = b {
        image::imageops::overlay(&mut out_b, b, 0, 0);
    }
    (out_a, out_b)
}

/// Scale two equally-sized images by the same factor to fit the box.
fn scale_pair(a: &RgbaImage, b: &RgbaImage, max_w: u32, max_h: u32) -> (RgbaImage, RgbaImage) {
    let (w, h) = (a.width(), a.height());
    if w == 0 || h == 0 || (w <= max_w && h <= max_h) {
        return (a.clone(), b.clone());
    }
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let nw = ((w as f64 * scale) as u32).max(1);
    let nh = ((h as f64 * scale) as u32).max(1);
    let f = image::imageops::FilterType::Triangle;
    (
        image::imageops::resize(a, nw, nh, f),
        image::imageops::resize(b, nw, nh, f),
    )
}

/// A vertical-split composite: left of `t` comes from `a`, right from `b`, with
/// a 1px divider at the split. `a` and `b` must share dimensions.
fn compose_swipe(a: &RgbaImage, b: &RgbaImage, t: f32) -> RgbaImage {
    let (w, h) = a.dimensions();
    let split = ((w as f32) * t.clamp(0.0, 1.0)) as u32;
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let px = if x < split { a } else { b }.get_pixel(x, y);
            out.put_pixel(x, y, *px);
        }
        if split < w {
            out.put_pixel(split, y, SWIPE_DIVIDER);
        }
    }
    out
}

/// A cross-fade: `a*(1-t) + b*t` per channel. `a` and `b` share dimensions.
fn compose_onion(a: &RgbaImage, b: &RgbaImage, t: f32) -> RgbaImage {
    let t = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let inv = 255 - t;
    let (w, h) = a.dimensions();
    let ra = a.as_raw();
    let rb = b.as_raw();
    let mut out = vec![0u8; ra.len()];
    for i in 0..ra.len() {
        out[i] = ((ra[i] as u32 * inv + rb[i] as u32 * t + 127) / 255) as u8;
    }
    RgbaImage::from_raw(w, h, out).expect("onion buffer matches dimensions")
}

/// A per-pixel difference heatmap at native resolution. Overlapping pixels are
/// colored by luminance-weighted difference (black→blue→green→yellow→red);
/// regions only one image covers stay magenta.
fn build_diff_heatmap(a: Option<&RgbaImage>, b: Option<&RgbaImage>) -> RgbaImage {
    let aw = a.map_or(0, |i| i.width());
    let ah = a.map_or(0, |i| i.height());
    let bw = b.map_or(0, |i| i.width());
    let bh = b.map_or(0, |i| i.height());
    let w = aw.max(bw).max(1);
    let h = ah.max(bh).max(1);
    let mut out = RgbaImage::from_pixel(w, h, Rgba([0xFF, 0x00, 0xFF, 0xFF]));

    // Only the overlapping rectangle gets a real difference value.
    let (Some(a), Some(b)) = (a, b) else {
        return out;
    };
    let common_w = aw.min(bw);
    let common_h = ah.min(bh);
    for y in 0..common_h {
        for x in 0..common_w {
            let pa = a.get_pixel(x, y).0;
            let pb = b.get_pixel(x, y).0;
            let dr = (pa[0] as i16 - pb[0] as i16).unsigned_abs() as u32;
            let dg = (pa[1] as i16 - pb[1] as i16).unsigned_abs() as u32;
            let db = (pa[2] as i16 - pb[2] as i16).unsigned_abs() as u32;
            let diff = (77 * dr + 151 * dg + 28 * db) as f32 / (255.0 * 256.0);
            out.put_pixel(x, y, heatmap_color(diff));
        }
    }
    out
}

/// Map a 0..1 difference magnitude to the black→blue→green→yellow→red ramp.
fn heatmap_color(t: f32) -> Rgba<u8> {
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.25 {
        (0.0, 0.0, t / 0.25)
    } else if t < 0.5 {
        let s = (t - 0.25) / 0.25;
        (0.0, s, 1.0 - s)
    } else if t < 0.75 {
        let s = (t - 0.5) / 0.25;
        (s, 1.0, 0.0)
    } else {
        let s = (t - 0.75) / 0.25;
        (1.0, 1.0 - s, 0.0)
    };
    Rgba([(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255])
}

/// Composite an RGBA image over the transparency checkerboard, yielding opaque
/// `0xAARRGGBB` pixels ready to blit.
fn flatten_to_canvas(img: &RgbaImage) -> Canvas {
    let (w, h) = img.dimensions();
    let mut argb = vec![0u32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let px = img.get_pixel(x, y).0;
            let a = px[3] as u32;
            let bg = checker(x, y);
            let blend = |s: u8, d: u8| (s as u32 * a + d as u32 * (255 - a)) / 255;
            let r = blend(px[0], bg[0]);
            let g = blend(px[1], bg[1]);
            let b = blend(px[2], bg[2]);
            argb[(y * w + x) as usize] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
    Canvas { w, h, argb }
}

/// The transparency-checker color at an absolute canvas pixel.
fn checker(x: u32, y: u32) -> [u8; 3] {
    if ((x / CHECK_SIZE) + (y / CHECK_SIZE)).is_multiple_of(2) {
        CHECK_LIGHT
    } else {
        CHECK_DARK
    }
}

/// Build the metadata summary: `FORMAT WxH SIZE`, with each property collapsed
/// to one value when both sides match and rendered `old→new` when they differ.
fn meta_line(blobs: &BlobPair, old: Option<&RgbaImage>, new: Option<&RgbaImage>) -> String {
    let fmt = pair_str(
        blobs.old.as_deref().and_then(format_name),
        blobs.new.as_deref().and_then(format_name),
    );
    let dims = pair_str(
        old.map(|i| format!("{}x{}", i.width(), i.height())),
        new.map(|i| format!("{}x{}", i.width(), i.height())),
    );
    let size = pair_str(
        blobs.old.as_ref().map(|b| human_size(b.len())),
        blobs.new.as_ref().map(|b| human_size(b.len())),
    );
    [fmt, dims, size]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ")
}

/// Collapse a per-side value into a display string: one value when the sides
/// match, `a→b` when they differ, and just the present side when one is absent.
fn pair_str(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), Some(b)) => Some(format!("{a}→{b}")),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// The image format name guessed from a blob's magic bytes.
fn format_name(bytes: &[u8]) -> Option<String> {
    use image::ImageFormat as F;
    let name = match image::guess_format(bytes).ok()? {
        F::Png => "PNG",
        F::Jpeg => "JPEG",
        F::Gif => "GIF",
        F::WebP => "WebP",
        F::Bmp => "BMP",
        F::Tiff => "TIFF",
        F::Ico => "ICO",
        F::Pnm => "PNM",
        F::Tga => "TGA",
        F::Qoi => "QOI",
        _ => return None,
    };
    Some(name.to_string())
}

/// Human-readable byte size, e.g. `1.4KiB`.
fn human_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.1}MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1}KiB", b / KIB)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a solid-color `w`×`h` PNG for tests.
    fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba(color));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn recognizes_image_extensions() {
        assert!(is_image_path("a/b/logo.png"));
        assert!(is_image_path("ICON.PNG"));
        assert!(is_image_path("photo.jpeg"));
        assert!(!is_image_path("src/main.rs"));
        assert!(!is_image_path("drawing.svg")); // SVG stays a text diff
        assert!(!is_image_path("Makefile"));
    }

    #[test]
    fn from_blobs_none_when_undecodable() {
        let blobs = BlobPair {
            old: Some(b"not an image".to_vec()),
            new: Some(b"still not".to_vec()),
        };
        assert!(ImageComparison::from_blobs(&blobs).is_none());
    }

    #[test]
    fn renders_every_mode_to_opaque_canvas() {
        let blobs = BlobPair {
            old: Some(png(8, 8, [255, 0, 0, 255])),
            new: Some(png(8, 8, [0, 0, 255, 255])),
        };
        let mut cmp = ImageComparison::from_blobs(&blobs).expect("decodes");
        for mode in CompareMode::ALL {
            let canvas = cmp.render(mode, 0.5, 64, 64);
            assert!(canvas.w > 0 && canvas.h > 0, "{mode:?} produced no canvas");
            assert_eq!(canvas.argb.len(), (canvas.w * canvas.h) as usize);
            // Every pixel must be fully opaque so the blit needs no alpha math.
            assert!(
                canvas.argb.iter().all(|p| p >> 24 == 0xFF),
                "{mode:?} left transparent pixels"
            );
        }
    }

    #[test]
    fn difference_of_identical_images_is_black() {
        let bytes = png(8, 8, [10, 200, 30, 255]);
        let blobs = BlobPair {
            old: Some(bytes.clone()),
            new: Some(bytes),
        };
        let mut cmp = ImageComparison::from_blobs(&blobs).expect("decodes");
        let canvas = cmp.render(CompareMode::Difference, 0.0, 32, 32);
        // Identical inputs → zero difference → black (0xFF000000) everywhere.
        assert!(canvas.argb.iter().all(|&p| p == 0xFF00_0000));
    }

    #[test]
    fn render_reuses_the_cached_canvas_until_the_key_changes() {
        let blobs = BlobPair {
            old: Some(png(8, 8, [255, 0, 0, 255])),
            new: Some(png(8, 8, [0, 0, 255, 255])),
        };
        let mut cmp = ImageComparison::from_blobs(&blobs).expect("decodes");
        // The buffer address identifies the cached canvas: a cache hit returns
        // the same stored `Canvas`, a miss composes a fresh one (a new alloc,
        // built while the old is still live, so the pointers always differ).
        let ptr = |c: &mut ImageComparison, m, s, w, h| c.render(m, s, w, h).argb.as_ptr();

        let a = ptr(&mut cmp, CompareMode::TwoUp, 0.5, 64, 64);
        assert_eq!(
            a,
            ptr(&mut cmp, CompareMode::TwoUp, 0.5, 64, 64),
            "same key reuses the cached canvas"
        );
        assert_eq!(
            a,
            ptr(&mut cmp, CompareMode::TwoUp, 0.9, 64, 64),
            "the slider is irrelevant in 2-up, so the cache stays warm"
        );
        let b = ptr(&mut cmp, CompareMode::TwoUp, 0.5, 80, 64);
        assert_ne!(a, b, "a different box size recomposes");
        let c = ptr(&mut cmp, CompareMode::Swipe, 0.5, 80, 64);
        assert_ne!(b, c, "a different mode recomposes");
        assert_eq!(
            c,
            ptr(&mut cmp, CompareMode::Swipe, 0.5, 80, 64),
            "a slider mode caches at a fixed slider position"
        );
        assert_ne!(
            c,
            ptr(&mut cmp, CompareMode::Swipe, 0.95, 80, 64),
            "moving the slider in a slider mode recomposes"
        );
    }

    #[test]
    fn added_image_has_one_side_and_collapsed_meta() {
        let blobs = BlobPair {
            old: None,
            new: Some(png(16, 16, [0, 0, 0, 255])),
        };
        let mut cmp = ImageComparison::from_blobs(&blobs).expect("decodes");
        assert!(cmp.meta().contains("16x16"));
        // Left (the missing old side) renders as the checkerboard only.
        let left = cmp.render(CompareMode::Left, 0.0, 32, 32);
        assert!(left.w > 0 && left.h > 0);
    }
}
