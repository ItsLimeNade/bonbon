use crate::models::{
    GraphEntry, GraphScaling, GraphTreatment, TimeAxisMode, TreatmentDisplayMode, UnitDisplay,
    UnitPreference,
};
use crate::theme::Theme;
use crate::utils::color::darken_color;
use crate::utils::drawing::{
    draw_dashed_horizontal_line, draw_dashed_vertical_line, draw_smart_circle, draw_smart_triangle,
};

use ab_glyph::{FontRef, PxScale};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut, draw_text_mut};

/// Configuration for the visual layout of the graph.
#[derive(Clone, Debug)]
pub struct LayoutConfig {
    pub width: u32,
    pub height: u32,
    /// If None, defaults to 80.0 * scale
    pub margin_top: Option<f32>,
    /// If None, defaults to 100.0 * scale
    pub margin_bottom: Option<f32>,
    /// If None, defaults to 120.0 * scale
    pub margin_left: Option<f32>,
    /// If None, defaults to 60.0 * scale
    pub margin_right: Option<f32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
            margin_top: None,
            margin_bottom: None,
            margin_left: None,
            margin_right: None,
        }
    }
}

struct GraphViewport {
    s: f32,
    plot_left: f32,
    plot_top: f32,
    plot_right: f32,
    plot_bottom: f32,
    plot_w: f32,
    plot_h: f32,
}

struct RenderContext<'a> {
    viewport: GraphViewport,
    start_time: chrono::DateTime<Utc>,
    #[allow(dead_code)]
    end_time: chrono::DateTime<Utc>,
    time_span_secs: f32,
    y_min: f32,
    y_max: f32,
    font: &'a FontRef<'a>,
}

impl<'a> RenderContext<'a> {
    fn project_x(&self, time: chrono::DateTime<Utc>) -> f32 {
        let offset = (time - self.start_time).num_seconds() as f32;
        self.viewport.plot_left + (offset / self.time_span_secs) * self.viewport.plot_w
    }

    fn project_y(&self, sgv: f32) -> f32 {
        let clamped = sgv.clamp(self.y_min, self.y_max);
        let ratio = (clamped - self.y_min) / (self.y_max - self.y_min);
        self.viewport.plot_bottom - (ratio * self.viewport.plot_h)
    }
}

type TimeRange = (chrono::DateTime<Utc>, chrono::DateTime<Utc>);

/// Builder for creating a Glucose Graph.
pub struct GlucoseGraphBuilder<'a> {
    entries: Vec<GraphEntry>,
    treatments: Vec<GraphTreatment>,
    target_low: f32,
    target_high: f32,
    unit_display: UnitDisplay,
    scaling: GraphScaling,
    treatment_mode: TreatmentDisplayMode,
    time_axis_mode: TimeAxisMode,
    fixed_duration: Option<Duration>,
    custom_start: Option<chrono::DateTime<Utc>>,
    timezone: Tz,
    layout: LayoutConfig,
    theme: Theme,
    font: &'a [u8],
    microbolus_threshold: f32,
}

impl<'a> GlucoseGraphBuilder<'a> {
    pub fn new(theme: Theme, font_data: &'a [u8]) -> Self {
        Self {
            entries: Vec::new(),
            treatments: Vec::new(),
            target_low: 70.0,
            target_high: 180.0,
            unit_display: UnitDisplay::MgDl,
            scaling: GraphScaling::default(),
            fixed_duration: None,
            treatment_mode: TreatmentDisplayMode::default(),
            time_axis_mode: TimeAxisMode::default(),
            custom_start: None,
            timezone: chrono_tz::UTC,
            layout: LayoutConfig::default(),
            theme,
            font: font_data,
            microbolus_threshold: 0.0,
        }
    }

    /// Sets the graph's entries to the given list, overwriting any existing ones.
    ///
    /// This method is generic: it accepts any iterator of items that can be converted
    /// into a `GraphEntry`.
    pub fn with_entries<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
    {
        self.entries = entries.into_iter().map(|e| e.into()).collect();
        self
    }

    /// Adds a list of entries to the graph, appending them to any existing ones.
    ///
    /// This method is generic: it accepts any iterator of items that can be converted
    /// into a `GraphEntry`.
    pub fn add_entries<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<GraphEntry>,
    {
        self.entries.extend(entries.into_iter().map(|e| e.into()));
        self
    }

    /// Sets the graph's treatments to the given list, overwriting any existing ones.
    ///
    /// This method is generic: it accepts any iterator of items that can be converted
    /// into a `GraphTreatment`.
    pub fn with_treatments<I>(mut self, treatments: I) -> Self
    where
        I: IntoIterator,
        I::Item: TryInto<GraphTreatment>,
    {
        self.treatments = treatments
            .into_iter()
            .filter_map(|t| t.try_into().ok())
            .collect();
        self
    }

