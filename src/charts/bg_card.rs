use std::path::PathBuf;

use ab_glyph::{FontRef, PxScale};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_antialiased_line_segment_mut, draw_text_mut};

use crate::theme::Theme;
use crate::utils::drawing::draw_filled_rounded_rect;

/// Glucose status used for gradient color, main value color, and sparkline segment colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlucoseStatus {
    Low,
    InRange,
    High,
}

/// Visual state for [`InfoPill`]. Controls background + text color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillState {
    /// Neutral state, blends in with the base theme.
    Normal,
    /// High-glucose alert, vivid `glucose_high` background with high-contrast text.
    AlertHigh,
    /// Low-glucose alert, vivid `glucose_low` background with high-contrast text.
    AlertLow,
}

/// Source for the pill icon. Loaded lazily at build time and resized to a square
/// matching the pill height. Non-square sources are stretched to fit.
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

/// A small contextual badge rendered in the top-right of the card.
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

/// A single data point in the 3-hour sparkline.
#[derive(Debug, Clone)]
pub struct SparklinePoint {
    /// Normalized position in the 3h window. 0.0 = oldest, 1.0 = most recent.
    pub t: f32,
    /// Raw glucose value in mg/dL, used for vertical positioning.
    pub sgv: f32,
    /// Status of this point, colors the segment from this point to the next.
    pub status: GlucoseStatus,
}

/// All pre-calculated, pre-formatted data for a BgCard.
#[derive(Debug)]
pub struct BgCardData {
    /// Current glucose value in mg/dL.
    pub current_sgv: f32,
    /// Status of the current reading, determines gradient and value color.
    pub status: GlucoseStatus,
    /// Trend arrow as a single Unicode character. E.g., `"→"`, `"↑"`, `"↗"`, `"↓"`.
    pub trend_arrow: String,
    /// Pre-formatted delta with sign. E.g., `"+3 mg/dl"`, `"-2 mmol/L"`.
    pub delta_str: String,
    /// Watermark displayed at the top right of the card.
    pub watermark_str: String,
    /// Pre-formatted data age. E.g., `"3 min ago"`, `"just now"`.
    pub age_str: String,
    /// Unit label. E.g., `"mg/dl"` or `"mmol/L"`.
    pub unit_str: String,
    /// Current local time for the header. E.g., `"15:45"`.
    pub time_str: String,
    /// Insulin on board, pre-formatted. E.g., `"IOB 2.5u"`. `None` hides the field.
    pub iob_str: Option<String>,
    /// Carbs on board, pre-formatted. E.g., `"COB 30g"`. `None` hides the field.
    pub cob_str: Option<String>,
    /// Sparkline data for the last 3 hours, oldest first.
    pub sparkline_points: Vec<SparklinePoint>,
    /// Optional contextual info pill rendered top-right under the time.
    pub info_pill: Option<InfoPill>,
}

/// Builder for the card.
///
/// Produces a fixed **640 × 320 px** RGBA image (or scaled up via `with_scale`).
pub struct BgCardBuilder<'a> {
    data: Option<BgCardData>,
    theme: Theme,
    font: &'a [u8],
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
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            scale: 1.0,
            #[cfg(feature = "beetroot")]
            sticker_set: None,
        }
    }

    /// Attach a [`StickerSet`](crate::charts::stickers::StickerSet) to the card.
    ///
    /// Stickers are layered behind the header/content so text and the
    /// sparkline remain on top. Only available with the `beetroot` feature.
    #[cfg(feature = "beetroot")]
    pub fn with_stickers(mut self, set: crate::charts::stickers::StickerSet) -> Self {
        self.sticker_set = Some(set);
        self
    }

    pub fn with_data(mut self, data: BgCardData) -> Self {
        self.data = Some(data);
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_font(mut self, font: &'a [u8]) -> Self {
        self.font = font;
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
        let font = FontRef::try_from_slice(self.font)?;
        let s = self.scale;

        let w = (640.0 * s) as u32;
        let h = (320.0 * s) as u32;

        let mut img = RgbaImage::from_pixel(w, h, self.theme.background);

        draw_bg_pattern(&mut img, &self.theme, w, h, s);
        draw_gradient(&mut img, &self.theme, data.status, w, h);

        #[cfg(feature = "beetroot")]
        if let Some(set) = self.sticker_set.as_ref() {
            use crate::charts::stickers;
            let bounds = stickers::bounds_from(0.0, 0.0, w as f32, h as f32);
            stickers::draw_on_card(&mut img, set, data.status, bounds);
        }

        draw_header(&mut img, &self.theme, &font, &data, w, s);
        draw_content(&mut img, &self.theme, &font, &data, w, s);
        draw_sparkline(&mut img, &self.theme, &data.sparkline_points, w, s);

        if let Some(pill) = &data.info_pill {
            draw_info_pill(&mut img, &self.theme, &font, pill, w, s)?;
        }

        Ok(img)
    }
}

