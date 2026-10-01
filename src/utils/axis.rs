//! Glucose axis helpers shared by the charts: units, value formatting and
//! round gridline values.

use crate::models::{UnitDisplay, UnitPreference};

pub(crate) const MGDL_PER_MMOL: f32 = 18.0182;

/// The unit labels are shown in first.
pub(crate) fn unit_preference(unit: UnitDisplay) -> UnitPreference {
    match unit {
        UnitDisplay::MgDl => UnitPreference::MgDl,
        UnitDisplay::MmolL => UnitPreference::MmolL,
        UnitDisplay::Dual { primary } => primary,
    }
}

pub(crate) fn other_unit(unit: UnitPreference) -> UnitPreference {
    match unit {
        UnitPreference::MgDl => UnitPreference::MmolL,
        UnitPreference::MmolL => UnitPreference::MgDl,
    }
}

/// The units glucose values are written in: the primary one, then the one
/// repeated smaller and dimmer next to it when both are displayed. Every
/// chart writes its glucose values through this, so they all show the second
/// unit the same way.
pub(crate) fn units(unit: UnitDisplay) -> (UnitPreference, Option<UnitPreference>) {
    let primary = unit_preference(unit);
    let secondary = matches!(unit, UnitDisplay::Dual { .. }).then(|| other_unit(primary));
    (primary, secondary)
}

pub(crate) fn unit_name(unit: UnitPreference) -> &'static str {
    match unit {
        UnitPreference::MgDl => "mg/dL",
        UnitPreference::MmolL => "mmol/L",
    }
}

/// `mgdl` in `unit`, without the unit: whole mg/dL or mmol/L to one decimal.
pub(crate) fn fmt_glucose(mgdl: f32, unit: UnitPreference) -> String {
    match unit {
        UnitPreference::MgDl => format!("{:.0}", mgdl),
        UnitPreference::MmolL => format!("{:.1}", mgdl / MGDL_PER_MMOL),
    }
}

/// `mgdl` in `unit`, followed by the unit: "154 mg/dL".
pub(crate) fn fmt_glucose_unit(mgdl: f32, unit: UnitPreference) -> String {
    format!("{} {}", fmt_glucose(mgdl, unit), unit_name(unit))
}

/// A change of `mgdl` in `unit`, signed: "+3", "-0.2", or "±0" when it
/// rounds to nothing.
pub(crate) fn fmt_delta(mgdl: f32, unit: UnitPreference) -> String {
    let magnitude = fmt_glucose(mgdl.abs(), unit);
    if magnitude.chars().all(|c| c == '0' || c == '.') {
        format!("±{magnitude}")
    } else if mgdl > 0.0 {
        format!("+{magnitude}")
    } else {
        format!("-{magnitude}")
    }
}

/// The spacing in mg/dL of gridlines over `range_mgdl` drawn `height_px`
/// tall: the finest round step of `unit` that leaves 1.5 label heights per
/// line and no more than 8 lines.
pub(crate) fn round_step(
    range_mgdl: f32,
    height_px: f32,
    label_h: f32,
    unit: UnitPreference,
) -> f32 {
    let (steps, per): (&[f32], f32) = match unit {
        UnitPreference::MgDl => (&[10.0, 20.0, 25.0, 50.0, 100.0, 200.0], 1.0),
        UnitPreference::MmolL => (&[0.5, 1.0, 2.0, 5.0, 10.0], MGDL_PER_MMOL),
    };
    steps
        .iter()
        .map(|st| st * per)
        .find(|&step| {
            let lines = range_mgdl / step;
            lines <= 8.0 && height_px / lines >= label_h * 1.5
        })
        .unwrap_or(steps[steps.len() - 1] * per)
}

