use crate::models::{
    GraphEntry, GraphScaling, GraphTreatment, TimeAxisMode, TreatmentDisplayMode, UnitDisplay,
    UnitPreference,
};
use crate::theme::Theme;
use crate::utils::drawing::{
    draw_dashed_horizontal_line, draw_dashed_vertical_line
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
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
            margin_top: 80.0,
            margin_bottom: 100.0,
            margin_left: 120.0,
            margin_right: 60.0,
        }
    }
}

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
            timezone: chrono_tz::UTC,
            layout: LayoutConfig::default(),
            theme,
            font: font_data,
            microbolus_threshold: 0.0,
        }
    }

    pub fn with_entries(mut self, entries: Vec<GraphEntry>) -> Self {
        self.entries = entries;
        self
    }

    pub fn with_treatments(mut self, treatments: Vec<GraphTreatment>) -> Self {
        self.treatments = treatments;
        self
    }

    pub fn with_targets(mut self, low: f32, high: f32) -> Self {
        self.target_low = low;
        self.target_high = high;
        self
    }

    pub fn with_units(mut self, display: UnitDisplay) -> Self {
        self.unit_display = display;
        self
    }

    pub fn with_scaling(mut self, scaling: GraphScaling) -> Self {
        self.scaling = scaling;
        self
    }

    pub fn with_timezone(mut self, tz: Tz) -> Self {
        self.timezone = tz;
        self
    }

    pub fn with_layout(mut self, layout: LayoutConfig) -> Self {
        self.layout = layout;
        self
    }

    pub fn with_font(mut self, font: &'a [u8]) -> Self {
        self.font = font;
        self
    }

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
    /// If set, this overrides the dynamic auto-scaling of the time axis.
    pub fn with_fixed_duration(mut self, duration: Duration) -> Self {
        self.fixed_duration = Some(duration);
        self
    }

    /// Sets the threshold for microboluses.
    /// Insulin treatments <= this value will be rendered as small ticks without text labels.
    pub fn with_microbolus_threshold(mut self, threshold: f32) -> Self {
        self.microbolus_threshold = threshold;
        self
    }

    pub fn build(self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let font = FontRef::try_from_slice(self.font)?;
        let mut img =
            RgbaImage::from_pixel(self.layout.width, self.layout.height, self.theme.background);

        let base_width = 1200.0;
        let base_height = 800.0;
        let scale_x = self.layout.width as f32 / base_width;
        let scale_y = self.layout.height as f32 / base_height;
        let s = scale_x.min(scale_y).max(0.5);

        let font_size_md = 30.0 * s;
        let font_size_sm = 24.0 * s;
        let font_size_xs = 20.0 * s;
        let font_size_ctx = 26.0 * s;

        let grid_width = (6.0 * s) as i32;
        let axis_thickness = (2.0 * s).ceil() as i32;
        let point_radius = if self.entries.len() > 100 {
            4.0 * s
        } else {
            6.0 * s
        };

        let mut sorted_entries = self.entries.clone();
        sorted_entries.sort_by_key(|e| e.date);

        let (start_time, end_time) = if let Some(duration) = self.fixed_duration {
            let now = Utc::now();
            let anchor = if sorted_entries.is_empty() {
                now
            } else {
                let last_entry = sorted_entries.last().unwrap().date;
                if (now - last_entry).num_hours() > 24 {
                    last_entry
                } else {
                    now
                }
            };
            (anchor - duration, anchor)
        } else {
            if sorted_entries.is_empty() {
                return Err("No entries provided and no fixed duration set".into());
            }
            let end = sorted_entries.last().unwrap().date;
            let start = sorted_entries.first().unwrap().date;
            let adjusted_start = if start == end {
                end - Duration::hours(1)
            } else {
                start
            };
            (adjusted_start, end)
        };

        let time_span_secs = (end_time - start_time).num_seconds().max(1) as f32;

        let visible_entries: Vec<&GraphEntry> = sorted_entries
            .iter()
            .filter(|e| e.date >= start_time && e.date <= end_time)
            .collect();

        let (y_min, y_max) = match self.scaling {
            GraphScaling::Static { min, max } => (min, max),
            GraphScaling::Dynamic {
                clamp_min,
                clamp_max,
            } => {
                if visible_entries.is_empty() {
                    (clamp_min, clamp_max)
                } else {
                    let max_sgv = visible_entries
                        .iter()
                        .map(|e| e.sgv)
                        .fold(0.0f32, |a, b| a.max(b));
                    let min_sgv = visible_entries
                        .iter()
                        .map(|e| e.sgv)
                        .fold(400.0f32, |a, b| a.min(b));
                    let calc_max = ((max_sgv + 20.0) / 10.0).ceil() * 10.0;
                    let calc_min = ((min_sgv - 20.0) / 10.0).floor() * 10.0;
                    (calc_min.max(clamp_min), calc_max.min(clamp_max))
                }
            }
        };

        let plot_w = self.layout.width as f32 - self.layout.margin_left - self.layout.margin_right;
        let plot_h = self.layout.height as f32 - self.layout.margin_top - self.layout.margin_bottom;
        let plot_top = self.layout.margin_top;
        let plot_left = self.layout.margin_left;
        let plot_bottom = plot_top + plot_h;

        let project_x = |time: chrono::DateTime<Utc>| -> f32 {
            let offset = (time - start_time).num_seconds() as f32;
            plot_left + (offset / time_span_secs) * plot_w
        };

        let project_y = |sgv: f32| -> f32 {
            let clamped = sgv.clamp(y_min, y_max);
            let ratio = (clamped - y_min) / (y_max - y_min);
            plot_bottom - (ratio * plot_h)
        };

        let high_y = project_y(self.target_high);
        let low_y = project_y(self.target_low);
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

        draw_dashed_horizontal_line(
            &mut img,
            high_y,
            plot_left,
            plot_left + plot_w,
            high_col,
            (10.0 * s) as i32,
            (10.0 * s) as i32,
        );
        draw_dashed_horizontal_line(
            &mut img,
            low_y,
            plot_left,
            plot_left + plot_w,
            low_col,
            (10.0 * s) as i32,
            (10.0 * s) as i32,
        );

        let draw_thick_vertical = |img: &mut RgbaImage, x: f32, color: Rgba<u8>| {
            let thickness = (3.0 * s) as i32;
            for i in 0..thickness {
                let offset = x + i as f32 - (thickness as f32 / 2.0);
                draw_line_segment_mut(img, (offset, plot_top), (offset, plot_bottom), color);
            }
        };

        match self.time_axis_mode {
            TimeAxisMode::Simple => {
                let mut current_day = start_time.with_timezone(&self.timezone).date_naive();
                let mut pointer = start_time;
                let step = if time_span_secs > 86400.0 * 2.0 {
                    Duration::hours(4)
                } else {
                    Duration::hours(1)
                };

                let remainder = pointer.timestamp() % 3600;
                if remainder != 0 {
                    pointer = pointer + Duration::seconds(3600 - remainder);
                }

                while pointer <= end_time {
                    let local_date = pointer.with_timezone(&self.timezone).date_naive();
                    let x = project_x(pointer);
                    if x >= plot_left && x <= plot_left + plot_w {
                        if local_date != current_day {
                            draw_thick_vertical(&mut img, x, self.theme.axis_lines);
                            let date_str = local_date.format("%d %b").to_string();
                            draw_text_mut(
                                &mut img,
                                self.theme.text_primary,
                                (x + 5.0 * s) as i32,
                                (plot_top + 10.0 * s) as i32,
                                PxScale::from(font_size_sm),
                                &font,
                                &date_str,
                            );
                            current_day = local_date;
                        }
                    }
                    pointer += step;
                }
            }
            TimeAxisMode::EquallyDistributed { count } => {
                let step_secs = time_span_secs / (count as f32);
                let mut current_day = start_time.with_timezone(&self.timezone).date_naive();

                for i in 0..=count {
                    let offset = i as f32 * step_secs;
                    let tick_time = start_time + Duration::seconds(offset as i64);
                    let x = plot_left + (offset / time_span_secs) * plot_w;

                    if x > plot_left + plot_w + 1.0 {
                        continue;
                    }

                    let local_time = tick_time.with_timezone(&self.timezone);
                    let local_date = local_time.date_naive();

                    if local_date != current_day {
                        draw_thick_vertical(&mut img, x, self.theme.axis_lines);
                        current_day = local_date;
                        let date_str = local_date.format("%d %b").to_string();
                        draw_text_mut(
                            &mut img,
                            self.theme.text_primary,
                            (x + 5.0 * s) as i32,
                            (plot_top + 10.0 * s) as i32,
                            PxScale::from(font_size_sm),
                            &font,
                            &date_str,
                        );
                    } else {
                        draw_dashed_vertical_line(
                            &mut img,
                            x,
                            plot_top,
                            plot_bottom,
                            self.theme.grid_major,
                            grid_width,
                            grid_width,
                        );
                    }

                    let time_str = local_time.format("%H:%M").to_string();
                    let dim_time = text_dimensions(&time_str, font_size_sm, &font);
                    draw_text_mut(
                        &mut img,
                        self.theme.text_primary,
                        (x - dim_time.0 / 2.0) as i32,
                        (plot_bottom + 10.0 * s) as i32,
                        PxScale::from(font_size_sm),
                        &font,
                        &time_str,
                    );

                    let diff_secs = (end_time - tick_time).num_seconds();
                    let hours = diff_secs as f32 / 3600.0;
                    let rel_str = if hours.abs() < 0.1 {
                        "-0h".to_string()
                    } else {
                        format!("-{:.1}h", hours)
                    };
                    let dim_rel = text_dimensions(&rel_str, font_size_xs, &font);
                    draw_text_mut(
                        &mut img,
                        self.theme.text_dim,
                        (x - dim_rel.0 / 2.0) as i32,
                        (plot_bottom + 10.0 * s + dim_time.1 + 4.0 * s) as i32,
                        PxScale::from(font_size_xs),
                        &font,
                        &rel_str,
                    );
                }
            }
        }

        for i in 0..axis_thickness {
            let offset = i as f32;
            draw_line_segment_mut(
                &mut img,
                (plot_left - offset, plot_top),
                (plot_left - offset, plot_bottom),
                self.theme.axis_lines,
            );
            draw_line_segment_mut(
                &mut img,
                (plot_left, plot_bottom + offset),
                (plot_left + plot_w, plot_bottom + offset),
                self.theme.axis_lines,
            );
        }

        let unit_label_y = plot_top - (50.0 * s);
        match self.unit_display {
            UnitDisplay::MgDl => {
                let text = "mg/dL";
                let dim = text_dimensions(text, font_size_sm, &font);
                draw_text_mut(
                    &mut img,
                    self.theme.text_primary,
                    (plot_left - dim.0 - 5.0 * s) as i32,
                    unit_label_y as i32,
                    PxScale::from(font_size_sm),
                    &font,
                    text,
                );
            }
            UnitDisplay::MmolL => {
                let text = "mmol/L";
                let dim = text_dimensions(text, font_size_sm, &font);
                draw_text_mut(
                    &mut img,
                    self.theme.text_primary,
                    (plot_left - dim.0 - 5.0 * s) as i32,
                    unit_label_y as i32,
                    PxScale::from(font_size_sm),
                    &font,
                    text,
                );
            }
            UnitDisplay::Dual { primary } => {
                let (u1, u2) = match primary {
                    UnitPreference::MgDl => ("mg/dL", "mmol/L"),
                    UnitPreference::MmolL => ("mmol/L", "mg/dL"),
                };
                let dim1 = text_dimensions(u1, font_size_sm, &font);
                let dim2 = text_dimensions(u2, font_size_xs, &font);
                draw_text_mut(
                    &mut img,
                    self.theme.text_primary,
                    (plot_left - dim1.0 - 5.0 * s) as i32,
                    unit_label_y as i32,
                    PxScale::from(font_size_sm),
                    &font,
                    u1,
                );
                draw_text_mut(
                    &mut img,
                    self.theme.text_dim,
                    (plot_left - dim2.0 - 5.0 * s) as i32,
                    (unit_label_y + dim1.1 + 5.0 * s) as i32,
                    PxScale::from(font_size_xs),
                    &font,
                    u2,
                );
            }
        }

        let steps = 6;
        let step_size = (y_max - y_min) / (steps as f32);
        for i in 0..=steps {
            let val = y_min + (i as f32 * step_size);
            let y_pos = project_y(val);
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
            let main_dim = text_dimensions(&main_text, font_size_md, &font);
            draw_text_mut(
                &mut img,
                self.theme.text_primary,
                (plot_left - main_dim.0 - 10.0 * s) as i32,
                (y_pos - main_dim.1 / 2.0) as i32,
                PxScale::from(font_size_md),
                &font,
                &main_text,
            );
            if let Some(sub) = sub_text {
                let sub_dim = text_dimensions(&sub, font_size_xs, &font);
                draw_text_mut(
                    &mut img,
                    self.theme.text_dim,
                    (plot_left - sub_dim.0 - 10.0 * s) as i32,
                    (y_pos + main_dim.1 / 2.0) as i32,
                    PxScale::from(font_size_xs),
                    &font,
                    &sub,
                );
            }
        }

        let mut visible_treatments: Vec<&GraphTreatment> = self
            .treatments
            .iter()
            .filter(|t| t.date >= start_time && t.date <= end_time)
            .collect();
        visible_treatments.sort_by_key(|t| t.date);

        for t in &visible_treatments {
            if let Some(mbg) = t.mbg {
                let x = project_x(t.date);
                let y = project_y(mbg);
                let outline_r = (point_radius * 1.5) as i32;
                let fill_r = point_radius as i32;
                draw_filled_circle_mut(
                    &mut img,
                    (x as i32, y as i32),
                    outline_r,
                    self.theme.glucose_reading_outline,
                );
                draw_filled_circle_mut(
                    &mut img,
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
                let dim = text_dimensions(&val_str, font_size_xs, &font);
                draw_text_mut(
                    &mut img,
                    self.theme.text_primary,
                    (x - dim.0 / 2.0) as i32,
                    (y - outline_r as f32 - dim.1 - 5.0 * s) as i32,
                    PxScale::from(font_size_xs),
                    &font,
                    &val_str,
                );
            }
        }

        match self.treatment_mode {
            TreatmentDisplayMode::Contextual => {
                let insulin_offset_ctx = 45.0 * s;
                let carbs_offset_ctx = 45.0 * s;
                let icon_scale = 1.6;
                let text_scale = PxScale::from(font_size_ctx);
                let text_distance = 15.0 * s;

                let dark_insulin = darken_color(self.theme.insulin, 0.6);
                let dark_carbs = darken_color(self.theme.carbs, 0.6);

                for t in &visible_treatments {
                    let x = project_x(t.date);
                    let closest = sorted_entries
                        .iter()
                        .min_by_key(|e| (e.date.timestamp() - t.date.timestamp()).abs());
                    let base_y = if let Some(entry) = closest {
                        project_y(entry.sgv)
                    } else {
                        plot_bottom
                    };

                    if let Some(ins) = t.insulin {
                        let min_size = 6.0 * s;
                        let max_size = 25.0 * s;
                        let micro_size = 3.5 * s;

                        let size = if ins <= self.microbolus_threshold {
                            micro_size
                        } else {
                            let calculated = (8.0 * s) + (ins * 2.0 * s);
                            calculated.clamp(min_size, max_size) * icon_scale
                        };

                        let y = base_y + insulin_offset_ctx;

                        draw_smart_triangle(
                            &mut img,
                            (x as i32, y as i32),
                            size,
                            self.theme.insulin,
                            dark_insulin,
                            self.theme.background,
                        );

                        if ins > self.microbolus_threshold {
                            let text = format!("{:.1}u", ins);
                            let dim = text_dimensions(&text, font_size_ctx, &font);
                            draw_text_mut(
                                &mut img,
                                self.theme.text_secondary,
                                (x - dim.0 / 2.0) as i32,
                                (y + size + text_distance) as i32,
                                text_scale,
                                &font,
                                &text,
                            );
                        }
                    }

                    if let Some(carbs) = t.carbs {
                        let y = base_y - carbs_offset_ctx;
                        let radius = 10.0 * s * icon_scale;

                        draw_smart_circle(
                            &mut img,
                            x as i32,
                            y as i32,
                            radius as i32,
                            self.theme.carbs,
                            dark_carbs,
                            self.theme.background,
                        );

                        let text = format!("{:.0}g", carbs);
                        let dim = text_dimensions(&text, font_size_ctx, &font);
                        draw_text_mut(
                            &mut img,
                            self.theme.text_secondary,
                            (x - dim.0 / 2.0) as i32,
                            (y - radius - dim.1 - text_distance) as i32,
                            text_scale,
                            &font,
                            &text,
                        );
                    }
                }
            }
            TreatmentDisplayMode::Timeline => {
                let mut major_treatments = Vec::new();
                for t in &visible_treatments {
                    let mut is_micro = false;
                    if let Some(ins) = t.insulin {
                        if ins <= self.microbolus_threshold && t.carbs.is_none() {
                            is_micro = true;
                            let x = project_x(t.date);
                            let tick_height = 8.0 * s;
                            draw_line_segment_mut(
                                &mut img,
                                (x, plot_bottom),
                                (x, plot_bottom - tick_height),
                                self.theme.insulin,
                            );
                        }
                    }
                    if !is_micro {
                        major_treatments.push(t);
                    }
                }

                let px_threshold = 45.0 * s;
                let mut groups: Vec<Vec<&GraphTreatment>> = Vec::new();
                for t in &major_treatments {
                    if let Some(last_group) = groups.last_mut() {
                        let last_t = last_group[0];
                        let x1 = project_x(last_t.date);
                        let x2 = project_x(t.date);
                        if (x2 - x1).abs() < px_threshold {
                            last_group.push(*t);
                            continue;
                        }
                    }
                    groups.push(vec![*t]);
                }

                for group in groups {
                    let x_sum: f32 = group.iter().map(|t| project_x(t.date)).sum();
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

                    let item_height = font_size_sm + (4.0 * s);
                    let stem_base_y = plot_bottom;
                    let stack_bottom_y = stem_base_y - (15.0 * s);
                    draw_line_segment_mut(
                        &mut img,
                        (x_center, stem_base_y),
                        (x_center, stack_bottom_y),
                        self.theme.axis_lines,
                    );

                    let total_stack_height = items.len() as f32 * item_height;
                    let top_y = stack_bottom_y - total_stack_height;

                    for (i, item) in items.iter().enumerate() {
                        let y_pos = top_y + (i as f32 * item_height);
                        let dim = text_dimensions(&item.text, font_size_sm, &font);
                        draw_text_mut(
                            &mut img,
                            item.color,
                            (x_center - dim.0 / 2.0) as i32,
                            y_pos as i32,
                            PxScale::from(font_size_sm),
                            &font,
                            &item.text,
                        );
                    }
                }
            }
        }

        for e in &visible_entries {
            let x = project_x(e.date);
            let y = project_y(e.sgv);
            let color = if e.sgv > self.target_high {
                self.theme.glucose_high
            } else if e.sgv < self.target_low {
                self.theme.glucose_low
            } else {
                self.theme.glucose_in_range
            };

            draw_filled_circle_mut(&mut img, (x as i32, y as i32), point_radius as i32, color);
        }

        Ok(img)
    }
}


