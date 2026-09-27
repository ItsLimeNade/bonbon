use std::format;
use std::string::ToString;

use crate::models::{
    GraphEntry, GraphScaling, GraphTreatment, SeriesPoint, TimeAxisMode, TreatmentDisplayMode,
    UnitDisplay, UnitPreference,
};
use crate::theme::Theme;
use crate::utils::color::darken_color;
use crate::utils::drawing::{
    blend_fast_rect, blend_pixel, blend_sprite, canvas_with_rounded_rects, create_aa_circle_sprite,
    create_aa_triangle_sprite, draw_dashed_horizontal_line, draw_smart_circle, draw_smart_triangle,
    Sprite,
};
use crate::utils::text::{draw_text as draw_text_mut, draw_text_with_outline};

use ab_glyph::{FontRef, PxScale};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};

/// Gap (before scaling) between the plot area and the rounded panel that frames
/// it. Shared by the panel and the axis labels so the exterior lines up.
const PANEL_PAD: f32 = 16.0;

/// Most labelled values on the glucose Y axis; any more lines and the plot
/// gets hard to read.
const MAX_VALUE_TICKS: usize = 7;

/// Steps between labelled values on the glucose Y axis, finest first. 30 sits
/// between 20 and 50 so a typical day gets 5 or 6 lines rather than 9 or 4.
const MGDL_STEPS: [f32; 11] = [
    1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 50.0, 100.0, 200.0, 500.0, 1000.0,
];
const MMOL_STEPS: [f32; 9] = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0];
/// Share of the plot height each mini graph takes from the glucose plot.
const LANE_SHARE: f32 = 0.1;

/// Gap (before scaling) between two stacked panels.
const PANEL_GAP: f32 = 12.0;

/// Configuration for the visual layout of the graph.
#[derive(Clone, Debug)]
pub struct LayoutConfig {
    pub width: u32,
    pub height: u32,
    /// If None, auto-sized for the unit caption (~56 * scale, ~78 for dual units).
    pub margin_top: Option<f32>,
    /// If None, defaults to 104.0 * scale
    pub margin_bottom: Option<f32>,
    /// If None, auto-sized for the Y-axis labels (~110 * scale, ~120 for mmol/L).
    pub margin_left: Option<f32>,
    /// If None, defaults to 32.0 * scale
    pub margin_right: Option<f32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
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
    /// Plot areas of the mini graphs under the glucose plot, top to bottom.
    /// They share the plot's horizontal extent.
    lanes: Vec<Lane>,
    /// Bottom of the lowest plot area, where the time axis hangs.
    axis_bottom: f32,
}

/// Where a mini graph's plot area sits.
struct Lane {
    /// Index of its [`MiniGraph`] in the builder.
    graph: usize,
    top: f32,
    bottom: f32,
}

/// A small graph under the glucose plot, sharing its time axis.
///
/// The glucose plot is always drawn; mini graphs are optional and stack
/// under it in the order they are added, each in its own panel, with the
/// glucose plot giving up the height they need. See
/// [`GlucoseGraphBuilder::add_mini_graph`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MiniGraph {
    /// Insulin on board, in units, with a triangle on the curve for each
    /// insulin treatment.
    Iob(OnBoard),
    /// Carbs on board, in grams, with a dot on the curve for each carb
    /// treatment.
    Cob(OnBoard),
}

/// Where a [`MiniGraph`] gets its values.
#[derive(Debug, Clone)]
pub enum OnBoard {
    /// From the graph's own treatments: each one counts in full when given
    /// and fades in a straight line to nothing over this duration, and the
    /// graph shows the total of those still on board.
    FromTreatments(Duration),
    /// Values reported by an AID system, e.g. from Nightscout device
    /// statuses.
    Reported(Vec<SeriesPoint>),
}

impl MiniGraph {
    /// Insulin on board from the graph's insulin treatments, each fading
    /// linearly to nothing over `duration` (your duration of insulin action).
    pub fn iob(duration: Duration) -> Self {
        Self::Iob(OnBoard::FromTreatments(duration))
    }

    /// Carbs on board from the graph's carb treatments, each fading linearly
    /// to nothing over `duration` (how long your carbs take to absorb).
    pub fn cob(duration: Duration) -> Self {
        Self::Cob(OnBoard::FromTreatments(duration))
    }

