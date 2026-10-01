//! The breakdown graph: time in range for each hour of the day or each day of
//! the week, and how much it changed from a previous period.

use ab_glyph::{FontRef, PxScale};
use chrono::{DateTime, Datelike, Timelike, Utc};
use chrono_tz::Tz;
use image::{Rgba, RgbaImage};

use crate::charts::compare::{dates_label, NightTargets};
use crate::charts::glucose::LayoutConfig;
use crate::charts::time_in_range::{band_color, TirBand, TirThresholds};
use crate::models::{GraphEntry, UnitDisplay, UnitPreference};
use crate::theme::Theme;
use crate::utils::axis::{fmt_glucose, unit_name, units};
use crate::utils::drawing::{blend_pixel, canvas_with_rounded_rects, draw_filled_rounded_rect};
use crate::utils::text::{draw_text, draw_text_right, text_w};

/// How readings are split into columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    /// 24 columns, one per hour of the day.
    Hour,
    /// 7 columns, Monday to Sunday.
    Weekday,
}

impl Grouping {
    fn columns(self) -> usize {
        match self {
            Grouping::Hour => 24,
            Grouping::Weekday => 7,
        }
    }

    fn label(self, column: usize) -> String {
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        match self {
            Grouping::Hour => format!("{column:02}h"),
            Grouping::Weekday => DAYS[column].to_string(),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Grouping::Hour => "Time in range by hour",
            Grouping::Weekday => "Time in range by day",
        }
    }
}

/// Readings per band, indexed by `TirBand as usize`.
#[derive(Debug, Clone, Copy, Default)]
struct Counts([usize; 5]);

impl Counts {
    fn total(&self) -> usize {
        self.0.iter().sum()
    }

    /// Whole-percent time in range, or `None` without readings.
    fn in_range(&self) -> Option<i32> {
        let total = self.total();
        (total > 0).then(|| {
            (self.0[TirBand::InRange as usize] as f64 * 100.0 / total as f64).round() as i32
        })
    }
}

/// One period split into columns.
struct Split {
    columns: Vec<Counts>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

/// Builder for the breakdown graph.
///
/// One column per hour of the day or per day of the week (see [`Grouping`]),
/// each with a bar stacked from very low to very high and its time in range
/// underneath. Given the period before (see
/// [`with_previous`](Self::with_previous)), every column also gets a chip
/// with how its time in range changed. The target range the columns are
/// measured against is shown top right, in the display unit.
pub struct BreakdownGraphBuilder<'a> {
    entries: Vec<GraphEntry>,
    previous: Option<Vec<GraphEntry>>,
    grouping: Grouping,
    thresholds: TirThresholds,
    night: Option<NightTargets>,
    unit_display: UnitDisplay,
    timezone: Tz,
    layout: LayoutConfig,
    theme: Theme,
    font: &'a [u8],
    title: Option<String>,
}