fn status_color(theme: &Theme, status: GlucoseStatus) -> Rgba<u8> {
    match status {
        GlucoseStatus::Low => theme.glucose_low,
        GlucoseStatus::InRange => theme.glucose_in_range,
        GlucoseStatus::High => theme.glucose_high,
    }
}

/// Approximate text width in pixels for a given font size.
fn approx_text_w(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.6
}

/// Subtle grid pattern drawn over the background before any content.
fn draw_bg_pattern(img: &mut RgbaImage, theme: &Theme, w: u32, h: u32, s: f32) {
    let spacing = (64.0 * s) as u32;
    let [br, bg, bb, ba] = theme.background.0;
    let line = Rgba([
        br.saturating_sub(7),
        bg.saturating_sub(7),
        bb.saturating_sub(7),
        ba,
    ]);
    let mut x = spacing;
    while x < w {
        for y in 0..h {
            img.put_pixel(x, y, line);
        }
        x += spacing;
    }
    let mut y = spacing;
    while y < h {
        for x in 0..w {
            img.put_pixel(x, y, line);
        }
        y += spacing;
    }
}

/// Draws the ambient gradient: status color fading from top.
fn draw_gradient(img: &mut RgbaImage, theme: &Theme, status: GlucoseStatus, w: u32, h: u32) {
    let c = status_color(theme, status);
    let gh = (h as f32 * 0.5) as u32;

    for y in 0..gh {
        let a = 55.0_f32 * (1.0 - y as f32 / gh as f32) / 255.0;
        let inv = 1.0 - a;
        for x in 0..w {
            let px = img.get_pixel_mut(x, y);
            let [dr, dg, db, da] = px.0;
            px.0 = [
                (c[0] as f32 * a + dr as f32 * inv) as u8,
                (c[1] as f32 * a + dg as f32 * inv) as u8,
                (c[2] as f32 * a + db as f32 * inv) as u8,
                da,
            ];
        }
    }
}

/// Draws the header: Watermark label on the left, current time on the right.
fn draw_header(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    data: &BgCardData,
    w: u32,
    s: f32,
) {
    let cy = 23.0 * s;
    let pad = 24.0 * s;
    let font_h = 22.0 * s;

    let label_y = (cy - font_h / 2.0) as i32;
    draw_text_mut(
        img,
        theme.text_primary,
        pad as i32,
        label_y,
        PxScale::from(font_h),
        font,
        &data.watermark_str,
    );

    let tw = approx_text_w(&data.time_str, font_h);
    let tx = (w as f32 - pad - tw) as i32;
    let ty = (cy - font_h / 2.0) as i32;
    draw_text_mut(
        img,
        theme.text_secondary,
        tx,
        ty,
        PxScale::from(font_h),
        font,
        &data.time_str,
    );
}