    /// Insulin on board as reported by an AID system, in units.
    pub fn iob_reported<I>(points: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<SeriesPoint>,
    {
        Self::Iob(OnBoard::Reported(
            points.into_iter().map(|p| p.into()).collect(),
        ))
    }

    /// Carbs on board as reported by an AID system, in grams.
    pub fn cob_reported<I>(points: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<SeriesPoint>,
    {
        Self::Cob(OnBoard::Reported(
            points.into_iter().map(|p| p.into()).collect(),
        ))
    }

    fn reported_mut(&mut self) -> Option<&mut Vec<SeriesPoint>> {
        match self {
            Self::Iob(OnBoard::Reported(points)) | Self::Cob(OnBoard::Reported(points)) => {
                Some(points)
            }
            _ => None,
        }
    }

    /// Everything that sets this kind of mini graph apart, bar its values.
    fn style(&self, theme: &Theme) -> MiniGraphStyle {
        match self {
            Self::Iob(_) => MiniGraphStyle {
                name: "IOB",
                color: theme.insulin,
                floor: 1.0,
                format: |v| format!("{:.1}u", v),
                amount: |t| t.insulin,
                marker: Marker::Triangle,
                microboluses: true,
            },
            Self::Cob(_) => MiniGraphStyle {
                name: "COB",
                color: theme.carbs,
                floor: 10.0,
                format: |v| format!("{:.0}g", v),
                amount: |t| t.carbs,
                marker: Marker::Dot,
                microboluses: false,
            },
        }
    }
}

/// How a kind of [`MiniGraph`] is drawn.
struct MiniGraphStyle {
    /// Shown in the left margin, in `color`.
    name: &'static str,
    color: Rgba<u8>,
    /// Lowest top of scale, so a few small doses or snacks don't fill the
    /// graph.
    floor: f32,
    /// Formats a value with its unit.
    format: fn(f32) -> String,
    /// The amount of a treatment this graph marks, if any.
    amount: fn(&GraphTreatment) -> Option<f32>,
    marker: Marker,
    /// Whether amounts up to the microbolus threshold get the smallest,
    /// unringed marker.
    microboluses: bool,
}

#[derive(Clone, Copy)]
enum Marker {
    Triangle,
    Dot,
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
    show_trace: bool,
    mini_graphs: Vec<MiniGraph>,
    #[cfg(feature = "beetroot")]
    sticker_set: Option<crate::charts::stickers::StickerSet>,
}

impl<'a> Default for GlucoseGraphBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> GlucoseGraphBuilder<'a> {
    pub fn new() -> Self {
        const DEFAULT_FONT: &[u8] = include_bytes!("../../assets/fonts/GeistMono-Regular.ttf");

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
            theme: Theme::dark(),
            font: DEFAULT_FONT,
            microbolus_threshold: 0.0,
            show_trace: true,
            mini_graphs: Vec::new(),
            #[cfg(feature = "beetroot")]
            sticker_set: None,
        }
    }

    /// Attach a [`StickerSet`](crate::charts::stickers::StickerSet) to the graph.
    ///
    /// Stickers are drawn behind the data points so the graph stays readable.
    /// Only available with the `beetroot` feature.
    #[cfg(feature = "beetroot")]
    pub fn with_stickers(mut self, set: crate::charts::stickers::StickerSet) -> Self {
        self.sticker_set = Some(set);
        self
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

    /// Toggles the connecting line drawn through the readings.
    ///
    /// When `true` (the default) consecutive readings are joined by a smooth,
    /// status-colored trace beneath the circles. Set to `false` for a plain
    /// scatter of points.
    pub fn with_trace(mut self, show: bool) -> Self {
        self.show_trace = show;
        self
    }

    /// Sets the mini graphs drawn under the glucose plot, replacing any
    /// earlier ones. They stack top to bottom in the given order, each in its
    /// own panel sharing the plot's time axis. With none (the default) only
    /// the glucose plot is drawn.
    pub fn with_mini_graphs<I>(mut self, graphs: I) -> Self
    where
        I: IntoIterator<Item = MiniGraph>,
    {
        self.mini_graphs = graphs.into_iter().collect();
        self
    }

    /// Adds a mini graph under the glucose plot, below any added before.
    pub fn add_mini_graph(mut self, graph: MiniGraph) -> Self {
        self.mini_graphs.push(graph);
        self
    }

    /// Builds the final image, returning an ImageBuffer.
    pub fn build(mut self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let font = FontRef::try_from_slice(self.font)?;

        // Calculate the graph's viewport first, this allows for a scalable margin system
        // and correct entry positionning later on.
        let viewport = self.calculate_viewport();

        // Flat background, framed plot panel included (the same soft panel the
        // bg/TIR cards use).
        let mut img = self.new_canvas(&viewport);

        // Here we're sorting the entries with worst-case scenario of O(n * log(n)) for the time complexity.
        // Sorting them will allow us to correctly fetch the graph's time span.
        self.entries.sort_unstable_by_key(|e| e.date);
        for points in self
            .mini_graphs
            .iter_mut()
            .filter_map(MiniGraph::reported_mut)
        {
            points.sort_unstable_by_key(|p| p.date);
        }
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

        // Faint gridlines, soft target dashes, then the axis annotations.
        self.draw_value_gridlines(&mut img, &ctx);
        self.draw_target_lines(&mut img, &ctx);
        self.draw_date_separators(&mut img, &ctx);
        self.draw_time_axis(&mut img, &ctx);
        self.draw_labels_and_units(&mut img, &ctx);

        // Same optimization than for entries.
        let visible_treatments = self.get_visible_treatments(start_time, end_time);

        for lane in &ctx.viewport.lanes {
            self.draw_lane(&mut img, &ctx, lane, &visible_treatments);
        }

        self.draw_treatments(&mut img, &ctx, &visible_treatments);

        // Stickers go behind entries so the readings stay clear on top.
        #[cfg(feature = "beetroot")]
        self.draw_stickers(&mut img, &ctx, visible_entries_slice);

        // An optional smooth trace connects the readings into one clean line,
        // then the (anti-aliased) circles sit on top of it.
        if self.show_trace {
            self.draw_entry_trace(&mut img, &ctx, visible_entries_slice);
        }
        self.draw_entries(&mut img, &ctx, visible_entries_slice);

        Ok(img)
    }

    /// Hands off to the `stickers` module. Behind the `beetroot` feature so
    /// non-users carry zero overhead in their build.
    #[cfg(feature = "beetroot")]
    fn draw_stickers(&self, img: &mut RgbaImage, ctx: &RenderContext, entries: &[GraphEntry]) {
        use crate::charts::stickers;
        let Some(set) = self.sticker_set.as_ref() else {
            return;
        };
        let bounds = stickers::bounds_from(
            ctx.viewport.plot_left,
            ctx.viewport.plot_top,
            ctx.viewport.plot_right,
            ctx.viewport.plot_bottom,
        );
        stickers::draw_on_graph(
            img,
            set,
            entries,
            bounds,
            &|t| ctx.project_x(t),
            &|sgv| ctx.project_y(sgv),
            self.target_low,
            self.target_high,
        );
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

        // Default margins are kept as tight as the labels allow so little of
        // the canvas is wasted (it scales down on Discord etc.). Top and left
        // adapt to the unit: the dual caption needs two lines, and mmol/L
        // labels are wider than mg/dL.
        let is_dual = matches!(self.unit_display, UnitDisplay::Dual { .. });
        let wide_y = matches!(
            self.unit_display,
            UnitDisplay::MmolL | UnitDisplay::Dual { .. }
        );
        let top_default = if is_dual { 78.0 } else { 56.0 };
        let left_default = if wide_y { 120.0 } else { 110.0 };

        let m_top = self.layout.margin_top.unwrap_or(top_default * s);
        let m_bottom = self.layout.margin_bottom.unwrap_or(104.0 * s);
        let m_left = self.layout.margin_left.unwrap_or(left_default * s);
        let m_right = self.layout.margin_right.unwrap_or(32.0 * s);

        let plot_w = self.layout.width as f32 - m_left - m_right;
        let full_h = self.layout.height as f32 - m_top - m_bottom;

        // Each mini graph gets its own panel under the glucose plot, which
        // gives up the height they need. With many of them the lanes shrink
        // so their panels take at most half the height, for as long as the
        // panels' padding alone fits in it.
        let pad = PANEL_PAD * s;
        let gap = PANEL_GAP * s;
        let count = self.mini_graphs.len() as f32;
        let lane_h = if count == 0.0 {
            0.0
        } else {
            let most = (full_h * 0.5) / count - 2.0 * pad - gap;
            (full_h * LANE_SHARE).max(36.0 * s).min(most).max(0.0)
        };
        let plot_h = full_h - count * (lane_h + 2.0 * pad + gap);

        // Panel edges: the glucose panel's bottom, then a gap and a padded
        // lane per mini graph.
        let mut edge = m_top + plot_h + pad;
        let lanes: Vec<Lane> = (0..self.mini_graphs.len())
            .map(|graph| {
                let top = edge + gap + pad;
                edge = top + lane_h + pad;
                Lane {
                    graph,
                    top,
                    bottom: top + lane_h,
                }
            })
            .collect();
        let axis_bottom = lanes.last().map_or(m_top + plot_h, |l| l.bottom);

        GraphViewport {
            s,
            plot_left: m_left,
            plot_top: m_top,
            plot_right: m_left + plot_w,
            plot_bottom: m_top + plot_h,
            plot_w,
            plot_h,
            lanes,
            axis_bottom,
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
                    let (min_sgv, max_sgv) = min_max(visible_entries.iter().map(|e| e.sgv));

                    let calc_max = max_sgv;
                    let calc_min = ((min_sgv - 20.0) / 10.0).floor() * 10.0;

                    let view_min = calc_min.min(default_min);
                    let view_max = calc_max.max(default_max);

                    (view_min.max(clamp_min), view_max.min(clamp_max))
                }
            }
        }
    }

    /// Creates the canvas: flat background plus the rounded panel that frames
    /// the plot area. The panel replaces the old protruding L-shaped axis
    /// spines (the main "unstyled plot" tell) with the same soft raised-card
    /// surface the other charts use.
    ///
    /// The panel is a translucent tint over a flat background, so it is one
    /// known color: the canvas is written in a single pass with it rather than
    /// filled and then blended pixel by pixel over most of its area.
    fn new_canvas(&self, viewport: &GraphViewport) -> RgbaImage {
        let s = viewport.s;
        let pad = PANEL_PAD * s;
        let x = (viewport.plot_left - pad) as i32;
        let w = (viewport.plot_w + 2.0 * pad) as u32;
        // The glucose plot's panel, then a matching one per mini graph.
        let panels: Vec<(i32, i32, u32, u32)> =
            std::iter::once((viewport.plot_top, viewport.plot_bottom))
                .chain(viewport.lanes.iter().map(|l| (l.top, l.bottom)))
                .map(|(top, bottom)| (x, (top - pad) as i32, w, (bottom - top + 2.0 * pad) as u32))
                .collect();
        canvas_with_rounded_rects(
            self.layout.width,
            self.layout.height,
            self.theme.background,
            &panels,
            (18.0 * s) as i32,
            self.panel_color(),
        )
    }

    /// The opaque color of the plot panels: `grid_major` tinted over the
    /// background.
    fn panel_color(&self) -> Rgba<u8> {
        let [gr, gg, gb, _] = self.theme.grid_major.0;
        blend_pixel(self.theme.background, Rgba([gr, gg, gb, 110]))
    }