impl<'a> Default for BreakdownGraphBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> BreakdownGraphBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

        Self {
            entries: Vec::new(),
            previous: None,
            grouping: Grouping::Hour,
            thresholds: TirThresholds::default(),
            night: None,
            unit_display: UnitDisplay::MgDl,
            timezone: chrono_tz::UTC,
            layout: LayoutConfig::default(),
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            title: None,
        }
    }

    /// Sets the readings to break down.
    pub fn with_entries<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
    {
        self.entries = entries.into_iter().map(|e| e.into()).collect();
        self
    }

    /// Sets the readings of the period before, to show how time in range
    /// changed.
    pub fn with_previous<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
    {
        self.previous = Some(entries.into_iter().map(|e| e.into()).collect());
        self
    }

    /// Sets whether columns are hours of the day or days of the week.
    /// Default: hours.
    pub fn with_grouping(mut self, grouping: Grouping) -> Self {
        self.grouping = grouping;
        self
    }

    /// Sets the low and high targets in mg/dL. Default: 70 and 180.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.thresholds.low = low;
        self.thresholds.high = high;
        self
    }

    /// Sets the very low and very high limits in mg/dL. Default: 54 and 250.
    pub fn with_extreme_targets(mut self, very_low: f32, very_high: f32) -> Self {
        self.thresholds.very_low = very_low;
        self.thresholds.very_high = very_high;
        self
    }

    /// Uses other low and high targets overnight. By hour, the night's
    /// columns are shaded.
    pub fn with_night_targets(mut self, night: NightTargets) -> Self {
        self.night = Some(night);
        self
    }

    /// Sets the unit the target range is shown in, top right. `Dual` repeats
    /// it in the other unit underneath. Default: mg/dL.
    pub fn with_units(mut self, display: UnitDisplay) -> Self {
        self.unit_display = display;
        self
    }

    /// Sets the timezone whose clock and calendar split the readings.
    pub fn with_timezone(mut self, tz: Tz) -> Self {
        self.timezone = tz;
        self
    }

    /// Sets the image size. Margins set the space around the panel.
    pub fn with_layout(mut self, layout: LayoutConfig) -> Self {
        self.layout = layout;
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

    /// Sets the title. Default: "Time in range by hour" or "by day".
    pub fn with_title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Renders the graph.
    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let font = FontRef::try_from_slice(self.font)?;
        let current = self
            .split(&self.entries)
            .ok_or("at least one entry is required - call with_entries() first")?;
        let previous = self.previous.as_deref().and_then(|p| self.split(p));

        let l = self.layout();
        let mut img = canvas_with_rounded_rects(
            self.layout.width,
            self.layout.height,
            self.theme.background,
            &[l.panel],
            (18.0 * l.s) as i32,
            self.panel_color(),
        );

        self.draw_header(&mut img, &l, &font, &current, previous.as_ref());
        for column in 0..self.grouping.columns() {
            let before = previous.as_ref().map(|p| &p.columns[column]);
            self.draw_column(
                &mut img,
                &l,
                &font,
                column,
                &current.columns[column],
                before,
            );
        }
        self.draw_legend(&mut img, &l, &font);
        Ok(img)
    }

    /// The thresholds in force at a minute of the day.
    fn thresholds_at(&self, minute: f32) -> TirThresholds {
        match self.night {
            Some(night) if night.covers(minute) => TirThresholds {
                low: night.low,
                high: night.high,
                ..self.thresholds
            },
            _ => self.thresholds,
        }
    }

    /// Counts `entries` per column and band, or `None` without readings.
    fn split(&self, entries: &[GraphEntry]) -> Option<Split> {
        let readings: Vec<&GraphEntry> = entries.iter().filter(|e| e.sgv.is_finite()).collect();
        let from = readings.iter().map(|e| e.date).min()?;
        let to = readings.iter().map(|e| e.date).max()?;
        let mut columns = vec![Counts::default(); self.grouping.columns()];
        for e in readings {
            let local = e.date.with_timezone(&self.timezone);
            let minute = (local.hour() * 60 + local.minute()) as f32;
            let band = self.thresholds_at(minute).classify(e.sgv) as usize;
            let column = match self.grouping {
                Grouping::Hour => local.hour() as usize,
                Grouping::Weekday => local.weekday().num_days_from_monday() as usize,
            };
            columns[column].0[band] += 1;
        }
        Some(Split { columns, from, to })
    }

    /// Where everything goes. `s` scales a 1200×800 design, like the other
    /// graphs.
    fn layout(&self) -> Layout {
        let (w, h) = (self.layout.width as f32, self.layout.height as f32);
        let s = (w / 1200.0).min(h / 800.0).max(0.5);
        let margin = |m: Option<f32>| m.unwrap_or(24.0 * s);
        let (m_top, m_bottom) = (
            margin(self.layout.margin_top),
            margin(self.layout.margin_bottom),
        );
        let (m_left, m_right) = (
            margin(self.layout.margin_left),
            margin(self.layout.margin_right),
        );
        let pad = 32.0 * s;

        let left = m_left + pad;
        let right = w - m_right - pad;
        let top = m_top + pad;
        let bottom = h - m_bottom - pad;

        let divider = top + 78.0 * s;
        let label_top = divider + 30.0 * s;
        let legend_top = bottom - 20.0 * s;
        let chips = self.previous.is_some();
        let chip_top = legend_top - 34.0 * s - 26.0 * s;
        let value_top = if chips {
            chip_top - 12.0 * s - 28.0 * s
        } else {
            legend_top - 34.0 * s - 28.0 * s
        };

        Layout {
            s,
            panel: (
                m_left as i32,
                m_top as i32,
                (w - m_left - m_right).max(1.0) as u32,
                (h - m_top - m_bottom).max(1.0) as u32,
            ),
            left,
            right,
            top,
            divider,
            label_top,
            bar_top: label_top + 21.0 * s + 18.0 * s,
            bar_bottom: value_top - 18.0 * s,
            value_top,
            chip_top,
            legend_top,
        }
    }

    /// The opaque color of the panel: `grid_major` tinted over the
    /// background, like the other graphs' panels.
    fn panel_color(&self) -> Rgba<u8> {
        let [gr, gg, gb, _] = self.theme.grid_major.0;
        blend_pixel(self.theme.background, Rgba([gr, gg, gb, 110]))
    }

    /// A change chip's colors: in range when time in range went up, high when
    /// it went down, neutral when it held.
    fn chip_color(&self, delta: i32) -> Rgba<u8> {
        match delta.signum() {
            1 => self.theme.glucose_in_range,
            -1 => self.theme.glucose_high,
            _ => self.theme.text_secondary,
        }
    }

    /// A pill with `text` at `size` and a soft tint of `color`, centered on
    /// (`cx`, `cy`).
    #[allow(clippy::too_many_arguments)]
    fn draw_chip(
        &self,
        img: &mut RgbaImage,
        font: &FontRef,
        size: f32,
        cx: f32,
        cy: f32,
        text: &str,
        color: Rgba<u8>,
    ) {
        let (h, pad) = (size * 1.45, size * 0.5);
        let top = cy - h / 2.0;
        let w = text_w(font, text, size) + 2.0 * pad;
        let x = cx - w / 2.0;
        draw_filled_rounded_rect(
            img,
            x as i32,
            top as i32,
            w as u32,
            h as u32,
            (h / 2.0) as i32,
            Rgba([color[0], color[1], color[2], 48]),
        );
        draw_text(
            img,
            color,
            (x + pad) as i32,
            (top + (h - size) / 2.0 - size * 0.05) as i32,
            PxScale::from(size),
            font,
            text,
        );
    }

    /// Title and dates on the left and the target range on the right, over a
    /// divider.
    fn draw_header(
        &self,
        img: &mut RgbaImage,
        l: &Layout,
        font: &FontRef,
        current: &Split,
        previous: Option<&Split>,
    ) {
        let s = l.s;
        let (size_title, size_sub) = (34.0 * s, 20.0 * s);
        let title = self
            .title
            .clone()
            .unwrap_or_else(|| self.grouping.title().to_string());
        draw_text(
            img,
            self.theme.text_primary,
            l.left as i32,
            l.top as i32,
            PxScale::from(size_title),
            font,
            &title,
        );
        let mut dates = dates_label(current.from, current.to, self.timezone);
        if let Some(p) = previous {
            dates = format!("{dates} vs {}", dates_label(p.from, p.to, self.timezone));
        }
        let dates_top = l.top + size_title + 10.0 * s;
        draw_text(
            img,
            self.theme.text_secondary,
            l.left as i32,
            dates_top as i32,
            PxScale::from(size_sub),
            font,
            &dates,
        );

        // The day's target range, level with the title, and in the second
        // unit level with the dates. Left out where it would run into them.
        let (pref, second) = units(self.unit_display);
        let range = |unit: UnitPreference| {
            format!(
                "{}–{} {}",
                fmt_glucose(self.thresholds.low, unit),
                fmt_glucose(self.thresholds.high, unit),
                unit_name(unit)
            )
        };
        let clear_of = |text: &str, size: f32, left_text: &str, left_size: f32| {
            l.right - text_w(font, text, size)
                >= l.left + text_w(font, left_text, left_size) + 32.0 * s
        };
        let target = format!("Target {}", range(pref));
        let size_target = 22.0 * s;
        if clear_of(&target, size_target, &title, size_title) {
            draw_text_right(
                img,
                self.theme.text_secondary,
                font,
                size_target,
                l.right,
                l.top + (size_title - size_target) * 0.75,
                &target,
            );
            if let Some(text) = second
                .map(range)
                .filter(|t| clear_of(t, size_sub, &dates, size_sub))
            {
                draw_text_right(
                    img,
                    self.theme.text_dim,
                    font,
                    size_sub,
                    l.right,
                    dates_top,
                    &text,
                );
            }
        }

        let [r, g, b, _] = self.theme.axis_lines.0;
        draw_filled_rounded_rect(
            img,
            l.left as i32,
            l.divider as i32,
            (l.right - l.left) as u32,
            s.ceil() as u32,
            0,
            Rgba([r, g, b, 40]),
        );
    }

    /// One column: its label, its stacked bar, its time in range and, with a
    /// previous period, its change chip. Night hours get a faint backdrop.
    fn draw_column(
        &self,
        img: &mut RgbaImage,
        l: &Layout,
        font: &FontRef,
        column: usize,
        counts: &Counts,
        before: Option<&Counts>,
    ) {
        let s = l.s;
        let width = (l.right - l.left) / self.grouping.columns() as f32;
        let cx = l.left + (column as f32 + 0.5) * width;

        let is_night = self.grouping == Grouping::Hour
            && self
                .night
                .is_some_and(|n| n.covers(column as f32 * 60.0 + 30.0));
        if is_night {
            let [r, g, b, _] = self.theme.axis_lines.0;
            let bottom = if before.is_some() {
                l.chip_top + 26.0 * s
            } else {
                l.value_top + 28.0 * s
            };
            draw_filled_rounded_rect(
                img,
                (cx - width / 2.0 + 3.0 * s) as i32,
                (l.label_top - 10.0 * s) as i32,
                (width - 6.0 * s) as u32,
                (bottom - l.label_top + 20.0 * s) as u32,
                (12.0 * s) as i32,
                Rgba([r, g, b, 12]),
            );
        }

        // Column label.
        let size_label = 21.0 * s;
        let label = self.grouping.label(column);
        draw_text(
            img,
            self.theme.text_secondary,
            (cx - text_w(font, &label, size_label) / 2.0) as i32,
            l.label_top as i32,
            PxScale::from(size_label),
            font,
            &label,
        );

        let bar_w = (width * 0.5).min(64.0 * s).max(4.0);
        self.draw_bar(img, s, cx, bar_w, l.bar_top, l.bar_bottom, counts);

        // Time in range, then its change, shrunk only as far as needed for
        // "100%" and a "-20%" chip to fit a column, even with 24 of them.
        let size_value = (28.0 * s).min(fit(font, "100%", 0.0, width * 0.88));
        let size_chip = (18.0 * s).min(fit(font, "-20%", 1.0, width * 0.9));
        let now = counts.in_range();
        let value = now.map_or("–".to_string(), |p| format!("{p}%"));
        draw_text(
            img,
            self.theme.text_primary,
            (cx - text_w(font, &value, size_value) / 2.0) as i32,
            (l.value_top + (28.0 * s - size_value) / 2.0) as i32,
            PxScale::from(size_value),
            font,
            &value,
        );
        if let (Some(now), Some(was)) = (now, before.and_then(Counts::in_range)) {
            let delta = now - was;
            self.draw_chip(
                img,
                font,
                size_chip,
                cx,
                l.chip_top + 13.0 * s,
                &signed_percent(delta),
                self.chip_color(delta),
            );
        }
    }

    /// A bar of rounded tiles stacked from very low (bottom) to very high,
    /// each as tall as its share of the readings. Small shares keep a minimum
    /// height so they stay visible, taken from the largest.
    #[allow(clippy::too_many_arguments)]
    fn draw_bar(
        &self,
        img: &mut RgbaImage,
        s: f32,
        cx: f32,
        bar_w: f32,
        top: f32,
        bottom: f32,
        counts: &Counts,
    ) {
        let x = (cx - bar_w / 2.0).round() as i32;
        let radius = (6.0 * s).min(bar_w / 2.0);
        let total = counts.total();
        if total == 0 {
            let c = self.theme.text_secondary;
            draw_filled_rounded_rect(
                img,
                x,
                top as i32,
                bar_w as u32,
                (bottom - top) as u32,
                radius as i32,
                Rgba([c[0], c[1], c[2], 25]),
            );
            return;
        }

        let bands: Vec<(TirBand, usize)> = TirBand::ALL
            .into_iter()
            .map(|b| (b, counts.0[b as usize]))
            .filter(|&(_, n)| n > 0)
            .collect();
        let gap = (3.0 * s).round().max(1.0);
        let room = bottom - top - gap * (bands.len() - 1) as f32;
        let min_h = 4.0 * s;
        let mut heights: Vec<f32> = bands
            .iter()
            .map(|&(_, n)| (n as f32 / total as f32 * room).max(min_h))
            .collect();
        let excess = heights.iter().sum::<f32>() - room;
        if let Some(largest) = heights.iter_mut().max_by(|a, b| a.total_cmp(b)) {
            *largest -= excess;
        }

        let mut y = bottom;
        for (&(band, _), h) in bands.iter().zip(heights) {
            let (y1, y0) = (y.round(), (y - h).round());
            draw_filled_rounded_rect(
                img,
                x,
                y0 as i32,
                bar_w as u32,
                (y1 - y0).max(1.0) as u32,
                radius.min((y1 - y0) / 2.0) as i32,
                band_color(&self.theme, band),
            );
            y -= h + gap;
        }
    }

    /// The bands' colors along the bottom, with the night hours when shaded.
    fn draw_legend(&self, img: &mut RgbaImage, l: &Layout, font: &FontRef) {
        let s = l.s;
        let size = 19.0 * s;
        let swatch = 14.0 * s;
        let mut items: Vec<(Rgba<u8>, String)> = [
            (TirBand::VeryHigh, "Very high"),
            (TirBand::High, "High"),
            (TirBand::InRange, "In range"),
            (TirBand::Low, "Low"),
            (TirBand::VeryLow, "Very low"),
        ]
        .into_iter()
        .map(|(b, name)| (band_color(&self.theme, b), name.to_string()))
        .collect();
        if let (Grouping::Hour, Some(night)) = (self.grouping, self.night) {
            let [r, g, b, _] = self.theme.axis_lines.0;
            items.push((
                blend_pixel(self.panel_color(), Rgba([r, g, b, 40])),
                format!(
                    "Night {:02}:{:02}–{:02}:{:02}",
                    night.from.hour(),
                    night.from.minute(),
                    night.until.hour(),
                    night.until.minute()
                ),
            ));
        }

        let mut x = l.left;
        for (color, label) in items {
            draw_filled_rounded_rect(
                img,
                x as i32,
                (l.legend_top + (size - swatch) / 2.0) as i32,
                swatch as u32,
                swatch as u32,
                (4.0 * s) as i32,
                color,
            );
            x += swatch + 8.0 * s;
            draw_text(
                img,
                self.theme.text_secondary,
                x as i32,
                (l.legend_top - 1.0 * s) as i32,
                PxScale::from(size),
                font,
                &label,
            );
            x += text_w(font, &label, size) + 26.0 * s;
        }
    }
}

