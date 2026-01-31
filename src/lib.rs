pub mod models;
pub mod theme;
mod utils {
    pub mod color;
    pub mod drawing;
}
pub mod charts {
    pub mod glucose;
}

pub mod prelude {
    pub use crate::charts::glucose::{GlucoseGraphBuilder, LayoutConfig};
    #[cfg(feature = "cinnamon")]
    pub use crate::integrations::*;
    pub use crate::models::*;
    pub use crate::theme::Theme;
}

#[cfg(feature = "cinnamon")]
pub mod integrations;
