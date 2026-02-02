use bonbon::prelude::*;
use chrono::{Duration, Utc};
use image::Rgba;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Define your custom theme
    let mut strawberry_theme = Theme::dark();

    strawberry_theme.background = Rgba([255, 240, 245, 255]);
    strawberry_theme.glucose_in_range = Rgba([50, 205, 50, 255]);
    strawberry_theme.glucose_high = Rgba([255, 69, 0, 255]);
    strawberry_theme.glucose_reading_fill = Rgba([255, 105, 180, 255]);

    strawberry_theme.text_primary = Rgba([40, 40, 40, 255]);
    strawberry_theme.text_secondary = Rgba([80, 80, 80, 255]);
    strawberry_theme.text_dim = Rgba([150, 150, 150, 255]);

    strawberry_theme.axis_lines = Rgba([100, 100, 100, 255]);
    strawberry_theme.grid_major = Rgba([200, 200, 200, 255]);

    let now = Utc::now();
    let entries = vec![
        GraphEntry {
            sgv: 120.0,
            date: now - Duration::hours(1),
        },
        GraphEntry {
            sgv: 180.0,
            date: now,
        },
    ];

    // Build
    let image = GlucoseGraphBuilder::new()
        // Add your custom theme
        .with_theme(strawberry_theme)
        .with_entries(entries)
        .build()?;

    image.save("strawberry_graph.png")?;
    println!("Graph saved to strawberry_graph.png");

    Ok(())
}