/// Where everything goes, in pixels.
struct Layout {
    s: f32,
    panel: (i32, i32, u32, u32),
    left: f32,
    right: f32,
    top: f32,
    divider: f32,
    label_top: f32,
    bar_top: f32,
    bar_bottom: f32,
    value_top: f32,
    chip_top: f32,
    legend_top: f32,
}

/// The font size at which `text`, plus `pad` font sizes of padding, is
/// `width` wide. Text width grows linearly with size.
fn fit(font: &FontRef, text: &str, pad: f32, width: f32) -> f32 {
    width / (text_w(font, text, 1.0) + pad)
}

/// "+5%", "-20%" or "±0%".
fn signed_percent(delta: i32) -> String {
    match delta.signum() {
        1 => format!("+{delta}%"),
        -1 => format!("{delta}%"),
        _ => "±0%".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveTime, TimeZone};

    fn entry(date: DateTime<Utc>, sgv: f32) -> GraphEntry {
        GraphEntry { sgv, date }
    }

    fn night() -> NightTargets {
        NightTargets {
            low: 90.0,
            high: 180.0,
            from: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
            until: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
        }
    }

    /// Two weeks of hourly readings, in range except `low_hour` every day.
    fn fortnight(low_hour: u32) -> Vec<GraphEntry> {
        let start = Utc.with_ymd_and_hms(2026, 9, 7, 0, 0, 0).unwrap(); // a Monday
        (0..14 * 24)
            .map(|h| {
                let sgv = if h % 24 == low_hour as i64 {
                    60.0
                } else {
                    120.0
                };
                entry(start + Duration::hours(h), sgv)
            })
            .collect()
    }

    #[test]
    fn changes_are_signed_percentages() {
        assert_eq!(signed_percent(5), "+5%");
        assert_eq!(signed_percent(-20), "-20%");
        assert_eq!(signed_percent(0), "±0%");
    }

    #[test]
    fn time_in_range_rounds_to_whole_percents() {
        let counts = Counts([0, 1, 2, 0, 0]);
        assert_eq!(counts.in_range(), Some(67));
        assert_eq!(Counts::default().in_range(), None);
    }

    #[test]
    fn readings_split_by_local_hour_and_weekday() {
        let b = BreakdownGraphBuilder::new().with_timezone(chrono_tz::UTC);
        let split = b.split(&fortnight(3)).unwrap();
        assert_eq!(split.columns.len(), 24);
        assert_eq!(split.columns[3].in_range(), Some(0));
        assert_eq!(split.columns[4].in_range(), Some(100));
        assert_eq!(
            split.columns.iter().map(Counts::total).sum::<usize>(),
            14 * 24
        );

        // 03:00 UTC is 05:00 in Paris in September.
        let paris = BreakdownGraphBuilder::new().with_timezone(chrono_tz::Europe::Paris);
        assert_eq!(
            paris.split(&fortnight(3)).unwrap().columns[5].in_range(),
            Some(0)
        );

        let weekly = b.with_grouping(Grouping::Weekday);
        let split = weekly.split(&fortnight(3)).unwrap();
        assert_eq!(split.columns.len(), 7);
        assert!(split.columns.iter().all(|c| c.total() == 2 * 24));
        // 2026-09-07 is a Monday.
        let monday_only = vec![entry(
            Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).unwrap(),
            100.0,
        )];
        assert_eq!(weekly.split(&monday_only).unwrap().columns[0].total(), 1);
    }

    #[test]
    fn night_targets_apply_overnight_only() {
        let b = BreakdownGraphBuilder::new()
            .with_timezone(chrono_tz::UTC)
            .with_targets(80.0, 180.0)
            .with_night_targets(night());
        let at = |h| Utc.with_ymd_and_hms(2026, 9, 7, h, 0, 0).unwrap();
        let split = b.split(&[entry(at(3), 85.0), entry(at(12), 85.0)]).unwrap();
        assert_eq!(split.columns[3].0[TirBand::Low as usize], 1);
        assert_eq!(split.columns[12].0[TirBand::InRange as usize], 1);
    }

    #[test]
    fn text_is_fitted_to_its_width() {
        let font =
            FontRef::try_from_slice(include_bytes!("../../assets/fonts/GeistMono-Regular.ttf"))
                .unwrap();
        let size = fit(&font, "100%", 0.0, 80.0);
        assert!((text_w(&font, "100%", size) - 80.0).abs() < 0.5);
        let size = fit(&font, "-20%", 1.0, 80.0);
        assert!((text_w(&font, "-20%", size) + size - 80.0).abs() < 0.5);
    }

    #[test]
    fn builds_for_every_setup() {
        let setups = vec![
            BreakdownGraphBuilder::new(),
            BreakdownGraphBuilder::new().with_grouping(Grouping::Weekday),
            BreakdownGraphBuilder::new()
                .with_previous(fortnight(5))
                .with_night_targets(night())
                .with_theme(Theme::paper_light()),
            BreakdownGraphBuilder::new()
                .with_grouping(Grouping::Weekday)
                .with_previous(fortnight(5))
                .with_title("Par jour"),
            BreakdownGraphBuilder::new()
                .with_previous(fortnight(5))
                .with_layout(LayoutConfig {
                    width: 480,
                    height: 270,
                    ..Default::default()
                }),
            // No previous readings: no chips, still builds.
            BreakdownGraphBuilder::new().with_previous(Vec::<GraphEntry>::new()),
        ];
        for b in setups {
            let img = b.with_entries(fortnight(3)).build().unwrap();
            assert!(img.width() > 0);
        }
    }

    #[test]
    fn hours_without_readings_still_build() {
        let only_noon: Vec<_> = fortnight(3)
            .into_iter()
            .filter(|e| e.date.hour() == 12)
            .collect();
        assert!(BreakdownGraphBuilder::new()
            .with_entries(only_noon)
            .build()
            .is_ok());
    }

    #[test]
    fn no_readings_is_an_error() {
        assert!(BreakdownGraphBuilder::new().build().is_err());
    }
}
