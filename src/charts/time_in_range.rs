use ab_glyph::{FontRef, PxScale};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;

use crate::models::{GraphEntry, UnitDisplay, UnitPreference};
use crate::theme::Theme;
use crate::utils::color::darken_color;
use crate::utils::drawing::{draw_fast_rect, draw_filled_rounded_rect};
use crate::utils::text::text_w;

const MGDL_PER_MMOL: f32 = 18.0182;

/// A glycemic band, ordered from lowest to highest glucose.
///
/// Doubles as an index into the per-band arrays on [`TirStats`]:
/// `stats.percentages[TirBand::Low as usize]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TirBand {
    VeryLow = 0,
    Low = 1,
    InRange = 2,
    High = 3,
    VeryHigh = 4,
}

impl TirBand {
    pub const ALL: [TirBand; 5] = [
        TirBand::VeryLow,
        TirBand::Low,
        TirBand::InRange,
        TirBand::High,
        TirBand::VeryHigh,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            TirBand::VeryLow => "Very Low",
            TirBand::Low => "Low",
            TirBand::InRange => "In Range",
            TirBand::High => "High",
            TirBand::VeryHigh => "Very High",
        }
    }
}

/// Band boundaries in **mg/dL** (inputs are always mg/dL, display conversion
/// is handled by the unit setting).
#[derive(Debug, Clone, Copy)]
pub struct TirThresholds {
    pub very_low: f32,
    pub low: f32,
    pub high: f32,
    pub very_high: f32,
}

impl Default for TirThresholds {
    fn default() -> Self {
        Self {
            very_low: 54.0,
            low: 70.0,
            high: 180.0,
            very_high: 250.0,
        }
    }
}

impl TirThresholds {
    pub fn classify(&self, sgv: f32) -> TirBand {
        if sgv < self.very_low {
            TirBand::VeryLow
        } else if sgv < self.low {
            TirBand::Low
        } else if sgv <= self.high {
            TirBand::InRange
        } else if sgv <= self.very_high {
            TirBand::High
        } else {
            TirBand::VeryHigh
        }
    }
}

/// Everything the card displays, computed from raw entries. Public so the
/// numbers can be reused without rendering an image.
#[derive(Debug, Clone)]
pub struct TirStats {
    /// Reading count per band, indexed by `TirBand as usize`.
    pub counts: [usize; 5],
    /// Share of readings per band, in percent (0–100).
    pub percentages: [f32; 5],
    /// Estimated time spent per band: total span × band share.
    pub time_per_band: [chrono::Duration; 5],
    /// Total number of readings.
    pub total: usize,
    /// Time between the oldest and newest reading.
    pub span: chrono::Duration,
    /// Mean glucose in mg/dL.
    pub mean_mgdl: f32,
    /// Population standard deviation in mg/dL.
    pub sd_mgdl: f32,
    /// Coefficient of variation in percent (SD / mean × 100). The common
    /// clinical stability target is ≤ 36%.
    pub cv_percent: f32,
    /// Glucose Management Indicator in percent (3.31 + 0.02392 × mean mg/dL),
    /// an estimated-HbA1c-like figure.
    pub gmi_percent: f32,
}

