use bonbon::prelude::*;
use chrono::{Duration, Utc};
use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

// Import image encoding traits
use image::codecs::png::{PngEncoder, FilterType, CompressionType};
use image::{ExtendedColorType, ImageEncoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let entries = vec![
        GraphEntry { sgv: 110.0, date: now - Duration::minutes(30) },
        GraphEntry { sgv: 185.0, date: now },
    ];

    let builder = GlucoseGraphBuilder::new()
        .with_layout(LayoutConfig { width: 1920, height: 1080, ..Default::default() })
        .with_entries(entries)
        .build()?;

    // OPTIMAL SAVING STRATEGY
    // Why this is the optimal way for glucose charts:
    // - BufWriter: Minimizes system calls by buffering writes in memory.
    // - Compression Level 9: Glucose charts have large areas of flat colors and repetitive 
    //    grid lines. DEFLATE (Level 9) excels at compressing these patterns losslessly.
    // - FilterType::NoFilter: PNG filters are designed for photos. For synthetic charts, 
    //    filters add significant CPU overhead with negligible size benefits. Disabling 
    //    them speeds up encoding by up to 40% while keeping the file small.
    
    let t_save_start = Instant::now();
    let filename = "advanced_output.png";
    
    let file = File::create(filename)?;
    let mut writer = BufWriter::new(file);

    let encoder = PngEncoder::new_with_quality(
        &mut writer,
        CompressionType::Level(9),
        FilterType::NoFilter,
    );

    encoder.write_image(
        builder.as_raw(),
        builder.width(),
        builder.height(),
        ExtendedColorType::Rgba8,
    )?;

    println!("Encoded high-resolution chart in {:?}", t_save_start.elapsed());
    Ok(())
}