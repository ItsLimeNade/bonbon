//! Renders every graph and card in each unit display: mg/dL, mmol/L, and
//! both at once with either one first. Each gets its four images and a sheet
//! of them side by side, under `tests/units/`.

mod common;

use bonbon::prelude::*;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use image::{imageops, RgbaImage};

type Rendered = Result<RgbaImage, Box<dyn std::error::Error>>;
/// Renders one graph or card in the given unit display.
type Render<'a> = Box<dyn Fn(UnitDisplay) -> Rendered + 'a>;

const UNITS: [(&str, UnitDisplay); 4] = [
    ("mgdl", UnitDisplay::MgDl),
    ("mmol", UnitDisplay::MmolL),
    (
        "dual_mgdl",
        UnitDisplay::Dual {
            primary: UnitPreference::MgDl,
        },
    ),
    (
        "dual_mmol",
        UnitDisplay::Dual {
            primary: UnitPreference::MmolL,
        },
    ),
];

/// Twelve hours with a meal, its bolus and two fingersticks.
fn half_day(now: DateTime<Utc>) -> (Vec<GraphEntry>, Vec<GraphTreatment>) {
    let start = now - Duration::hours(12);
    let at = |h: f32| start + Duration::minutes((h * 60.0) as i64);
    let bump = |t: f32, peak: f32| {
        if t <= 0.0 {
            0.0
        } else {
            (t / peak) * (1.0 - t / peak).exp()
        }
    };
    let entries = (0..=144)
        .map(|i| {
            let h = i as f32 / 12.0;
            let sgv = 125.0 + 8.0 * (h * 2.3).sin() + 150.0 * bump(h - 2.0, 1.0)
                - 95.0 * bump(h - 5.0, 2.0);
            GraphEntry {
                sgv,
                date: start + Duration::minutes(i * 5),
            }
        })
        .collect();
    let treatments = vec![
        GraphTreatment {
            insulin: Some(5.5),
            carbs: Some(60.0),
            mbg: None,
            date: at(2.0),
            is_isf: false,
        },
        GraphTreatment {
            insulin: None,
            carbs: None,
            mbg: Some(150.0),
            date: at(4.5),
            is_isf: false,
        },
        GraphTreatment {
            insulin: None,
            carbs: Some(15.0),
            mbg: Some(110.0),
            date: at(8.0),
            is_isf: false,
        },
    ];
    (entries, treatments)
}

fn bg_card_data(now: DateTime<Utc>) -> BgCardData {
    let sparkline_points = (0..36)
        .map(|i| {
            let t = i as f32 / 35.0;
            SparklinePoint {
                t,
                sgv: 150.0 + 45.0 * (t * 4.0).sin(),
                status: GlucoseStatus::InRange,
            }
        })
        .collect();
    BgCardData {
        current_sgv: 126.0,
        status: GlucoseStatus::InRange,
        trend_arrow: "→".to_string(),
        delta: Some(-7.0),
        age_str: "3 min ago".to_string(),
        time_str: now.format("%H:%M").to_string(),
        watermark_str: "Bonbon".to_string(),
        iob_str: Some("IOB 2.5u".to_string()),
        cob_str: Some("COB 30g".to_string()),
        sparkline_points,
        info_pill: None,
    }
}

/// The four images in a 2×2 grid.
fn sheet(images: &[RgbaImage]) -> RgbaImage {
    let (w, h) = images[0].dimensions();
    let mut sheet = RgbaImage::new(w * 2, h * 2);
    for (i, img) in images.iter().enumerate() {
        let (col, row) = (i as u32 % 2, i as u32 / 2);
        imageops::replace(&mut sheet, img, (col * w) as i64, (row * h) as i64);
    }
    sheet
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tz = chrono_tz::Europe::Paris;
    let now = Utc::now();
    let (entries, treatments) = half_day(now);
    let current = common::fortnight(tz, 0, 0x5eed_b0b0);
    let previous = common::fortnight(tz, 1, 0xfeed_cafe);
    let layout = LayoutConfig {
        width: 1200,
        height: 800,
        ..Default::default()
    };
    let night = NightTargets {
        low: 90.0,
        high: 180.0,
        from: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
        until: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
    };

    let charts: Vec<(&str, Render)> = vec![
        (
            "glucose",
            Box::new(|units| {
                GlucoseGraphBuilder::new()
                    .with_layout(layout.clone())
                    .with_timezone(tz)
                    .with_units(units)
                    .with_entries(entries.clone())
                    .with_treatments(treatments.clone())
                    .build()
            }),
        ),
        (
            "bg_card",
            Box::new(|units| {
                BgCardBuilder::new()
                    .with_data(bg_card_data(now))
                    .with_units(units)
                    .with_scale(2.0)
                    .build()
            }),
        ),
        (
            "time_in_range",
            Box::new(|units| {
                TimeInRangeBuilder::new()
                    .with_entries(current.clone())
                    .with_units(units)
                    .with_scale(2.0)
                    .build()
            }),
        ),
        (
            "percentile",
            Box::new(|units| {
                PercentileGraphBuilder::new()
                    .with_layout(layout.clone())
                    .with_timezone(tz)
                    .with_units(units)
                    .with_entries(current.clone())
                    .build()
            }),
        ),
        (
            "compare",
            Box::new(|units| {
                CompareGraphBuilder::new()
                    .with_layout(layout.clone())
                    .with_timezone(tz)
                    .with_targets(80.0, 180.0)
                    .with_night_targets(night)
                    .with_units(units)
                    .with_periods(previous.clone(), current.clone())
                    .build()
            }),
        ),
        (
            "breakdown",
            Box::new(|units| {
                BreakdownGraphBuilder::new()
                    .with_layout(layout.clone())
                    .with_timezone(tz)
                    .with_targets(80.0, 180.0)
                    .with_night_targets(night)
                    .with_units(units)
                    .with_entries(current.clone())
                    .with_previous(previous.clone())
                    .build()
            }),
        ),
    ];

    let dir = std::path::Path::new("tests/units");
    std::fs::create_dir_all(dir)?;
    for (name, render) in &charts {
        let mut images = Vec::new();
        for (unit, display) in UNITS {
            let image = render(display)?;
            image.save(dir.join(format!("{name}_{unit}.png")))?;
            images.push(image);
        }
        sheet(&images).save(dir.join(format!("{name}.png")))?;
        println!("Saved -> {}", dir.join(format!("{name}.png")).display());
    }
    Ok(())
}
