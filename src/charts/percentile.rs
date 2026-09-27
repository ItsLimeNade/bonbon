//! The percentile graph: how glucose typically runs through the day, from
//! readings spread over any number of days (an Ambulatory Glucose Profile).

use ab_glyph::{FontRef, PxScale};
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use image::{Rgba, RgbaImage};

use crate::charts::glucose::LayoutConfig;
use crate::models::{GraphEntry, GraphScaling, UnitDisplay};
use crate::theme::Theme;
use crate::utils::axis::{
    fmt_glucose, other_unit, round_step, round_ticks, unit_name, unit_preference,
};
use crate::utils::drawing::{
    blend_fast_rect, blend_pixel, blend_sprite, canvas_with_rounded_rects, create_aa_circle_sprite,
    draw_dashed_horizontal_line, draw_filled_rounded_rect, Sprite,
};
use crate::utils::text::{draw_text, draw_text_right, text_w};

/// Gap (before scaling) between the plot area and the rounded panel that
/// frames it, as on the glucose graph.
const PANEL_PAD: f32 = 16.0;

const MINUTES_PER_DAY: u32 = 24 * 60;

/// The day is cut into slots this many minutes apart.
const SLOT_MINUTES: u32 = 5;
const SLOTS: usize = (MINUTES_PER_DAY / SLOT_MINUTES) as usize;

/// A slot's percentiles come from every reading within this many minutes of
/// it, on any day, which smooths the curves the way AGP reports do.
const WINDOW_MINUTES: i32 = 30;

/// Slots with fewer readings than this near them are left blank.
const MIN_READINGS: usize = 5;

/// Triangular weights the curves are finally smoothed with, over the slots
/// around each one: a reading entering or leaving a slot's window otherwise
/// shows as a step.
const SMOOTHING: [f32; 7] = [1.0, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0];

/// Band opacities (out of 255), over the panel.
const INNER_ALPHA: f32 = 110.0;
const OUTER_ALPHA: f32 = 42.0;

/// Which percentiles the graph shows, each from 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PercentileBands {
    /// The strong band around the median. Default: 25th to 75th.
    pub inner: (f32, f32),
    /// The light band around the inner one, or `None` for none. Default:
    /// 5th to 95th.
    pub outer: Option<(f32, f32)>,
    /// Whether to draw the median line. Default: `true`.
    pub median: bool,
}

impl Default for PercentileBands {
    fn default() -> Self {
        Self {
            inner: (25.0, 75.0),
            outer: Some((5.0, 95.0)),
            median: true,
        }
    }
}

impl PercentileBands {
    /// Every percentile the bands need, lower bounds first.
    fn percentiles(&self) -> Vec<f32> {
        let mut all = vec![self.inner.0, self.inner.1];
        if let Some((lo, hi)) = self.outer {
            all.extend([lo, hi]);
        }
        if self.median {
            all.push(50.0);
        }
        all
    }
}

