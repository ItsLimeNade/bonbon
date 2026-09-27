use bonbon::prelude::*;
use chrono::{DateTime, Duration, NaiveTime, Utc};

/// 14 days of 5-minute readings from `start`: a gentle daily rhythm around
/// `base` with a late-morning peak, day-to-day drift and some noise.
fn fortnight(start: DateTime<Utc>, base: f32, seed: f32) -> Vec<GraphEntry> {
    // battle royale
    // Cheap deterministic noise so the example needs no extra crates.
    let noise = |n: f32| ((n * 12.9898 + seed).sin() * 43758.547).rem_euclid(1.0) - 0.5;
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
            let mut sgv = base - 12.0 * bump(h - 3.0, 3.0) + 38.0 * bump(h - 9.0, 2.0);
            sgv += 14.0 * bump(h - 13.5, 2.5) + 10.0 * bump(h - 19.0, 3.0);
            sgv += 70.0 * noise(day * 5.0 + (h / 2.0).floor());
            sgv += 30.0 * noise(day * 3.0 + 0.5) + 12.0 * noise(i as f32);
            GraphEntry {
                sgv,
                date: start + Duration::minutes(i as i64 * 5),
            }
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let before = fortnight(now - Duration::days(28), 158.0, 1.0);
    let after = fortnight(now - Duration::days(14), 154.0, 2.0);

    let image = CompareGraphBuilder::new()
        .with_layout(LayoutConfig {
            width: 1920,
            height: 1080,
            ..Default::default()
        })
        .with_theme(Theme::dark())
        .with_timezone(chrono_tz::Europe::Paris)
        .with_targets(80.0, 180.0)
        // A higher low target overnight, from 23:00 to 07:00.
        .with_night_targets(NightTargets {
            low: 90.0,
            high: 180.0,
            from: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
            until: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
        })
        .with_periods(before, after)
        .build()?;

    image.save("compare.png")?;
    Ok(())
}
