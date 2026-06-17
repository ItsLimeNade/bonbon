use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

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
