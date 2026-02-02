use bonbon::prelude::*;
use chrono::Utc;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the theme from a JSON file
    let file = File::open("examples/theme.json")?;
    let reader = BufReader::new(file);
    let custom_theme: Theme = serde_json::from_reader(reader)?;

    // Use it in the builder
    let now = Utc::now();
    let entries = vec![GraphEntry {
        sgv: 110.0,
        date: now,
    }];

    let image = GlucoseGraphBuilder::new()
        .with_theme(custom_theme)
        .with_entries(entries)
        .build()?;

    image.save("json_theme_graph.png")?;

    Ok(())
}