impl TirStats {
    /// Computes all summary numbers for `entries`. Returns `None` when
    /// `entries` is empty. Order does not matter.
    pub fn compute(entries: &[GraphEntry], thresholds: &TirThresholds) -> Option<Self> {
        if entries.is_empty() {
            return None;
        }

        let mut counts = [0usize; 5];
        let mut sum = 0.0f64;
        let mut min_date = entries[0].date;
        let mut max_date = entries[0].date;
        for e in entries {
            counts[thresholds.classify(e.sgv) as usize] += 1;
            sum += e.sgv as f64;
            min_date = min_date.min(e.date);
            max_date = max_date.max(e.date);
        }

        let total = entries.len();
        let mean = sum / total as f64;
        let var = entries
            .iter()
            .map(|e| {
                let d = e.sgv as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / total as f64;
        let sd = var.sqrt();

        let span = max_date - min_date;
        let span_mins = span.num_minutes();

        let mut percentages = [0.0f32; 5];
        let mut time_per_band = [chrono::Duration::zero(); 5];
        for i in 0..5 {
            let frac = counts[i] as f64 / total as f64;
            percentages[i] = (frac * 100.0) as f32;
            time_per_band[i] = chrono::Duration::minutes((span_mins as f64 * frac).round() as i64);
        }

        Some(Self {
            counts,
            percentages,
            time_per_band,
            total,
            span,
            mean_mgdl: mean as f32,
            sd_mgdl: sd as f32,
            cv_percent: if mean > 0.0 {
                (sd / mean * 100.0) as f32
            } else {
                0.0
            },
            gmi_percent: (3.31 + 0.02392 * mean) as f32,
        })
    }
}

/// Builder for the time-in-range card.
///
/// Produces a fixed **640 × 400 px** RGBA image (or scaled up via
/// [`with_scale`](Self::with_scale)), styled like the other bonbon charts.
pub struct TimeInRangeBuilder<'a> {
    entries: Vec<GraphEntry>,
    thresholds: TirThresholds,
    include_extremes: bool,
    unit_display: UnitDisplay,
    title: String,
    period_label: Option<String>,
    show_footer: bool,
    theme: Theme,
    font: &'a [u8],
    scale: f32,
    #[cfg(feature = "beetroot")]
    sticker_set: Option<crate::charts::stickers::StickerSet>,
}

impl<'a> Default for TimeInRangeBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> TimeInRangeBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");
        Self {
            entries: Vec::new(),
            thresholds: TirThresholds::default(),
            include_extremes: true,
            unit_display: UnitDisplay::MgDl,
            title: "Time in Range".to_string(),
            period_label: None,
            show_footer: true,
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            scale: 1.0,
            #[cfg(feature = "beetroot")]
            sticker_set: None,
        }
    }

    /// Readings to summarize. Order does not matter.
    pub fn with_entries(mut self, entries: Vec<GraphEntry>) -> Self {
        self.entries = entries;
        self
    }

    /// In-range boundaries in mg/dL. Default: 70–180.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.thresholds.low = low;
        self.thresholds.high = high;
        self
    }

    /// Very-low / very-high boundaries in mg/dL. Default: 54 / 250.
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

    /// When `false`, very-low readings fold into Low and very-high into
    /// High, rendering a 3-band card instead of 5. Default: `true`.
    pub fn with_extremes(mut self, include: bool) -> Self {
        self.include_extremes = include;
        self
    }

    /// Unit used for displayed values (average, SD, target range).
    /// `Dual` falls back to its primary unit. Default: mg/dL.
    pub fn with_units(mut self, unit: UnitDisplay) -> Self {
        self.unit_display = unit;
        self
    }

    /// Header title. Default: `"Time in Range"`.
    pub fn with_title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = title.into();
        self
    }

    /// Header right-hand label. Default: computed from the data, e.g.
    /// `"7d · 2016 readings"`.
    pub fn with_period_label<S: Into<String>>(mut self, label: S) -> Self {
        self.period_label = Some(label.into());
        self
    }

    /// Toggles the statistics footer (average, SD, CV, GMI, target).
    /// Default: `true`.
    pub fn with_footer(mut self, show: bool) -> Self {
        self.show_footer = show;
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

    /// Multiplies all pixel dimensions by `scale`. Use `2.0` for a
    /// 1280×800 output.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Attach a [`StickerSet`](crate::charts::stickers::StickerSet) to the
    /// card. Stickers are layered behind the bar, rows and footer so the
    /// numbers remain on top. Only available with the `beetroot` feature.
    #[cfg(feature = "beetroot")]
    pub fn with_stickers(mut self, set: crate::charts::stickers::StickerSet) -> Self {
        self.sticker_set = Some(set);
        self
    }

    /// Renders and returns the card image at `640*scale × 400*scale` pixels.
    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let stats = TirStats::compute(&self.entries, &self.thresholds)
            .ok_or("at least one entry is required - call with_entries() first")?;
        let font = FontRef::try_from_slice(self.font)?;
        let s = self.scale;

        let w = (640.0 * s) as u32;
        let h = (400.0 * s) as u32;
        let mut img = RgbaImage::from_pixel(w, h, self.theme.background);

        draw_bg_pattern(&mut img, &self.theme, w, h, s);
        draw_gradient(&mut img, dominant_color(&self.theme, &stats), w, h);

        #[cfg(feature = "beetroot")]
        if let Some(set) = self.sticker_set.as_ref() {
            use crate::charts::stickers;
            let bounds = stickers::bounds_from(0.0, 0.0, w as f32, h as f32);
            stickers::draw_on_card(&mut img, set, dominant_status(&stats), bounds);
        }

        let bands = display_bands(&stats, self.include_extremes);

        let content_bottom = if self.show_footer { 296.0 } else { 360.0 };
        draw_bar(&mut img, &self.theme, &bands, s, content_bottom);
        draw_band_rows(&mut img, &self.theme, &font, &bands, s, content_bottom);
        if self.show_footer {
            draw_footer(
                &mut img,
                &self.theme,
                &font,
                &stats,
                &self.thresholds,
                self.unit_display,
                s,
            );
        }
        draw_header(
            &mut img,
            &self.theme,
            &font,
            &self.title,
            &self
                .period_label
                .unwrap_or_else(|| default_period_label(&stats)),
            w,
            s,
        );

        Ok(img)
    }
}