/// Glucose percentiles through a typical day, computed from readings over
/// any number of days. Public so the numbers can be reused without rendering
/// an image.
///
/// The day is cut into 5-minute slots, and each slot's percentiles come from
/// every reading within 30 minutes of it on any day (wrapping around
/// midnight), in the given timezone.
#[derive(Debug, Clone)]
pub struct PercentileProfile {
    /// Minute of the day at each slot: 0, 5, …, 1435.
    pub minutes: Vec<u32>,
    /// The percentiles computed, in the order asked for.
    pub percentiles: Vec<f32>,
    /// Per percentile (same order as `percentiles`), its value in mg/dL at
    /// each slot, or `None` where fewer than 5 readings fall near the slot.
    pub curves: Vec<Vec<Option<f32>>>,
    /// Number of readings used.
    pub readings: usize,
    /// Oldest and newest reading.
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl PercentileProfile {
    /// Computes the `percentiles` (each 0–100) of `entries` through the day
    /// in `timezone`. Returns `None` when there are no readings. Order does
    /// not matter.
    pub fn compute(entries: &[GraphEntry], timezone: Tz, percentiles: &[f32]) -> Option<Self> {
        let readings: Vec<&GraphEntry> = entries.iter().filter(|e| e.sgv.is_finite()).collect();
        let from = readings.iter().map(|e| e.date).min()?;
        let to = readings.iter().map(|e| e.date).max()?;

        let mut by_minute = vec![Vec::new(); MINUTES_PER_DAY as usize];
        for e in &readings {
            let local = e.date.with_timezone(&timezone);
            by_minute[(local.hour() * 60 + local.minute()) as usize].push(e.sgv);
        }

        let mut window: Vec<f32> = Vec::new();
        let mut curves = vec![Vec::with_capacity(SLOTS); percentiles.len()];
        for slot in 0..SLOTS {
            let center = (slot as u32 * SLOT_MINUTES) as i32;
            window.clear();
            for offset in -WINDOW_MINUTES..=WINDOW_MINUTES {
                let minute = (center + offset).rem_euclid(MINUTES_PER_DAY as i32);
                window.extend_from_slice(&by_minute[minute as usize]);
            }
            let enough = window.len() >= MIN_READINGS;
            if enough {
                window.sort_unstable_by(f32::total_cmp);
            }
            for (curve, &p) in curves.iter_mut().zip(percentiles) {
                curve.push(enough.then(|| percentile(&window, p)));
            }
        }
        let curves = curves.iter().map(|c| smooth(c)).collect();

        Some(Self {
            minutes: (0..SLOTS as u32).map(|s| s * SLOT_MINUTES).collect(),
            percentiles: percentiles.to_vec(),
            curves,
            readings: readings.len(),
            from,
            to,
        })
    }

    /// The curve of `percentile`, if it was computed.
    pub fn curve(&self, percentile: f32) -> Option<&[Option<f32>]> {
        let i = self.percentiles.iter().position(|&p| p == percentile)?;
        Some(&self.curves[i])
    }
}

/// `curve` averaged over [`SMOOTHING`], wrapping around midnight. Blank slots
/// stay blank and are left out of their neighbours' averages. All curves of a
/// profile share their blank slots, so smoothing keeps them in order.
fn smooth(curve: &[Option<f32>]) -> Vec<Option<f32>> {
    let reach = (SMOOTHING.len() / 2) as isize;
    (0..curve.len())
        .map(|i| {
            curve[i]?;
            let (sum, weight) = SMOOTHING
                .iter()
                .enumerate()
                .filter_map(|(k, &w)| {
                    let j = (i as isize + k as isize - reach).rem_euclid(curve.len() as isize);
                    curve[j as usize].map(|v| (v * w, w))
                })
                .fold((0.0, 0.0), |(s, t), (v, w)| (s + v, t + w));
            Some(sum / weight)
        })
        .collect()
}

/// The `p`th percentile (0–100) of ascending, non-empty `sorted`,
/// interpolating between neighbouring readings (the method spreadsheets and
/// NumPy use by default).
fn percentile(sorted: &[f32], p: f32) -> f32 {
    let h = (sorted.len() - 1) as f32 * (p / 100.0).clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (h - lo as f32)
}

/// A curve's value at a (fractional) minute of the day, interpolated between
/// its slots and wrapping around midnight.
pub(crate) fn curve_at(curve: &[Option<f32>], minute: f32) -> Option<f32> {
    let f = minute / SLOT_MINUTES as f32;
    let i = f.floor().max(0.0) as usize;
    let a = curve[i % SLOTS]?;
    let b = curve[(i + 1) % SLOTS]?;
    Some(a + (b - a) * (f - i as f32))
}

/// Builder for the percentile graph.
///
/// Shows the median of every reading at each time of day as a line, with the
/// 25th–75th percentile band around it and the lighter 5th–95th band around
/// that (see [`PercentileBands`] to change them), over the same framed panel,
/// target lines and axes as the glucose graph. Everything is colored by
/// glucose status: above target, in range, below target.
///
/// Give it the readings for the period to summarize, typically 14 days.
pub struct PercentileGraphBuilder<'a> {
    entries: Vec<GraphEntry>,
    bands: PercentileBands,
    target_low: f32,
    target_high: f32,
    unit_display: UnitDisplay,
    scaling: GraphScaling,
    timezone: Tz,
    layout: LayoutConfig,
    theme: Theme,
    font: &'a [u8],
    title: String,
    period_label: Option<String>,
}

