mod common;

use bonbon::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tz = chrono_tz::Europe::Paris;
    let image = PercentileGraphBuilder::new()
        .with_layout(LayoutConfig {
            width: 1920,
            height: 1080,
            ..Default::default()
        })
        .with_theme(Theme::dark())
        .with_targets(70.0, 180.0)
        .with_timezone(tz)
        // Median with 25th-75th and 5th-95th bands by default. For other
        // percentiles, e.g. a single 15th-75th band:
        // .with_bands(PercentileBands { inner: (15.0, 75.0), outer: None, median: true })
        .with_entries(common::fortnight(tz, 0, 0x5eed_b0b0))
        .build()?;

    image.save("percentile.png")?;
    Ok(())
}
