//! # bonbon
//!
//! `bonbon` is a high-performance glucose data visualization library designed for
//! rendering clear, informative blood glucose charts. It focuses on efficiency and
//! visual clarity, making it suitable for both web backends and embedded systems.
//!
//! ## Features
//!
//! * **High Performance**: Leverages `rayon` for parallel data processing and optimized sprite-based rendering.
//! * **Dynamic Scaling**: Automatically adjusts Y-axis bounds based on data range.
//! * **Flexible Unit Support**: Native support for mg/dL and mmol/L, including dual-unit display modes.
//! * **Treatment Visualization**: Render insulin boluses, carbohydrate intake, and manual fingerstick calibrations.
//! * **Theming**: Fully customizable color palettes.
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//! * [`models`]: Data structures for glucose readings, treatments, and axis configurations.
//! * [`charts`]: The primary plotting logic, including [`charts::glucose::GlucoseGraphBuilder`] and [`charts::bg_card::BgCardBuilder`].
//! * [`theme`]: Styling and color management.
//! * [`prelude`]: A convenient module to import common traits and structures.

pub mod models;
pub mod theme;
mod utils {
    pub mod color;
    pub mod drawing;
    pub mod text;
}
pub mod charts {
    pub mod bg_card;
    pub mod glucose;
    #[cfg(feature = "beetroot")]
    pub mod stickers;
    pub mod time_in_range;
}

pub mod prelude {
    pub use crate::charts::bg_card::{
        builtin_icons, BgCardBuilder, BgCardData, GlucoseStatus, InfoPill, PillIcon, PillState,
        SparklinePoint,
    };
    pub use crate::charts::glucose::{GlucoseGraphBuilder, LayoutConfig};
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
