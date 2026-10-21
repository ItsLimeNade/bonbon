//! The BG card: the current reading and the last 3 hours, at a glance.
//!
//! The card is flat: solid fills, no gradients, no outlines. It follows
//! Apple's Human Interface Guidelines for widgets and charts (one idea read
//! at a glance, numbers large with a smaller grey unit, 16 pt margins,
//! continuous corners, axis labels on the trailing side, a headline that
//! says what the chart shows, color kept for meaning) and keeps bonbon's own
//! marks: a panel a shade off the background, the target range as a band,
//! and soft tinted chips.

use std::path::PathBuf;

use ab_glyph::{Font, FontRef, InvalidFont, PxScale, ScaleFont, VariableFont};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};

use crate::charts::time_in_range::TirThresholds;
use crate::models::{UnitDisplay, UnitPreference};
use crate::theme::Theme;
use crate::utils::axis::{fmt_delta, fmt_glucose, unit_name, units};
use crate::utils::color::darken_color;
use crate::utils::drawing::{
    blend_pixel, blend_sprite, create_aa_circle_sprite, draw_dashed_horizontal_line,
};
use crate::utils::text::{draw_text, text_w};

/// Glucose status: colors the reading, the latest dot on the chart and the
/// chart's headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlucoseStatus {
    Low,
    InRange,
    High,
}

/// Visual state for [`InfoPill`]: the color of its tint, icon and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillState {
    /// Neutral: a soft grey tint with the text in the primary color.
    Normal,
    /// High-glucose alert: a tint of `glucose_high`, with the icon and text
    /// in it.
    AlertHigh,
    /// Low-glucose alert: a tint of `glucose_low`, with the icon and text in
    /// it.
    AlertLow,
}

/// Source for the pill icon. Loaded at build time and resized to a square
/// matching the pill's text. Non-square sources are stretched to fit.
///
/// The icon is drawn as a mask: only its transparency is used, and it takes
/// the pill's color, so any glyph-like icon matches the text beside it.
#[derive(Debug, Clone)]
pub enum PillIcon {
    /// Read the icon from a file path. Any format the `image` crate supports.
    Path(PathBuf),
    /// Decode the icon from raw image bytes (PNG, JPEG, ...).
    Bytes(Vec<u8>),
}

impl PillIcon {
    pub fn from_path<P: Into<PathBuf>>(path: P) -> Self {
        Self::Path(path.into())
    }

    pub fn from_bytes<B: Into<Vec<u8>>>(bytes: B) -> Self {
        Self::Bytes(bytes.into())
    }
}

/// A small contextual badge shown just above the reading.
///
/// Composition is icon-on-left, text-on-right, both centered vertically.
/// What you put inside is entirely up to you.
#[derive(Debug, Clone)]
pub struct InfoPill {
    pub icon: PillIcon,
    pub text: String,
    pub state: PillState,
}

/// Built-in icon byte slices (PNG) bundled with the crate for quick use with
/// [`PillIcon::from_bytes`]. All are 50x50 RGBA PNGs.
pub mod builtin_icons {
    pub const FAST_DROP: &[u8] = include_bytes!("../../assets/icons/fast_drop.png");
    pub const FAST_RISE: &[u8] = include_bytes!("../../assets/icons/fast_rise.png");
    pub const FINGERPRICK: &[u8] = include_bytes!("../../assets/icons/fingerprick.png");
    pub const WARNING: &[u8] = include_bytes!("../../assets/icons/warning.png");
    pub const WATER: &[u8] = include_bytes!("../../assets/icons/water.png");
}

/// The icons beside insulin and carbs on board.
const SYRINGE: &[u8] = include_bytes!("../../assets/icons/syringe.png");
const WHEAT: &[u8] = include_bytes!("../../assets/icons/wheat.png");

/// A single reading of the last 3 hours.
#[derive(Debug, Clone)]
pub struct SparklinePoint {
    /// Normalized position in the 3h window. 0.0 = oldest, 1.0 = most recent.
    pub t: f32,
    /// Raw glucose value in mg/dL, used for vertical positioning.
    pub sgv: f32,
    /// Status of this reading, used to tell how long the current status has
    /// lasted.
    pub status: GlucoseStatus,
}

/// All pre-calculated, pre-formatted data for a BgCard.
///
/// Glucose values are in mg/dL whatever the card displays: the unit is set
/// with [`BgCardBuilder::with_units`].
#[derive(Debug)]
pub struct BgCardData {
    /// Current glucose value in mg/dL.
    pub current_sgv: f32,
    /// Status of the current reading: colors the reading and the latest dot
    /// on the chart.
    pub status: GlucoseStatus,
    /// Trend arrow as a single Unicode character. E.g., `"→"`, `"↑"`, `"↗"`, `"↓"`.
    pub trend_arrow: String,
    /// Change since the previous reading in mg/dL, shown signed in the
    /// card's unit. E.g., `Some(3.0)` for `"+3 mg/dL"`. `None` hides the field.
    pub delta: Option<f32>,
    /// Name shown at the top left of the card, before the data age.
    pub watermark_str: String,
    /// Pre-formatted data age. E.g., `"3 min ago"`, `"just now"`.
    pub age_str: String,
    /// Current local time, shown under the end of the chart. E.g., `"15:45"`.
    /// Empty shows `"Now"`.
    pub time_str: String,
    /// Insulin on board, pre-formatted. E.g., `"IOB 2.5u"`: what follows the
    /// first space is shown, beside a syringe. `None` hides the field.
    pub iob_str: Option<String>,
    /// Carbs on board, pre-formatted. E.g., `"COB 30g"`: what follows the
    /// first space is shown, beside a wheat ear. `None` hides the field.
    pub cob_str: Option<String>,
    /// Readings of the last 3 hours, oldest first. The last one is taken as
    /// the current reading.
    pub sparkline_points: Vec<SparklinePoint>,
    /// Optional contextual info pill, shown just above the reading.
    pub info_pill: Option<InfoPill>,
}