    /// Adds a list of treatments to the graph, appending them to any existing ones.
    ///
    /// This method is generic: it accepts ANY iterator of items that can be converted
    /// into a `GraphTreatment`. This works for:
    /// - `Vec<GraphTreatment>` (Native)
    /// - `Vec<Treatment>` (Cinnamon, via TryFrom)
    /// - `Vec<MbgEntry>` (Cinnamon, via From -> TryInto)
    pub fn add_treatments<I, T>(mut self, new_treatments: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: TryInto<GraphTreatment>,
    {
        self.treatments
            .extend(new_treatments.into_iter().filter_map(|t| t.try_into().ok()));

        self
    }

    /// Sets the graph's low and high targets. They will appear as dashed lines, that when crossed will
    /// change the entrie's color to the corresponding state's color.
    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.target_low = low;
        self.target_high = high;
        self
    }

    /// Sets the graph Y axis unit measurement system. Adds corresponding labels to the graph.
    pub fn with_units(mut self, display: UnitDisplay) -> Self {
        self.unit_display = display;
        self
    }

    /// Sets the graph's viewport scaling.
    ///
    /// If `GraphScaling::Static` is used, the graph will always keep the given range no matter the entries' max and min values.
    ///
    /// If `GraphScaling::Dynamic` is used, the graph's viewport scale will by default be `default_min` and `default_max`'s values.
    /// If an entry goes over/under the default values, the graph's viewport will scale accordingly.
    pub fn with_scaling(mut self, scaling: GraphScaling) -> Self {
        self.scaling = scaling;
        self
    }

    /// Sets the graph's timezone to generate timestamp labels on the X axis accurate to the entries' data.
    pub fn with_timezone(mut self, tz: Tz) -> Self {
        self.timezone = tz;
        self
    }

    /// Set the graph's internal layout.
    pub fn with_layout(mut self, layout: LayoutConfig) -> Self {
        self.layout = layout;
        self
    }

    /// Sets the graph's font.
    pub fn with_font(mut self, font: &'a [u8]) -> Self {
        self.font = font;
        self
    }

    /// Sets the graph's theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets how treatments are positioned (Timeline vs Contextual).
    pub fn with_treatment_mode(mut self, mode: TreatmentDisplayMode) -> Self {
        self.treatment_mode = mode;
        self
    }

    /// Sets the style of the X-axis dates.
    pub fn with_time_axis(mut self, mode: TimeAxisMode) -> Self {
        self.time_axis_mode = mode;
        self
    }

    /// Forces the graph to display a specific time range (e.g., 24 hours) ending at "now" (or the latest entry).
    ///
    /// If set, this overrides the dynamic auto-scaling of the time axis.
    pub fn with_fixed_duration(mut self, duration: Duration) -> Self {
        self.fixed_duration = Some(duration);
        self
    }

    /// Sets the specific start time for the graph window.
    ///
    /// When used with `with_fixed_duration`, this creates a precise window: [start, start + duration].
    /// When used without a duration, it shows data from `date` to the last available entry.
    pub fn start_at(mut self, date: chrono::DateTime<Utc>) -> Self {
        self.custom_start = Some(date);
        self
    }

    /// Sets the threshold for microboluses.
    /// Insulin treatments <= this value will be rendered as small ticks without text labels.
    pub fn with_microbolus_threshold(mut self, threshold: f32) -> Self {
        self.microbolus_threshold = threshold;
        self
    }

    /// Builds the final image, returning an ImageBuffer.
    pub fn build(mut self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let font = FontRef::try_from_slice(self.font)?;
        let mut img =
            RgbaImage::from_pixel(self.layout.width, self.layout.height, self.theme.background);

        // Calculate the graph's viewport first, this allows for a scalable margin system
        // and correct entry positionning later on.
        let viewport = self.calculate_viewport();

        // Here we're sorting the entries with worst-case scenario of O(n * log(n)) for the time complexity.
        // Sorting them will allow us to correctly fetch the graph's time span.
        self.entries.sort_unstable_by_key(|e| e.date);
        let (start_time, end_time) = self.determine_time_range()?;
        let time_span_secs = (end_time - start_time).num_seconds().max(1) as f32;

        // Optimization function used to remove any entries that are not present in the graph.
        // If we were to render all entries given by the user, some would not be rendered on the
        // graph but would take calculation time.
        // By removing them we're optimizing a bit in some scenarios the graph rendering time.
        let visible_entries_slice = self.get_visible_entries(start_time, end_time);

        let (y_min, y_max) = self.calculate_y_scaling(visible_entries_slice);

        // Easly reusable context object for the other private helper functions.
        // Could've made it inside the graph's struct but ultimately this is the best option
        // in my opinion.
        let ctx = RenderContext {
            viewport,
            start_time,
            end_time,
            time_span_secs,
            y_min,
            y_max,
            font: &font,
        };

        // Pretty self-explanatory, helper functions to render the graphics.
        self.draw_target_lines(&mut img, &ctx);
        self.draw_date_separators(&mut img, &ctx);
        self.draw_time_axis(&mut img, &ctx);
        self.draw_axis_border(&mut img, &ctx);
        self.draw_labels_and_units(&mut img, &ctx);

        // Same optimization than for entries.
        let visible_treatments = self.get_visible_treatments(start_time, end_time);
        self.draw_treatments(&mut img, &ctx, &visible_treatments);

        // Draw entries last so they appear on top, making for a clear view of the graph.
        self.draw_entries(&mut img, &ctx, visible_entries_slice);

        Ok(img)
    }

