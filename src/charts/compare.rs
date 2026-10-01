//! The comparison graph: two periods side by side, each with its typical day
//! and its statistics, on the same scale so they can be compared at a glance.

use ab_glyph::{FontRef, PxScale};
use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc};
use chrono_tz::Tz;
use image::{Rgba, RgbaImage};

use crate::charts::glucose::LayoutConfig;
use crate::charts::percentile::{curve_at, PercentileProfile};
use crate::charts::time_in_range::{TirStats, TirThresholds};
use crate::models::{GraphEntry, GraphScaling, UnitDisplay, UnitPreference};
use crate::theme::Theme;
use crate::utils::axis::{
    fmt_glucose, fmt_glucose_unit, other_unit, round_step, round_ticks, unit_name, unit_preference,
    units,
};
use crate::utils::drawing::{
    blend_fast_rect, blend_pixel, blend_sprite, canvas_with_rounded_rects, create_aa_circle_sprite,
    draw_dashed_horizontal_line, draw_filled_rounded_rect,
};
use crate::utils::text::{draw_text, draw_text_right, draw_text_runs, text_w};

const MINUTES_PER_DAY: f32 = 24.0 * 60.0;

/// Each bar covers this many minutes of the day.
const BAR_MINUTES: f32 = 15.0;

/// Share of a bar's slot its width takes.
const BAR_WIDTH: f32 = 0.56;

/// Bar opacity (out of 255), over the card.
const BAR_ALPHA: f32 = 170.0;

/// Targets that apply overnight instead of the day ones, e.g. a higher low
/// target while asleep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightTargets {
    /// Low target overnight, in mg/dL.
    pub low: f32,
    /// High target overnight, in mg/dL.
    pub high: f32,
    /// When the night starts, e.g. 23:00.
    pub from: NaiveTime,
    /// When it ends, e.g. 07:00.
    pub until: NaiveTime,
}

impl NightTargets {
    /// Whether `minute` of the day falls in the night, which may wrap past
    /// midnight.
    pub(crate) fn covers(&self, minute: f32) -> bool {
        let (from, until) = (minutes_of(self.from), minutes_of(self.until));
        if from <= until {
            (from..until).contains(&minute)
        } else {
            minute >= from || minute < until
        }
    }
}

fn minutes_of(t: NaiveTime) -> f32 {
    (t.hour() * 60 + t.minute()) as f32
}

/// Builder for the comparison graph.
///
/// Draws two periods side by side, typically the last 14 days and the 14
/// before them. Each gets a card with its typical day, where every 15
/// minutes a bar spans the middle half of the readings (25th to 75th
/// percentile) under a dotted median line, colored by glucose status, and
/// its average glucose, GMI, standard deviation and coefficient of
/// variation. Both days share one scale, and the second card shows how each
/// statistic changed.
pub struct CompareGraphBuilder<'a> {
    periods: [Vec<GraphEntry>; 2],
    titles: [Option<String>; 2],
    tags: [String; 2],
    target_low: f32,
    target_high: f32,
    night: Option<NightTargets>,
    unit_display: UnitDisplay,
    scaling: GraphScaling,
    timezone: Tz,
    layout: LayoutConfig,
    theme: Theme,
    font: &'a [u8],
}