impl<'a> Default for PercentileGraphBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> PercentileGraphBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

        Self {
            entries: Vec::new(),
            bands: PercentileBands::default(),
            target_low: 70.0,
            target_high: 180.0,
            unit_display: UnitDisplay::MgDl,
            scaling: GraphScaling::Dynamic {
                clamp_min: 40.0,
                clamp_max: 400.0,
                default_min: 40.0,
                default_max: 250.0,
            },
            timezone: chrono_tz::UTC,
            layout: LayoutConfig::default(),
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            title: "Glucose profile".to_string(),
            period_label: None,
        }
    }

    /// Sets the readings to summarize, replacing any earlier ones.
    pub fn with_entries<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
    {
        self.entries = entries.into_iter().map(|e| e.into()).collect();
        self
    }

    /// Sets which percentiles are shaded and whether the median is drawn.
    pub fn with_bands(mut self, bands: PercentileBands) -> Self {
        self.bands = bands;
        self
    }

    /// Sets the low and high targets in mg/dL, drawn as dashed lines. They
    /// also decide where the bands turn to the low and high colors.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.target_low = low;
        self.target_high = high;
        self
    }

    /// Sets the Y axis unit.
    pub fn with_units(mut self, display: UnitDisplay) -> Self {
        self.unit_display = display;
        self
    }

    /// Sets the Y axis range. By default it shows 40–250 mg/dL, widening to
    /// fit the outermost band (up to 400).
    pub fn with_scaling(mut self, scaling: GraphScaling) -> Self {
        self.scaling = scaling;
        self
    }

    /// Sets the timezone whose clock the readings are grouped by.
    pub fn with_timezone(mut self, tz: Tz) -> Self {
        self.timezone = tz;
        self
    }

    /// Sets the image size and margins.
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

    /// Sets the title shown top left. Default: "Glucose profile".
    pub fn with_title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the period shown after the title. Default: the dates of the
    /// first and last reading and the number of days between them.
    pub fn with_period_label<S: Into<String>>(mut self, label: S) -> Self {
        self.period_label = Some(label.into());
        self
    }

    /// Renders the graph.
    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let bands = self.checked_bands()?;
        let profile =
            PercentileProfile::compute(&self.entries, self.timezone, &bands.percentiles())
                .ok_or("at least one entry is required - call with_entries() first")?;
        let font = FontRef::try_from_slice(self.font)?;

        let mut plot = self.plot_area();
        (plot.y_min, plot.y_max) = self.y_range(&profile);
        let step = self.tick_step(&plot);
        if let GraphScaling::Dynamic {
            clamp_min,
            clamp_max,
            ..
        } = self.scaling
        {
            // Out to the next round values, so the outermost band stays off
            // the panel's edges and the edges get labels.
            plot.y_min = ((plot.y_min / step).floor() * step).max(clamp_min);
            plot.y_max = ((plot.y_max / step).ceil() * step).min(clamp_max);
        }
        let ticks = value_ticks(&plot, step);

        let mut img = self.new_canvas(&plot);
        self.draw_gridlines(&mut img, &plot, &ticks);
        self.draw_targets(&mut img, &plot);
        self.draw_bands(&mut img, &plot, &profile, &bands);
        if bands.median {
            self.draw_median(&mut img, &plot, &profile);
        }
        self.draw_value_labels(&mut img, &plot, &font, &ticks);
        self.draw_time_labels(&mut img, &plot, &font);
        self.draw_header(&mut img, &plot, &font, &profile, &bands);

        Ok(img)
    }

    /// The bands with each pair in order, or an error for a percentile
    /// outside 0–100.
    fn checked_bands(&self) -> Result<PercentileBands, Box<dyn std::error::Error>> {
        let order = |(a, b): (f32, f32)| if a <= b { (a, b) } else { (b, a) };
        let bands = PercentileBands {
            inner: order(self.bands.inner),
            outer: self.bands.outer.map(order),
            ..self.bands
        };
        if bands
            .percentiles()
            .iter()
            .all(|p| (0.0..=100.0).contains(p))
        {
            Ok(bands)
        } else {
            Err("percentiles must be between 0 and 100".into())
        }
    }

    /// Where the plot sits, sized like the glucose graph's: `s` scales
    /// everything from a 1200×800 design.
    fn plot_area(&self) -> Plot {
        let s = (self.layout.width as f32 / 1200.0)
            .min(self.layout.height as f32 / 800.0)
            .max(0.5);
        let is_dual = matches!(self.unit_display, UnitDisplay::Dual { .. });
        let wide_y = !matches!(self.unit_display, UnitDisplay::MgDl);

        let top = self
            .layout
            .margin_top
            .unwrap_or(if is_dual { 136.0 } else { 114.0 } * s);
        let bottom = self.layout.margin_bottom.unwrap_or(66.0 * s);
        let left = self
            .layout
            .margin_left
            .unwrap_or(if wide_y { 120.0 } else { 110.0 } * s);
        let right = self.layout.margin_right.unwrap_or(32.0 * s);

        Plot {
            s,
            left,
            top,
            right: self.layout.width as f32 - right,
            bottom: self.layout.height as f32 - bottom,
            y_min: 0.0,
            y_max: 1.0,
        }
    }

    /// The Y range in mg/dL: static, or the default view widened to the
    /// profile's outermost values within the clamp.
    fn y_range(&self, profile: &PercentileProfile) -> (f32, f32) {
        match self.scaling {
            GraphScaling::Static { min, max } => (min, max),
            GraphScaling::Dynamic {
                clamp_min,
                clamp_max,
                default_min,
                default_max,
            } => {
                let values = profile.curves.iter().flatten().flatten().copied();
                let (lo, hi) = values.fold((default_min, default_max), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                });
                (lo.max(clamp_min), hi.min(clamp_max))
            }
        }
    }

    /// The spacing in mg/dL of the Y gridlines and labels: the finest round
    /// step of the display unit that leaves room for each label and no more
    /// than 8 lines.
    fn tick_step(&self, plot: &Plot) -> f32 {
        let label_h = 31.0 * plot.s
            + if matches!(self.unit_display, UnitDisplay::Dual { .. }) {
                21.0 * plot.s
            } else {
                0.0
            };
        round_step(
            plot.y_max - plot.y_min,
            plot.h(),
            label_h,
            unit_preference(self.unit_display),
        )
    }

    /// The opaque color of the plot panel: `grid_major` tinted over the
    /// background, as on the glucose graph.
    fn panel_color(&self) -> Rgba<u8> {
        let [gr, gg, gb, _] = self.theme.grid_major.0;
        blend_pixel(self.theme.background, Rgba([gr, gg, gb, 110]))
    }

    fn new_canvas(&self, plot: &Plot) -> RgbaImage {
        let pad = PANEL_PAD * plot.s;
        canvas_with_rounded_rects(
            self.layout.width,
            self.layout.height,
            self.theme.background,
            &[(
                (plot.left - pad) as i32,
                (plot.top - pad) as i32,
                (plot.w() + 2.0 * pad) as u32,
                (plot.h() + 2.0 * pad) as u32,
            )],
            (18.0 * plot.s) as i32,
            self.panel_color(),
        )
    }

    /// Faint horizontal lines at the value ticks and fainter vertical ones at
    /// the hour labels.
    fn draw_gridlines(&self, img: &mut RgbaImage, plot: &Plot, ticks: &[f32]) {
        let thickness = plot.s.ceil() as u32;
        let [r, g, b, _] = self.theme.axis_lines.0;
        for &v in ticks {
            let y = plot.y(v);
            blend_fast_rect(
                img,
                plot.left as i32,
                (y - thickness as f32 / 2.0).round() as i32,
                plot.w() as u32,
                thickness,
                Rgba([r, g, b, 30]),
            );
        }
        for hour in self.hour_marks(plot) {
            let x = plot.x(hour as f32 * 60.0);
            blend_fast_rect(
                img,
                (x - thickness as f32 / 2.0).round() as i32,
                plot.top as i32,
                thickness,
                plot.h() as u32,
                Rgba([r, g, b, 16]),
            );
        }
    }

    /// Dashed target lines, as on the glucose graph.
    fn draw_targets(&self, img: &mut RgbaImage, plot: &Plot) {
        let s = plot.s;
        for (value, color) in [
            (self.target_high, self.theme.glucose_high),
            (self.target_low, self.theme.glucose_low),
        ] {
            if value < plot.y_min || value > plot.y_max {
                continue;
            }
            draw_dashed_horizontal_line(
                img,
                plot.y(value),
                plot.left,
                plot.right,
                Rgba([color[0], color[1], color[2], 80]),
                (10.0 * s) as i32,
                (10.0 * s) as i32,
                s.ceil() as i32,
            );
        }
    }

    /// The status color of a glucose value.
    fn status_color(&self, mgdl: f32) -> Rgba<u8> {
        if mgdl > self.target_high {
            self.theme.glucose_high
        } else if mgdl < self.target_low {
            self.theme.glucose_low
        } else {
            self.theme.glucose_in_range
        }
    }

    /// Fills the bands column by column. Each pixel takes the status color of
    /// the value it stands for, at the inner or outer band's opacity, with
    /// anti-aliased edges; the inner band replaces the outer one rather than
    /// stacking on it.
    fn draw_bands(
        &self,
        img: &mut RgbaImage,
        plot: &Plot,
        profile: &PercentileProfile,
        bands: &PercentileBands,
    ) {
        let curve = |p: f32| profile.curve(p).unwrap_or(&[]);
        let inner = (curve(bands.inner.0), curve(bands.inner.1));
        let outer = bands.outer.map(|(lo, hi)| (curve(lo), curve(hi)));

        // Status colors per row, worked out once.
        let (row0, row1) = (plot.top.floor() as i32, plot.bottom.ceil() as i32);
        let row_colors: Vec<[f32; 3]> = (row0..row1)
            .map(|row| {
                let c = self.status_color(plot.value_at(row as f32 + 0.5));
                [c[0] as f32, c[1] as f32, c[2] as f32]
            })
            .collect();

        let (img_w, img_h) = (img.width() as i32, img.height() as i32);
        let raw = img.as_mut();
        for x in plot.left.round() as i32..=plot.right.round() as i32 {
            if !(0..img_w).contains(&x) {
                continue;
            }
            let minute = (x as f32 - plot.left) / plot.w() * MINUTES_PER_DAY as f32;
            let span = |(lo, hi): (&[Option<f32>], &[Option<f32>])| {
                Some((plot.y(curve_at(hi, minute)?), plot.y(curve_at(lo, minute)?)))
            };
            let Some(inner) = span(inner) else {
                continue;
            };
            let outer = outer.and_then(span).unwrap_or(inner);
            let (top, bottom) = (outer.0.min(inner.0), outer.1.max(inner.1));

            for row in (top.floor() as i32).max(row0)..(bottom.ceil() as i32).min(row1).min(img_h) {
                let covers =
                    |(t, b): (f32, f32)| (b.min(row as f32 + 1.0) - t.max(row as f32)).max(0.0);
                let in_inner = covers(inner);
                let alpha = (in_inner * INNER_ALPHA
                    + (covers(outer) - in_inner).max(0.0) * OUTER_ALPHA)
                    / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let [cr, cg, cb] = row_colors[(row - row0) as usize];
                let i = (row * img_w + x) as usize * 4;
                let px = &mut raw[i..i + 3];
                px[0] = (cr * alpha + px[0] as f32 * (1.0 - alpha)) as u8;
                px[1] = (cg * alpha + px[1] as f32 * (1.0 - alpha)) as u8;
                px[2] = (cb * alpha + px[2] as f32 * (1.0 - alpha)) as u8;
            }
        }
    }

    /// The median as a smooth line, stamped with anti-aliased discs like the
    /// glucose trace and colored by status along its length.
    fn draw_median(&self, img: &mut RgbaImage, plot: &Plot, profile: &PercentileProfile) {
        let Some(median) = profile.curve(50.0) else {
            return;
        };
        let half = (2.2 * plot.s).round().max(1.0) as i32;
        let pens = [
            create_aa_circle_sprite(half, self.theme.glucose_low),
            create_aa_circle_sprite(half, self.theme.glucose_in_range),
            create_aa_circle_sprite(half, self.theme.glucose_high),
        ];
        let pen = |mgdl: f32| -> &Sprite {
            if mgdl > self.target_high {
                &pens[2]
            } else if mgdl < self.target_low {
                &pens[0]
            } else {
                &pens[1]
            }
        };

        let x0 = plot.left.round() as i32;
        let points: Vec<Option<f32>> = (x0..=plot.right.round() as i32)
            .map(|x| {
                curve_at(
                    median,
                    (x as f32 - plot.left) / plot.w() * MINUTES_PER_DAY as f32,
                )
            })
            .collect();
        let mut last_stamp = None;
        for (i, pair) in points.windows(2).enumerate() {
            let [Some(a), Some(b)] = [pair[0], pair[1]] else {
                last_stamp = None;
                continue;
            };
            let (ya, yb) = (plot.y(a), plot.y(b));
            let steps = (1.0 + (yb - ya) * (yb - ya)).sqrt().ceil() as i32;
            for k in 0..=steps {
                let t = k as f32 / steps as f32;
                let p = (
                    x0 + i as i32 + t.round() as i32,
                    (ya + (yb - ya) * t).round() as i32,
                );
                if last_stamp != Some(p) {
                    blend_sprite(img, pen(a + (b - a) * t), p.0, p.1);
                    last_stamp = Some(p);
                }
            }
        }
    }

    /// Hours of the day labelled on the X axis: every 3 hours, or every 6
    /// when the plot is too narrow for that.
    fn hour_marks(&self, plot: &Plot) -> Vec<u32> {
        let every = if plot.w() / 8.0 >= 100.0 * plot.s {
            3
        } else {
            6
        };
        (0..=24).step_by(every).collect()
    }

    /// Y value labels in the left margin, each on its gridline, plus the
    /// unit caption above them, as on the glucose graph.
    fn draw_value_labels(&self, img: &mut RgbaImage, plot: &Plot, font: &FontRef, ticks: &[f32]) {
        let s = plot.s;
        let (size_md, size_sm, size_xs) = (31.0 * s, 25.0 * s, 21.0 * s);
        let right = plot.left - PANEL_PAD * s - 12.0 * s;
        let pref = unit_preference(self.unit_display);
        let dual = matches!(self.unit_display, UnitDisplay::Dual { .. });

        let caption_bottom = plot.top - PANEL_PAD * s - 6.0 * s;
        let mut y = caption_bottom;
        if dual {
            let other = other_unit(pref);
            y -= size_xs;
            draw_text_right(
                img,
                self.theme.text_dim,
                font,
                size_xs,
                right,
                y,
                unit_name(other),
            );
            y -= 2.0 * s;
        }
        draw_text_right(
            img,
            self.theme.text_secondary,
            font,
            size_sm,
            right,
            y - size_sm,
            unit_name(pref),
        );

        for &v in ticks {
            let y = plot.y(v);
            draw_text_right(
                img,
                self.theme.text_primary,
                font,
                size_md,
                right,
                y - size_md / 2.0,
                &fmt_glucose(v, pref),
            );
            if dual {
                draw_text_right(
                    img,
                    self.theme.text_dim,
                    font,
                    size_xs,
                    right,
                    y + size_md / 2.0,
                    &fmt_glucose(v, other_unit(pref)),
                );
            }
        }
    }

    /// Time of day under the plot, with a short tick for each.
    fn draw_time_labels(&self, img: &mut RgbaImage, plot: &Plot, font: &FontRef) {
        let s = plot.s;
        let size = 25.0 * s;
        let [r, g, b, _] = self.theme.axis_lines.0;
        for hour in self.hour_marks(plot) {
            let x = plot.x(hour as f32 * 60.0);
            blend_fast_rect(
                img,
                x.round() as i32,
                plot.bottom as i32,
                s.ceil() as u32,
                (7.0 * s) as u32,
                Rgba([r, g, b, 70]),
            );
            let text = format!("{hour:02}:00");
            let w = text_w(font, &text, size);
            let tx = (x - w / 2.0).clamp(plot.left, plot.right - w);
            draw_text(
                img,
                self.theme.text_primary,
                tx as i32,
                (plot.bottom + 28.0 * s) as i32,
                PxScale::from(size),
                font,
                &text,
            );
        }
    }

    /// Title and period top left, legend top right, lined up with the panel.
    fn draw_header(
        &self,
        img: &mut RgbaImage,
        plot: &Plot,
        font: &FontRef,
        profile: &PercentileProfile,
        bands: &PercentileBands,
    ) {
        let s = plot.s;
        let pad = PANEL_PAD * s;
        let (size_title, size_sm) = (31.0 * s, 23.0 * s);
        let cy = 38.0 * s;
        let (left, right) = (plot.left - pad, plot.right + pad);

        // Legend, laid out right to left from the panel's right edge.
        let panel = self.panel_color();
        let tint = |alpha: f32| {
            let c = self.theme.glucose_in_range;
            blend_pixel(panel, Rgba([c[0], c[1], c[2], alpha as u8]))
        };
        let mut items: Vec<(Swatch, String)> = Vec::new();
        if let Some(outer) = bands.outer {
            items.push((Swatch::Band(tint(OUTER_ALPHA)), fmt_range(outer)));
        }
        items.push((Swatch::Band(tint(INNER_ALPHA)), fmt_range(bands.inner)));
        if bands.median {
            items.push((
                Swatch::Line(self.theme.glucose_in_range),
                "Median".to_string(),
            ));
        }
        let (swatch_w, swatch_h, gap) = (26.0 * s, 14.0 * s, 10.0 * s);
        let mut x = right;
        for (swatch, label) in items.iter().rev() {
            let w = text_w(font, label, size_sm);
            x -= w;
            draw_text(
                img,
                self.theme.text_secondary,
                x as i32,
                (cy - size_sm / 2.0) as i32,
                PxScale::from(size_sm),
                font,
                label,
            );
            x -= gap + swatch_w;
            match *swatch {
                Swatch::Band(color) => draw_filled_rounded_rect(
                    img,
                    x as i32,
                    (cy - swatch_h / 2.0) as i32,
                    swatch_w as u32,
                    swatch_h as u32,
                    (4.0 * s) as i32,
                    color,
                ),
                Swatch::Line(color) => {
                    let h = (2.2 * s).round().max(1.0) * 2.0;
                    draw_filled_rounded_rect(
                        img,
                        x as i32,
                        (cy - h / 2.0) as i32,
                        swatch_w as u32,
                        h as u32,
                        (h / 2.0) as i32,
                        color,
                    )
                }
            }
            x -= 28.0 * s;
        }
        let legend_left = x + 28.0 * s;

        draw_text(
            img,
            self.theme.text_primary,
            left as i32,
            (cy - size_title / 2.0) as i32,
            PxScale::from(size_title),
            font,
            &self.title,
        );
        // The period follows the title when it fits before the legend.
        let period = self
            .period_label
            .clone()
            .unwrap_or_else(|| default_period_label(profile, self.timezone));
        let px = left + text_w(font, &self.title, size_title) + 18.0 * s;
        if px + text_w(font, &period, size_sm) <= legend_left - 32.0 * s {
            draw_text(
                img,
                self.theme.text_secondary,
                px as i32,
                (cy - size_sm / 2.0) as i32,
                PxScale::from(size_sm),
                font,
                &period,
            );
        }
    }
}

