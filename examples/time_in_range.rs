//! Renders the time-in-range card in both built-in themes from a week of
//! synthetic CGM data.

use bonbon::prelude::*;
use chrono::{Duration, Utc};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let entries = synthetic_week();

    // The same numbers the card displays are available without rendering.
    let stats = TirStats::compute(&entries, &TirThresholds::default()).unwrap();
    println!(
        "in range {:.0}% · mean {:.0} mg/dl · CV {:.1}% · GMI {:.1}%",
        stats.percentages[TirBand::InRange as usize],
        stats.mean_mgdl,
        stats.cv_percent,
        stats.gmi_percent,
    );

    let dark = TimeInRangeBuilder::new()
        .with_entries(entries.clone())
        .with_targets(70.0, 180.0)
        .with_extreme_targets(54.0, 250.0)
        .with_units(UnitDisplay::MgDl)
        .with_theme(Theme::dark())
        .with_scale(2.0)
        .build()?;
    write_png("tir_dark.png", &dark)?;
    println!("Saved -> tir_dark.png");

    let light = TimeInRangeBuilder::new()
        .with_entries(entries)
        .with_units(UnitDisplay::MmolL)
        .with_theme(Theme::light())
        .with_scale(2.0)
        .build()?;
    write_png("tir_light.png", &light)?;
    println!("Saved -> tir_light.png");

    Ok(())
}

/// A week of 5-minute readings: daily sine wave + meal spikes + a couple of
/// overnight lows, so every band gets some data.
fn synthetic_week() -> Vec<GraphEntry> {
    let now = Utc::now();
    let count = 7 * 288;
    (0..count)
        .map(|i| {
            let mins = i as f32 * 5.0;
            let day_phase = (mins / 1440.0) * std::f32::consts::TAU;
            let meal_phase = (mins / 280.0) * std::f32::consts::TAU;
            let drift = (mins / 3900.0).sin() * 52.0;
            let sgv = 128.0 + day_phase.sin() * 46.0 + meal_phase.sin() * 36.0 + drift;
            GraphEntry {
                sgv: sgv.clamp(40.0, 350.0),
                date: now - Duration::minutes(((count - 1 - i) * 5) as i64),
            }
        })
        .collect()
}

fn write_png(path: &str, img: &image::RgbaImage) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufWriter;
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    PngEncoder::new_with_quality(writer, CompressionType::Level(6), FilterType::NoFilter)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )?;
    Ok(())
}