/// One renderable band: percentages/counts already merged when extremes are
/// folded in. Ordered top (highest glucose) to bottom (lowest).
struct DisplayBand {
    band: TirBand,
    pct: f32,
    count: usize,
    time: chrono::Duration,
}

fn display_bands(stats: &TirStats, include_extremes: bool) -> Vec<DisplayBand> {
    let take = |b: TirBand| DisplayBand {
        band: b,
        pct: stats.percentages[b as usize],
        count: stats.counts[b as usize],
        time: stats.time_per_band[b as usize],
    };
    let merge = |kept: TirBand, folded: TirBand| DisplayBand {
        band: kept,
        pct: stats.percentages[kept as usize] + stats.percentages[folded as usize],
        count: stats.counts[kept as usize] + stats.counts[folded as usize],
        time: stats.time_per_band[kept as usize] + stats.time_per_band[folded as usize],
    };

    if include_extremes {
        vec![
            take(TirBand::VeryHigh),
            take(TirBand::High),
            take(TirBand::InRange),
            take(TirBand::Low),
            take(TirBand::VeryLow),
        ]
    } else {
        vec![
            merge(TirBand::High, TirBand::VeryHigh),
            take(TirBand::InRange),
            merge(TirBand::Low, TirBand::VeryLow),
        ]
    }
}

fn band_color(theme: &Theme, band: TirBand) -> Rgba<u8> {
    match band {
        TirBand::VeryLow => darken_color(theme.glucose_low, 0.62),
        TirBand::Low => theme.glucose_low,
        TirBand::InRange => theme.glucose_in_range,
        TirBand::High => theme.glucose_high,
        TirBand::VeryHigh => darken_color(theme.glucose_high, 0.62),
    }
}

/// Color of the band group (lows / in range / highs) holding the most
/// readings, used for the ambient header gradient.
fn dominant_color(theme: &Theme, stats: &TirStats) -> Rgba<u8> {
    let lows = stats.counts[TirBand::VeryLow as usize] + stats.counts[TirBand::Low as usize];
    let highs = stats.counts[TirBand::VeryHigh as usize] + stats.counts[TirBand::High as usize];
    let in_range = stats.counts[TirBand::InRange as usize];
    if in_range >= lows && in_range >= highs {
        theme.glucose_in_range
    } else if highs >= lows {
        theme.glucose_high
    } else {
        theme.glucose_low
    }
}

#[cfg(feature = "beetroot")]
fn dominant_status(stats: &TirStats) -> crate::charts::bg_card::GlucoseStatus {
    use crate::charts::bg_card::GlucoseStatus;
    let lows = stats.counts[TirBand::VeryLow as usize] + stats.counts[TirBand::Low as usize];
    let highs = stats.counts[TirBand::VeryHigh as usize] + stats.counts[TirBand::High as usize];
    let in_range = stats.counts[TirBand::InRange as usize];
    if in_range >= lows && in_range >= highs {
        GlucoseStatus::InRange
    } else if highs >= lows {
        GlucoseStatus::High
    } else {
        GlucoseStatus::Low
    }
}

/// Subtle grid pattern drawn over the background before any content.
/// Mirrors the bg card so the family looks related.
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