/// The multiples of `step` (mg/dL) inside the plot's range.
fn value_ticks(plot: &Plot, step: f32) -> Vec<f32> {
    round_ticks(plot.y_min, plot.y_max, step)
}

/// A legend entry's sample.
enum Swatch {
    Band(Rgba<u8>),
    Line(Rgba<u8>),
}

/// The plot area in pixels and the glucose range it shows, in mg/dL.
struct Plot {
    s: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    y_min: f32,
    y_max: f32,
}

impl Plot {
    fn w(&self) -> f32 {
        self.right - self.left
    }

    fn h(&self) -> f32 {
        self.bottom - self.top
    }

    /// X of a minute of the day.
    fn x(&self, minute: f32) -> f32 {
        self.left + minute / MINUTES_PER_DAY as f32 * self.w()
    }

    /// Y of a glucose value, clamped to the plot.
    fn y(&self, mgdl: f32) -> f32 {
        let ratio = (mgdl.clamp(self.y_min, self.y_max) - self.y_min) / (self.y_max - self.y_min);
        self.bottom - ratio * self.h()
    }

    /// The glucose value at a Y.
    fn value_at(&self, y: f32) -> f32 {
        self.y_min + (self.bottom - y) / self.h() * (self.y_max - self.y_min)
    }
}