    /// Private helper function used to calculate the graph's view port and help
    /// automatically scaling the margins for a clean-looking graph.
    fn calculate_viewport(&self) -> GraphViewport {
        let base_width = 1200.0;
        let base_height = 800.0;
        let scale_x = self.layout.width as f32 / base_width;
        let scale_y = self.layout.height as f32 / base_height;
        // I actually completely forgot I what made here TwT"
        let s = scale_x.min(scale_y).max(0.5);

        // `.unwrap_or` used to set the default values
        let m_top = self.layout.margin_top.unwrap_or(80.0 * s);
        let m_bottom = self.layout.margin_bottom.unwrap_or(100.0 * s);
        let m_left = self.layout.margin_left.unwrap_or(120.0 * s);
        let m_right = self.layout.margin_right.unwrap_or(60.0 * s);

        let plot_w = self.layout.width as f32 - m_left - m_right;
        let plot_h = self.layout.height as f32 - m_top - m_bottom;

        GraphViewport {
            s,
            plot_left: m_left,
            plot_top: m_top,
            plot_right: m_left + plot_w,
            plot_bottom: m_top + plot_h,
            plot_w,
            plot_h,
        }
    }

    /// Private helper function used to find the graph's viewport data time range.
    fn determine_time_range(&self) -> Result<TimeRange, Box<dyn std::error::Error>> {
        // Set explicite start date mode
        if let Some(start) = self.custom_start {
            let end = if let Some(duration) = self.fixed_duration {
                // Window = [Start, Start + Duration]
                start + duration
            } else {
                // Window = [Start, End of Data]
                if self.entries.is_empty() {
                    return Err(
                        "No entries provided and no fixed duration set with custom start".into(),
                    );
                }
                self.entries.last().unwrap().date
            };
            return Ok((start, end));
        }

        // Fixed Duration Mode
        // Auto-calculates the end anchor based on data recency
        if let Some(duration) = self.fixed_duration {
            let now = Utc::now();
            let anchor = if self.entries.is_empty() {
                now
            } else {
                let last_entry = self.entries.last().unwrap().date;
                // ! Questionable practice, should I delete?
                // If the data is older than 24h, anchor to the data instead of "now"
                if (now - last_entry).num_hours() > 24 {
                    last_entry
                } else {
                    now
                }
            };
            Ok((anchor - duration, anchor))
        }
        // Auto-Fit Mode
        // Fits the window to exactly cover all provided entries
        else {
            if self.entries.is_empty() {
                // ! Remember to implement custom errors!
                return Err("No entries provided and no fixed duration set".into());
            }
            let end = self.entries.last().unwrap().date;
            let start = self.entries.first().unwrap().date;
            let adjusted_start = if start == end {
                end - Duration::hours(1)
            } else {
                start
            };
            Ok((adjusted_start, end))
        }
    }

    /// Helper function that filters unrendered entries to optimize compute time.
    fn get_visible_entries(
        &self,
        start: chrono::DateTime<Utc>,
        end: chrono::DateTime<Utc>,
    ) -> &[GraphEntry] {
        let start_idx = self.entries.partition_point(|e| e.date < start);
        let end_idx = self.entries.partition_point(|e| e.date <= end);
        &self.entries[start_idx..end_idx]
    }