/// The multiples of `step` from `min` to `max` (mg/dL).
pub(crate) fn round_ticks(min: f32, max: f32, step: f32) -> Vec<f32> {
    let first = (min / step - 1e-3).ceil() as i32;
    let last = (max / step + 1e-3).floor() as i32;
    (first..=last).map(|k| k as f32 * step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charts::bg_card::{BgCardBuilder, BgCardData, GlucoseStatus};
    use crate::charts::breakdown::BreakdownGraphBuilder;
    use crate::charts::compare::CompareGraphBuilder;
    use crate::charts::glucose::GlucoseGraphBuilder;
    use crate::charts::percentile::PercentileGraphBuilder;
    use crate::charts::time_in_range::TimeInRangeBuilder;
    use crate::models::{GraphEntry, GraphTreatment};
    use chrono::{Duration, TimeZone, Utc};
    use image::RgbaImage;

    const DUAL_MMOL: UnitDisplay = UnitDisplay::Dual {
        primary: UnitPreference::MmolL,
    };

    #[test]
    fn only_dual_displays_have_a_second_unit() {
        assert_eq!(units(UnitDisplay::MgDl), (UnitPreference::MgDl, None));
        assert_eq!(units(UnitDisplay::MmolL), (UnitPreference::MmolL, None));
        assert_eq!(
            units(DUAL_MMOL),
            (UnitPreference::MmolL, Some(UnitPreference::MgDl))
        );
    }

    #[test]
    fn values_are_written_in_each_unit() {
        assert_eq!(fmt_glucose(126.0, UnitPreference::MgDl), "126");
        assert_eq!(fmt_glucose(126.0, UnitPreference::MmolL), "7.0");
        assert_eq!(
            fmt_glucose_unit(180.0, UnitPreference::MmolL),
            "10.0 mmol/L"
        );
        assert_eq!(fmt_glucose_unit(70.0, UnitPreference::MgDl), "70 mg/dL");
    }

    #[test]
    fn deltas_are_signed() {
        assert_eq!(fmt_delta(3.0, UnitPreference::MgDl), "+3");
        assert_eq!(fmt_delta(-7.0, UnitPreference::MmolL), "-0.4");
        assert_eq!(fmt_delta(0.4, UnitPreference::MgDl), "±0");
        assert_eq!(fmt_delta(-0.5, UnitPreference::MmolL), "±0.0");
    }

    /// Every graph and card, rendered in `unit` from the same readings.
    fn render_all(unit: UnitDisplay) -> Vec<(&'static str, RgbaImage)> {
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let entries: Vec<GraphEntry> = (0..14 * 288)
            .map(|i| GraphEntry {
                sgv: 140.0 + 60.0 * (i as f32 / 40.0).sin() + (i % 7) as f32,
                date: start + Duration::minutes(i * 5),
            })
            .collect();
        let fingerstick = GraphTreatment {
            insulin: None,
            carbs: None,
            mbg: Some(112.0),
            date: start + Duration::hours(6),
            is_isf: false,
        };
        let card = BgCardData {
            current_sgv: 126.0,
            status: GlucoseStatus::InRange,
            trend_arrow: "→".to_string(),
            delta: Some(-7.0),
            watermark_str: String::new(),
            age_str: String::new(),
            time_str: String::new(),
            iob_str: None,
            cob_str: None,
            sparkline_points: Vec::new(),
            info_pill: None,
        };
        let e = || entries.clone();
        vec![
            (
                "glucose",
                GlucoseGraphBuilder::new()
                    .with_units(unit)
                    .with_entries(entries[..288].to_vec())
                    .with_treatments(vec![fingerstick])
                    .build(),
            ),
            (
                "bg card",
                BgCardBuilder::new()
                    .with_units(unit)
                    .with_data(card)
                    .build(),
            ),
            (
                "time in range",
                TimeInRangeBuilder::new()
                    .with_units(unit)
                    .with_entries(e())
                    .build(),
            ),
            (
                "percentile",
                PercentileGraphBuilder::new()
                    .with_units(unit)
                    .with_entries(e())
                    .build(),
            ),
            (
                "compare",
                CompareGraphBuilder::new()
                    .with_units(unit)
                    .with_periods(e(), e())
                    .build(),
            ),
            (
                "breakdown",
                BreakdownGraphBuilder::new()
                    .with_units(unit)
                    .with_entries(e())
                    .build(),
            ),
        ]
        .into_iter()
        .map(|(name, image)| (name, image.unwrap()))
        .collect()
    }

    #[test]
    fn every_chart_shows_the_second_unit() {
        for primary in [UnitPreference::MgDl, UnitPreference::MmolL] {
            let single = render_all(match primary {
                UnitPreference::MgDl => UnitDisplay::MgDl,
                UnitPreference::MmolL => UnitDisplay::MmolL,
            });
            let dual = render_all(UnitDisplay::Dual { primary });
            for ((name, single), (_, dual)) in single.iter().zip(&dual) {
                assert!(single != dual, "{name} ignores the second unit");
            }
        }
    }

    #[test]
    fn every_chart_follows_its_unit() {
        let mgdl = render_all(UnitDisplay::MgDl);
        let mmol = render_all(UnitDisplay::MmolL);
        for ((name, mgdl), (_, mmol)) in mgdl.iter().zip(&mmol) {
            assert!(mgdl != mmol, "{name} ignores its unit");
        }
    }
}