/// "25–75%", keeping a decimal only when a bound has one.
fn fmt_range((lo, hi): (f32, f32)) -> String {
    let p = |v: f32| {
        if v.fract() == 0.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.1}")
        }
    };
    format!("{}–{}%", p(lo), p(hi))
}

/// "12 Sep – 25 Sep · 14 days" in `timezone`.
fn default_period_label(profile: &PercentileProfile, timezone: Tz) -> String {
    let (from, to) = (
        profile.from.with_timezone(&timezone),
        profile.to.with_timezone(&timezone),
    );
    let days = ((profile.to - profile.from).num_minutes() as f64 / 1440.0)
        .round()
        .max(1.0) as i64;
    let days = if days == 1 {
        "1 day".to_string()
    } else {
        format!("{days} days")
    };
    if from.date_naive() == to.date_naive() {
        format!("{} · {days}", from.format("%-d %b"))
    } else {
        format!(
            "{} – {} · {days}",
            from.format("%-d %b"),
            to.format("%-d %b")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UnitPreference;
    use crate::utils::axis::MGDL_PER_MMOL;
    use chrono::{Duration, TimeZone};

    fn at(day: i64, minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + Duration::days(day)
            + Duration::minutes(minute)
    }

    fn entry(day: i64, minute: i64, sgv: f32) -> GraphEntry {
        GraphEntry {
            sgv,
            date: at(day, minute),
        }
    }

    /// 14 days of 5-minute readings: a daily wave plus a per-day offset so
    /// the percentiles spread.
    fn fortnight() -> Vec<GraphEntry> {
        (0..14)
            .flat_map(|day| {
                (0..288).map(move |i| {
                    let h = i as f32 / 12.0;
                    let sgv = 130.0
                        + 40.0 * (h / 24.0 * std::f32::consts::TAU).sin()
                        + (day as f32 - 6.5) * 6.0;
                    entry(day, i * 5, sgv)
                })
            })
            .collect()
    }

    #[test]
    fn percentile_interpolates_between_readings() {
        let sorted = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile(&sorted, 0.0), 10.0);
        assert_eq!(percentile(&sorted, 50.0), 30.0);
        assert_eq!(percentile(&sorted, 100.0), 50.0);
        assert_eq!(percentile(&sorted, 25.0), 20.0);
        assert!((percentile(&sorted, 10.0) - 14.0).abs() < 1e-4);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
    }

    #[test]
    fn profile_percentiles_are_ordered_and_bounded() {
        let entries = fortnight();
        let p =
            PercentileProfile::compute(&entries, chrono_tz::UTC, &[5.0, 25.0, 50.0, 75.0, 95.0])
                .unwrap();
        assert_eq!(p.readings, 14 * 288);
        assert_eq!(p.minutes.len(), SLOTS);
        for slot in 0..SLOTS {
            let values: Vec<f32> = p.curves.iter().map(|c| c[slot].unwrap()).collect();
            assert!(
                values.windows(2).all(|w| w[0] <= w[1]),
                "slot {slot}: {values:?}"
            );
        }
        // The wave peaks at 06:00 and bottoms at 18:00.
        let median = p.curve(50.0).unwrap();
        assert!(median[6 * 12].unwrap() > median[18 * 12].unwrap() + 60.0);
    }

    #[test]
    fn the_window_wraps_around_midnight() {
        // Readings only at 23:50, on 6 days.
        let entries: Vec<_> = (0..6).map(|d| entry(d, 23 * 60 + 50, 100.0)).collect();
        let p = PercentileProfile::compute(&entries, chrono_tz::UTC, &[50.0]).unwrap();
        let median = p.curve(50.0).unwrap();
        assert_eq!(median[0], Some(100.0), "00:00 sees 23:50");
        assert_eq!(median[SLOTS - 1], Some(100.0));
        assert_eq!(median[12], None, "01:00 is too far");
    }

    #[test]
    fn sparse_slots_are_left_blank() {
        let entries: Vec<_> = (0..4).map(|d| entry(d, 12 * 60, 100.0)).collect();
        let p = PercentileProfile::compute(&entries, chrono_tz::UTC, &[50.0]).unwrap();
        assert!(p.curve(50.0).unwrap().iter().all(Option::is_none));
    }

    #[test]
    fn readings_are_grouped_by_local_time() {
        // 12:00 UTC is 13:00 in Paris in January.
        let entries: Vec<_> = (0..6).map(|d| entry(d, 12 * 60, 100.0)).collect();
        let p = PercentileProfile::compute(&entries, chrono_tz::Europe::Paris, &[50.0]).unwrap();
        let median = p.curve(50.0).unwrap();
        assert_eq!(median[13 * 12], Some(100.0));
        assert_eq!(median[11 * 12], None);
    }

    #[test]
    fn value_ticks_are_round_and_readable() {
        let builder = PercentileGraphBuilder::new();
        let mut plot = builder.plot_area();
        (plot.y_min, plot.y_max) = (40.0, 250.0);
        let ticks = value_ticks(&plot, builder.tick_step(&plot));
        assert!(ticks.len() >= 4 && ticks.len() <= 9, "{ticks:?}");
        assert!(ticks
            .iter()
            .all(|t| t % 10.0 == 0.0 && (40.0..=250.0).contains(t)));

        let mmol = PercentileGraphBuilder::new().with_units(UnitDisplay::MmolL);
        let ticks = value_ticks(&plot, mmol.tick_step(&plot));
        for t in ticks {
            let v = t / MGDL_PER_MMOL;
            assert!((v * 2.0 - (v * 2.0).round()).abs() < 1e-3, "{v}");
        }
    }

    #[test]
    fn smoothing_keeps_blanks_and_flattens_steps() {
        let mut curve = vec![Some(100.0); SLOTS];
        curve[10] = None;
        curve[50] = Some(170.0);
        let smoothed = smooth(&curve);
        assert_eq!(smoothed[10], None);
        assert_eq!(smoothed[100], Some(100.0));
        assert!((smoothed[50].unwrap() - (100.0 + 70.0 * 4.0 / 16.0)).abs() < 1e-3);
        assert!(smoothed[49].unwrap() > 100.0 && smoothed[49] < smoothed[50]);
        // Wraps around midnight.
        curve[0] = Some(130.0);
        assert!(smooth(&curve)[SLOTS - 1].unwrap() > 100.0);
    }

    #[test]
    fn range_label_keeps_decimals_only_when_needed() {
        assert_eq!(fmt_range((25.0, 75.0)), "25–75%");
        assert_eq!(fmt_range((2.5, 97.5)), "2.5–97.5%");
    }

    #[test]
    fn builds_for_every_setup() {
        let setups: Vec<PercentileGraphBuilder> = vec![
            PercentileGraphBuilder::new(),
            PercentileGraphBuilder::new().with_bands(PercentileBands {
                inner: (15.0, 75.0),
                outer: None,
                median: false,
            }),
            PercentileGraphBuilder::new()
                .with_units(UnitDisplay::Dual {
                    primary: UnitPreference::MmolL,
                })
                .with_theme(Theme::paper_light()),
            PercentileGraphBuilder::new()
                .with_scaling(GraphScaling::Static {
                    min: 0.0,
                    max: 400.0,
                })
                .with_layout(LayoutConfig {
                    width: 500,
                    height: 300,
                    ..Default::default()
                }),
        ];
        for builder in setups {
            let img = builder.with_entries(fortnight()).build().unwrap();
            assert!(img.width() > 0);
        }
    }

    #[test]
    fn sparse_data_still_builds() {
        let entries: Vec<_> = (0..3).map(|d| entry(d, 600, 120.0)).collect();
        assert!(PercentileGraphBuilder::new()
            .with_entries(entries)
            .build()
            .is_ok());
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(PercentileGraphBuilder::new().build().is_err());
        let out_of_range = PercentileGraphBuilder::new()
            .with_entries(fortnight())
            .with_bands(PercentileBands {
                inner: (25.0, 175.0),
                ..Default::default()
            });
        assert!(out_of_range.build().is_err());
    }
}