    /// Private helper function that calculates the graph's viewport scaling, helping with the
    /// dynamic scaling mode.
    fn calculate_y_scaling(&self, visible_entries: &[GraphEntry]) -> (f32, f32) {
        match self.scaling {
            GraphScaling::Static { min, max } => (min, max),
            GraphScaling::Dynamic {
                clamp_min,
                clamp_max,
                default_min,
                default_max,
            } => {
                if visible_entries.is_empty() {
                    (clamp_min, clamp_max)
                } else {
                    let (min_sgv, max_sgv) = visible_entries
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(min, max), e| {
                            (min.min(e.sgv), max.max(e.sgv))
                        });

                    let calc_max = ((max_sgv + 20.0) / 10.0).ceil() * 10.0;
                    let calc_min = ((min_sgv - 20.0) / 10.0).floor() * 10.0;

                    let view_min = calc_min.min(default_min);
                    let view_max = calc_max.max(default_max);

                    (view_min.max(clamp_min), view_max.min(clamp_max))
                }
            }
        }
    }

    /// Private helper function that draws the target on the graph.
    /// Automatically adapts it's thickness to the graph's size.
    fn draw_target_lines(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let high_y = ctx.project_y(self.target_high);
        let low_y = ctx.project_y(self.target_low);

        let high_col = Rgba([
            self.theme.glucose_high[0],
            self.theme.glucose_high[1],
            self.theme.glucose_high[2],
            80,
        ]);
        let low_col = Rgba([
            self.theme.glucose_low[0],
            self.theme.glucose_low[1],
            self.theme.glucose_low[2],
            80,
        ]);

        let grid_thickness = (1.0 * ctx.viewport.s).ceil() as i32;

        draw_dashed_horizontal_line(
            img,
            high_y,
            ctx.viewport.plot_left,
            ctx.viewport.plot_right,
            high_col,
            (10.0 * ctx.viewport.s) as i32,
            (10.0 * ctx.viewport.s) as i32,
            grid_thickness,
        );
        draw_dashed_horizontal_line(
            img,
            low_y,
            ctx.viewport.plot_left,
            ctx.viewport.plot_right,
            low_col,
            (10.0 * ctx.viewport.s) as i32,
            (10.0 * ctx.viewport.s) as i32,
            grid_thickness,
        );
    }

    /// Private helper function that draws dashed date separators on the graph when the date
    /// changes.
    fn draw_date_separators(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let local_start = ctx.start_time.with_timezone(&self.timezone);
        let local_end = ctx.end_time.with_timezone(&self.timezone);
        let font_size_sm = (24.0 + 1.0) * ctx.viewport.s;
        let grid_dash = (6.0 * ctx.viewport.s) as i32;
        let grid_thickness = (1.0 * ctx.viewport.s).ceil() as i32;
        let separator_thickness = (grid_thickness as f32 * 1.5).ceil() as i32 + 1;

        let mut pointer = local_start
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(self.timezone)
            .unwrap();
        if pointer < local_start {
            pointer = pointer + Duration::days(1);
        }

        while pointer <= local_end {
            let x = ctx.project_x(pointer.with_timezone(&Utc));
            if x >= ctx.viewport.plot_left && x <= ctx.viewport.plot_right {
                draw_dashed_vertical_line(
                    img,
                    x,
                    ctx.viewport.plot_top,
                    ctx.viewport.plot_bottom,
                    self.theme.axis_lines,
                    grid_dash,
                    grid_dash,
                    separator_thickness,
                );

                let date_str = pointer.format("%d/%m").to_string();
                let tx = (x + 5.0 * ctx.viewport.s) as i32;
                let ty = (ctx.viewport.plot_top + 5.0 * ctx.viewport.s) as i32;

                draw_text_mut(
                    img,
                    self.theme.text_secondary,
                    tx,
                    ty,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    &date_str,
                );
            }
            pointer = pointer + Duration::days(1);
        }
    }

    /// Private helper function that draws the time axis onto the graph.
    fn draw_time_axis(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        if let TimeAxisMode::EquallyDistributed { count } = self.time_axis_mode {
            let step_secs = ctx.time_span_secs / (count as f32);
            let font_size_sm = (24.0 + 1.0) * ctx.viewport.s;
            let font_size_xs = (20.0 + 1.0) * ctx.viewport.s;
            let grid_dash = (6.0 * ctx.viewport.s) as i32;
            let grid_thickness = (1.0 * ctx.viewport.s).ceil() as i32;

            for i in 0..=count {
                let offset = i as f32 * step_secs;
                let tick_time = ctx.start_time + Duration::seconds(offset as i64);
                let x =
                    ctx.viewport.plot_left + (offset / ctx.time_span_secs) * ctx.viewport.plot_w;

                if x > ctx.viewport.plot_right + 1.0 {
                    continue;
                }

                draw_dashed_vertical_line(
                    img,
                    x,
                    ctx.viewport.plot_top,
                    ctx.viewport.plot_bottom,
                    self.theme.grid_major,
                    grid_dash,
                    grid_dash,
                    grid_thickness,
                );

                let local_time = tick_time.with_timezone(&self.timezone);
                let time_str = local_time.format("%H:%M").to_string();
                let dim_time = text_dimensions(&time_str, font_size_sm, ctx.font);

                let mut tx = (x - dim_time.0 / 2.0) as i32;
                let min_tx = ctx.viewport.plot_left as i32;
                let max_tx = (ctx.viewport.plot_right - dim_time.0) as i32;
                tx = tx.clamp(min_tx, max_tx);

                let ty = (ctx.viewport.plot_bottom + 25.0 * ctx.viewport.s) as i32;

                draw_text_mut(
                    img,
                    self.theme.text_primary,
                    tx,
                    ty,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    &time_str,
                );

                let diff_secs = (ctx.end_time - tick_time).num_seconds();
                let hours = diff_secs as f32 / 3600.0;
                let rel_str = if hours.abs() < 0.1 {
                    "-0h".to_string()
                } else {
                    format!("-{:.1}h", hours)
                };
                let dim_rel = text_dimensions(&rel_str, font_size_xs, ctx.font);
                let mut rx = (x - dim_rel.0 / 2.0) as i32;
                let max_rx = (ctx.viewport.plot_right - dim_rel.0) as i32;
                rx = rx.clamp(min_tx, max_rx);
                let ry = (ctx.viewport.plot_bottom
                    + 25.0 * ctx.viewport.s
                    + dim_time.1
                    + 4.0 * ctx.viewport.s) as i32;

                draw_text_mut(
                    img,
                    self.theme.text_dim,
                    rx,
                    ry,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    &rel_str,
                );
            }
        }
    }

    /// Private helper function that draws the... you guessed it! Borders of the axis lines!
    fn draw_axis_border(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let axis_thickness = (2.0 * ctx.viewport.s).ceil() as i32;
        for i in 0..axis_thickness {
            let offset = i as f32;
            let overlap_offset: f32 = 3.0 * axis_thickness as f32;
            // Y axis
            draw_line_segment_mut(
                img,
                (
                    ctx.viewport.plot_left - offset - overlap_offset,
                    ctx.viewport.plot_top - overlap_offset,
                ),
                (
                    ctx.viewport.plot_left - offset - overlap_offset,
                    ctx.viewport.plot_bottom + overlap_offset,
                ),
                self.theme.axis_lines,
            );
            // X axis
            draw_line_segment_mut(
                img,
                (
                    ctx.viewport.plot_left - overlap_offset,
                    ctx.viewport.plot_bottom + offset + overlap_offset,
                ),
                (
                    ctx.viewport.plot_right + overlap_offset,
                    ctx.viewport.plot_bottom + offset + overlap_offset,
                ),
                self.theme.axis_lines,
            );
        }
    }

    fn draw_labels_and_units(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        // Easy access to size changes
        let font_size_md = (30.0 + 1.0) * ctx.viewport.s;
        let font_size_sm = (24.0 + 1.0) * ctx.viewport.s;
        let font_size_xs = (20.0 + 1.0) * ctx.viewport.s;

        let unit_anchor_x = ctx.viewport.plot_left - (10.0 * ctx.viewport.s);
        let unit_start_y =
            ctx.viewport.plot_bottom + (font_size_md / 2.0) + (15.0 * ctx.viewport.s);

        match self.unit_display {
            UnitDisplay::MgDl => {
                let text = "mg/dL";
                let dim = text_dimensions(text, font_size_sm, ctx.font);
                draw_text_mut(
                    img,
                    self.theme.text_primary,
                    (unit_anchor_x - dim.0) as i32,
                    unit_start_y as i32,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    text,
                );
            }
            UnitDisplay::MmolL => {
                let text = "mmol/L";
                let dim = text_dimensions(text, font_size_sm, ctx.font);
                draw_text_mut(
                    img,
                    self.theme.text_primary,
                    (unit_anchor_x - dim.0) as i32,
                    unit_start_y as i32,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    text,
                );
            }
            UnitDisplay::Dual { primary } => {
                let (u1, u2) = match primary {
                    UnitPreference::MgDl => ("mg/dL", "mmol/L"),
                    UnitPreference::MmolL => ("mmol/L", "mg/dL"),
                };
                let dim1 = text_dimensions(u1, font_size_sm, ctx.font);
                let dim2 = text_dimensions(u2, font_size_xs, ctx.font);

                let tx1 = (unit_anchor_x - dim1.0) as i32;
                let ty1 = unit_start_y as i32;
                draw_text_mut(
                    img,
                    self.theme.text_primary,
                    tx1,
                    ty1,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    u1,
                );

                let tx2 = (unit_anchor_x - dim2.0) as i32;
                let ty2 = (unit_start_y + dim1.1 + 2.0 * ctx.viewport.s) as i32;
                draw_text_mut(
                    img,
                    self.theme.text_dim,
                    tx2,
                    ty2,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    u2,
                );
            }
        }

        let steps = 6;
        let step_size = (ctx.y_max - ctx.y_min) / (steps as f32);
        for i in 0..=steps {
            let val = ctx.y_min + (i as f32 * step_size);
            let y_pos = ctx.project_y(val);
            let (main_text, sub_text) = match self.unit_display {
                UnitDisplay::MgDl => (format!("{:.0}", val), None),
                UnitDisplay::MmolL => (format!("{:.1}", val / 18.0), None),
                UnitDisplay::Dual { primary } => match primary {
                    UnitPreference::MgDl => {
                        (format!("{:.0}", val), Some(format!("{:.1}", val / 18.0)))
                    }
                    UnitPreference::MmolL => {
                        (format!("{:.1}", val / 18.0), Some(format!("{:.0}", val)))
                    }
                },
            };

            let main_dim = text_dimensions(&main_text, font_size_md, ctx.font);
            let main_tx = (ctx.viewport.plot_left - main_dim.0 - 10.0 * ctx.viewport.s) as i32;
            let main_ty = (y_pos - main_dim.1 / 2.0) as i32;

            draw_text_mut(
                img,
                self.theme.text_primary,
                main_tx,
                main_ty,
                PxScale::from(font_size_md),
                ctx.font,
                &main_text,
            );

            if let Some(sub) = sub_text {
                let sub_dim = text_dimensions(&sub, font_size_xs, ctx.font);
                let sub_tx = (ctx.viewport.plot_left - sub_dim.0 - 10.0 * ctx.viewport.s) as i32;
                let sub_ty = (y_pos + main_dim.1 / 2.0) as i32;

                draw_text_mut(
                    img,
                    self.theme.text_dim,
                    sub_tx,
                    sub_ty,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    &sub,
                );
            }
        }
    }

    /// Private helper function to avoid rendering out-of-bounds treatments.
    fn get_visible_treatments(
        &self,
        start: chrono::DateTime<Utc>,
        end: chrono::DateTime<Utc>,
    ) -> Vec<&GraphTreatment> {
        let mut visible: Vec<&GraphTreatment> = self
            .treatments
            .iter()
            .filter(|t| t.date >= start && t.date <= end)
            .collect();
        visible.sort_by_key(|t| t.date);
        visible
    }

    /// Private helper functions to draw treatments on the graph.
    fn draw_treatments(
        &self,
        img: &mut RgbaImage,
        ctx: &RenderContext,
        treatments: &[&GraphTreatment],
    ) {
        let point_radius = (6.0 + 1.0) * ctx.viewport.s;
        let font_size_xs = (20.0 + 1.0) * ctx.viewport.s;
        let font_size_sm = (24.0 + 1.0) * ctx.viewport.s;
        let font_size_ctx = (26.0 + 1.0) * ctx.viewport.s;

        // Draw MBG Circles first (independent of mode)
        for t in treatments {
            if let Some(mbg) = t.mbg {
                let x = ctx.project_x(t.date);
                let y = ctx.project_y(mbg);
                let outline_r = (point_radius * 1.5) as i32;
                let fill_r = point_radius as i32;
                draw_filled_circle_mut(
                    img,
                    (x as i32, y as i32),
                    outline_r,
                    self.theme.glucose_reading_outline,
                );
                draw_filled_circle_mut(
                    img,
                    (x as i32, y as i32),
                    fill_r,
                    self.theme.glucose_reading_fill,
                );

                let (val_str, _) = match self.unit_display {
                    UnitDisplay::MgDl
                    | UnitDisplay::Dual {
                        primary: UnitPreference::MgDl,
                    } => (format!("{:.0}", mbg), "mg/dL"),
                    UnitDisplay::MmolL
                    | UnitDisplay::Dual {
                        primary: UnitPreference::MmolL,
                    } => (format!("{:.1}", mbg / 18.0), "mmol/L"),
                };
                let dim = text_dimensions(&val_str, font_size_xs, ctx.font);
                draw_text_mut(
                    img,
                    self.theme.text_primary,
                    (x - dim.0 / 2.0) as i32,
                    (y - outline_r as f32 - dim.1 - 5.0 * ctx.viewport.s) as i32,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    &val_str,
                );
            }
        }

        match self.treatment_mode {
            TreatmentDisplayMode::Contextual => {
                let insulin_offset_ctx = 45.0 * ctx.viewport.s;
                let carbs_offset_ctx = 45.0 * ctx.viewport.s;
                let icon_scale = 1.6;
                let text_scale = PxScale::from(font_size_ctx);
                let text_distance = 15.0 * ctx.viewport.s;

                let dark_insulin = darken_color(self.theme.insulin, 0.6);
                let dark_carbs = darken_color(self.theme.carbs, 0.6);

                let all_ins_values: Vec<f32> = treatments
                    .iter()
                    .filter_map(|t| t.insulin)
                    .filter(|&v| v > self.microbolus_threshold)
                    .collect();
                let all_carb_values: Vec<f32> = treatments.iter().filter_map(|t| t.carbs).collect();

                let (ins_min_val, ins_max_val) = min_max(&all_ins_values);
                let (carb_min_val, carb_max_val) = min_max(&all_carb_values);

                let ins_base_max = 22.0 * ctx.viewport.s;
                let ins_base_min = 6.0 * ctx.viewport.s;
                let ins_micro_size = 3.5 * ctx.viewport.s;

                let carb_base_max = 25.0 * ctx.viewport.s;
                let carb_base_min = 8.0 * ctx.viewport.s;

                let mut text_regions: Vec<(i32, i32, i32, i32)> = Vec::new();
                let margin_overlap = 4.0 * ctx.viewport.s;

                for t in treatments {
                    let x = ctx.project_x(t.date);
                    let closest = self
                        .entries
                        .iter()
                        .min_by_key(|e| (e.date.timestamp() - t.date.timestamp()).abs());
                    let base_y = if let Some(entry) = closest {
                        ctx.project_y(entry.sgv)
                    } else {
                        ctx.viewport.plot_bottom
                    };

                    if let Some(ins) = t.insulin {
                        let size = if ins <= self.microbolus_threshold {
                            ins_micro_size
                        } else {
                            let calculated = calculate_dynamic_size(
                                ins,
                                ins_min_val,
                                ins_max_val,
                                ins_base_min,
                                ins_base_max,
                            );
                            calculated * icon_scale
                        };

                        let y = base_y + insulin_offset_ctx;
                        draw_smart_triangle(
                            img,
                            (x as i32, y as i32),
                            size,
                            self.theme.insulin,
                            dark_insulin,
                            self.theme.background,
                        );

                        if ins > self.microbolus_threshold {
                            let text = format!("{:.1}u", ins);
                            let dim = text_dimensions(&text, font_size_ctx, ctx.font);
                            let w = dim.0 as i32;
                            let h = dim.1 as i32;
                            let text_x = (x - dim.0 / 2.0) as i32;
                            let mut text_y = (y + size + text_distance) as i32;

                            let mut attempts = 0;
                            while attempts < 10 {
                                let mut collision = false;
                                for r in &text_regions {
                                    if rects_intersect((text_x, text_y, text_x + w, text_y + h), *r)
                                    {
                                        collision = true;
                                        break;
                                    }
                                }
                                if collision {
                                    text_y += (h as f32 + margin_overlap) as i32;
                                    attempts += 1;
                                } else {
                                    break;
                                }
                            }

                            draw_text_mut(
                                img,
                                self.theme.text_secondary,
                                text_x,
                                text_y,
                                text_scale,
                                ctx.font,
                                &text,
                            );
                            text_regions.push((text_x, text_y, text_x + w, text_y + h));
                        }
                    }

                    if let Some(carbs) = t.carbs {
                        let y = base_y - carbs_offset_ctx;
                        let calculated = calculate_dynamic_size(
                            carbs,
                            carb_min_val,
                            carb_max_val,
                            carb_base_min,
                            carb_base_max,
                        );
                        let radius = calculated * icon_scale;

                        draw_smart_circle(
                            img,
                            x as i32,
                            y as i32,
                            radius as i32,
                            self.theme.carbs,
                            dark_carbs,
                            self.theme.background,
                        );

                        let text = format!("{:.0}g", carbs);
                        let dim = text_dimensions(&text, font_size_ctx, ctx.font);
                        let w = dim.0 as i32;
                        let h = dim.1 as i32;
                        let text_x = (x - dim.0 / 2.0) as i32;
                        let mut text_y = (y - radius - dim.1 - text_distance) as i32;

                        let mut attempts = 0;
                        while attempts < 10 {
                            let mut collision = false;
                            for r in &text_regions {
                                if rects_intersect((text_x, text_y, text_x + w, text_y + h), *r) {
                                    collision = true;
                                    break;
                                }
                            }
                            if collision {
                                text_y -= (h as f32 + margin_overlap) as i32;
                                attempts += 1;
                            } else {
                                break;
                            }
                        }

                        draw_text_mut(
                            img,
                            self.theme.text_secondary,
                            text_x,
                            text_y,
                            text_scale,
                            ctx.font,
                            &text,
                        );
                        text_regions.push((text_x, text_y, text_x + w, text_y + h));
                    }
                }
            }
            TreatmentDisplayMode::Timeline => {
                let mut major_treatments = Vec::new();
                for t in treatments {
                    let mut is_micro = false;
                    if let Some(ins) = t.insulin {
                        if ins <= self.microbolus_threshold && t.carbs.is_none() {
                            is_micro = true;
                            let x = ctx.project_x(t.date);
                            let tick_height = 8.0 * ctx.viewport.s;
                            draw_line_segment_mut(
                                img,
                                (x, ctx.viewport.plot_bottom),
                                (x, ctx.viewport.plot_bottom - tick_height),
                                self.theme.insulin,
                            );
                        }
                    }
                    if !is_micro {
                        major_treatments.push(t);
                    }
                }

                let px_threshold = 45.0 * ctx.viewport.s;
                let mut groups: Vec<Vec<&GraphTreatment>> = Vec::new();
                for t in &major_treatments {
                    if let Some(last_group) = groups.last_mut() {
                        let last_t = last_group[0];
                        let x1 = ctx.project_x(last_t.date);
                        let x2 = ctx.project_x(t.date);
                        if (x2 - x1).abs() < px_threshold {
                            last_group.push(*t);
                            continue;
                        }
                    }
                    groups.push(vec![*t]);
                }

                for group in groups {
                    let x_sum: f32 = group.iter().map(|t| ctx.project_x(t.date)).sum();
                    let x_center = x_sum / group.len() as f32;
                    let mut sorted_group = group.clone();
                    sorted_group.sort_by_key(|t| std::cmp::Reverse(t.date));

                    struct StackItem {
                        text: String,
                        color: Rgba<u8>,
                    }
                    let mut items = Vec::new();
                    for t in sorted_group {
                        if let Some(ins) = t.insulin {
                            items.push(StackItem {
                                text: format!("{:.1}u", ins),
                                color: self.theme.insulin,
                            });
                        }
                        if let Some(carbs) = t.carbs {
                            items.push(StackItem {
                                text: format!("{:.0}g", carbs),
                                color: self.theme.carbs,
                            });
                        }
                    }
                    if items.is_empty() {
                        continue;
                    }

                    let item_height = font_size_sm + (4.0 * ctx.viewport.s);
                    let stem_base_y = ctx.viewport.plot_bottom;
                    let stack_bottom_y = stem_base_y - (15.0 * ctx.viewport.s);
                    draw_line_segment_mut(
                        img,
                        (x_center, stem_base_y),
                        (x_center, stack_bottom_y),
                        self.theme.axis_lines,
                    );

                    let total_stack_height = items.len() as f32 * item_height;
                    let top_y = stack_bottom_y - total_stack_height;

                    for (i, item) in items.iter().enumerate() {
                        let y_pos = top_y + (i as f32 * item_height);
                        let dim = text_dimensions(&item.text, font_size_sm, ctx.font);
                        draw_text_mut(
                            img,
                            item.color,
                            (x_center - dim.0 / 2.0) as i32,
                            y_pos as i32,
                            PxScale::from(font_size_sm),
                            ctx.font,
                            &item.text,
                        );
                    }
                }
            }
        }
    }

    /// Private helper function to draw entries on the graph.
    fn draw_entries(&self, img: &mut RgbaImage, ctx: &RenderContext, entries: &[GraphEntry]) {
        let base_point_radius = if self.entries.len() > 100 { 4.0 } else { 6.0 };
        let point_radius = (base_point_radius + 1.0) * ctx.viewport.s;

        for e in entries {
            let x = ctx.project_x(e.date);
            let y = ctx.project_y(e.sgv);
            let color = if e.sgv > self.target_high {
                self.theme.glucose_high
            } else if e.sgv < self.target_low {
                self.theme.glucose_low
            } else {
                self.theme.glucose_in_range
            };

            draw_filled_circle_mut(img, (x as i32, y as i32), point_radius as i32, color);
        }
    }
}

// -----------------------------------------------------------------------//
// Other helper functions here cuz I don't think they fit inside the impl.//
// Doesn't mean they're not useful I love them very much :3               //
// -----------------------------------------------------------------------//

fn text_dimensions(text: &str, size: f32, _font: &FontRef) -> (f32, f32) {
    let width = text.len() as f32 * (size * 0.6);
    (width, size)
}

fn min_max(values: &[f32]) -> (f32, f32) {
    values.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
        (min.min(v), max.max(v))
    })
}

fn calculate_dynamic_size(
    val: f32,
    min_val: f32,
    max_val: f32,
    min_size: f32,
    max_size: f32,
) -> f32 {
    if (max_val - min_val).abs() < f32::EPSILON {
        return max_size * (2.0 / 3.0);
    }
    let ratio = (val - min_val) / (max_val - min_val);
    min_size + ratio * (max_size - min_size)
}

fn rects_intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.2 && a.2 > b.0 && a.1 < b.3 && a.3 > b.1
}