impl<'a> Default for CompareGraphBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> CompareGraphBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

        Self {
            periods: [Vec::new(), Vec::new()],
            titles: [None, None],
            tags: ["Old".to_string(), "New".to_string()],
            target_low: 70.0,
            target_high: 180.0,
            night: None,
            unit_display: UnitDisplay::MgDl,
            scaling: GraphScaling::Dynamic {
                clamp_min: 40.0,
                clamp_max: 400.0,
                default_min: 50.0,
                default_max: 250.0,
            },
            timezone: chrono_tz::UTC,
            layout: LayoutConfig::default(),
            theme: Theme::dark(),
            font: DEFAULT_FONT,
        }
    }

    /// Sets the readings of the two periods: the earlier one is drawn on the
    /// left, and the right one's statistics show the change from it.
    pub fn with_periods<I, J>(mut self, first: I, second: J) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
        J: IntoIterator,
        J::Item: Into<GraphEntry>,
    {
        self.periods = [
            first.into_iter().map(|e| e.into()).collect(),
            second.into_iter().map(|e| e.into()).collect(),
        ];
        self
    }

    /// Sets the card titles. Default: each period's length, e.g. "14 days".
    pub fn with_titles<S: Into<String>, T: Into<String>>(mut self, first: S, second: T) -> Self {
        self.titles = [Some(first.into()), Some(second.into())];
        self
    }

    /// Sets the pills top right of each card that tell the periods apart.
    /// Default: "Old" and "New".
    pub fn with_tags<S: Into<String>, T: Into<String>>(mut self, first: S, second: T) -> Self {
        self.tags = [first.into(), second.into()];
        self
    }

    /// Sets the low and high targets in mg/dL, drawn as dashed lines. They
    /// also decide where bars turn to the low and high colors, and apply all
    /// day unless [`with_night_targets`](Self::with_night_targets) is set.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.target_low = low;
        self.target_high = high;
        self
    }

    /// Uses other targets overnight. The night is shaded on the graph and the
    /// target lines step at its edges.
    pub fn with_night_targets(mut self, night: NightTargets) -> Self {
        self.night = Some(night);
        self
    }

    /// Sets the glucose unit. `Dual` repeats every glucose value in the other
    /// unit, smaller: on the axis, the target lines and the statistics.
    pub fn with_units(mut self, display: UnitDisplay) -> Self {
        self.unit_display = display;
        self
    }

    /// Sets the Y axis range, shared by both cards. By default it shows
    /// 50–250 mg/dL, widening to fit the bars (from 40 up to 400).
    pub fn with_scaling(mut self, scaling: GraphScaling) -> Self {
        self.scaling = scaling;
        self
    }

    /// Sets the timezone whose clock the readings are grouped by.
    pub fn with_timezone(mut self, tz: Tz) -> Self {
        self.timezone = tz;
        self
    }

    /// Sets the image size. Margins set the space around the two cards.
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

    /// Renders the graph.
    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let font = FontRef::try_from_slice(self.font)?;
        let thresholds = TirThresholds {
            low: self.target_low,
            high: self.target_high,
            ..TirThresholds::default()
        };
        let mut periods = Vec::with_capacity(2);
        for entries in &self.periods {
            let profile = PercentileProfile::compute(entries, self.timezone, &[25.0, 50.0, 75.0]);
            let stats = TirStats::compute(entries, &thresholds);
            let (Some(profile), Some(stats)) = (profile, stats) else {
                return Err("both periods need readings - call with_periods() first".into());
            };
            periods.push(Period { profile, stats });
        }

        let mut cards = self.cards();
        let (y_min, y_max) = self.y_range(&periods);
        let step = self.tick_step(&cards[0].plot, y_max - y_min);
        let (y_min, y_max) = match self.scaling {
            // Out to the next round values, so bars stay off the edges and
            // the edges get labels.
            GraphScaling::Dynamic {
                clamp_min,
                clamp_max,
                ..
            } => (
                ((y_min / step).floor() * step).max(clamp_min),
                ((y_max / step).ceil() * step).min(clamp_max),
            ),
            GraphScaling::Static { .. } => (y_min, y_max),
        };
        for card in &mut cards {
            (card.plot.y_min, card.plot.y_max) = (y_min, y_max);
        }
        let ticks = round_ticks(y_min, y_max, step);

        let mut img = canvas_with_rounded_rects(
            self.layout.width,
            self.layout.height,
            self.theme.background,
            &cards
                .iter()
                .map(|c| (c.x as i32, c.y as i32, c.w as u32, c.h as u32))
                .collect::<Vec<_>>(),
            (18.0 * cards[0].s) as i32,
            self.panel_color(),
        );
        for (i, (card, period)) in cards.iter().zip(&periods).enumerate() {
            self.draw_night(&mut img, &card.plot);
            self.draw_gridlines(&mut img, card, &ticks);
            self.draw_target_lines(&mut img, &card.plot);
            self.draw_bars(&mut img, &card.plot, &period.profile);
            self.draw_median(&mut img, &card.plot, &period.profile);
            self.draw_target_tags(&mut img, &card.plot, &font);
            self.draw_value_labels(&mut img, card, &font, &ticks);
            self.draw_time_labels(&mut img, card, &font);
            self.draw_header(&mut img, card, &font, i, period);
            let before = (i == 1).then(|| &periods[0].stats);
            self.draw_stats(&mut img, card, &font, &period.stats, before);
        }

        Ok(img)
    }

    /// The two cards and the plot inside each, sized like the other graphs:
    /// `s` scales everything from a 1200×800 design.
    fn cards(&self) -> [Card; 2] {
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
        let gap = 24.0 * s;
        let card_w = ((w - m_left - m_right - gap) / 2.0).max(1.0);
        let card_h = (h - m_top - m_bottom).max(1.0);
        let pad = 28.0 * s;

        let dual = matches!(self.unit_display, UnitDisplay::Dual { .. });
        let label_w = if matches!(self.unit_display, UnitDisplay::MgDl) {
            52.0
        } else {
            64.0
        } * s;
        let card = |x: f32| {
            let y = m_top;
            let header_bottom = y + pad + 44.0 * s;
            // With two units the statistics get a second line.
            let stats_top = y + card_h - pad - if dual { 126.0 } else { 104.0 } * s;
            let plot = Plot {
                left: x + pad + label_w + 16.0 * s,
                right: x + card_w - pad,
                top: header_bottom + 34.0 * s + if dual { 58.0 } else { 36.0 } * s,
                bottom: stats_top - 76.0 * s,
                y_min: 0.0,
                y_max: 1.0,
            };
            Card {
                s,
                x,
                y,
                w: card_w,
                h: card_h,
                pad,
                header_bottom,
                stats_top,
                plot,
            }
        };
        [card(m_left), card(m_left + card_w + gap)]
    }

    /// The shared Y range in mg/dL: static, or the default view widened to
    /// both periods' bars and medians within the clamp.
    fn y_range(&self, periods: &[Period]) -> (f32, f32) {
        match self.scaling {
            GraphScaling::Static { min, max } => (min, max),
            GraphScaling::Dynamic {
                clamp_min,
                clamp_max,
                default_min,
                default_max,
            } => {
                let values = periods
                    .iter()
                    .flat_map(|p| p.profile.curves.iter().flatten().flatten().copied());
                let (lo, hi) = values.fold((default_min, default_max), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                });
                (lo.max(clamp_min), hi.min(clamp_max))
            }
        }
    }

    /// Round spacing for the Y gridlines over `range`, leaving room for each
    /// label (and its second unit).
    fn tick_step(&self, plot: &Plot, range: f32) -> f32 {
        let label_h = self.scale() * (27.0 + if self.is_dual() { 19.0 } else { 0.0 });
        round_step(range, plot.h(), label_h, unit_preference(self.unit_display))
    }

    fn scale(&self) -> f32 {
        (self.layout.width as f32 / 1200.0)
            .min(self.layout.height as f32 / 800.0)
            .max(0.5)
    }

    fn is_dual(&self) -> bool {
        matches!(self.unit_display, UnitDisplay::Dual { .. })
    }

    /// The opaque color of the cards: `grid_major` tinted over the
    /// background, like the other graphs' panels.
    fn panel_color(&self) -> Rgba<u8> {
        let [gr, gg, gb, _] = self.theme.grid_major.0;
        blend_pixel(self.theme.background, Rgba([gr, gg, gb, 110]))
    }

    /// The low and high targets at a minute of the day.
    fn targets_at(&self, minute: f32) -> (f32, f32) {
        match self.night {
            Some(night) if night.covers(minute) => (night.low, night.high),
            _ => (self.target_low, self.target_high),
        }
    }

    /// The status color of a value at a minute of the day.
    fn status_color(&self, mgdl: f32, minute: f32) -> Rgba<u8> {
        let (low, high) = self.targets_at(minute);
        if mgdl > high {
            self.theme.glucose_high
        } else if mgdl < low {
            self.theme.glucose_low
        } else {
            self.theme.glucose_in_range
        }
    }

    /// A faint shade over the night hours.
    fn draw_night(&self, img: &mut RgbaImage, plot: &Plot) {
        let Some(night) = self.night else {
            return;
        };
        let [r, g, b, _] = self.theme.axis_lines.0;
        let (from, until) = (minutes_of(night.from), minutes_of(night.until));
        let spans = if from <= until {
            vec![(from, until)]
        } else {
            vec![(from, MINUTES_PER_DAY), (0.0, until)]
        };
        for (a, b_) in spans {
            let (x0, x1) = (plot.x(a).round(), plot.x(b_).round());
            blend_fast_rect(
                img,
                x0 as i32,
                plot.top as i32,
                (x1 - x0).max(0.0) as u32,
                plot.h() as u32,
                Rgba([r, g, b, 10]),
            );
        }
    }

    /// Faint horizontal lines at the value ticks and fainter vertical ones at
    /// the hour labels.
    fn draw_gridlines(&self, img: &mut RgbaImage, card: &Card, ticks: &[f32]) {
        let plot = &card.plot;
        let thickness = card.s.ceil() as u32;
        let [r, g, b, _] = self.theme.axis_lines.0;
        for &v in ticks {
            blend_fast_rect(
                img,
                plot.left as i32,
                (plot.y(v) - thickness as f32 / 2.0).round() as i32,
                plot.w() as u32,
                thickness,
                Rgba([r, g, b, 30]),
            );
        }
        for hour in hour_marks(card) {
            blend_fast_rect(
                img,
                (plot.x(hour as f32 * 60.0) - thickness as f32 / 2.0).round() as i32,
                plot.top as i32,
                thickness,
                plot.h() as u32,
                Rgba([r, g, b, 16]),
            );
        }
    }

    /// The day split into stretches of one target, for the high (`true`) or
    /// low target: `(from, to, value)` in minutes and mg/dL.
    fn target_stretches(&self, high: bool) -> Vec<(f32, f32, f32)> {
        // Minutes where the targets may change.
        let mut edges = vec![0.0, MINUTES_PER_DAY];
        if let Some(night) = self.night {
            edges.extend([minutes_of(night.from), minutes_of(night.until)]);
        }
        edges.sort_by(f32::total_cmp);
        edges.dedup();

        let mut stretches: Vec<(f32, f32, f32)> = Vec::new();
        for w in edges.windows(2) {
            let (low, hi) = self.targets_at((w[0] + w[1]) / 2.0);
            let v = if high { hi } else { low };
            match stretches.last_mut() {
                Some(last) if last.2 == v => last.1 = w[1],
                _ => stretches.push((w[0], w[1], v)),
            }
        }
        stretches
    }

    fn target_colors(&self) -> [(bool, Rgba<u8>); 2] {
        [
            (true, self.theme.glucose_high),
            (false, self.theme.glucose_low),
        ]
    }

    /// Dashed target lines, stepping where the night targets start and end.
    /// Drawn under the bars.
    fn draw_target_lines(&self, img: &mut RgbaImage, plot: &Plot) {
        let s = self.scale();
        let dash = (10.0 * s) as i32;
        let thickness = s.ceil() as i32;
        for (high, color) in self.target_colors() {
            let line = Rgba([color[0], color[1], color[2], 120]);
            let mut previous_y: Option<f32> = None;
            for (from, to, v) in self.target_stretches(high) {
                let (y, x0) = (plot.y(v), plot.x(from));
                draw_dashed_horizontal_line(img, y, x0, plot.x(to), line, dash, dash, thickness);
                if let Some(py) = previous_y.filter(|&py| py != y) {
                    // Dashed step between two stretches.
                    let (top, bottom) = (py.min(y) as i32, py.max(y) as i32);
                    for yy in (top..bottom).step_by((2 * dash).max(2) as usize) {
                        blend_fast_rect(
                            img,
                            (x0 - thickness as f32 / 2.0) as i32,
                            yy,
                            thickness as u32,
                            dash.min(bottom - yy) as u32,
                            line,
                        );
                    }
                }
                previous_y = Some(y);
            }
        }
    }

    /// Each target stretch's value in a solid pill on its line, near its
    /// start, followed by its value in the second unit when that fits the
    /// stretch too. Drawn over the bars so it always reads.
    fn draw_target_tags(&self, img: &mut RgbaImage, plot: &Plot, font: &FontRef) {
        let s = self.scale();
        let (size, size_second) = (17.0 * s, 14.0 * s);
        let (pill_h, pad_x) = (22.0 * s, 8.0 * s);
        let (pref, second) = units(self.unit_display);
        for (high, color) in self.target_colors() {
            let ink = self.theme.background;
            let faint = blend_pixel(color, Rgba([ink[0], ink[1], ink[2], 170]));
            for (from, to, v) in self.target_stretches(high) {
                let text = fmt_glucose(v, pref);
                let (x0, x1) = (plot.x(from), plot.x(to));
                let fits = |pill_w: f32| x1 - x0 >= pill_w + 12.0 * s;
                let text_w_px = text_w(font, &text, size);
                let also = second
                    .map(|u| format!(" {}", fmt_glucose(v, u)))
                    .filter(|t| fits(text_w_px + text_w(font, t, size_second) + 2.0 * pad_x))
                    .unwrap_or_default();
                let pill_w = text_w_px + text_w(font, &also, size_second) + 2.0 * pad_x;
                if !fits(pill_w) {
                    continue;
                }
                let (px, py) = (x0 + 6.0 * s, plot.y(v) - pill_h / 2.0);
                draw_filled_rounded_rect(
                    img,
                    px as i32,
                    py as i32,
                    pill_w as u32,
                    pill_h as u32,
                    (pill_h / 2.0) as i32,
                    color,
                );
                draw_text_runs(
                    img,
                    font,
                    px + pad_x,
                    (py + (pill_h - size) / 2.0 - 1.0 * s).floor(),
                    &[(&text, size, ink), (&also, size_second, faint)],
                );
            }
        }
    }

    /// One rounded bar per 15 minutes from the 25th to the 75th percentile,
    /// colored along its length by the status of each value it covers.
    fn draw_bars(&self, img: &mut RgbaImage, plot: &Plot, profile: &PercentileProfile) {
        let (Some(p25), Some(p75)) = (profile.curve(25.0), profile.curve(75.0)) else {
            return;
        };
        let slot_px = plot.w() * BAR_MINUTES / MINUTES_PER_DAY;
        let half_w = (slot_px * BAR_WIDTH / 2.0).max(1.0);
        let (img_w, img_h) = (img.width() as i32, img.height() as i32);
        let raw = img.as_mut();

        let bars = (MINUTES_PER_DAY / BAR_MINUTES) as usize;
        for bar in 0..bars {
            let minute = (bar as f32 + 0.5) * BAR_MINUTES;
            let (Some(lo), Some(hi)) = (curve_at(p25, minute), curve_at(p75, minute)) else {
                continue;
            };
            let cx = plot.x(minute);
            // A capsule: the segment between the end caps' centers, widened
            // by the radius. Short bars shrink to a dot.
            let (top, bottom) = (plot.y(hi), plot.y(lo));
            let r = half_w;
            let (seg_top, seg_bottom) = if bottom - top > 2.0 * r {
                (top + r, bottom - r)
            } else {
                let mid = (top + bottom) / 2.0;
                (mid, mid)
            };

            let x0 = ((cx - r).floor() as i32).max(0);
            let x1 = ((cx + r).ceil() as i32).min(img_w - 1);
            let y0 = ((seg_top - r).floor() as i32).max(0);
            let y1 = ((seg_bottom + r).ceil() as i32).min(img_h - 1);
            for py in y0..=y1 {
                let fy = py as f32 + 0.5;
                let c = self.status_color(plot.value_at(fy), minute);
                let (cr, cg, cb) = (c[0] as f32, c[1] as f32, c[2] as f32);
                let dy = fy - fy.clamp(seg_top, seg_bottom);
                for px in x0..=x1 {
                    let dx = px as f32 + 0.5 - cx;
                    let coverage = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let a = coverage * BAR_ALPHA / 255.0;
                    let i = (py * img_w + px) as usize * 4;
                    let dst = &mut raw[i..i + 3];
                    dst[0] = (cr * a + dst[0] as f32 * (1.0 - a)) as u8;
                    dst[1] = (cg * a + dst[1] as f32 * (1.0 - a)) as u8;
                    dst[2] = (cb * a + dst[2] as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }

    /// The median as a dotted line over the bars.
    fn draw_median(&self, img: &mut RgbaImage, plot: &Plot, profile: &PercentileProfile) {
        let Some(median) = profile.curve(50.0) else {
            return;
        };
        let s = self.scale();
        let dot =
            create_aa_circle_sprite((1.8 * s).round().max(1.0) as i32, self.theme.text_primary);
        let spacing = 7.0 * s;

        // Walk the curve pixel by pixel, dropping a dot every `spacing` of
        // path length so steep stretches keep the rhythm.
        let mut walked = spacing;
        let mut last: Option<(f32, f32)> = None;
        for x in plot.left.round() as i32..=plot.right.round() as i32 {
            let minute = (x as f32 - plot.left) / plot.w() * MINUTES_PER_DAY;
            let Some(v) = curve_at(median, minute) else {
                last = None;
                continue;
            };
            let p = (x as f32, plot.y(v));
            if let Some((lx, ly)) = last {
                walked += ((p.0 - lx).powi(2) + (p.1 - ly).powi(2)).sqrt();
            }
            if walked >= spacing {
                blend_sprite(img, &dot, p.0.round() as i32, p.1.round() as i32);
                walked = 0.0;
            }
            last = Some(p);
        }
    }

    /// Y value labels left of the plot, each on its gridline, with the unit
    /// above them.
    fn draw_value_labels(&self, img: &mut RgbaImage, card: &Card, font: &FontRef, ticks: &[f32]) {
        let (s, plot) = (card.s, &card.plot);
        let (size, size_xs) = (27.0 * s, 19.0 * s);
        let right = plot.left - 16.0 * s;
        let pref = unit_preference(self.unit_display);

        let mut y = plot.top - 22.0 * s;
        if self.is_dual() {
            y -= size_xs;
            draw_text_right(
                img,
                self.theme.text_dim,
                font,
                size_xs,
                right,
                y,
                unit_name(other_unit(pref)),
            );
            y -= 2.0 * s;
        }
        draw_text_right(
            img,
            self.theme.text_secondary,
            font,
            size_xs + 2.0 * s,
            right,
            y - size_xs - 2.0 * s,
            unit_name(pref),
        );

        for &v in ticks {
            let y = plot.y(v);
            draw_text_right(
                img,
                self.theme.text_primary,
                font,
                size,
                right,
                y - size / 2.0,
                &fmt_glucose(v, pref),
            );
            if self.is_dual() {
                draw_text_right(
                    img,
                    self.theme.text_dim,
                    font,
                    size_xs,
                    right,
                    y + size / 2.0,
                    &fmt_glucose(v, other_unit(pref)),
                );
            }
        }
    }

    /// Time of day under the plot, with a short tick for each.
    fn draw_time_labels(&self, img: &mut RgbaImage, card: &Card, font: &FontRef) {
        let (s, plot) = (card.s, &card.plot);
        let size = 23.0 * s;
        let [r, g, b, _] = self.theme.axis_lines.0;
        for hour in hour_marks(card) {
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
            let tx = (x - w / 2.0).clamp(plot.left - 6.0 * s, plot.right - w + 6.0 * s);
            draw_text(
                img,
                self.theme.text_primary,
                tx as i32,
                (plot.bottom + 22.0 * s) as i32,
                PxScale::from(size),
                font,
                &text,
            );
        }
    }

    /// The card's title ("14 days" by default) and dates, its old/new pill
    /// on the right, over a divider.
    fn draw_header(
        &self,
        img: &mut RgbaImage,
        card: &Card,
        font: &FontRef,
        i: usize,
        period: &Period,
    ) {
        let s = card.s;
        let (size_title, size_sm) = (36.0 * s, 23.0 * s);
        let (x, cy) = (card.x + card.pad, card.y + card.pad + 20.0 * s);
        let title = self.titles[i]
            .clone()
            .unwrap_or_else(|| days_label(period.profile.from, period.profile.to));

        // The tag pill: subtle for the first period, solid for the second so
        // the latest one stands out.
        let tag = &self.tags[i];
        let (size_tag, pill_h, pad_x) = (19.0 * s, 32.0 * s, 13.0 * s);
        let pill_w = text_w(font, tag, size_tag) + 2.0 * pad_x;
        let pill_x = card.x + card.w - card.pad - pill_w;
        let (fill, ink) = if i == 1 {
            (self.theme.text_primary, self.theme.background)
        } else {
            let c = self.theme.text_secondary;
            (Rgba([c[0], c[1], c[2], 45]), self.theme.text_secondary)
        };
        if !tag.is_empty() {
            draw_filled_rounded_rect(
                img,
                pill_x as i32,
                (cy - pill_h / 2.0) as i32,
                pill_w as u32,
                pill_h as u32,
                (pill_h / 2.0) as i32,
                fill,
            );
            draw_text(
                img,
                ink,
                (pill_x + pad_x) as i32,
                (cy - size_tag / 2.0 - 1.0 * s) as i32,
                PxScale::from(size_tag),
                font,
                tag,
            );
        }

        draw_text(
            img,
            self.theme.text_primary,
            x as i32,
            (cy - size_title / 2.0) as i32,
            PxScale::from(size_title),
            font,
            &title,
        );
        let dates = dates_label(period.profile.from, period.profile.to, self.timezone);
        let dx = x + text_w(font, &title, size_title) + 18.0 * s;
        if dx + text_w(font, &dates, size_sm) <= pill_x - 16.0 * s {
            draw_text(
                img,
                self.theme.text_secondary,
                dx as i32,
                (cy - size_sm / 2.0 + 3.0 * s) as i32,
                PxScale::from(size_sm),
                font,
                &dates,
            );
        }

        let [r, g, b, _] = self.theme.axis_lines.0;
        blend_fast_rect(
            img,
            x as i32,
            (card.header_bottom + 10.0 * s) as i32,
            (card.w - 2.0 * card.pad) as u32,
            s.ceil() as u32,
            Rgba([r, g, b, 40]),
        );
    }

    /// Average glucose, GMI, SD and CV in a row, over a divider. With
    /// `before`, each value gets a chip with its change from it. With two
    /// units, glucose values and changes are repeated in the second one.
    fn draw_stats(
        &self,
        img: &mut RgbaImage,
        card: &Card,
        font: &FontRef,
        stats: &TirStats,
        before: Option<&TirStats>,
    ) {
        let s = card.s;
        let (pref, second) = units(self.unit_display);
        let (size_label, size_value, size_unit) = (19.0 * s, 44.0 * s, 19.0 * s);
        let x = card.x + card.pad;
        let width = card.w - 2.0 * card.pad;
        let [r, g, b, _] = self.theme.axis_lines.0;
        blend_fast_rect(
            img,
            x as i32,
            (card.stats_top - 22.0 * s) as i32,
            width as u32,
            s.ceil() as u32,
            Rgba([r, g, b, 40]),
        );

        let stat = |label, value, before, glucose| Stat {
            label,
            value,
            before,
            glucose,
        };
        let items = [
            stat(
                "Average",
                stats.mean_mgdl,
                before.map(|b| b.mean_mgdl),
                true,
            ),
            stat(
                "GMI",
                stats.gmi_percent,
                before.map(|b| b.gmi_percent),
                false,
            ),
            stat("SD", stats.sd_mgdl, before.map(|b| b.sd_mgdl), true),
            stat("CV", stats.cv_percent, before.map(|b| b.cv_percent), false),
        ];
        // Glucose columns are wider than percentage ones: their unit is
        // longer, and "10.2 mmol/L" has to fit.
        let share = |glucose: bool| if glucose { 1.15 } else { 0.85 };
        let column = width / items.iter().map(|i| share(i.glucose)).sum::<f32>();
        let mut cx = x;
        for item in items {
            let fmt = |v: f32, unit: UnitPreference| {
                if item.glucose {
                    fmt_glucose(v, unit)
                } else {
                    format!("{v:.1}")
                }
            };
            let (label, value, was) = (item.label, item.value, item.before);
            let unit = if item.glucose { unit_name(pref) } else { "%" };
            // Compares what is shown, so the change matches the numbers.
            let change = |unit: UnitPreference| {
                let now = fmt(value, unit);
                was.map(|was| signed(parse(&now) - parse(&fmt(was, unit)), &now))
            };
            let second = second.filter(|_| item.glucose);
            let top = card.stats_top;
            draw_text(
                img,
                self.theme.text_secondary,
                cx as i32,
                top as i32,
                PxScale::from(size_label),
                font,
                label,
            );
            let value_text = fmt(value, pref);
            let value_top = top + size_label + 8.0 * s;
            draw_text(
                img,
                self.theme.text_primary,
                cx as i32,
                value_top as i32,
                PxScale::from(size_value),
                font,
                &value_text,
            );
            let vw = text_w(font, &value_text, size_value);
            draw_text(
                img,
                self.theme.text_secondary,
                (cx + vw + 6.0 * s) as i32,
                (value_top + size_value - size_unit - 6.0 * s) as i32,
                PxScale::from(size_unit),
                font,
                unit,
            );

            // The second unit's line pushes every column's chip down, so
            // they stay level.
            let mut chip_top = value_top + size_value + 8.0 * s;
            if self.is_dual() {
                chip_top += size_unit + 2.0 * s;
            }
            if let Some(unit) = second {
                draw_text(
                    img,
                    self.theme.text_dim,
                    cx as i32,
                    (value_top + size_value + 2.0 * s) as i32,
                    PxScale::from(size_unit),
                    font,
                    &fmt_glucose_unit(value, unit),
                );
            }

            if let Some(text) = change(pref) {
                let chip_h = 26.0 * s;
                let tw = text_w(font, &text, size_unit);
                let c = self.theme.text_secondary;
                draw_filled_rounded_rect(
                    img,
                    cx as i32,
                    chip_top as i32,
                    (tw + 18.0 * s) as u32,
                    chip_h as u32,
                    (chip_h / 2.0) as i32,
                    Rgba([c[0], c[1], c[2], 45]),
                );
                draw_text(
                    img,
                    self.theme.text_primary,
                    (cx + 9.0 * s) as i32,
                    (chip_top + (chip_h - size_unit) / 2.0 - 1.0 * s) as i32,
                    PxScale::from(size_unit),
                    font,
                    &text,
                );
                if let Some(text) = second.and_then(change) {
                    draw_text(
                        img,
                        self.theme.text_dim,
                        (cx + tw + 26.0 * s) as i32,
                        (chip_top + (chip_h - size_unit) / 2.0 - 1.0 * s) as i32,
                        PxScale::from(size_unit),
                        font,
                        &text,
                    );
                }
            }
            cx += column * share(item.glucose);
        }
    }
}

/// One number in a card's statistics row.
struct Stat {
    label: &'static str,
    value: f32,
    /// The same number for the first period, shown on the second card as a
    /// change.
    before: Option<f32>,
    /// A glucose value, in the display unit, rather than a percentage.
    glucose: bool,
}

/// A period's numbers.
struct Period {
    profile: PercentileProfile,
    stats: TirStats,
}

/// One period's card and the plot inside it.
struct Card {
    s: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    pad: f32,
    header_bottom: f32,
    stats_top: f32,
    plot: Plot,
}

/// The plot area in pixels and the glucose range it shows, in mg/dL.
struct Plot {
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
        self.left + minute / MINUTES_PER_DAY * self.w()
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

/// Hours of the day labelled on the X axis: every 6 hours, or every 3 when
/// the plot is wide enough.
fn hour_marks(card: &Card) -> Vec<u32> {
    let every = if card.plot.w() / 8.0 >= 110.0 * card.s {
        3
    } else {
        6
    };
    (0..=24).step_by(every).collect()
}

fn parse(text: &str) -> f32 {
    text.parse().unwrap_or(0.0)
}

/// `delta` signed, with as many decimals as `like`.
fn signed(delta: f32, like: &str) -> String {
    let decimals = like.split('.').nth(1).map_or(0, str::len);
    let magnitude = format!("{:.*}", decimals, delta.abs());
    if magnitude.chars().all(|c| c == '0' || c == '.') {
        format!("±{magnitude}")
    } else if delta > 0.0 {
        format!("+{magnitude}")
    } else {
        format!("-{magnitude}")
    }
}

/// "14 days", from the first to the last reading.
fn days_label(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    let days = ((to - from).num_minutes() as f64 / 1440.0).round().max(1.0) as i64;
    if days == 1 {
        "1 day".to_string()
    } else {
        format!("{days} days")
    }
}

/// "30 Aug – 12 Sep" in `timezone`, with years when they differ.
pub(crate) fn dates_label(from: DateTime<Utc>, to: DateTime<Utc>, timezone: Tz) -> String {
    let (from, to) = (from.with_timezone(&timezone), to.with_timezone(&timezone));
    let pattern = if from.year_ce() != to.year_ce() {
        "%-d %b %Y"
    } else {
        "%-d %b"
    };
    format!("{} – {}", from.format(pattern), to.format(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UnitPreference;
    use chrono::{Duration, TimeZone};

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn night() -> NightTargets {
        NightTargets {
            low: 90.0,
            high: 180.0,
            from: time(23, 0),
            until: time(7, 0),
        }
    }

    /// 14 days of 5-minute readings around `base`, starting `days_ago`.
    fn fortnight(days_ago: i64, base: f32) -> Vec<GraphEntry> {
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap() - Duration::days(days_ago);
        (0..14 * 288)
            .map(|i| GraphEntry {
                sgv: base
                    + 30.0 * ((i % 288) as f32 / 288.0 * std::f32::consts::TAU).sin()
                    + ((i / 288) as f32 - 6.5) * 4.0,
                date: start + Duration::minutes(i as i64 * 5),
            })
            .collect()
    }

    #[test]
    fn night_wraps_past_midnight() {
        let n = night();
        assert!(n.covers(23.0 * 60.0));
        assert!(n.covers(0.0));
        assert!(n.covers(6.0 * 60.0 + 59.0));
        assert!(!n.covers(7.0 * 60.0));
        assert!(!n.covers(12.0 * 60.0));

        let afternoon = NightTargets {
            from: time(13, 0),
            until: time(15, 0),
            ..n
        };
        assert!(afternoon.covers(14.0 * 60.0));
        assert!(!afternoon.covers(0.0));
    }

    #[test]
    fn targets_step_at_the_night_edges() {
        let b = CompareGraphBuilder::new()
            .with_targets(80.0, 180.0)
            .with_night_targets(night());
        assert_eq!(b.targets_at(2.0 * 60.0), (90.0, 180.0));
        assert_eq!(b.targets_at(12.0 * 60.0), (80.0, 180.0));
        assert_eq!(
            b.target_stretches(false),
            vec![
                (0.0, 420.0, 90.0),
                (420.0, 1380.0, 80.0),
                (1380.0, 1440.0, 90.0)
            ]
        );
        // The high target doesn't change, so it stays one stretch.
        assert_eq!(b.target_stretches(true), vec![(0.0, 1440.0, 180.0)]);
        assert_eq!(
            CompareGraphBuilder::new().target_stretches(false),
            vec![(0.0, 1440.0, 70.0)]
        );
    }

    #[test]
    fn changes_are_signed_like_the_values() {
        assert_eq!(signed(-4.0, "161"), "-4");
        assert_eq!(signed(0.1, "7.2"), "+0.1");
        assert_eq!(signed(0.04, "7.2"), "±0.0");
        assert_eq!(signed(-0.0, "34"), "±0");
    }

    #[test]
    fn labels_describe_the_period() {
        let from = Utc.with_ymd_and_hms(2026, 8, 30, 0, 0, 0).unwrap();
        let to = from + Duration::days(14) - Duration::minutes(5);
        assert_eq!(days_label(from, to), "14 days");
        assert_eq!(days_label(from, from), "1 day");
        assert_eq!(dates_label(from, to, chrono_tz::UTC), "30 Aug – 12 Sep");
        let new_year = Utc.with_ymd_and_hms(2026, 12, 25, 0, 0, 0).unwrap();
        assert_eq!(
            dates_label(new_year, new_year + Duration::days(10), chrono_tz::UTC),
            "25 Dec 2026 – 4 Jan 2027"
        );
    }

    #[test]
    fn both_periods_share_one_scale() {
        let b = CompareGraphBuilder::new();
        let thresholds = TirThresholds::default();
        let periods: Vec<Period> = [fortnight(28, 110.0), fortnight(14, 230.0)]
            .iter()
            .map(|e| Period {
                profile: PercentileProfile::compute(e, chrono_tz::UTC, &[25.0, 50.0, 75.0])
                    .unwrap(),
                stats: TirStats::compute(e, &thresholds).unwrap(),
            })
            .collect();
        let (lo, hi) = b.y_range(&periods);
        assert!(lo <= 90.0 && hi >= 250.0, "{lo}..{hi}");
    }

    #[test]
    fn builds_for_every_setup() {
        let setups = vec![
            CompareGraphBuilder::new(),
            CompareGraphBuilder::new()
                .with_night_targets(night())
                .with_titles("Before", "After")
                .with_tags("Avant", "Après"),
            CompareGraphBuilder::new().with_tags("", ""),
            CompareGraphBuilder::new()
                .with_units(UnitDisplay::Dual {
                    primary: UnitPreference::MmolL,
                })
                .with_theme(Theme::paper_light()),
            CompareGraphBuilder::new()
                .with_scaling(GraphScaling::Static {
                    min: 0.0,
                    max: 400.0,
                })
                .with_layout(LayoutConfig {
                    width: 600,
                    height: 340,
                    ..Default::default()
                }),
        ];
        for b in setups {
            let img = b
                .with_periods(fortnight(28, 160.0), fortnight(14, 150.0))
                .build()
                .unwrap();
            assert!(img.width() > 0);
        }
    }

    #[test]
    fn a_missing_period_is_an_error() {
        assert!(CompareGraphBuilder::new().build().is_err());
        let one_sided =
            CompareGraphBuilder::new().with_periods(fortnight(14, 150.0), Vec::<GraphEntry>::new());
        assert!(one_sided.build().is_err());
    }
}