/// Ambient gradient fading from the top, tinted by the dominant band.
fn draw_gradient(img: &mut RgbaImage, c: Rgba<u8>, w: u32, h: u32) {
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

fn draw_header(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    title: &str,
    period: &str,
    w: u32,
    s: f32,
) {
    let cy = 23.0 * s;
    let pad = 24.0 * s;
    let font_title = 22.0 * s;
    let font_period = 16.0 * s;

    draw_text_mut(
        img,
        theme.text_primary,
        pad as i32,
        (cy - font_title / 2.0) as i32,
        PxScale::from(font_title),
        font,
        title,
    );

    let tw = text_w(font, period, font_period);
    draw_text_mut(
        img,
        theme.text_secondary,
        (w as f32 - pad - tw) as i32,
        (cy - font_period / 2.0) as i32,
        PxScale::from(font_period),
        font,
        period,
    );
}

fn default_period_label(stats: &TirStats) -> String {
    let readings = if stats.total == 1 {
        "1 reading".to_string()
    } else {
        format!("{} readings", stats.total)
    };
    let mins = stats.span.num_minutes();
    if mins >= 1440 {
        let days = ((mins as f64) / 1440.0).round().max(1.0) as i64;
        format!("{days}d · {readings}")
    } else if mins >= 60 {
        format!("{}h · {readings}", mins / 60)
    } else {
        readings
    }
}

/// The vertical stacked bar on the left. Highest band on top, lowest at the
/// bottom (same orientation as the glucose graph's Y axis).
fn draw_bar(img: &mut RgbaImage, theme: &Theme, bands: &[DisplayBand], s: f32, bottom: f32) {
    let bar_x = 28.0 * s;
    let bar_w = 52.0 * s;
    let bar_top = 64.0 * s;
    let bar_bottom = bottom * s;
    let bar_h = bar_bottom - bar_top;
    let gap = 4.0 * s;
    let radius = (7.0 * s) as i32;
    let min_h = 10.0 * s;

    let [tr, tg, tb, _] = theme.grid_major.0;
    draw_filled_rounded_rect(
        img,
        (bar_x - 5.0 * s) as i32,
        (bar_top - 5.0 * s) as i32,
        (bar_w + 10.0 * s) as u32,
        (bar_h + 10.0 * s) as u32,
        (11.0 * s) as i32,
        Rgba([tr, tg, tb, 130]),
    );

    let visible: Vec<&DisplayBand> = bands.iter().filter(|b| b.count > 0).collect();
    if visible.is_empty() {
        return;
    }
    let avail = bar_h - gap * (visible.len() as f32 - 1.0);
    let total_pct: f32 = visible.iter().map(|b| b.pct).sum();
    let mut heights: Vec<f32> = visible
        .iter()
        .map(|b| (b.pct / total_pct * avail).max(min_h))
        .collect();
    let sum: f32 = heights.iter().sum();
    if sum > avail {
        let excess = sum - avail;
        let flexible: f32 = heights.iter().map(|&h| (h - min_h).max(0.0)).sum();
        if flexible > 0.0 {
            for h in &mut heights {
                *h -= (*h - min_h).max(0.0) / flexible * excess;
            }
        }
    }

    let mut y = bar_top;
    for (b, seg_h) in visible.iter().zip(heights) {
        draw_filled_rounded_rect(
            img,
            bar_x as i32,
            y as i32,
            bar_w as u32,
            seg_h as u32,
            radius,
            band_color(theme, b.band),
        );
        y += seg_h + gap;
    }
}

/// One row per band beside the bar: swatch, label, duration + reading count,
/// and a big percentage on the right colored like its segment.
fn draw_band_rows(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    bands: &[DisplayBand],
    s: f32,
    bottom: f32,
) {
    let rows_left = 108.0 * s;
    let rows_right = 616.0 * s;
    let rows_top = 64.0 * s;
    let rows_bottom = bottom * s;
    let row_h = (rows_bottom - rows_top) / bands.len() as f32;

    let font_label = 17.0 * s;
    let font_sub = 13.0 * s;
    let swatch = 14.0 * s;

    for (i, b) in bands.iter().enumerate() {
        let cy = rows_top + (i as f32 + 0.5) * row_h;
        let color = band_color(theme, b.band);

        draw_filled_rounded_rect(
            img,
            rows_left as i32,
            (cy - 17.0 * s + (font_label - swatch) / 2.0) as i32,
            swatch as u32,
            swatch as u32,
            (4.0 * s) as i32,
            color,
        );

        draw_text_mut(
            img,
            theme.text_primary,
            (rows_left + swatch + 10.0 * s) as i32,
            (cy - 17.0 * s) as i32,
            PxScale::from(font_label),
            font,
            b.band.label(),
        );

        let readings = if b.count == 1 { "reading" } else { "readings" };
        let sub = format!(
            "{} · {} {readings}",
            fmt_duration(b.time.num_minutes()),
            b.count
        );
        draw_text_mut(
            img,
            theme.text_dim,
            (rows_left + swatch + 10.0 * s) as i32,
            (cy + 4.0 * s) as i32,
            PxScale::from(font_sub),
            font,
            &sub,
        );

        let font_pct = if b.band == TirBand::InRange {
            32.0 * s
        } else {
            24.0 * s
        };
        let pct_str = fmt_pct(b.pct);
        let pct_color = if b.count == 0 { theme.text_dim } else { color };
        let tw = text_w(font, &pct_str, font_pct);
        draw_text_mut(
            img,
            pct_color,
            (rows_right - tw) as i32,
            (cy - font_pct / 2.0) as i32,
            PxScale::from(font_pct),
            font,
            &pct_str,
        );
    }
}

fn draw_footer(
    img: &mut RgbaImage,
    theme: &Theme,
    font: &FontRef,
    stats: &TirStats,
    thresholds: &TirThresholds,
    unit: UnitDisplay,
    s: f32,
) {
    let pad = 24.0 * s;
    let w = img.width() as f32;
    let divider_y = 314.0 * s;
    let label_y = 330.0 * s;
    let value_y = 348.0 * s;
    let font_label = 12.0 * s;
    let font_value = 21.0 * s;

    draw_fast_rect(
        img,
        pad as i32,
        divider_y as i32,
        (w - 2.0 * pad) as u32,
        (1.5 * s).max(1.0) as u32,
        theme.grid_major,
    );

    let pref = unit_preference(unit);
    let blocks: [(&str, String); 5] = [
        ("AVG", fmt_glucose(stats.mean_mgdl, pref, true)),
        ("SD", fmt_glucose(stats.sd_mgdl, pref, false)),
        ("CV", format!("{:.1}%", stats.cv_percent)),
        ("GMI", format!("{:.1}%", stats.gmi_percent)),
        (
            "TARGET",
            format!(
                "{}-{}",
                fmt_glucose(thresholds.low, pref, false),
                fmt_glucose(thresholds.high, pref, false)
            ),
        ),
    ];

    let block_w = (w - 2.0 * pad) / blocks.len() as f32;
    for (i, (label, value)) in blocks.iter().enumerate() {
        let x = (pad + i as f32 * block_w) as i32;
        draw_text_mut(
            img,
            theme.text_dim,
            x,
            label_y as i32,
            PxScale::from(font_label),
            font,
            label,
        );
        draw_text_mut(
            img,
            theme.text_primary,
            x,
            value_y as i32,
            PxScale::from(font_value),
            font,
            value,
        );
    }
}

fn unit_preference(unit: UnitDisplay) -> UnitPreference {
    match unit {
        UnitDisplay::MgDl => UnitPreference::MgDl,
        UnitDisplay::MmolL => UnitPreference::MmolL,
        UnitDisplay::Dual { primary } => primary,
    }
}

/// Formats a mg/dL value in the preferred unit, optionally with the unit
/// label appended.
fn fmt_glucose(mgdl: f32, pref: UnitPreference, with_unit: bool) -> String {
    match (pref, with_unit) {
        (UnitPreference::MgDl, true) => format!("{mgdl:.0} mg/dl"),
        (UnitPreference::MgDl, false) => format!("{mgdl:.0}"),
        (UnitPreference::MmolL, true) => format!("{:.1} mmol/L", mgdl / MGDL_PER_MMOL),
        (UnitPreference::MmolL, false) => format!("{:.1}", mgdl / MGDL_PER_MMOL),
    }
}

fn fmt_pct(p: f32) -> String {
    if p <= 0.0 {
        "0%".to_string()
    } else if p < 1.0 {
        "<1%".to_string()
    } else {
        format!("{p:.0}%")
    }
}

fn fmt_duration(mins: i64) -> String {
    if mins >= 1440 {
        format!("{}d {}h", mins / 1440, (mins % 1440) / 60)
    } else if mins >= 60 {
        format!("{}h {:02}m", mins / 60, mins % 60)
    } else {
        format!("{mins}m")
    }
}