/// Draws the main content zone: age, SGV + trend, unit, delta, IOB, COB.
fn draw_content(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    data: &BgCardData,
    w: u32,
    s: f32,
) {
    let pad = 24.0 * s;
    let font_age = 16.0 * s;
    let font_sgv = 76.0 * s;
    let font_unit = 16.0 * s;
    let font_delta = 21.0 * s;
    let font_iob_cob = 30.0 * s;
    let age_y = 62.0 * s;

    let sgv_y = age_y + font_age + 4.0 * s;
    let unit_y = sgv_y + font_sgv + 3.0 * s;
    let delta_y = unit_y + font_unit + 4.0 * s;
    let iob_y = sgv_y;
    let cob_y = iob_y + font_iob_cob + 6.0 * s;

    draw_text_mut(
        img,
        theme.text_secondary,
        pad as i32,
        age_y as i32,
        PxScale::from(font_age),
        font,
        &data.age_str,
    );

    let value_str = format!("{:.0} {}", data.current_sgv, data.trend_arrow);
    draw_text_mut(
        img,
        status_color(theme, data.status),
        pad as i32,
        sgv_y as i32,
        PxScale::from(font_sgv),
        font,
        &value_str,
    );

    draw_text_mut(
        img,
        theme.text_dim,
        pad as i32,
        unit_y as i32,
        PxScale::from(font_unit),
        font,
        &data.unit_str,
    );

    draw_text_mut(
        img,
        theme.text_primary,
        pad as i32,
        delta_y as i32,
        PxScale::from(font_delta),
        font,
        &data.delta_str,
    );

    let x_right = w as f32 - pad;
    let gap = approx_text_w(" ", font_iob_cob);

    let iob_parts = data.iob_str.as_deref().and_then(|s| s.split_once(' '));
    let cob_parts = data.cob_str.as_deref().and_then(|s| s.split_once(' '));

    let max_val_w = [iob_parts, cob_parts]
        .iter()
        .flatten()
        .map(|(_, v)| approx_text_w(v, font_iob_cob))
        .fold(0.0_f32, f32::max);

    let val_x = (x_right - max_val_w) as i32;
    let label_x = (x_right - max_val_w - gap - approx_text_w("IOB", font_iob_cob)) as i32;

    if let Some((label, value)) = iob_parts {
        draw_text_mut(
            img,
            theme.text_secondary,
            label_x,
            iob_y as i32,
            PxScale::from(font_iob_cob),
            font,
            label,
        );
        draw_text_mut(
            img,
            theme.text_secondary,
            val_x,
            iob_y as i32,
            PxScale::from(font_iob_cob),
            font,
            value,
        );
    }

    if let Some((label, value)) = cob_parts {
        draw_text_mut(
            img,
            theme.text_secondary,
            label_x,
            cob_y as i32,
            PxScale::from(font_iob_cob),
            font,
            label,
        );
        draw_text_mut(
            img,
            theme.text_secondary,
            val_x,
            cob_y as i32,
            PxScale::from(font_iob_cob),
            font,
            value,
        );
    }
}

/// Draws the colored sparkline in the bottom zone with a gradient fill underneath.
fn draw_sparkline(img: &mut RgbaImage, theme: &Theme, points: &[SparklinePoint], w: u32, s: f32) {
    let zone_top = 224.0 * s;
    let zone_h = 66.0 * s;
    let thickness = (3.0 * s).round() as i32;
    let grad_h = 40.0 * s;
    let plot_w = w as f32;
    let bottom_y = zone_top + zone_h;

    if points.len() < 2 {
        return;
    }

    let sgv_min = points.iter().map(|p| p.sgv).fold(f32::MAX, f32::min);
    let sgv_max = points.iter().map(|p| p.sgv).fold(f32::MIN, f32::max);
    let sgv_range = (sgv_max - sgv_min).max(20.0);

    let map_x = |t: f32| t * plot_w;
    let map_y = |sgv: f32| bottom_y - ((sgv - sgv_min) / sgv_range) * zone_h;

    const GRAD_ALPHA: f32 = 55.0;
    let img_w = img.width();
    let img_h = img.height();

    for i in 0..points.len() - 1 {
        let p0 = &points[i];
        let p1 = &points[i + 1];
        let x0 = map_x(p0.t);
        let x1 = map_x(p1.t);
        let color = status_color(theme, p0.status);

        let col_start = (x0 as i32).max(0) as u32;
        let col_end = if i + 1 < points.len() - 1 {
            (x1 as i32).max(0) as u32
        } else {
            (x1.ceil() as i32).min(img_w as i32) as u32
        };

        for col in col_start..col_end {
            let frac = if x1 > x0 {
                (col as f32 - x0) / (x1 - x0)
            } else {
                0.0
            };
            let curve_y = map_y(p0.sgv + frac * (p1.sgv - p0.sgv));
            let fill_top = (curve_y as i32).max(0) as u32;
            let fill_bot = ((curve_y + grad_h) as i32).min(img_h as i32) as u32;

            for row in fill_top..fill_bot {
                let a = GRAD_ALPHA * (1.0 - (row as f32 - curve_y) / grad_h) / 255.0;
                let inv = 1.0 - a;
                let px = img.get_pixel_mut(col, row);
                let [dr, dg, db, da] = px.0;
                px.0 = [
                    (color[0] as f32 * a + dr as f32 * inv) as u8,
                    (color[1] as f32 * a + dg as f32 * inv) as u8,
                    (color[2] as f32 * a + db as f32 * inv) as u8,
                    da,
                ];
            }
        }
    }

    for i in 0..points.len() - 1 {
        let p0 = &points[i];
        let p1 = &points[i + 1];
        let x0 = map_x(p0.t);
        let y0 = map_y(p0.sgv);
        let x1 = map_x(p1.t);
        let y1 = map_y(p1.sgv);
        let color = status_color(theme, p0.status);

        for t in -(thickness / 2)..=(thickness / 2) {
            let tf = t as f32;
            draw_antialiased_line_segment_mut(
                img,
                (x0 as i32, (y0 + tf) as i32),
                (x1 as i32, (y1 + tf) as i32),
                color,
                |bg: Rgba<u8>, _, alpha| {
                    let inv = 1.0 - alpha;
                    Rgba([
                        (color[0] as f32 * alpha + bg[0] as f32 * inv) as u8,
                        (color[1] as f32 * alpha + bg[1] as f32 * inv) as u8,
                        (color[2] as f32 * alpha + bg[2] as f32 * inv) as u8,
                        bg[3],
                    ])
                },
            );
        }
    }
}

