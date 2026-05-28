//! The stickers themselves are generated in-memory as solid-color squares so
//! the example is self-contained. In a real app you'd load your own PNGs via
//! `StickerSource::from_path("...")` or `StickerSource::from_bytes(include_bytes!("..."))`.

use bonbon::prelude::*;
use chrono::{Duration, Utc};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Color cheat sheet:
    //   orange -> High         green  -> FastRise
    //   blue   -> InRange      purple -> FastDrop
    //   red    -> Low          yellow -> Background (anywhere on the canvas)
    // `.with_fast_rate_threshold(N)` controls what counts as a "fast" change
    // on the *graph* (mg/dL per minute). The default is 2.0.
    let stickers = StickerSet::new(12)
        .with_fast_rate_threshold(2.5)
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([255, 140, 0, 230], 96)),
            StickerCategory::High,
        ))
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([60, 130, 240, 230], 96)),
            StickerCategory::InRange,
        ))
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([230, 50, 50, 230], 96)),
            StickerCategory::Low,
        ))
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([60, 200, 90, 230], 96)),
            StickerCategory::FastRise,
        ))
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([170, 80, 200, 230], 96)),
            StickerCategory::FastDrop,
        ))
        .push_sticker(Sticker::new(
            StickerSource::from_bytes(solid_png([240, 220, 90, 180], 96)),
            StickerCategory::Background,
        ));

    let now = Utc::now();
    let entries: Vec<GraphEntry> = (0..288)
        .map(|i| {
            let t = i as f32 / 287.0;
            let sgv = 150.0 + (t * std::f32::consts::TAU * 2.0).sin() * 100.0;
            GraphEntry {
                sgv: sgv.clamp(40.0, 320.0),
                date: now - Duration::minutes(((287 - i) * 5) as i64),
            }
        })
        .collect();

    let graph = GlucoseGraphBuilder::new()
        .with_layout(LayoutConfig {
            width: 1600,
            height: 900,
            ..Default::default()
        })
        .with_theme(Theme::dark())
        .with_units(UnitDisplay::MgDl)
        .with_targets(70.0, 180.0)
        .with_time_axis(TimeAxisMode::EquallyDistributed { count: 6 })
        .with_entries(entries)
        // The `with_stickers` method is only available with `--features beetroot`.
        .with_stickers(stickers.clone())
        .build()?;

    write_png("stickers_graph.png", graph.as_raw(), graph.width(), graph.height())?;
    println!("Saved -> stickers_graph.png");

    let card_data = BgCardData {
        current_sgv: 243.0,
        status: GlucoseStatus::High,
        trend_arrow: "↑".to_string(),
        delta_str: "+18 mg/dl".to_string(),
        age_str: "5 min ago".to_string(),
        unit_str: "mg/dl".to_string(),
        time_str: now.format("%H:%M").to_string(),
        watermark_str: "Beetroot".to_string(),
        iob_str: Some("IOB 4.2u".to_string()),
        cob_str: Some("COB 60g".to_string()),
        sparkline_points: sparkline(36, 230.0, 25.0),
        info_pill: None,
    };

    // For the card, lower the limit and explicitly tell it the current
    // rate of change. When `|current_rate| >= fast_rate_threshold`
    let card_stickers = StickerSet {
        limit: 4,
        ..stickers
    }
    .with_current_rate(3.0);

    let card = BgCardBuilder::new()
        .with_data(card_data)
        .with_theme(Theme::dark())
        .with_scale(2.0)
        .with_stickers(card_stickers)
        .build()?;

    write_png("stickers_card.png", card.as_raw(), card.width(), card.height())?;
    println!("Saved -> stickers_card.png");

    Ok(())
}

fn solid_png(rgba: [u8; 4], size: u32) -> Vec<u8> {
    let buf: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(size, size, |_, _| Rgba(rgba));
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::NoFilter)
        .write_image(buf.as_raw(), size, size, ExtendedColorType::Rgba8)
        .expect("encoding a tiny RGBA square should never fail");
    bytes
}

fn sparkline(count: usize, base_sgv: f32, amplitude: f32) -> Vec<SparklinePoint> {
    (0..count)
        .map(|i| {
            let t = if count == 1 {
                0.0
            } else {
                i as f32 / (count - 1) as f32
            };
            let sgv = (base_sgv + (t * std::f32::consts::TAU).sin() * amplitude).clamp(20.0, 400.0);
            let status = if sgv < 70.0 {
                GlucoseStatus::Low
            } else if sgv > 180.0 {
                GlucoseStatus::High
            } else {
                GlucoseStatus::InRange
            };
            SparklinePoint { t, sgv, status }
        })
        .collect()
}

fn write_png(path: &str, raw: &[u8], w: u32, h: u32) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufWriter;
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    PngEncoder::new_with_quality(&mut writer, CompressionType::Level(6), FilterType::NoFilter)
        .write_image(raw, w, h, ExtendedColorType::Rgba8)?;
    Ok(())
}