/// Builder for the card.
///
/// Produces a fixed **640 × 320 px** RGBA image (or scaled up via `with_scale`).
pub struct BgCardBuilder<'a> {
    data: Option<BgCardData>,
    unit_display: UnitDisplay,
    thresholds: TirThresholds,
    theme: Theme,
    font: &'a [u8],
    number_font: Option<&'a [u8]>,
    scale: f32,
    #[cfg(feature = "beetroot")]
    sticker_set: Option<crate::charts::stickers::StickerSet>,
}

impl<'a> Default for BgCardBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> BgCardBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");
        Self {
            data: None,
            unit_display: UnitDisplay::MgDl,
            thresholds: TirThresholds::default(),
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            number_font: None,
            scale: 1.0,
            #[cfg(feature = "beetroot")]
            sticker_set: None,
        }
    }

    /// Attach a [`StickerSet`](crate::charts::stickers::StickerSet) to the card.
    ///
    /// Stickers are layered behind the text and the chart so both remain on
    /// top. Only available with the `beetroot` feature.
    #[cfg(feature = "beetroot")]
    pub fn with_stickers(mut self, set: crate::charts::stickers::StickerSet) -> Self {
        self.sticker_set = Some(set);
        self
    }

    pub fn with_data(mut self, data: BgCardData) -> Self {
        self.data = Some(data);
        self
    }

    /// Unit the reading, its change and the chart's targets are shown in.
    /// `Dual` repeats the reading and its change in the other unit, smaller,
    /// under the first. Default: mg/dL.
    pub fn with_units(mut self, unit: UnitDisplay) -> Self {
        self.unit_display = unit;
        self
    }

    /// In-range boundaries in mg/dL: the band on the chart, and what colors
    /// the readings on it. Default: 70–180.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.thresholds.low = low;
        self.thresholds.high = high;
        self
    }

    /// Very-low / very-high boundaries in mg/dL, drawn on the chart once a
    /// reading has crossed them. Default: 54 / 250.
    pub fn with_extreme_targets(mut self, very_low: f32, very_high: f32) -> Self {
        self.thresholds.very_low = very_low;
        self.thresholds.very_high = very_high;
        self
    }

    /// All four boundaries at once.
    pub fn with_thresholds(mut self, thresholds: TirThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// The font all text is set in, and the numbers too unless
    /// [`with_number_font`](Self::with_number_font) gives them their own.
    /// A variable font is set in the weights the card asks for (a weight
    /// axis and, if it has one, an optical size axis); a static font is
    /// used as it is.
    pub fn with_font(mut self, font: &'a [u8]) -> Self {
        self.font = font;
        self
    }

    /// The font glucose values and amounts are set in, such as a rounded
    /// companion to the text font. Default: the text font.
    pub fn with_number_font(mut self, font: &'a [u8]) -> Self {
        self.number_font = Some(font);
        self
    }

    /// Multiplies all pixel dimensions by `scale`. Use `4.0` for a 2560×1280 output.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Renders and returns the card image at `640*scale × 320*scale` pixels.
    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let data = self
            .data
            .ok_or("BgCardData is required - call with_data() first")?;
        let fonts = Fonts::new(self.font, self.number_font.unwrap_or(self.font))?;
        let (pref, second) = units(self.unit_display);
        Card {
            data: &data,
            theme: &self.theme,
            fonts,
            s: self.scale,
            pref,
            second,
            targets: self.thresholds,
            #[cfg(feature = "beetroot")]
            stickers: self.sticker_set.as_ref(),
        }
        .draw()
    }
}

/// One font at one weight, with where its cap line and baseline sit under
/// the top of a line of text, in fractions of the size.
struct Face<'a> {
    font: FontRef<'a>,
    cap: f32,
    base: f32,
}

impl<'a> Face<'a> {
    fn new(bytes: &'a [u8], weight: f32, optical_size: f32) -> Result<Self, InvalidFont> {
        let mut font = FontRef::try_from_slice(bytes)?;
        font.set_variation(b"wght", weight);
        font.set_variation(b"opsz", optical_size);
        let scaled = font.as_scaled(PxScale::from(100.0));
        let base = scaled.ascent() / 100.0;
        let id = scaled.glyph_id('H');
        let glyph = id.with_scale_and_position(100.0, ab_glyph::point(0.0, scaled.ascent()));
        let cap = font
            .outline_glyph(glyph)
            .map_or(base * 0.3, |g| g.px_bounds().min.y / 100.0);
        Ok(Self { font, cap, base })
    }

    /// The height of a capital letter at `size`.
    fn cap_height(&self, size: f32) -> f32 {
        (self.base - self.cap) * size
    }
}

/// The faces the card is set in.
struct Fonts<'a> {
    /// Body text and secondary lines.
    text: Face<'a>,
    /// Target labels.
    label: Face<'a>,
    /// Name, headline and pill.
    title: Face<'a>,
    /// The trend arrow.
    symbol: Face<'a>,
    /// Glucose values and amounts.
    number: Face<'a>,
    /// The reading in the second unit.
    number_medium: Face<'a>,
}