fn text_dimensions(text: &str, size: f32, _font: &FontRef) -> (f32, f32) {
    let width = text.len() as f32 * (size * 0.6);
    (width, size)
}

fn darken_color(c: Rgba<u8>, factor: f32) -> Rgba<u8> {
    Rgba([
        (c[0] as f32 * factor) as u8,
        (c[1] as f32 * factor) as u8,
        (c[2] as f32 * factor) as u8,
        c[3],
    ])
}

fn draw_smart_circle(
    img: &mut RgbaImage,
    cx: i32,
    cy: i32,
    radius: i32,
    color: Rgba<u8>,
    dark_color: Rgba<u8>,
    bg_color: Rgba<u8>,
) {
    let r2 = (radius * radius) as i32;
    let min_x = (cx - radius).max(0);
    let max_x = (cx + radius).min(img.width() as i32 - 1);
    let min_y = (cy - radius).max(0);
    let max_y = (cy + radius).min(img.height() as i32 - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                let current_px = img.get_pixel(x as u32, y as u32);
                let draw_col = if current_px == &bg_color {
                    color
                } else {
                    dark_color
                };
                img.put_pixel(x as u32, y as u32, draw_col);
            }
        }
    }
}

fn draw_smart_triangle(
    img: &mut RgbaImage,
    center: (i32, i32),
    size: f32,
    color: Rgba<u8>,
    dark_color: Rgba<u8>,
    bg_color: Rgba<u8>,
) {
    let x = center.0 as f32;
    let y = center.1 as f32;
    
    let p1 = ((x - size) as i32, (y - size) as i32);
    let p2 = ((x + size) as i32, (y - size) as i32);
    let p3 = (x as i32, (y + size) as i32);

    let min_x = p1.0.min(p2.0).min(p3.0).max(0);
    let max_x = p1.0.max(p2.0).max(p3.0).min(img.width() as i32 - 1);
    let min_y = p1.1.min(p2.1).min(p3.1).max(0);
    let max_y = p1.1.max(p2.1).max(p3.1).min(img.height() as i32 - 1);

    let sign = |p1: (i32, i32), p2: (i32, i32), p3: (i32, i32)| -> i32 {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let pt = (x, y);
            let d1 = sign(pt, p1, p2);
            let d2 = sign(pt, p2, p3);
            let d3 = sign(pt, p3, p1);

            let has_neg = (d1 < 0) || (d2 < 0) || (d3 < 0);
            let has_pos = (d1 > 0) || (d2 > 0) || (d3 > 0);

            if !(has_neg && has_pos) {
                let current_px = img.get_pixel(x as u32, y as u32);
                let draw_col = if current_px == &bg_color {
                    color
                } else {
                    dark_color
                };
                img.put_pixel(x as u32, y as u32, draw_col);
            }
        }
    }
}
