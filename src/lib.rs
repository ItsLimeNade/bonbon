//! # bonbon
//!
//! `bonbon` is a high-performance glucose data visualization library designed for
//! rendering clear, informative blood glucose charts. It focuses on efficiency and
//! visual clarity, making it suitable for both web backends and embedded systems.
//!
//! ## Features
//!
//! * **High Performance**: Tight, allocation-light pixel loops and pre-computed sprites and glyph masks, with no thread pool to spin up.
//! * **Dynamic Scaling**: Automatically adjusts Y-axis bounds based on data range.
//! * **Flexible Unit Support**: Native support for mg/dL and mmol/L, including dual-unit display modes.
//! * **Treatment Visualization**: Render insulin boluses, carbohydrate intake, and manual fingerstick calibrations.
//! * **Compare Graph**: Two periods side by side with their typical days and statistics.
//! * **Percentile Graph**: A typical day from any number of days, with median and percentile bands.
//! * **Mini Graphs**: Optional insulin and carbs on board graphs under the glucose graph, from treatments or reported values.
//! * **Theming**: Fully customizable color palettes.
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//! * [`models`]: Data structures for glucose readings, treatments, and axis configurations.
//! * [`charts`]: The primary plotting logic, including [`charts::glucose::GlucoseGraphBuilder`], [`charts::percentile::PercentileGraphBuilder`], [`charts::compare::CompareGraphBuilder`] and [`charts::bg_card::BgCardBuilder`].
//! * [`theme`]: Styling and color management.
//! * [`prelude`]: A convenient module to import common traits and structures.

pub mod models;
pub mod theme;
mod utils {
    pub mod axis;
    pub mod color;
    pub mod drawing;
    pub mod text;
}
pub mod charts {
    pub mod bg_card;
    pub mod compare;
    pub mod glucose;
    pub mod percentile;
    #[cfg(feature = "beetroot")]
    pub mod stickers;
    pub mod time_in_range;
}

pub mod prelude {
    pub use crate::charts::bg_card::{
        builtin_icons, BgCardBuilder, BgCardData, GlucoseStatus, InfoPill, PillIcon, PillState,
        SparklinePoint,
    };
    pub use crate::charts::compare::{CompareGraphBuilder, NightTargets};
    pub use crate::charts::glucose::{GlucoseGraphBuilder, LayoutConfig, MiniGraph, OnBoard};
    pub use crate::charts::percentile::{
        PercentileBands, PercentileGraphBuilder, PercentileProfile,
    };
    #[cfg(feature = "beetroot")]
    pub use crate::charts::stickers::{Sticker, StickerCategory, StickerSet, StickerSource};
    pub use crate::charts::time_in_range::{TimeInRangeBuilder, TirBand, TirStats, TirThresholds};
    #[cfg(feature = "cinnamon")]
    pub use crate::integrations::*;
    pub use crate::models::*;
    pub use crate::theme::Theme;
}

#[cfg(feature = "cinnamon")]
pub mod integrations;