impl<'a> Fonts<'a> {
    fn new(text: &'a [u8], numbers: &'a [u8]) -> Result<Self, InvalidFont> {
        Ok(Self {
            text: Face::new(text, 400.0, 17.0)?,
            label: Face::new(text, 510.0, 17.0)?,
            title: Face::new(text, 590.0, 17.0)?,
            symbol: Face::new(text, 600.0, 40.0)?,
            number: Face::new(numbers, 600.0, 40.0)?,
            number_medium: Face::new(numbers, 500.0, 40.0)?,
        })
    }
}

/// A run of text: face, text, size and color.
type Run<'f, 't> = (&'f Face<'f>, &'t str, f32, Rgba<u8>);

/// A rectangle in design units: `(x, y, w, h)`.
type Rect = (f32, f32, f32, f32);

/// The chart's area, in design units (before scaling).
struct Plot {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

/// The card's size in design units.
const W: f32 = 640.0;
const H: f32 = 320.0;
/// The outer margin, Apple's standard for widgets.
const MARGIN: f32 = 16.0;
/// Corner radius of the chart panel.
const PANEL_RADIUS: f32 = 22.0;
/// Height of a chip and of the info pill.
const CHIP_H: f32 = 30.0;
/// How long the chart covers.
const WINDOW_MINUTES: f32 = 180.0;

fn with_alpha(c: Rgba<u8>, a: u8) -> Rgba<u8> {
    Rgba([c[0], c[1], c[2], a])
}

/// Distance from (`x`, `y`) to the edge of a rectangle centered on `center`
/// with half-size `half` and continuous corners of `radius`: negative
/// inside.
///
/// The corners are superellipse quarters rather than circle quarters, the
/// smoother curve Apple draws its cards with: a circular corner meets the
/// straight edge with a visible kink in curvature, a superellipse eases into
/// it. They are drawn larger than `radius` so they look about as round as a
/// circular one of that radius.
fn squircle_distance(x: f32, y: f32, center: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    const N: f32 = 4.0;
    let r = (radius * 1.45).min(half.0).min(half.1);
    let qx = (x - center.0).abs() - (half.0 - r);
    let qy = (y - center.1).abs() - (half.1 - r);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    // Only the corners need the superellipse; along the straight edges the
    // distance is one coordinate.
    let outside = if oy == 0.0 {
        ox
    } else if ox == 0.0 {
        oy
    } else {
        (ox.powf(N) + oy.powf(N)).powf(1.0 / N)
    };
    outside + qx.max(qy).min(0.0) - r
}

/// "45 min", "1 hr 5 min", "2 hr", rounded to 5 minutes.
fn fmt_minutes(minutes: f32) -> String {
    let m = (minutes / 5.0).round() as i32 * 5;
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// One card being drawn. Every helper takes design units (the 640×320
/// layout) and scales them itself.
struct Card<'a> {
    data: &'a BgCardData,
    theme: &'a Theme,
    fonts: Fonts<'a>,
    s: f32,
    pref: UnitPreference,
    second: Option<UnitPreference>,
    targets: TirThresholds,
    #[cfg(feature = "beetroot")]
    stickers: Option<&'a crate::charts::stickers::StickerSet>,
}

impl Card<'_> {
    fn px(&self, v: f32) -> i32 {
        (v * self.s).round() as i32
    }

    fn status_color(&self, status: GlucoseStatus) -> Rgba<u8> {
        match status {
            GlucoseStatus::Low => self.theme.glucose_low,
            GlucoseStatus::InRange => self.theme.glucose_in_range,
            GlucoseStatus::High => self.theme.glucose_high,
        }
    }

    fn is_light(&self) -> bool {
        let [r, g, b, _] = self.theme.background.0;
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32 > 128.0
    }

    /// The opaque color of the panel: `grid_major` tinted over the
    /// background, like the other graphs' panels.
    fn panel_color(&self) -> Rgba<u8> {
        blend_pixel(
            self.theme.background,
            with_alpha(self.theme.grid_major, 110),
        )
    }

    /// The fill of a chip in `color`: a soft tint of it, as on the other
    /// graphs' chips.
    fn tint(&self, color: Rgba<u8>) -> Rgba<u8> {
        with_alpha(color, if self.is_light() { 40 } else { 50 })
    }

    // --- Text ---------------------------------------------------------------

    fn w(&self, face: &Face, text: &str, size: f32) -> f32 {
        text_w(&face.font, text, size * self.s) / self.s
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        img: &mut RgbaImage,
        face: &Face,
        color: Rgba<u8>,
        x: f32,
        y: f32,
        size: f32,
        text: &str,
    ) {
        draw_text(
            img,
            color,
            self.px(x),
            self.px(y),
            PxScale::from(size * self.s),
            &face.font,
            text,
        );
    }

    /// `text` with its baseline at `base`, starting at `x`. Returns its end.
    #[allow(clippy::too_many_arguments)]
    fn on_base(
        &self,
        img: &mut RgbaImage,
        face: &Face,
        color: Rgba<u8>,
        x: f32,
        base: f32,
        size: f32,
        text: &str,
    ) -> f32 {
        self.text(img, face, color, x, base - face.base * size, size, text);
        x + self.w(face, text, size)
    }

    /// `text` with its capitals centered on `cy`, starting at `x`.
    #[allow(clippy::too_many_arguments)]
    fn centered(
        &self,
        img: &mut RgbaImage,
        face: &Face,
        color: Rgba<u8>,
        x: f32,
        cy: f32,
        size: f32,
        text: &str,
    ) -> f32 {
        let base = cy + face.cap_height(size) / 2.0;
        self.on_base(img, face, color, x, base, size, text)
    }

    /// Runs of text sharing the baseline `base`, from `x`. Returns the end.
    fn runs(&self, img: &mut RgbaImage, x: f32, base: f32, runs: &[Run]) -> f32 {
        let mut x = x;
        for &(face, text, size, color) in runs {
            x = self.on_base(img, face, color, x, base, size, text);
        }
        x
    }

    /// The vertical middle of the ink of `text`'s first glyph, under the top
    /// of its line, in pixels at `size` pixels: for centering a glyph such
    /// as an arrow on something else.
    fn ink_middle(&self, face: &Face, text: &str, size: f32) -> f32 {
        let scaled = face.font.as_scaled(PxScale::from(size));
        text.chars()
            .filter_map(|c| {
                let glyph = scaled
                    .glyph_id(c)
                    .with_scale_and_position(size, ab_glyph::point(0.0, scaled.ascent()));
                face.font.outline_glyph(glyph)
            })
            .map(|g| (g.px_bounds().min.y + g.px_bounds().max.y) / 2.0)
            .next()
            .unwrap_or(size / 2.0)
    }

    // --- Shapes -------------------------------------------------------------

    /// Fills a rectangle with continuous corners, anti-aliased.
    fn squircle(&self, img: &mut RgbaImage, (x, y, w, h): Rect, radius: f32, color: Rgba<u8>) {
        let s = self.s;
        let (x0, y0, x1, y1) = (x * s, y * s, (x + w) * s, (y + h) * s);
        let center = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let half = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let (img_w, img_h) = (img.width() as i32, img.height() as i32);
        for py in (y0.floor() as i32).max(0)..(y1.ceil() as i32).min(img_h) {
            for px in (x0.floor() as i32).max(0)..(x1.ceil() as i32).min(img_w) {
                let d =
                    squircle_distance(px as f32 + 0.5, py as f32 + 0.5, center, half, radius * s);
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let dst = img.get_pixel_mut(px as u32, py as u32);
                    *dst = blend_pixel(*dst, with_alpha(color, (color[3] as f32 * cover) as u8));
                }
            }
        }
    }

    /// An icon, tinted `color` through its alpha, `size` wide and tall with
    /// its top left corner at `at`.
    fn icon(
        &self,
        img: &mut RgbaImage,
        icon: &DynamicImage,
        at: (f32, f32),
        size: f32,
        color: Rgba<u8>,
    ) {
        let side = (size * self.s).round() as u32;
        if side == 0 {
            return;
        }
        let mask = icon
            .resize_exact(side, side, FilterType::Lanczos3)
            .into_rgba8();
        let (x0, y0) = (self.px(at.0), self.px(at.1));
        for (mx, my, p) in mask.enumerate_pixels() {
            let (ix, iy) = (x0 + mx as i32, y0 + my as i32);
            if p[3] > 0 && ix >= 0 && iy >= 0 {
                if let Some(dst) = img.get_pixel_mut_checked(ix as u32, iy as u32) {
                    *dst = blend_pixel(*dst, with_alpha(color, p[3]));
                }
            }
        }
    }

    /// A chip: an icon and a text on a soft tint of `color`, in a capsule
    /// with its left edge at `x`, centered on `cy`. The tint is `color`'s,
    /// the icon `icon_ink` and the text `ink`. Returns its width.
    #[allow(clippy::too_many_arguments)]
    fn chip(
        &self,
        img: &mut RgbaImage,
        x: f32,
        cy: f32,
        icon: &DynamicImage,
        face: &Face,
        text: &str,
        (color, icon_ink, ink): (Rgba<u8>, Rgba<u8>, Rgba<u8>),
    ) -> f32 {
        let (icon_size, size, pad_left, pad_right) = (16.0, 16.0, 10.0, 13.0);
        let gap = if text.is_empty() { 0.0 } else { 6.0 };
        let w = pad_left + icon_size + gap + self.w(face, text, size) + pad_right;
        let rect = (x, cy - CHIP_H / 2.0, w, CHIP_H);
        self.squircle(img, rect, CHIP_H / 2.0 / 1.45, self.tint(color));
        let at = (x + pad_left, cy - icon_size / 2.0);
        self.icon(img, icon, at, icon_size, icon_ink);
        let text_x = x + pad_left + icon_size + gap;
        self.centered(img, face, ink, text_x, cy, size, text);
        w
    }

    // --- Data ---------------------------------------------------------------

    /// What is on board: each amount with its icon and color.
    fn on_board(&self) -> Vec<(&'static [u8], Rgba<u8>, &str)> {
        [
            (&self.data.iob_str, SYRINGE, self.theme.insulin),
            (&self.data.cob_str, WHEAT, self.theme.carbs),
        ]
        .into_iter()
        .filter_map(|(text, icon, color)| {
            let text = text.as_deref()?;
            let value = text.split_once(' ').map_or(text, |(_, value)| value);
            Some((icon, color, value))
        })
        .collect()
    }

    /// What the chart shows, in a sentence: how long the current reading's
    /// status has lasted. "In range for 2 hr 40 min", "Above 180 for 45 min".
    fn headline(&self) -> String {
        let pts = &self.data.sparkline_points;
        let status = self.data.status;
        let state = match status {
            GlucoseStatus::InRange => "In range".to_string(),
            GlucoseStatus::High => format!("Above {}", fmt_glucose(self.targets.high, self.pref)),
            GlucoseStatus::Low => format!("Below {}", fmt_glucose(self.targets.low, self.pref)),
        };
        let Some(last) = pts.last() else {
            return state;
        };
        match pts.iter().rposition(|p| p.status != status) {
            None => format!("{state} for {}", fmt_minutes(WINDOW_MINUTES)),
            Some(i) => {
                // Counted from the first reading in the status, and at
                // least 5 minutes, so a status just entered is not "0 min".
                let first = pts.get(i + 1).map_or(last.t, |p| p.t);
                let minutes = (last.t - first) * WINDOW_MINUTES;
                format!("{state} for {}", fmt_minutes(minutes.max(5.0)))
            }
        }
    }

    // --- Reading ------------------------------------------------------------

    /// How tall [`reading`](Self::reading) is at `size`, from the value's
    /// cap line to its last baseline.
    fn reading_h(&self, size: f32) -> f32 {
        let second = if self.second.is_some() { 36.0 } else { 0.0 };
        self.fonts.number.cap_height(size) + 28.0 + second
    }

    /// The size the value is drawn at to fit `max_w` with its arrow, up to
    /// `size`.
    fn reading_size(&self, size: f32, max_w: f32) -> f32 {
        let f = &self.fonts;
        let value = fmt_glucose(self.data.current_sgv, self.pref);
        let width = self.w(&f.number, &value, size)
            + 0.1 * size
            + self.w(&f.symbol, &self.data.trend_arrow, size * 0.58);
        size * (max_w / width).min(1.0)
    }

    /// The reading, the card's centerpiece: the value in its status color
    /// with the trend arrow beside it, its cap line at `cap`. Under it, the
    /// unit and the change on one line; with two units, the reading again in
    /// the second one, smaller.
    fn reading(&self, img: &mut RgbaImage, x: f32, cap: f32, size: f32) {
        let d = self.data;
        let t = self.theme;
        let f = &self.fonts;
        let value = fmt_glucose(d.current_sgv, self.pref);
        let base = cap + f.number.cap_height(size);
        let color = self.status_color(d.status);

        let end = self.on_base(img, &f.number, color, x, base, size, &value) + 0.1 * size;
        // The arrow's ink centered between the cap line and the baseline.
        let arrow = size * 0.58;
        let ink = self.ink_middle(&f.symbol, &d.trend_arrow, arrow * self.s) / self.s;
        let arrow_y = (cap + base) / 2.0 - ink;
        self.text(img, &f.symbol, color, end, arrow_y, arrow, &d.trend_arrow);

        // Unit and change, then the second unit's reading.
        let small = 17.0;
        let line = |img: &mut RgbaImage, base: f32, value: Option<&str>, u| {
            let delta = d.delta.map(|v| fmt_delta(v, u)).unwrap_or_default();
            let mut runs: Vec<Run> = Vec::new();
            if let Some(value) = value {
                runs.push((&f.number_medium, value, 28.0, t.text_primary));
                runs.push((&f.number, " ", small, t.text_secondary));
            }
            runs.push((&f.number, unit_name(u), small, t.text_secondary));
            runs.push((&f.number, "   ", small, t.text_secondary));
            runs.push((&f.number, &delta, small, t.text_primary));
            self.runs(img, x, base, &runs);
        };
        line(img, base + 28.0, None, self.pref);
        if let Some(u) = self.second {
            let value = fmt_glucose(d.current_sgv, u);
            line(img, base + 28.0 + 36.0, Some(&value), u);
        }
    }

    /// The info pill as a chip in its state's color, its left edge at `x`,
    /// centered on `cy`.
    fn info_pill(
        &self,
        img: &mut RgbaImage,
        x: f32,
        cy: f32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(InfoPill { icon, text, state }) = &self.data.info_pill else {
            return Ok(());
        };
        let t = self.theme;
        let icon = match icon {
            PillIcon::Path(p) => image::open(p)?,
            PillIcon::Bytes(b) => image::load_from_memory(b)?,
        };
        // The fill is a tint of the state's color, the icon and text in the
        // text's.
        let (color, ink) = match state {
            PillState::Normal => (t.text_secondary, t.text_primary),
            PillState::AlertHigh => (t.glucose_high, t.glucose_high),
            PillState::AlertLow => (t.glucose_low, t.glucose_low),
        };
        let face = &self.fonts.title;
        self.chip(img, x, cy, &icon, face, text, (color, ink, ink));
        Ok(())
    }

    // --- Chart --------------------------------------------------------------

    /// The last 3 hours, as one capsule per quarter hour spanning the
    /// lowest to the highest reading in it, over the target range, and the
    /// latest reading as a dot of its own.
    ///
    /// The scale is fixed, 40 to 260 mg/dL, widened only if a reading or a
    /// target falls outside it, as on Dexcom's and Libre's graphs: the same
    /// height always means the same glucose, and the target range always
    /// sits in the same place, so where the readings stand reads at a glance
    /// without looking at a single label. The range is a flat band and the
    /// only reference drawn; its two values sit on the trailing side.
    ///
    /// The capsules come from bonbon's compare graph and Apple Health's
    /// heart rate chart: a steady quarter hour is a round dot, a moving one
    /// a taller capsule, so the speed of a change shows as well as its
    /// direction. Each capsule takes the color of the values it covers,
    /// along its length: in range it stays neutral, and only what is out of
    /// range is colored, as in Tufte's glucose sparkline.
    ///
    /// The latest reading is a larger dot at now, at its exact level, in its
    /// status color. The capsules stop short of it and it is cut out of
    /// whatever it overlaps, so it reads apart from the readings before it
    /// however they fall.
    ///
    /// The very low and very high lines only appear once a reading of the
    /// window has crossed them, dotted: the rest of the time they would only
    /// crowd the chart.
    fn plot(&self, img: &mut RgbaImage, a: &Plot) {
        let s = self.s;
        let t = self.theme;
        let f = &self.fonts;
        let pts = &self.data.sparkline_points;
        let (low, high) = (self.targets.low, self.targets.high);

        let (dmin, dmax) = pts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.sgv), hi.max(p.sgv))
        });
        let lo = 40.0_f32.min(dmin - 10.0).min(low - 20.0);
        let hi = 260.0_f32.max(dmax + 20.0).max(high + 30.0);

        let (left, right, top, bottom) = (a.left * s, a.right * s, a.top * s, a.bottom * s);
        let y = |v: f32| bottom - (v - lo) / (hi - lo) * (bottom - top);
        // The latest reading's dot, and the gap cut around it, in design
        // units. Time runs up to its center, so the dot stays inside the
        // plot.
        const NOW_DOT: f32 = 7.0;
        const NOW_GAP: f32 = 4.0;
        let end = right - (NOW_DOT + NOW_GAP) * s;
        let x = |at: f32| left + at.clamp(0.0, 1.0) * (end - left);
        // The past runs up to a little before now, so the last capsule
        // clears the latest reading's dot.
        let past_end = end - 6.0 * s;
        let x_past = |at: f32| left + at.clamp(0.0, 1.0) * (past_end - left);
        let color_of = |v: f32| {
            if v > high {
                t.glucose_high
            } else if v < low {
                t.glucose_low
            } else {
                t.glucose_in_range
            }
        };

        // The target range: one flat band with softly rounded ends.
        let alpha = if self.is_light() { 34 } else { 30 };
        let band_color = with_alpha(t.glucose_in_range, alpha);
        let band_h = (y(low) - y(high)) / s;
        let band = (a.left, y(high) / s, a.right - a.left, band_h);
        self.squircle(img, band, 5.0, band_color);

        // The crossed extremes, dotted.
        let extremes: Vec<(f32, Rgba<u8>)> = [
            (
                self.targets.very_high,
                t.glucose_high,
                dmax >= self.targets.very_high,
            ),
            (
                self.targets.very_low,
                t.glucose_low,
                dmin <= self.targets.very_low,
            ),
        ]
        .into_iter()
        .filter(|&(_, _, crossed)| crossed && !pts.is_empty())
        .map(|(v, color, _)| (v, color))
        .collect();
        for &(v, color) in &extremes {
            draw_dashed_horizontal_line(
                img,
                y(v),
                left,
                right,
                with_alpha(color, 150),
                (2.0 * s) as i32,
                (4.0 * s) as i32,
                s.ceil() as i32,
            );
        }

        // One capsule per quarter hour, counted back from now, of the
        // readings before the latest, which has its own dot. A quarter hour
        // without readings is left empty.
        let history = &pts[..pts.len().saturating_sub(1)];
        const QUARTERS: usize = 12;
        let slot = (past_end - left) / QUARTERS as f32;
        let r = (slot * 0.36).min(9.0 * s) / 2.0;
        for q in 0..QUARTERS {
            let to = 1.0 - q as f32 / QUARTERS as f32;
            let from = to - 1.0 / QUARTERS as f32;
            let inside: Vec<f32> = history
                .iter()
                .filter(|p| p.t >= from - 1e-4 && (p.t < to - 1e-4 || q == 0))
                .map(|p| p.sgv)
                .collect();
            if inside.is_empty() {
                continue;
            }
            let (vmin, vmax) = inside
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let cx = x_past((from + to) / 2.0);
            // The capsule's straight part runs between the two extremes'
            // levels, its round ends beyond them.
            let (y0, y1) = (y(vmax), y(vmin));
            for py in (y0 - r - 1.0).floor() as i32..(y1 + r + 1.0).ceil() as i32 {
                let fy = py as f32 + 0.5;
                let along = fy.clamp(y0, y1);
                let value = lo + (bottom - fy) / (bottom - top) * (hi - lo);
                let color = color_of(value.clamp(vmin, vmax));
                for px in (cx - r - 1.0).floor() as i32..(cx + r + 1.0).ceil() as i32 {
                    let fx = px as f32 + 0.5;
                    let d = ((fx - cx).powi(2) + (fy - along).powi(2)).sqrt() - r;
                    let cover = (0.5 - d).clamp(0.0, 1.0);
                    if cover <= 0.0 || px < 0 || py < 0 {
                        continue;
                    }
                    if let Some(dst) = img.get_pixel_mut_checked(px as u32, py as u32) {
                        *dst = blend_pixel(*dst, with_alpha(color, (255.0 * cover) as u8));
                    }
                }
            }
        }

        // The latest reading, cut out of whatever it overlaps by a gap the
        // color of what lies under it: apart without drawing a ring.
        if let Some(latest) = pts.last() {
            let (cx, cy) = (x(1.0).round() as i32, y(latest.sgv).round() as i32);
            let in_band = (y(high)..=y(low)).contains(&(cy as f32));
            let under = if in_band {
                blend_pixel(self.panel_color(), band_color)
            } else {
                self.panel_color()
            };
            let gap = create_aa_circle_sprite(((NOW_DOT + NOW_GAP) * s).round() as i32, under);
            blend_sprite(img, &gap, cx, cy);
            let color = self.status_color(self.data.status);
            let dot = create_aa_circle_sprite((NOW_DOT * s).round() as i32, color);
            blend_sprite(img, &dot, cx, cy);
        }

        // Value labels on the trailing side, each centered on its level.
        let size = 14.0;
        let cap_height = f.label.cap_height(size);
        let mut labels: Vec<(f32, f32, Rgba<u8>)> = vec![
            (y(high) / s, high, t.glucose_high),
            (y(low) / s, low, t.glucose_low),
        ];
        for &(v, color) in &extremes {
            labels.push((y(v) / s, v, darken_color(color, 0.8)));
        }
        // Labels too close to read apart are pushed apart, as little as
        // needed, staying within the plot's height.
        labels.sort_by(|a, b| a.0.total_cmp(&b.0));
        let spacing = cap_height + 7.0;
        for i in 1..labels.len() {
            labels[i].0 = labels[i].0.max(labels[i - 1].0 + spacing);
        }
        if let Some(last) = labels.last_mut() {
            last.0 = last.0.min(a.bottom + 2.0);
        }
        for i in (0..labels.len().saturating_sub(1)).rev() {
            labels[i].0 = labels[i].0.min(labels[i + 1].0 - spacing);
        }
        for (line_y, v, color) in labels {
            let text = fmt_glucose(v, self.pref);
            self.centered(img, &f.label, color, a.right + 10.0, line_y, size, &text);
        }

        // Time along the bottom: where the window starts, the hours, and
        // the clock time it ends at.
        let base = a.bottom + 26.0;
        let color = t.text_secondary;
        self.on_base(img, &f.text, color, a.left, base, size, "3h ago");
        for (hour, text) in [(1, "2h"), (2, "1h")] {
            let cx = x_past(hour as f32 / 3.0) / s - self.w(&f.text, text, size) / 2.0;
            self.on_base(img, &f.text, color, cx, base, size, text);
        }
        let now = if self.data.time_str.is_empty() {
            "Now"
        } else {
            &self.data.time_str
        };
        let now_x = a.right - self.w(&f.text, now, size);
        self.on_base(img, &f.text, color, now_x, base, size, now);
    }

    // --- Card ---------------------------------------------------------------

    /// The whole card, in two halves the way a medium widget is laid out:
    /// now on the left, the last 3 hours on the right.
    ///
    /// The left half sits straight on the background and answers "where am
    /// I": who and how fresh on top, the reading as large as the column
    /// allows with the info pill over it and its trend, change and second
    /// unit under it, and what is on board as two chips at the foot. The
    /// right half is the one panel and answers "how did I get here": a
    /// headline saying how long the current status has lasted, over the
    /// chart.
    fn draw(&self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let t = self.theme;
        let f = &self.fonts;
        let d = self.data;
        let (w, h) = ((W * self.s) as u32, (H * self.s) as u32);
        let mut img = RgbaImage::from_pixel(w, h, t.background);
        let img = &mut img;

        let column_w = 212.0;
        let x = MARGIN + 8.0;
        let panel_x = x + column_w + 16.0;
        let panel = (panel_x, MARGIN, W - MARGIN - panel_x, H - 2.0 * MARGIN);
        self.squircle(img, panel, PANEL_RADIUS, self.panel_color());

        #[cfg(feature = "beetroot")]
        if let Some(set) = self.stickers {
            use crate::charts::stickers;
            let bounds = stickers::bounds_from(0.0, 0.0, w as f32, h as f32);
            stickers::draw_on_card(img, set, d.status, bounds);
        }

        // --- Left: now ---
        let title_base = MARGIN + 26.0;
        let mut title: Vec<Run> = Vec::new();
        if !d.watermark_str.is_empty() {
            title.push((&f.title, &d.watermark_str, 16.0, t.text_primary));
        }
        if !d.age_str.is_empty() {
            if !title.is_empty() {
                title.push((&f.text, "  ·  ", 15.0, t.text_secondary));
            }
            title.push((&f.text, &d.age_str, 15.0, t.text_secondary));
        }
        self.runs(img, x, title_base, &title);

        // What is on board: one chip per amount, in its color.
        let on_board = self.on_board();
        let chips_cy = H - MARGIN - 8.0 - CHIP_H / 2.0;
        let mut chip_x = x;
        for &(png, color, value) in &on_board {
            let icon = image::load_from_memory(png)?;
            let inks = (color, color, t.text_primary);
            chip_x += self.chip(img, chip_x, chips_cy, &icon, &f.number, value, inks) + 8.0;
        }

        // The info pill over the reading, the two centered together in the
        // room between.
        let room_top = title_base + 14.0;
        let room_bottom = if on_board.is_empty() {
            H - MARGIN - 8.0
        } else {
            chips_cy - CHIP_H / 2.0 - 14.0
        };
        let size = self.reading_size(96.0, column_w);
        let pill_h = if d.info_pill.is_some() {
            CHIP_H + 18.0
        } else {
            0.0
        };
        let free = (room_bottom - room_top) - pill_h - self.reading_h(size);
        let block_top = room_top + free.max(0.0) / 2.0;
        self.info_pill(img, x, block_top + CHIP_H / 2.0)?;
        self.reading(img, x, block_top + pill_h, size);

        // --- Right: the last 3 hours ---
        let (px, py, pw, ph) = panel;
        let inner = px + 20.0;
        let headline_base = py + 26.0 + f.title.cap_height(17.0) / 2.0;
        let headline = self.headline();
        let color = t.text_primary;
        self.on_base(img, &f.title, color, inner, headline_base, 17.0, &headline);
        let label_w = [self.targets.very_high, self.targets.high, self.targets.low]
            .into_iter()
            .map(|v| self.w(&f.label, &fmt_glucose(v, self.pref), 14.0))
            .fold(0.0, f32::max);
        self.plot(
            img,
            &Plot {
                left: inner,
                right: px + pw - 20.0 - label_w - 10.0,
                top: headline_base + 26.0,
                bottom: py + ph - 20.0 - 26.0,
            },
        );
        Ok(std::mem::take(img))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

    fn status(sgv: f32) -> GlucoseStatus {
        if sgv < 70.0 {
            GlucoseStatus::Low
        } else if sgv > 180.0 {
            GlucoseStatus::High
        } else {
            GlucoseStatus::InRange
        }
    }

    /// A card whose readings over 3 hours, one every 5 minutes, are
    /// `curve(i)` for reading `i` of 37.
    fn data(curve: impl Fn(usize) -> f32) -> BgCardData {
        let sparkline_points: Vec<SparklinePoint> = (0..37)
            .map(|i| {
                let sgv = curve(i);
                SparklinePoint {
                    t: i as f32 / 36.0,
                    sgv,
                    status: status(sgv),
                }
            })
            .collect();
        let last = sparkline_points[36].sgv;
        BgCardData {
            current_sgv: last,
            status: status(last),
            trend_arrow: "→".to_string(),
            delta: Some(2.0),
            watermark_str: "Bonbon".to_string(),
            age_str: "2 min ago".to_string(),
            time_str: "12:00".to_string(),
            iob_str: Some("IOB 1.5u".to_string()),
            cob_str: Some("COB 20g".to_string()),
            sparkline_points,
            info_pill: None,
        }
    }

    fn headline(data: &BgCardData) -> String {
        let theme = Theme::dark();
        Card {
            data,
            theme: &theme,
            fonts: Fonts::new(FONT, FONT).unwrap(),
            s: 1.0,
            pref: UnitPreference::MgDl,
            second: None,
            targets: TirThresholds::default(),
            #[cfg(feature = "beetroot")]
            stickers: None,
        }
        .headline()
    }

    #[test]
    fn minutes_round_to_five() {
        assert_eq!(fmt_minutes(43.0), "45 min");
        assert_eq!(fmt_minutes(60.0), "1 hr");
        assert_eq!(fmt_minutes(77.0), "1 hr 15 min");
        assert_eq!(fmt_minutes(180.0), "3 hr");
    }

    #[test]
    fn headline_says_how_long_the_status_has_lasted() {
        assert_eq!(headline(&data(|_| 120.0)), "In range for 3 hr");
        // High from reading 24 on: 12 readings later, an hour.
        let rising = data(|i| if i < 24 { 120.0 } else { 200.0 });
        assert_eq!(headline(&rising), "Above 180 for 1 hr");
        let low = data(|i| if i < 30 { 100.0 } else { 60.0 });
        assert_eq!(headline(&low), "Below 70 for 30 min");
        let just_low = data(|i| if i < 36 { 100.0 } else { 60.0 });
        assert_eq!(headline(&just_low), "Below 70 for 5 min");
        let mut empty = data(|_| 120.0);
        empty.sparkline_points.clear();
        assert_eq!(headline(&empty), "In range");
    }

    #[test]
    fn builds_for_every_setup() {
        let dual = UnitDisplay::Dual {
            primary: UnitPreference::MmolL,
        };
        let pill = || InfoPill {
            icon: PillIcon::from_bytes(builtin_icons::FAST_RISE),
            text: "Rising".to_string(),
            state: PillState::AlertHigh,
        };
        let cases: Vec<(BgCardData, UnitDisplay)> = vec![
            (data(|_| 120.0), UnitDisplay::MgDl),
            (data(|i| 150.0 + i as f32 * 4.0), dual),
            (data(|i| 140.0 - i as f32 * 3.0), UnitDisplay::MmolL),
            (data(|_| 300.0), UnitDisplay::MgDl),
            (data(|_| 45.0), UnitDisplay::MgDl),
            (
                BgCardData {
                    info_pill: Some(pill()),
                    ..data(|i| 120.0 + (i % 5) as f32 * 30.0)
                },
                dual,
            ),
            (
                BgCardData {
                    sparkline_points: Vec::new(),
                    iob_str: None,
                    cob_str: None,
                    watermark_str: String::new(),
                    age_str: String::new(),
                    time_str: String::new(),
                    delta: None,
                    ..data(|_| 120.0)
                },
                UnitDisplay::MgDl,
            ),
        ];
        for (data, unit) in cases {
            let img = BgCardBuilder::new()
                .with_data(data)
                .with_units(unit)
                .with_scale(2.0)
                .build()
                .unwrap();
            assert_eq!(img.dimensions(), (1280, 640));
        }
    }

    #[test]
    fn data_is_required() {
        assert!(BgCardBuilder::new().build().is_err());
    }
}
