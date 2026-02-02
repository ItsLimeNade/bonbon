use bonbon::prelude::*;
use chrono::{Duration, Utc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Sample glucose data
    let now = Utc::now();
    let entries = vec![
        GraphEntry {
            sgv: 110.0,
            date: now - Duration::minutes(30),
        },
        GraphEntry {
            sgv: 145.0,
            date: now - Duration::minutes(15),
        },
        GraphEntry {
            sgv: 185.0,
            date: now,
        },
    ];

    // Sample treatment data
    let treatments = vec![GraphTreatment {
        insulin: Some(2.5),
        carbs: Some(30.0),
        mbg: None,
        date: now - Duration::minutes(45),
        is_isf: false,
    }];

    // Configure the graph layout
    let layout = LayoutConfig {
        width: 1280,
        height: 720,
        ..Default::default()
    };

    // Build the graph
    let image = GlucoseGraphBuilder::new()
        .with_layout(layout)
        .with_theme(Theme::dark())
        .with_units(UnitDisplay::MgDl)
        .with_targets(70.0, 180.0)
        .with_entries(entries)
        .with_treatments(treatments)
        .with_time_axis(TimeAxisMode::EquallyDistributed { count: 6 })
        .build()?;

    // Save the output
    // Though this works just fine, it is recommended to look at the optimal_compression.rs example.
    // With a few additional lines of code, it is possible to increase the compression speeds by
    // a big margin while keeping a very reasonable file size.
    image.save("glucose_report.png")?;

    Ok(())
}
