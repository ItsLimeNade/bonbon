use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

/// One rasterized glyph: its anti-aliasing coverage and where it sits
/// relative to the text origin.
struct GlyphMask {
    dx: i32,
    dy: i32,
    w: usize,
    h: usize,
    coverage: Vec<f32>,
}

/// Lays out and rasterizes `text` into coverage masks, placing every glyph
/// exactly where `imageproc::drawing::draw_text_mut` does (including its
/// quirk of applying kerning after a glyph is positioned), so drawing the
/// masks is pixel-identical to calling it.
fn rasterize(font: &FontRef, scale: PxScale, text: &str) -> Vec<GlyphMask> {
    let scaled = font.as_scaled(scale);
    let mut glyphs = Vec::with_capacity(text.len());
    let mut w = 0.0;
    let mut prev = None;

    for c in text.chars() {
        let id = scaled.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(w, scaled.ascent()));
        w += scaled.h_advance(id);
        let Some(g) = scaled.outline_glyph(glyph) else {
            continue;
        };
        if let Some(prev) = prev {
            w += scaled.kern(id, prev);
        }
        prev = Some(id);

        let bb = g.px_bounds();
        let (gw, gh) = (bb.width() as usize, bb.height() as usize);
        let mut coverage = vec![0.0; gw * gh];
        g.draw(|x, y, v| coverage[y as usize * gw + x as usize] = v.clamp(0.0, 1.0));
        glyphs.push(GlyphMask {
            dx: bb.min.x.round() as i32,
            dy: bb.min.y.round() as i32,
            w: gw,
            h: gh,
            coverage,
        });
    }
    glyphs
}

/// Blends `glyphs` onto `img` with the text origin at (`x`, `y`), using the
/// same per-channel `dst * (1 - v) + color * v` blend as imageproc. Masks are
/// clipped to the canvas once per glyph.
///
/// Pixels go 4 at a time: fully empty groups (much of a large glyph's box)
/// are skipped, and the rest are blended with SIMD, empty pixels included
/// since at `v = 0` the blend is exactly `dst * 1 + color * 0 = dst`.
fn blend_glyphs(img: &mut RgbaImage, glyphs: &[GlyphMask], x: i32, y: i32, color: Rgba<u8>) {
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let c = color.0.map(|v| v as f32);
    let c16: [f32; 16] = std::array::from_fn(|i| c[i % 4]);
    let raw = img.as_mut();

    for g in glyphs {
        let (ox, oy) = (x + g.dx, y + g.dy);
        let (gx0, gx1) = ((-ox).max(0), (g.w as i32).min(img_w - ox));
        let (gy0, gy1) = ((-oy).max(0), (g.h as i32).min(img_h - oy));
        if gx0 >= gx1 {
            continue;
        }

        for gy in gy0..gy1 {
            let cov = &g.coverage[gy as usize * g.w..][gx0 as usize..gx1 as usize];
            let i = ((oy + gy) * img_w + ox + gx0) as usize * 4;
            let dst = &mut raw[i..i + cov.len() * 4];

            let mut dst4 = dst.chunks_exact_mut(16);
            let mut cov4 = cov.chunks_exact(4);
            for (d, v) in (&mut dst4).zip(&mut cov4) {
                if v == [0.0; 4] {
                    continue;
                }
                let v16: [f32; 16] = std::array::from_fn(|i| v[i / 4]);
                for i in 0..16 {
                    d[i] = (d[i] as f32 * (1.0 - v16[i]) + c16[i] * v16[i]) as u8;
                }
            }
            for (d, &v) in dst4
                .into_remainder()
                .chunks_exact_mut(4)
                .zip(cov4.remainder())
            {
                for (d, c) in d.iter_mut().zip(c) {
                    *d = (*d as f32 * (1.0 - v) + c * v) as u8;
                }
            }
        }
    }
}

/// Drop-in for `imageproc::drawing::draw_text_mut` on an `RgbaImage`, with
/// identical output.
pub(crate) fn draw_text(
    img: &mut RgbaImage,
    color: Rgba<u8>,
    x: i32,
    y: i32,
    scale: PxScale,
    font: &FontRef,
    text: &str,
) {
    blend_glyphs(img, &rasterize(font, scale, text), x, y, color);
}

/// Draws `text` with a 1px `outline_color` halo, as 8 offset copies under the
/// text itself. The glyphs are rasterized once and the masks reused for all
/// nine passes instead of re-rasterizing the string every time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_text_with_outline(
    img: &mut RgbaImage,
    text_color: Rgba<u8>,
    outline_color: Rgba<u8>,
    x: i32,
    y: i32,
    scale: PxScale,
    font: &FontRef,
    text: &str,
) {
    let glyphs = rasterize(font, scale, text);
    for ox in -1..=1 {
        for oy in -1..=1 {
            if ox == 0 && oy == 0 {
                continue;
            }
            blend_glyphs(img, &glyphs, x + ox, y + oy, outline_color);
        }
    }
    blend_glyphs(img, &glyphs, x, y, text_color);
}

/// Real rendered width in pixels of `text` at `size`, using the same glyph
/// advances `draw_text_mut` uses, so a box sized with this exactly fits the text.
///
/// This replaces the old `chars * size * 0.6` heuristic, which overestimated the
/// width for the bundled monospace font (whose `height_unscaled` is larger than
/// one em, so ab_glyph's height-scaled advances come out near `size * 0.45`). The
/// inflated estimate left trailing whitespace inside auto-sized boxes such as the
/// bg-card info pill; measuring with the real advances makes the box fit exactly.
pub(crate) fn text_w(font: &FontRef, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars()
        .map(|c| scaled.h_advance(scaled.glyph_id(c)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

    fn font() -> FontRef<'static> {
        FontRef::try_from_slice(FONT).expect("bundled font should parse")
    }

    fn old_approx_w(text: &str, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.6
    }

    #[test]
    fn empty_text_has_zero_width() {
        assert_eq!(text_w(&font(), "", 32.0), 0.0);
    }

    #[test]
    fn width_is_additive_across_chars() {
        let f = font();
        let combined = text_w(&f, "High, keep monitoring", 18.0);
        let parts: f32 = "High, keep monitoring"
            .chars()
            .map(|c| text_w(&f, &c.to_string(), 18.0))
            .sum();
        assert!((combined - parts).abs() < 1e-3, "{combined} vs {parts}");
    }

    #[test]
    fn monospace_advance_is_uniform() {
        let f = font();
        let one = text_w(&f, "M", 30.0);
        let four = text_w(&f, "MMMM", 30.0);
        assert!((four - one * 4.0).abs() < 1e-3, "{four} vs {}", one * 4.0);
    }

    /// The bug: the old heuristic reserved ~25–35% more width per character than
    /// the renderer actually used, so longer labels showed trailing whitespace.
    /// The real measurement must be clearly narrower than the old estimate.
    #[test]
    fn measured_width_is_narrower_than_old_heuristic() {
        let f = font();
        let size = 18.0;
        let label = "High, keep monitoring";

        let measured = text_w(&f, label, size);
        let old = old_approx_w(label, size);

        assert!(
            measured < old,
            "real width {measured} should be below the old estimate {old}"
        );

        let shrink = (old - measured) / old;
        assert!(
            (0.15..0.45).contains(&shrink),
            "expected the box to tighten by 15–45%, got {:.1}%",
            shrink * 100.0
        );
    }
}