    /// Draws faint horizontal gridlines at each Y-axis tick. The ticks come
    /// from [`Self::value_ticks`], like the labels [`Self::draw_labels_and_units`]
    /// draws, so every label sits on the line of the value it names.
    ///
    /// Lines are a low-alpha blend of `axis_lines` over the panel, so they give
    /// the eye a reference without ever competing with the data points.
    fn draw_value_gridlines(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let grid_thickness = (1.0 * ctx.viewport.s).ceil() as u32;
        let [lr, lg, lb, _] = self.theme.axis_lines.0;
        let stripe_color = Rgba([lr, lg, lb, 30]);

        for val in self.value_ticks(&ctx.viewport, ctx.y_min, ctx.y_max) {
            let y = ctx.project_y(val);
            blend_fast_rect(
                img,
                ctx.viewport.plot_left as i32,
                (y - grid_thickness as f32 / 2.0).round() as i32,
                ctx.viewport.plot_w as u32,
                grid_thickness,
                stripe_color,
            );
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

    /// Marks each midnight roll-over inside the window with a faint vertical
    /// dashed separator spanning the plot, labelled "DD/MM" at the top. The line
    /// makes the day boundary unmistakable; the dashes and low alpha keep it
    /// behind the data.
    fn draw_date_separators(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let local_start = ctx.start_time.with_timezone(&self.timezone);
        let local_end = ctx.end_time.with_timezone(&self.timezone);
        let s = ctx.viewport.s;
        let font_size_sm = (24.0 + 1.0) * s;

        let [ar, ag, ab, _] = self.theme.axis_lines.0;
        let line_color = Rgba([ar, ag, ab, 55]);
        let dash = (9.0 * s).max(2.0) as i32;
        let gap = (7.0 * s).max(2.0) as i32;
        let thickness = (1.0 * s).ceil() as u32;

        let mut pointer = local_start
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(self.timezone)
            .unwrap();
        if pointer < local_start {
            pointer += Duration::days(1);
        }

        // The glucose plot and every mini graph under it.
        let spans: Vec<(f32, f32)> =
            std::iter::once((ctx.viewport.plot_top, ctx.viewport.plot_bottom))
                .chain(ctx.viewport.lanes.iter().map(|l| (l.top, l.bottom)))
                .collect();

        while pointer <= local_end {
            let x = ctx.project_x(pointer.with_timezone(&Utc));
            if x >= ctx.viewport.plot_left && x <= ctx.viewport.plot_right {
                // Vertical dashed separator across each plot's height.
                for &(top, bottom) in &spans {
                    let mut y = top as i32;
                    let bottom = bottom as i32;
                    while y < bottom {
                        let seg = dash.min(bottom - y);
                        blend_fast_rect(
                            img,
                            (x - thickness as f32 / 2.0) as i32,
                            y,
                            thickness,
                            seg as u32,
                            line_color,
                        );
                        y += dash + gap;
                    }
                }

                let date_str = pointer.format("%d/%m").to_string();
                draw_text_with_outline(
                    img,
                    self.theme.text_secondary,
                    self.theme.background,
                    (x + 6.0 * s) as i32,
                    (ctx.viewport.plot_top + 5.0 * s) as i32,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    &date_str,
                );
            }
            pointer += Duration::days(1);
        }
    }

    /// Draws the X-axis time ticks: local `HH:MM` plus the relative `-Xh` offset
    /// underneath. A short tick mark drops from the axis at each label so the
    /// time scale is unmistakable.
    ///
    /// Both axis modes render here. [`TimeAxisMode::Simple`] auto-picks a tick
    /// count from the plot width; [`TimeAxisMode::EquallyDistributed`] honors the
    /// caller's `count`.
    fn draw_time_axis(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let count = match self.time_axis_mode {
            TimeAxisMode::EquallyDistributed { count } => count.max(1),
            // Aim for roughly one label per ~180px so labels never crowd.
            TimeAxisMode::Simple => (ctx.viewport.plot_w / (180.0 * ctx.viewport.s))
                .round()
                .clamp(2.0, 10.0) as u32,
        };

        let step_secs = ctx.time_span_secs / (count as f32);
        let font_size_sm = (24.0 + 1.0) * ctx.viewport.s;
        let font_size_xs = (20.0 + 1.0) * ctx.viewport.s;

        let tick_color = {
            let [r, g, b, _] = self.theme.axis_lines.0;
            Rgba([r, g, b, 70])
        };
        let tick_h = 7.0 * ctx.viewport.s;

        for i in 0..=count {
            let offset = i as f32 * step_secs;
            let tick_time = ctx.start_time + Duration::seconds(offset as i64);
            let x = ctx.viewport.plot_left + (offset / ctx.time_span_secs) * ctx.viewport.plot_w;

            if x > ctx.viewport.plot_right + 1.0 {
                continue;
            }

            // Small tick mark just below the plot to anchor the label.
            blend_fast_rect(
                img,
                x.round() as i32,
                ctx.viewport.axis_bottom as i32,
                (1.0 * ctx.viewport.s).ceil() as u32,
                tick_h as u32,
                tick_color,
            );

            let local_time = tick_time.with_timezone(&self.timezone);
            let time_str = local_time.format("%H:%M").to_string();
            let dim_time = text_dimensions(&time_str, font_size_sm, ctx.font);

            let mut tx = (x - dim_time.0 / 2.0) as i32;
            let min_tx = ctx.viewport.plot_left as i32;
            let max_tx = (ctx.viewport.plot_right - dim_time.0) as i32;
            tx = tx.clamp(min_tx, max_tx);

            let ty = (ctx.viewport.axis_bottom + 28.0 * ctx.viewport.s) as i32;

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
            let ry = (ctx.viewport.axis_bottom
                + 28.0 * ctx.viewport.s
                + dim_time.1
                + 4.0 * ctx.viewport.s) as i32;

            draw_text_mut(
                img,
                self.theme.text_secondary,
                rx,
                ry,
                PxScale::from(font_size_xs),
                ctx.font,
                &rel_str,
            );
        }
    }

    /// Draws the left-margin Y-axis value numbers and the unit caption.
    ///
    /// Everything shares a single right edge that sits clear of the framed
    /// panel, and the unit caption is a small header above the axis rather than
    /// crammed into the busy bottom-left corner, so the exterior reads as a tidy
    /// margin around the plot.
    fn draw_labels_and_units(&self, img: &mut RgbaImage, ctx: &RenderContext) {
        let s = ctx.viewport.s;
        let font_size_md = (30.0 + 1.0) * s;
        let font_size_sm = (24.0 + 1.0) * s;
        let font_size_xs = (20.0 + 1.0) * s;

        // Common right edge for the whole left margin, kept clear of the panel.
        let label_right = ctx.viewport.plot_left - PANEL_PAD * s - 12.0 * s;

        // Unit caption: a small header sitting just above the top of the axis.
        let unit_bottom = ctx.viewport.plot_top - PANEL_PAD * s - 6.0 * s;
        match self.unit_display {
            UnitDisplay::MgDl | UnitDisplay::MmolL => {
                let text = if let UnitDisplay::MmolL = self.unit_display {
                    "mmol/L"
                } else {
                    "mg/dL"
                };
                let dim = text_dimensions(text, font_size_sm, ctx.font);
                draw_text_mut(
                    img,
                    self.theme.text_secondary,
                    (label_right - dim.0) as i32,
                    (unit_bottom - dim.1) as i32,
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

                let ty2 = unit_bottom - dim2.1;
                let ty1 = ty2 - 2.0 * s - dim1.1;
                draw_text_mut(
                    img,
                    self.theme.text_secondary,
                    (label_right - dim1.0) as i32,
                    ty1 as i32,
                    PxScale::from(font_size_sm),
                    ctx.font,
                    u1,
                );
                draw_text_mut(
                    img,
                    self.theme.text_dim,
                    (label_right - dim2.0) as i32,
                    ty2 as i32,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    u2,
                );
            }
        }

        for val in self.value_ticks(&ctx.viewport, ctx.y_min, ctx.y_max) {
            let y_pos = ctx.project_y(val);
            let (main_text, sub_text) = self.tick_labels(val);

            let main_dim = text_dimensions(&main_text, font_size_md, ctx.font);
            let main_tx = (label_right - main_dim.0) as i32;
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
                let sub_tx = (label_right - sub_dim.0) as i32;
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

    /// The values (in mg/dL) that get a gridline and a label on the Y axis:
    /// every multiple of a round step inside `[y_min, y_max]`, e.g. 30 mg/dL or
    /// 2 mmol/L depending on the primary unit. The step is the finest that
    /// keeps the labels well apart without crowding the axis.
    fn value_ticks(&self, viewport: &GraphViewport, y_min: f32, y_max: f32) -> Vec<f32> {
        // mg/dL per primary unit, and that unit's steps.
        let (per_unit, steps): (f32, &[f32]) = match self.unit_display {
            UnitDisplay::MgDl
            | UnitDisplay::Dual {
                primary: UnitPreference::MgDl,
            } => (1.0, &MGDL_STEPS),
            UnitDisplay::MmolL
            | UnitDisplay::Dual {
                primary: UnitPreference::MmolL,
            } => (18.0, &MMOL_STEPS),
        };
        // A label is 31px tall before scaling, 52px with the converted value
        // under it in dual mode; keep at least a label's height between
        // neighbours.
        let min_gap = match self.unit_display {
            UnitDisplay::Dual { .. } => 52.0 + 31.0,
            _ => 31.0 + 31.0,
        } * viewport.s;
        let px_per_unit = viewport.plot_h / (y_max - y_min) * per_unit;

        round_ticks(
            y_min / per_unit,
            y_max / per_unit,
            steps,
            min_gap / px_per_unit,
            MAX_VALUE_TICKS,
        )
        .into_iter()
        .map(|v| v * per_unit)
        .collect()
    }

    /// The label of the Y-axis tick at `val` mg/dL, in the primary unit, and
    /// in dual mode the same value in the other unit to go under it.
    fn tick_labels(&self, val: f32) -> (String, Option<String>) {
        let mgdl = format!("{:.0}", val);
        let mmol = format!("{:.1}", val / 18.0);
        match self.unit_display {
            UnitDisplay::MgDl => (mgdl, None),
            UnitDisplay::MmolL => (mmol, None),
            UnitDisplay::Dual {
                primary: UnitPreference::MgDl,
            } => (mgdl, Some(mmol)),
            UnitDisplay::Dual {
                primary: UnitPreference::MmolL,
            } => (mmol, Some(mgdl)),
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

        match self.treatment_mode {
            TreatmentDisplayMode::Contextual => {
                let insulin_offset_ctx = 45.0 * ctx.viewport.s;
                let carbs_offset_ctx = 45.0 * ctx.viewport.s;
                let icon_scale = 1.6;
                let text_scale = PxScale::from(font_size_ctx);
                let text_distance = 15.0 * ctx.viewport.s;

                let dark_insulin = darken_color(self.theme.insulin, 0.6);
                let dark_carbs = darken_color(self.theme.carbs, 0.6);

                let (ins_min_val, ins_max_val) = min_max(
                    treatments
                        .iter()
                        .filter_map(|t| t.insulin)
                        .filter(|&v| v > self.microbolus_threshold),
                );
                let (carb_min_val, carb_max_val) =
                    min_max(treatments.iter().filter_map(|t| t.carbs));

                let ins_base_max = (22.0 * 2.0 / 3.0) * ctx.viewport.s;
                let ins_base_min = (6.0 * 5.0 / 3.0) * ctx.viewport.s;
                let ins_micro_size = 3.5 * ctx.viewport.s;

                let carb_base_max = (25.0 * 2.0 / 3.0) * ctx.viewport.s;
                let carb_base_min = (8.0 * 5.0 / 3.0) * ctx.viewport.s;

                let mut text_regions: Vec<(i32, i32, i32, i32)> = Vec::new();
                let margin_overlap = 4.0 * ctx.viewport.s;

                let overlap_targets = [
                    self.theme.insulin,
                    dark_insulin,
                    self.theme.carbs,
                    dark_carbs,
                ];

                for t in treatments {
                    let x = ctx.project_x(t.date);
                    let base_y = if let Some(entry) = closest_entry(&self.entries, t.date) {
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
                            &overlap_targets,
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

                            draw_text_with_outline(
                                img,
                                self.theme.text_secondary,
                                self.theme.background,
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
                            &overlap_targets,
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

                        draw_text_with_outline(
                            img,
                            self.theme.text_secondary,
                            self.theme.background,
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

                for mut group in groups {
                    let x_sum: f32 = group.iter().map(|t| ctx.project_x(t.date)).sum();
                    let x_center = x_sum / group.len() as f32;
                    group.sort_by_key(|t| std::cmp::Reverse(t.date));

                    struct StackItem {
                        text: String,
                        color: Rgba<u8>,
                    }
                    let mut items = Vec::new();
                    for t in group {
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

                        draw_text_with_outline(
                            img,
                            item.color,
                            self.theme.background,
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
                draw_text_with_outline(
                    img,
                    self.theme.text_primary,
                    self.theme.background,
                    (x - dim.0 / 2.0) as i32,
                    (y - outline_r as f32 - dim.1 - 5.0 * ctx.viewport.s) as i32,
                    PxScale::from(font_size_xs),
                    ctx.font,
                    &val_str,
                );
            }
        }
    }

    /// Samples a mini graph's series once per pixel column of the plot.
    /// Columns are `None` where there is nothing to draw: outside or across a
    /// gap in reported values, or past "now" for ones from treatments.
    fn lane_values(&self, ctx: &RenderContext, graph: &MiniGraph) -> Vec<Option<f32>> {
        let vp = &ctx.viewport;
        let times: Vec<chrono::DateTime<Utc>> = (vp.plot_left.round() as i32
            ..=vp.plot_right.round() as i32)
            .map(|x| {
                let frac = (x as f32 - vp.plot_left) / vp.plot_w;
                ctx.start_time + Duration::milliseconds((frac * ctx.time_span_secs * 1e3) as i64)
            })
            .collect();
        let until = ctx.end_time.min(Utc::now());
        let up_to_now = |values: Vec<f32>| {
            values
                .into_iter()
                .zip(&times)
                .map(|(v, &t)| (t <= until).then_some(v))
                .collect()
        };

        let (MiniGraph::Iob(source) | MiniGraph::Cob(source)) = graph;
        match source {
            OnBoard::Reported(points) => interpolate_samples(points, &times),
            OnBoard::FromTreatments(duration) => {
                let amount = graph.style(&self.theme).amount;
                let doses: Vec<(chrono::DateTime<Utc>, f32)> = self
                    .treatments
                    .iter()
                    .filter_map(|t| amount(t).map(|a| (t.date, a)))
                    .collect();
                up_to_now(linear_on_board(doses, *duration, &times))
            }
        }
    }

    /// Draws one mini graph in its lane: a line over a soft gradient area
    /// reaching down to a zero baseline, its name in the left margin, a marker
    /// for each of its treatments, and its peak value, which gives the curve
    /// its scale.
    fn draw_lane(
        &self,
        img: &mut RgbaImage,
        ctx: &RenderContext,
        lane: &Lane,
        treatments: &[&GraphTreatment],
    ) {
        let vp = &ctx.viewport;
        let s = vp.s;
        let font_size_sm = (24.0 + 1.0) * s;
        let font_size_xs = (20.0 + 1.0) * s;
        let panel = self.panel_color();
        let graph = &self.mini_graphs[lane.graph];
        let style = graph.style(&self.theme);
        let (color, name, floor) = (style.color, style.name, style.floor);

        // Lane name, right-aligned with the glucose axis labels.
        let label_right = vp.plot_left - PANEL_PAD * s - 12.0 * s;
        let dim = text_dimensions(name, font_size_sm, ctx.font);
        draw_text_mut(
            img,
            color,
            (label_right - dim.0) as i32,
            ((lane.top + lane.bottom - dim.1) / 2.0) as i32,
            PxScale::from(font_size_sm),
            ctx.font,
            name,
        );

        let values = self.lane_values(ctx, graph);
        let (lo, hi) = min_max(values.iter().flatten().copied());
        // Negative IOB (suspended basal) dips below the baseline.
        let v_min = lo.min(0.0);
        let v_max = hi.max(floor);
        // Headroom so the peak label, centered on the peak, stays in the lane.
        let top = lane.top + font_size_xs / 2.0;
        let project = |v: f32| lane.bottom - (v - v_min) / (v_max - v_min) * (lane.bottom - top);
        let zero_y = project(0.0);

        // Zero baseline, as faint as the glucose gridlines.
        let hairline = (1.0 * s).ceil() as u32;
        let [lr, lg, lb, _] = self.theme.axis_lines.0;
        blend_fast_rect(
            img,
            vp.plot_left as i32,
            (zero_y - hairline as f32 / 2.0).round() as i32,
            vp.plot_w as u32,
            hairline,
            Rgba([lr, lg, lb, 30]),
        );

        let x0 = vp.plot_left.round() as i32;
        let ys: Vec<Option<f32>> = values.iter().map(|v| v.map(project)).collect();
        fill_area(img, color, x0, &ys, zero_y);

        // Stretches at zero are left to the baseline so the line only draws
        // the eye where something is on board.
        let resting: Vec<bool> = values
            .iter()
            .map(|v| v.is_some_and(|v| v.abs() < floor * 0.01))
            .collect();
        let half = (1.6 * s).round().max(1.0) as i32;
        stroke_curve(
            img,
            &create_aa_circle_sprite(half, color),
            x0,
            &ys,
            &resting,
        );

        let markers = self.draw_lane_treatments(img, ctx, &style, treatments, x0, &ys, zero_y);

        // Peak value, beside whichever side keeps it clear of the line and
        // the markers.
        let peak = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (i, v)))
            .fold(None, |best: Option<(usize, f32)>, (i, v)| match best {
                Some((_, b)) if b >= v => best,
                _ => Some((i, v)),
            });
        let Some((i, peak)) = peak.filter(|&(_, v)| v >= floor * 0.05) else {
            return;
        };
        let text = (style.format)(peak);
        let dim = text_dimensions(&text, font_size_xs, ctx.font);

        // A peak usually follows the dose or meal that caused it within a
        // few minutes (microboluses can keep IOB climbing), so the label
        // joins that marker. Otherwise the peak gets a ringed dot of its own.
        let (px, py) = ((x0 + i as i32) as f32, project(peak));
        let cause = markers
            .iter()
            .filter(|m| m.0 <= px + m.2 && px - m.0 <= dim.0)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let (px, py, marker_r) = match cause {
            Some(&(mx, my, mr)) => (mx, my, mr),
            None => {
                let dot_r = (4.0 * s).round().max(2.0) as i32;
                let ring =
                    create_aa_circle_sprite(dot_r + (2.0 * s).round().max(1.0) as i32, panel);
                blend_sprite(img, &ring, px as i32, py as i32);
                blend_sprite(
                    img,
                    &create_aa_circle_sprite(dot_r, color),
                    px as i32,
                    py as i32,
                );
                (px, py, dot_r as f32)
            }
        };

        let offset = marker_r + 8.0 * s;
        // Kept inside the lane; a lane shorter than the label keeps its top.
        let ty = (py - dim.1 / 2.0).min(lane.bottom - dim.1).max(lane.top);
        // How many of the label's columns the line crosses, with any marker
        // under the label counting as a whole label's worth.
        let collisions = |tx: f32| {
            let pad = half as f32 + 2.0 * s;
            let (c0, c1) = ((tx as i32 - x0).max(0), (tx + dim.0) as i32 - x0);
            let crossings = (c0..=c1)
                .filter_map(|c| ys.get(c as usize).copied().flatten())
                .filter(|&y| y >= ty - pad && y <= ty + dim.1 + pad)
                .count();
            let covered = markers
                .iter()
                .filter(|m| {
                    rects_intersect(
                        (
                            tx as i32,
                            ty as i32,
                            (tx + dim.0) as i32,
                            (ty + dim.1) as i32,
                        ),
                        (
                            (m.0 - m.2) as i32,
                            (m.1 - m.2) as i32,
                            (m.0 + m.2) as i32,
                            (m.1 + m.2) as i32,
                        ),
                    )
                })
                .count();
            crossings + covered * dim.0 as usize
        };
        let sides = [px - offset - dim.0, px + offset];
        let tx = sides
            .into_iter()
            .filter(|&tx| tx >= vp.plot_left && tx + dim.0 <= vp.plot_right)
            .min_by_key(|&tx| collisions(tx))
            .unwrap_or(sides[0].max(vp.plot_left));
        draw_text_with_outline(
            img,
            color,
            panel,
            tx as i32,
            ty as i32,
            PxScale::from(font_size_xs),
            ctx.font,
            &text,
        );
    }

    /// Marks every treatment a mini graph tracks on its curve (a triangle
    /// per insulin dose, a dot per carb entry, like the contextual markers of
    /// the glucose plot). Each sits where it makes the curve step up and is
    /// sized by its amount against the largest, ringed in the panel color to
    /// stand clear of the line. Microboluses get the smallest marker and no
    /// ring, so a steady stream of them studs the line instead of breaking it
    /// up.
    ///
    /// Returns the center and outer radius of each marker bar the
    /// microboluses.
    #[allow(clippy::too_many_arguments)]
    fn draw_lane_treatments(
        &self,
        img: &mut RgbaImage,
        ctx: &RenderContext,
        style: &MiniGraphStyle,
        treatments: &[&GraphTreatment],
        x0: i32,
        ys: &[Option<f32>],
        zero_y: f32,
    ) -> Vec<(f32, f32, f32)> {
        let s = ctx.viewport.s;
        let panel = self.panel_color();
        let (color, amount) = (style.color, style.amount);
        let is_micro = |v: f32| style.microboluses && v <= self.microbolus_threshold;

        let amounts: Vec<(f32, f32)> = treatments
            .iter()
            .filter_map(|t| {
                amount(t)
                    .filter(|&v| v > 0.0)
                    .map(|v| (ctx.project_x(t.date), v))
            })
            .collect();
        let largest = amounts
            .iter()
            .map(|&(_, v)| v)
            .filter(|&v| !is_micro(v))
            .fold(0.0, f32::max);
        let ring = (1.5 * s).max(1.0);

        // Sizes repeat (every microbolus is the same), so each size's
        // sprites are built once, keyed by the size in half pixels.
        let mut sprites: std::collections::HashMap<(u32, bool), (Option<Sprite>, Sprite)> =
            std::collections::HashMap::new();
        amounts
            .into_iter()
            .filter_map(|(x, v)| {
                let micro = is_micro(v);
                // Square-root scaling keeps similar amounts similar in size
                // while a small correction still reads as small.
                let size = if micro {
                    2.5 * s
                } else {
                    (4.5 + 3.5 * (v / largest).sqrt()) * s
                };
                let key = (size * 2.0).round().max(2.0) as u32;
                let size = key as f32 / 2.0;
                let (under, marker) =
                    sprites
                        .entry((key, micro))
                        .or_insert_with(|| match style.marker {
                            Marker::Triangle => (
                                (!micro).then(|| create_aa_triangle_sprite(size, ring, panel)),
                                create_aa_triangle_sprite(size, 0.0, color),
                            ),
                            // A dot reads as big as a triangle a size larger.
                            Marker::Dot => (
                                Some(create_aa_circle_sprite(
                                    (size * 0.8 + ring).round() as i32,
                                    panel,
                                )),
                                create_aa_circle_sprite((size * 0.8).round() as i32, color),
                            ),
                        });

                // The first columns at or after the treatment hold the top of
                // the step it causes.
                let col = (x.ceil() as i32 - x0).max(0) as usize;
                let y = [col, col + 1]
                    .into_iter()
                    .filter_map(|c| ys.get(c).copied().flatten())
                    .reduce(f32::min)
                    .unwrap_or(zero_y);
                let (cx, cy) = (x.round() as i32, y.round() as i32);
                if let Some(under) = under {
                    blend_sprite(img, under, cx, cy);
                }
                blend_sprite(img, marker, cx, cy);
                (!micro).then_some((x, y, size + ring))
            })
            .collect()
    }

    /// Draws the status-colored line connecting consecutive readings. Sits
    /// underneath the circles and turns a loose scatter of dots into one
    /// continuous, easy-to-follow trace.
    ///
    /// The line is built by stamping a small anti-aliased disc along each
    /// segment, which gives it an even thickness and rounded joins on every
    /// slope (a plain offset line goes thin and ragged on steep climbs — the
    /// "horrible" look). Segments are only drawn between readings that are
    /// genuinely contiguous: a gap well beyond the data's own typical cadence
    /// (e.g. a sensor dropout) is left open instead of being bridged by a long
    /// misleading line, so the trace stays clean for any data thrown at it.
    fn draw_entry_trace(&self, img: &mut RgbaImage, ctx: &RenderContext, entries: &[GraphEntry]) {
        if entries.len() < 2 {
            return;
        }

        // Robust "expected" spacing (median gap) so the bridge threshold adapts
        // to 1-, 5- or 15-minute data alike.
        let mut gaps: Vec<i64> = entries
            .windows(2)
            .map(|w| (w[1].date - w[0].date).num_seconds())
            .collect();
        let mid = gaps.len() / 2;
        let median_gap = (*gaps.select_nth_unstable(mid).1).max(1);
        let max_gap = (median_gap as f32 * 2.5) as i64;

        // Line a little thinner than the markers so the dots still lead.
        let base_point_radius = if self.entries.len() > 100 { 4.0 } else { 6.0 };
        let point_radius = (base_point_radius + 1.0) * ctx.viewport.s;
        let trace_half = (point_radius * 0.45).round().max(1.0) as i32;

        let sp_high = create_aa_circle_sprite(trace_half, self.theme.glucose_high);
        let sp_low = create_aa_circle_sprite(trace_half, self.theme.glucose_low);
        let sp_range = create_aa_circle_sprite(trace_half, self.theme.glucose_in_range);
        let pick = |sgv: f32| -> &Sprite {
            if sgv > self.target_high {
                &sp_high
            } else if sgv < self.target_low {
                &sp_low
            } else {
                &sp_range
            }
        };

        let mut last_stamp: Option<(i32, i32)> = None;
        for w in entries.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if (b.date - a.date).num_seconds() > max_gap {
                last_stamp = None;
                continue;
            }

            let x0 = ctx.project_x(a.date);
            let y0 = ctx.project_y(a.sgv);
            let x1 = ctx.project_x(b.date);
            let y1 = ctx.project_y(b.sgv);

            let dx = x1 - x0;
            let dy = y1 - y0;
            let steps = (dx * dx + dy * dy).sqrt().ceil().max(1.0) as i32;

            for k in 0..=steps {
                let t = k as f32 / steps as f32;
                let px = (x0 + dx * t).round() as i32;
                let py = (y0 + dy * t).round() as i32;
                if last_stamp == Some((px, py)) {
                    continue;
                }
                let sgv = a.sgv + (b.sgv - a.sgv) * t;
                blend_sprite(img, pick(sgv), px, py);
                last_stamp = Some((px, py));
            }
        }
    }

    /// Draws the reading markers: soft, anti-aliased circles colored by status.
    /// Kept on top of the trace so each reading is still a distinct point.
    fn draw_entries(&self, img: &mut RgbaImage, ctx: &RenderContext, entries: &[GraphEntry]) {
        if entries.is_empty() {
            return;
        }

        let base_point_radius = if self.entries.len() > 100 { 4.0 } else { 6.0 };
        let point_radius = (base_point_radius + 1.0) * ctx.viewport.s;
        let radius_i32 = point_radius.max(1.0) as i32;

        let sprite_high = create_aa_circle_sprite(radius_i32, self.theme.glucose_high);
        let sprite_low = create_aa_circle_sprite(radius_i32, self.theme.glucose_low);
        let sprite_range = create_aa_circle_sprite(radius_i32, self.theme.glucose_in_range);

        let mut last_draw_pos: Option<(i32, i32)> = None;
        let min_dist_sq = (point_radius * 0.5).powf(2.0);

        for e in entries {
            let ix = ctx.project_x(e.date) as i32;
            let iy = ctx.project_y(e.sgv) as i32;

            if let Some((lx, ly)) = last_draw_pos {
                let dx = (ix - lx) as f32;
                let dy = (iy - ly) as f32;
                if dx * dx + dy * dy < min_dist_sq {
                    continue;
                }
            }

            let sprite = if e.sgv > self.target_high {
                &sprite_high
            } else if e.sgv < self.target_low {
                &sprite_low
            } else {
                &sprite_range
            };

            blend_sprite(img, sprite, ix, iy);
            last_draw_pos = Some((ix, iy));
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

/// The multiples of one of `steps` (finest first) that lie in `[lo, hi]`,
/// ascending. The step is the finest that is at least `min_step` and leaves
/// at most `max_ticks` multiples; with no such step there are no ticks.
fn round_ticks(lo: f32, hi: f32, steps: &[f32], min_step: f32, max_ticks: usize) -> Vec<f32> {
    steps
        .iter()
        .copied()
        .filter(|&step| step >= min_step)
        .find_map(|step| {
            // A little slack so a bound that is itself a multiple of the step
            // keeps its tick despite float noise.
            let first = (lo / step - 1e-3).ceil() as i64;
            let last = (hi / step + 1e-3).floor() as i64;
            (last - first < max_ticks as i64)
                .then(|| (first..=last).map(|k| k as f32 * step).collect())
        })
        .unwrap_or_default()
}

/// `(min, max)` of `values`, or `(f32::MAX, f32::MIN)` when empty.
///
/// Deliberately sequential: these are at most a few thousand floats, far too
/// little work to pay for waking a thread pool.
fn min_max(values: impl IntoIterator<Item = f32>) -> (f32, f32) {
    values
        .into_iter()
        .fold((f32::MAX, f32::MIN), |(min, max), v| {
            (min.min(v), max.max(v))
        })
}

/// The entry closest in time to `t` (whole-second resolution, earliest entry
/// on ties), found by binary search in `entries`, which must be sorted by date.
fn closest_entry(entries: &[GraphEntry], t: chrono::DateTime<Utc>) -> Option<&GraphEntry> {
    let ts = t.timestamp();
    let key = |e: &GraphEntry| e.date.timestamp();
    // First entry at or after `t`: the closest one from the right.
    let idx = entries.partition_point(|e| key(e) < ts);
    // Closest from the left is the last timestamp before `t`; take the first
    // entry sharing it so ties resolve to the earliest entry.
    let before = idx.checked_sub(1).map(|i| {
        let lts = key(&entries[i]);
        &entries[entries.partition_point(|e| key(e) < lts)]
    });
    match (before, entries.get(idx)) {
        (Some(l), Some(r)) if ts - key(l) <= key(r) - ts => Some(l),
        (_, Some(r)) => Some(r),
        (l, None) => l,
    }
}

/// Fills the area between a lane's curve (`ys`, one per pixel column from
/// `x0`) and its zero line with a vertical gradient of `color`, strongest at
/// the curve. Blended in integer math with the alpha stepping linearly per
/// row, as this touches every pixel under the curve.
fn fill_area(img: &mut RgbaImage, color: Rgba<u8>, x0: i32, ys: &[Option<f32>], zero_y: f32) {
    const FILL_AT_CURVE: f32 = 64.0;
    const FILL_AT_ZERO: f32 = 6.0;
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let stride = img_w as usize * 4;
    let [cr, cg, cb] = [color[0], color[1], color[2]].map(|c| c as u32);
    let raw = img.as_mut();
    for (i, y) in ys.iter().enumerate() {
        let (Some(y), x) = (*y, x0 + i as i32) else {
            continue;
        };
        if !(0..img_w).contains(&x) {
            continue;
        }
        let (from, to) = (y.min(zero_y), y.max(zero_y));
        let rows = (from.round() as i32).max(0)..(to.round() as i32).min(img_h);
        if rows.is_empty() {
            continue;
        }
        // Alpha at the first row, then its change per row going down:
        // fading when the curve is above zero, strengthening below it.
        let slope = (FILL_AT_ZERO - FILL_AT_CURVE) / (to - from).max(1.0);
        let first = rows.start as f32 + 0.5;
        let (mut a, step) = if y <= zero_y {
            (FILL_AT_CURVE + slope * (first - y), slope)
        } else {
            (FILL_AT_CURVE + slope * (y - first), -slope)
        };
        let mut idx = rows.start as usize * stride + x as usize * 4;
        for _ in rows {
            let ai = (a.clamp(FILL_AT_ZERO, FILL_AT_CURVE) * 256.0 / 255.0) as u32;
            let inv = 256 - ai;
            let px = &mut raw[idx..idx + 3];
            px[0] = ((cr * ai + px[0] as u32 * inv) >> 8) as u8;
            px[1] = ((cg * ai + px[1] as u32 * inv) >> 8) as u8;
            px[2] = ((cb * ai + px[2] as u32 * inv) >> 8) as u8;
            a += step;
            idx += stride;
        }
    }
}

/// Draws a lane's curve (`ys`, one per pixel column from `x0`) by stamping
/// `pen` along it, like the glucose trace. Segments between two `resting`
/// columns are skipped, and a lone sample still gets a dot.
fn stroke_curve(img: &mut RgbaImage, pen: &Sprite, x0: i32, ys: &[Option<f32>], resting: &[bool]) {
    let mut last_stamp: Option<(i32, i32)> = None;
    for i in 0..ys.len() {
        let Some(ya) = ys[i] else {
            continue;
        };
        let next = ys.get(i + 1).copied().flatten();
        let flat = resting[i] && resting.get(i + 1).copied().unwrap_or(false);
        let (Some(yb), false) = (next, flat) else {
            let prev = i.checked_sub(1).and_then(|j| ys[j]);
            if prev.is_none() && next.is_none() && !resting[i] {
                blend_sprite(img, pen, x0 + i as i32, ya.round() as i32);
            }
            last_stamp = None;
            continue;
        };
        let xa = (x0 + i as i32) as f32;
        let steps = (1.0 + (yb - ya) * (yb - ya)).sqrt().ceil() as i32;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = ((xa + t).round() as i32, (ya + (yb - ya) * t).round() as i32);
            if last_stamp != Some(p) {
                blend_sprite(img, pen, p.0, p.1);
                last_stamp = Some(p);
            }
        }
    }
}

/// The total still on board at each of the ascending `times`, when every
/// `(date, amount)` counts in full at its date and fades linearly to nothing
/// over `duration`.
///
/// The doses on board at a time `t` are exactly those given in
/// `(t - duration, t]`, and their total is `Σa - (t·Σa - Σa·d) / duration`,
/// so only the two sums need updating as that window slides along.
fn linear_on_board(
    mut doses: Vec<(chrono::DateTime<Utc>, f32)>,
    duration: Duration,
    times: &[chrono::DateTime<Utc>],
) -> Vec<f32> {
    let span = duration.num_milliseconds() as f64;
    if span <= 0.0 {
        return vec![0.0; times.len()];
    }
    doses.sort_by_key(|&(date, _)| date);
    // Milliseconds from the first time keep the sums well within f64.
    let origin = times.first().map_or(0, |t| t.timestamp_millis());
    let ms = |t: chrono::DateTime<Utc>| (t.timestamp_millis() - origin) as f64;
    let doses: Vec<(f64, f64)> = doses.into_iter().map(|(d, a)| (ms(d), a as f64)).collect();

    let (mut lo, mut hi) = (0, 0);
    let (mut sum, mut weighted) = (0.0, 0.0);
    times
        .iter()
        .map(|&t| {
            let t = ms(t);
            while hi < doses.len() && doses[hi].0 <= t {
                sum += doses[hi].1;
                weighted += doses[hi].1 * doses[hi].0;
                hi += 1;
            }
            while lo < hi && t - doses[lo].0 >= span {
                sum -= doses[lo].1;
                weighted -= doses[lo].1 * doses[lo].0;
                lo += 1;
            }
            if lo == hi {
                // Nothing on board; also clears rounding left in the sums.
                (sum, weighted) = (0.0, 0.0);
                return 0.0;
            }
            (sum - (t * sum - weighted) / span) as f32
        })
        .collect()
}

/// Linearly interpolates date-sorted `points` at each of the ascending
/// `times`. `None` outside the points, and across gaps well beyond their usual
/// spacing (never closer than 15 minutes, as AID systems skip the odd upload)
/// so a missing stretch is not drawn as a made-up line.
fn interpolate_samples(
    points: &[SeriesPoint],
    times: &[chrono::DateTime<Utc>],
) -> Vec<Option<f32>> {
    let max_gap = if points.len() > 1 {
        let mut gaps: Vec<i64> = points
            .windows(2)
            .map(|w| (w[1].date - w[0].date).num_seconds())
            .collect();
        let mid = gaps.len() / 2;
        let median_gap = *gaps.select_nth_unstable(mid).1;
        ((median_gap as f32 * 2.5) as i64).max(15 * 60)
    } else {
        0
    };

    let mut k = 0;
    times
        .iter()
        .map(|&t| {
            while k + 1 < points.len() && points[k + 1].date <= t {
                k += 1;
            }
            let a = points.get(k)?;
            if t < a.date {
                return None;
            }
            if t == a.date {
                return Some(a.value);
            }
            let b = points.get(k + 1)?;
            let span = b.date - a.date;
            if span.num_seconds() > max_gap {
                return None;
            }
            let f = (t - a.date).num_milliseconds() as f32 / span.num_milliseconds() as f32;
            Some(a.value + (b.value - a.value) * f)
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minutes: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + Duration::minutes(minutes)
    }

    fn point(minutes: i64, value: f32) -> SeriesPoint {
        SeriesPoint {
            value,
            date: at(minutes),
        }
    }

    #[test]
    fn samples_interpolate_between_points_only() {
        let points = [point(0, 1.0), point(5, 2.0), point(10, 4.0)];
        let times: Vec<_> = [-1, 0, 2, 5, 10, 11].map(at).to_vec();
        let values = interpolate_samples(&points, &times);
        assert_eq!(values[0], None);
        assert_eq!(values[1], Some(1.0));
        assert!((values[2].unwrap() - 1.4).abs() < 1e-6);
        assert_eq!(values[3], Some(2.0));
        assert_eq!(values[4], Some(4.0));
        assert_eq!(values[5], None);
    }

    #[test]
    fn doses_fade_linearly_over_the_duration() {
        let doses = vec![(at(0), 4.0)];
        let times: Vec<_> = [-1, 0, 60, 120, 239, 240, 300].map(at).to_vec();
        let values = linear_on_board(doses, Duration::hours(4), &times);
        let expected = [0.0, 4.0, 3.0, 2.0, 4.0 / 240.0, 0.0, 0.0];
        for (v, e) in values.iter().zip(expected) {
            assert!((v - e).abs() < 1e-4, "{values:?}");
        }
    }

    #[test]
    fn overlapping_doses_add_up() {
        // 2u at 0 and 3u at 30 min, 1 hour duration.
        let doses = vec![(at(30), 3.0), (at(0), 2.0)];
        let times: Vec<_> = [15, 30, 45, 60, 75, 90].map(at).to_vec();
        let values = linear_on_board(doses, Duration::hours(1), &times);
        let expected = [1.5, 1.0 + 3.0, 0.5 + 2.25, 1.5, 0.75, 0.0];
        for (v, e) in values.iter().zip(expected) {
            assert!((v - e).abs() < 1e-4, "{values:?}");
        }
    }

    #[test]
    fn no_duration_means_nothing_on_board() {
        let values = linear_on_board(vec![(at(0), 5.0)], Duration::zero(), &[at(0), at(1)]);
        assert_eq!(values, vec![0.0, 0.0]);
    }

    #[test]
    fn samples_leave_long_gaps_open() {
        // 5 minute cadence with a 40 minute hole.
        let points: Vec<_> = [0, 5, 10, 50, 55, 60].map(|m| point(m, 1.0)).to_vec();
        let values = interpolate_samples(&points, &[at(8), at(30), at(52)]);
        assert_eq!(values, vec![Some(1.0), None, Some(1.0)]);
    }

    #[test]
    fn samples_handle_empty_and_single_points() {
        assert_eq!(interpolate_samples(&[], &[at(0)]), vec![None]);
        let single = [point(0, 3.0)];
        assert_eq!(
            interpolate_samples(&single, &[at(-1), at(0), at(1)]),
            vec![None, Some(3.0), None]
        );
    }

    fn graph() -> GlucoseGraphBuilder<'static> {
        let entries: Vec<GraphEntry> = (0..=36)
            .map(|i| GraphEntry {
                sgv: 110.0 + i as f32,
                date: at(i * 5),
            })
            .collect();
        let treatments = vec![GraphTreatment {
            insulin: Some(3.0),
            carbs: Some(40.0),
            mbg: None,
            date: at(30),
            is_isf: false,
        }];
        GlucoseGraphBuilder::new()
            .with_layout(LayoutConfig {
                width: 600,
                height: 400,
                ..Default::default()
            })
            .with_entries(entries)
            .with_treatments(treatments)
    }

    #[test]
    fn round_ticks_take_the_finest_step_that_fits() {
        let every = |step: f32, ks: std::ops::RangeInclusive<i32>| -> Vec<f32> {
            ks.map(|k| k as f32 * step).collect()
        };
        let mmol = (40.0 / 18.0, 200.0 / 18.0);
        // Room for a label every 15 mg/dL: every 20 (10 is too tight).
        let mg = round_ticks(40.0, 200.0, &MGDL_STEPS, 15.0, 10);
        assert_eq!(mg, every(20.0, 2..=10));
        // Nine ticks are too many: every 30.
        let mg = round_ticks(40.0, 200.0, &MGDL_STEPS, 15.0, 7);
        assert_eq!(mg, every(30.0, 2..=6));
        // mmol/L goes from 1 straight to 2.
        let mmol_ticks = round_ticks(mmol.0, mmol.1, &MMOL_STEPS, 0.6, 10);
        assert_eq!(mmol_ticks, every(1.0, 3..=11));
        let mmol_ticks = round_ticks(mmol.0, mmol.1, &MMOL_STEPS, 0.6, 7);
        assert_eq!(mmol_ticks, every(2.0, 2..=5));
        // Only multiples inside the range, bounds included when they are one.
        let mg = round_ticks(65.0, 185.0, &MGDL_STEPS, 40.0, 10);
        assert_eq!(mg, every(50.0, 2..=3));
        let mg = round_ticks(50.0, 150.0, &MGDL_STEPS, 40.0, 10);
        assert_eq!(mg, every(50.0, 1..=3));
    }

    #[test]
    fn round_ticks_survive_degenerate_ranges() {
        let ticks = |lo, hi, min_step| round_ticks(lo, hi, &MGDL_STEPS, min_step, 10);
        assert_eq!(ticks(100.0, 100.0, 0.0), vec![100.0]);
        assert!(ticks(200.0, 100.0, 1.0).is_empty());
        assert!(ticks(100.0, 200.0, f32::INFINITY).is_empty());
        assert!(ticks(100.0, 200.0, f32::NAN).is_empty());
    }

    #[test]
    fn a_typical_day_gets_a_handful_of_lines() {
        let at_1080 = |unit| {
            let builder = graph()
                .with_units(unit)
                .with_layout(LayoutConfig::default());
            builder.value_ticks(&builder.calculate_viewport(), 40.0, 200.0)
        };
        assert_eq!(
            at_1080(UnitDisplay::MgDl),
            [60.0, 90.0, 120.0, 150.0, 180.0]
        );
        // 4, 6, 8 and 10 mmol/L.
        assert_eq!(at_1080(UnitDisplay::MmolL), [72.0, 108.0, 144.0, 180.0]);
    }

    #[test]
    fn value_labels_name_their_gridlines() {
        let units = [
            UnitDisplay::MgDl,
            UnitDisplay::MmolL,
            UnitDisplay::Dual {
                primary: UnitPreference::MgDl,
            },
            UnitDisplay::Dual {
                primary: UnitPreference::MmolL,
            },
        ];
        for unit in units {
            let mmol_first = matches!(
                unit,
                UnitDisplay::MmolL
                    | UnitDisplay::Dual {
                        primary: UnitPreference::MmolL
                    }
            );
            let dual = matches!(unit, UnitDisplay::Dual { .. });
            for (width, height) in [(1920, 1080), (800, 450), (800, 800)] {
                // A short plot too, as a large bottom margin leaves.
                for short in [false, true] {
                    let builder = graph().with_units(unit).with_layout(LayoutConfig {
                        width,
                        height,
                        margin_bottom: short.then_some(height as f32 * 0.35),
                        ..Default::default()
                    });
                    let vp = builder.calculate_viewport();
                    for (y_min, y_max) in
                        [(40.0, 200.0), (60.0, 200.0), (43.0, 213.0), (40.0, 400.0)]
                    {
                        let case =
                            format!("{unit:?} {width}x{height} short: {short} {y_min}..{y_max}");
                        let ticks = builder.value_ticks(&vp, y_min, y_max);
                        assert!(
                            (2..=MAX_VALUE_TICKS).contains(&ticks.len()),
                            "{case}: {ticks:?}"
                        );
                        for &val in &ticks {
                            assert!(val > y_min - 0.1 && val < y_max + 0.1, "{case}: {val}");
                            let (main, sub) = builder.tick_labels(val);
                            let (main, sub): (f32, Option<f32>) =
                                (main.parse().unwrap(), sub.map(|s| s.parse().unwrap()));
                            // The label is exactly the gridline's value...
                            let main_mgdl = if mmol_first { main * 18.0 } else { main };
                            assert!((main_mgdl - val).abs() < 1e-3, "{case}: {main} on {val}");
                            // ...and the one under it the same value converted.
                            if let Some(sub) = sub {
                                let (sub_mgdl, precision) = if mmol_first {
                                    (sub, 0.5)
                                } else {
                                    (sub * 18.0, 0.9)
                                };
                                assert!(
                                    (sub_mgdl - val).abs() <= precision + 1e-3,
                                    "{case}: {sub} under {val}"
                                );
                            }
                        }
                        // Labels (31px, 52px with the converted value) keep
                        // at least a label's height between them.
                        let label_h = if dual { 52.0 } else { 31.0 } * vp.s;
                        for pair in ticks.windows(2) {
                            let gap = (pair[1] - pair[0]) / (y_max - y_min) * vp.plot_h;
                            assert!(gap >= label_h + 31.0 * vp.s - 1e-3, "{case}: {ticks:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mini_graphs_take_height_from_the_plot_and_move_the_axis() {
        let plain = graph().calculate_viewport();
        assert!(plain.lanes.is_empty());
        assert_eq!(plain.axis_bottom, plain.plot_bottom);

        let one = graph()
            .add_mini_graph(MiniGraph::cob(Duration::hours(3)))
            .calculate_viewport();
        assert_eq!(one.lanes.len(), 1);
        assert!(one.plot_bottom < plain.plot_bottom);
        assert_eq!(one.axis_bottom, one.lanes[0].bottom);

        let two = graph()
            .add_mini_graph(MiniGraph::iob(Duration::hours(4)))
            .add_mini_graph(MiniGraph::cob(Duration::hours(3)))
            .calculate_viewport();
        assert_eq!(two.lanes.len(), 2);
        assert!(two.plot_bottom < one.plot_bottom);
        assert_eq!(two.axis_bottom, two.lanes[1].bottom);
        // Separate panels: a full gap and both paddings between the lanes.
        let between = two.lanes[1].top - two.lanes[0].bottom;
        assert_eq!(between, (2.0 * PANEL_PAD + PANEL_GAP) * two.s);
        // Every lane keeps the same height, and the plot ends where the
        // time axis always did.
        assert_eq!(
            two.lanes[0].bottom - two.lanes[0].top,
            one.lanes[0].bottom - one.lanes[0].top
        );
        assert!((two.axis_bottom - plain.axis_bottom).abs() < 1e-3);
    }

    #[test]
    fn mini_graphs_stack_in_the_order_added() {
        let builder = graph()
            .add_mini_graph(MiniGraph::cob_reported(vec![point(0, 1.0)]))
            .add_mini_graph(MiniGraph::iob_reported(vec![point(0, 1.0)]));
        assert!(matches!(builder.mini_graphs[0], MiniGraph::Cob(_)));
        assert!(matches!(builder.mini_graphs[1], MiniGraph::Iob(_)));
        let vp = builder.calculate_viewport();
        assert_eq!(vp.lanes.iter().map(|l| l.graph).collect::<Vec<_>>(), [0, 1]);

        let replaced = builder.with_mini_graphs([MiniGraph::iob_reported(vec![point(0, 1.0)])]);
        assert_eq!(replaced.mini_graphs.len(), 1);
        assert!(replaced
            .with_mini_graphs([])
            .calculate_viewport()
            .lanes
            .is_empty());
    }

    #[test]
    fn glucose_plot_keeps_half_the_height_with_many_mini_graphs() {
        let full = |b: GlucoseGraphBuilder<'static>| {
            b.with_layout(LayoutConfig::default()).calculate_viewport()
        };
        let plain = full(graph());
        let crowded =
            full(graph().with_mini_graphs((0..4).map(|_| MiniGraph::iob(Duration::hours(4)))));
        assert_eq!(crowded.lanes.len(), 4);
        assert!(crowded.plot_h >= plain.plot_h * 0.5 - 1e-3);
        assert!((crowded.axis_bottom - plain.axis_bottom).abs() < 1e-3);
    }

    #[test]
    fn far_too_many_mini_graphs_still_build() {
        let img = graph()
            .with_mini_graphs((0..8).map(|_| MiniGraph::cob(Duration::hours(3))))
            .build()
            .expect("an overcrowded graph should still build");
        assert_eq!(img.dimensions(), (600, 400));
    }

    #[test]
    fn mini_graphs_render_with_awkward_data() {
        let cases: Vec<Vec<SeriesPoint>> = vec![
            vec![],
            vec![point(60, 1.0)],
            vec![point(-600, 2.0), point(-595, 2.0)],
            (0..=36)
                .map(|i| point(i * 5, -0.5 + (i % 7) as f32 * 0.4))
                .collect(),
            (0..=36).map(|i| point(i * 5, 0.0)).collect(),
        ];
        for points in cases {
            let img = graph()
                .add_mini_graph(MiniGraph::iob_reported(points.clone()))
                .add_mini_graph(MiniGraph::cob_reported(points))
                .build()
                .expect("graph with mini graphs should build");
            assert_eq!(img.dimensions(), (600, 400));
        }
    }

    #[test]
    fn mini_graphs_render_dense_microboluses() {
        let mut builder = graph().with_microbolus_threshold(0.5);
        builder.treatments.extend((0..36).map(|i| GraphTreatment {
            insulin: Some(0.1 + (i % 4) as f32 * 0.1),
            carbs: None,
            mbg: None,
            date: at(i * 5 + 2),
            is_isf: true,
        }));
        let img = builder
            .add_mini_graph(MiniGraph::iob(Duration::hours(4)))
            .add_mini_graph(MiniGraph::cob(Duration::hours(3)))
            .build()
            .expect("graph with microbolus markers should build");
        assert_eq!(img.dimensions(), (600, 400));
    }

    #[test]
    fn no_extra_panel_without_mini_graphs() {
        let img = graph().build().unwrap();
        let vp = graph().calculate_viewport();
        // Right under the glucose panel is plain background again.
        let below = (vp.plot_bottom + PANEL_PAD * vp.s + 4.0) as u32;
        assert_eq!(*img.get_pixel(300, below), Theme::dark().background);
    }
}
