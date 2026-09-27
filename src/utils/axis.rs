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