/// Returns `(background, text)` color for the pill, picked from the theme.
fn pill_colors(theme: &Theme, state: PillState) -> (Rgba<u8>, Rgba<u8>) {
    match state {
        PillState::Normal => {
            let c = theme.text_secondary;
            (Rgba([c[0], c[1], c[2], 60]), theme.text_primary)
        }
        PillState::AlertHigh => {
            let c = theme.glucose_high;
            (Rgba([c[0], c[1], c[2], 240]), theme.background)
        }
        PillState::AlertLow => {
            let c = theme.glucose_low;
            (Rgba([c[0], c[1], c[2], 240]), theme.background)
        }
    }
}

/// Resizes `icon_src` to `size × size` (Lanczos3) and alpha-blends it at `(x, y)`.
fn draw_pill_icon(img: &mut RgbaImage, icon_src: &DynamicImage, x: i32, y: i32, size: u32) {
    if size == 0 {
        return;
    }
    let resized = icon_src.resize_exact(size, size, FilterType::Lanczos3);
    let rgba = resized.to_rgba8();

    let img_w = img.width() as i32;
    let img_h = img.height() as i32;

    for (sx, sy, pixel) in rgba.enumerate_pixels() {
        let px = x + sx as i32;
        let py = y + sy as i32;
        if px < 0 || py < 0 || px >= img_w || py >= img_h {
            continue;
        }
        let alpha = pixel.0[3] as f32 / 255.0;
        if alpha == 0.0 {
            continue;
        }
        let inv = 1.0 - alpha;
        let dst = img.get_pixel_mut(px as u32, py as u32);
        dst.0 = [
            (pixel.0[0] as f32 * alpha + dst.0[0] as f32 * inv) as u8,
            (pixel.0[1] as f32 * alpha + dst.0[1] as f32 * inv) as u8,
            (pixel.0[2] as f32 * alpha + dst.0[2] as f32 * inv) as u8,
            dst.0[3],
        ];
    }
}

/// Draws the info pill (rounded rect + icon + text) in the top-right of the card.
fn draw_info_pill(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    pill: &InfoPill,
    w: u32,
    s: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let header_cy = 23.0 * s;
    let pill_h = 30.0 * s;
    let icon_size = 22.0 * s;
    let font_h = 18.0 * s;
    let pill_pad = 10.0 * s;
    let icon_gap = if pill.text.is_empty() { 0.0 } else { 7.0 * s };
    let radius = (pill_h / 2.0).round() as i32;

    let icon_img = match &pill.icon {
        PillIcon::Path(p) => image::open(p)?,
        PillIcon::Bytes(b) => image::load_from_memory(b)?,
    };

    let text_w = approx_text_w(&pill.text, font_h);
    let pill_w = pill_pad + icon_size + icon_gap + text_w + pill_pad;
    let pill_x = (w as f32 - pill_w) / 2.0;
    let pill_y = header_cy - pill_h / 2.0;

    let (bg_color, text_color) = pill_colors(theme, pill.state);

    draw_filled_rounded_rect(
        img,
        pill_x as i32,
        pill_y as i32,
        pill_w as u32,
        pill_h as u32,
        radius,
        bg_color,
    );

    let icon_x = (pill_x + pill_pad) as i32;
    let icon_y = (pill_y + (pill_h - icon_size) / 2.0) as i32;
    draw_pill_icon(img, &icon_img, icon_x, icon_y, icon_size as u32);

    if !pill.text.is_empty() {
        let text_x = (pill_x + pill_pad + icon_size + icon_gap) as i32;
        let text_y = (pill_y + (pill_h - font_h) / 2.0) as i32;
        draw_text_mut(
            img,
            text_color,
            text_x,
            text_y,
            PxScale::from(font_h),
            font,
            &pill.text,
        );
    }

    Ok(())
}
