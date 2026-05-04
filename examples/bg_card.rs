use bonbon::prelude::*;
use chrono::Utc;
use std::fs::File;
use std::io::BufWriter;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();

    // Build a 3-hour sparkline from sine-wave data (oldest → newest)
    let point_count = 36;
    let sparkline_points: Vec<SparklinePoint> = (0..point_count)
        .map(|i| {
            let t = i as f32 / (point_count - 1) as f32;
            let sgv = 120.0 + 30.0 * (t * std::f32::consts::TAU).sin();
            let status = if sgv < 70.0 {
                GlucoseStatus::Low
            } else if sgv > 180.0 {
                GlucoseStatus::High
            } else {
                GlucoseStatus::InRange
            };
            SparklinePoint { t, sgv, status }
        })
        .collect();

    let data = BgCardData {
        current_sgv: 126.0,
        status: GlucoseStatus::InRange,
        trend_arrow: "→".to_string(),
        delta_str: "+3 mg/dl".to_string(),
        age_str: "3 min ago".to_string(),
        unit_str: "mg/dl".to_string(),
        time_str: now.format("%H:%M").to_string(),
        watermark_str: "Beetroot".to_string(),
        iob_str: Some("IOB 2.5u".to_string()),
        cob_str: Some("COB 30g".to_string()),
        sparkline_points,
    };

    let img = BgCardBuilder::new()
        .with_data(data)
        .with_theme(Theme::dark())
        .with_scale(1.0) // use 4.0 for a 2560×1280 high-res output
        .build()?;

    let filename = "bg_card.png";
    let file = File::create(filename)?;
    let mut writer = BufWriter::new(file);
    PngEncoder::new_with_quality(&mut writer, CompressionType::Level(6), FilterType::NoFilter)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)?;

    println!("Saved → {}", filename);
    Ok(())
}
