use bonbon::prelude::*;
use chrono::{Duration, Utc};

/// 14 days of 5-minute readings: a dawn rise, three meals a day that land a
/// little differently each day, and a few overnight lows.
fn fortnight() -> Vec<GraphEntry> {
    let start = Utc::now() - Duration::days(14);
    // Cheap deterministic noise so the example needs no extra crates.
    let noise = |n: f32| ((n * 12.9898).sin() * 43758.547).fract() - 0.5;
    let bump = |t: f32, peak: f32| {
        if t <= 0.0 {
            0.0
        } else {
            (t / peak) * (1.0 - t / peak).exp()
        }
    };

    (0..14 * 288)
        .map(|i| {
            let day = (i / 288) as f32;
            let h = (i % 288) as f32 / 12.0;
            let mut sgv = 115.0 + 18.0 * bump(h - 4.0, 3.0);
            for (k, meal) in [7.5, 12.5, 19.5].into_iter().enumerate() {
                let shift = noise(day * 3.0 + k as f32) * 1.5;
                let size = 55.0 + 45.0 * noise(day * 7.0 + k as f32 + 0.5).abs() * 2.0;
                sgv += size * bump(h - meal - shift, 1.2);
                sgv -= size * 0.45 * bump(h - meal - shift - 1.5, 2.0);
            }
            if day as i32 % 4 == 1 {
                sgv -= 45.0 * bump(h - 2.5, 1.2);
            }
            sgv += 14.0 * noise(i as f32);
            GraphEntry {
                sgv: sgv.max(40.0),
                date: start + Duration::minutes(i as i64 * 5),
            }
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = PercentileGraphBuilder::new()
        .with_layout(LayoutConfig {
            width: 1920,
            height: 1080,
            ..Default::default()
        })
        .with_theme(Theme::dark())
        .with_targets(70.0, 180.0)
        .with_timezone(chrono_tz::Europe::Paris)
        // Median with 25th-75th and 5th-95th bands by default. For other
        // percentiles, e.g. a single 15th-75th band:
        // .with_bands(PercentileBands { inner: (15.0, 75.0), outer: None, median: true })
        .with_entries(fortnight())
        .build()?;

    image.save("percentile.png")?;
    Ok(())
}
