use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Represents a single glucose reading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEntry {
    pub sgv: f32,
    pub date: DateTime<Utc>,
}

/// Represents a treatment (Insulin, Carbs, or Manual BG).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTreatment {
    pub insulin: Option<f32>,
    pub carbs: Option<f32>,
    pub mbg: Option<f32>,
    pub date: DateTime<Utc>,
    pub is_isf: bool, // Distinguish SMBs/Microboluses if needed
}

/// Preferences for unit display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UnitDisplay {
    #[default]
    MgDl,
    MmolL,
    /// Shows both, with the first type as the primary (larger) label.
    Dual {
        primary: UnitPreference,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnitPreference {
    MgDl,
    MmolL,
}

/// Defines how the Y-Axis scales.
#[derive(Debug, Clone, Copy)]
pub enum GraphScaling {
    /// Manually set Min and Max (e.g., 40, 400).
    Static { min: f32, max: f32 },
    /// Automatically detected based on data, clamped to specific bounds.
    Dynamic { clamp_min: f32, clamp_max: f32 },
}

impl Default for GraphScaling {
    fn default() -> Self {
        Self::Dynamic {
            clamp_min: 40.0,
            clamp_max: 400.0,
        }
    }
}

/// Determines how treatments (Insulin/Carbs) are positioned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TreatmentDisplayMode {
    /// Treatments are shown in fixed lanes at the bottom of the graph (Timeline view).
    Timeline,
    /// Treatments are "attached" to the closest SGV point in time.
    /// Carbs appear above the SGV, Insulin appears below.
    Contextual,
}

impl Default for TreatmentDisplayMode {
    fn default() -> Self {
        Self::Timeline
    }
}

/// Configuration for the X-Axis time labels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeAxisMode {
    /// Standard hourly ticks.
    Simple,
    /// Distributes labels equally (e.g., every N hours) and shows both:
    /// 1. Local Time (HH:MM)
    /// 2. Relative Time (-2h)
    EquallyDistributed { count: u32 },
    // I'll add more in the future... Probably?
}

impl Default for TimeAxisMode {
    fn default() -> Self {
        Self::Simple
    }
}
