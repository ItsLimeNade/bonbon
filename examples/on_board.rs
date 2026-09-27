use bonbon::prelude::*;
use chrono::{DateTime, Duration, Utc};

/// A rough, deterministic 12 hour day: three meals with their boluses, a few
/// automatic microboluses, a post-lunch high and a late afternoon low.
fn simulate(now: DateTime<Utc>) -> (Vec<GraphEntry>, Vec<GraphTreatment>) {
    let start = now - Duration::hours(12);
    let meals = [(1.0, 45.0, 4.5), (5.5, 70.0, 5.0), (10.5, 55.0, 5.5)];
    let smbs = [2.2, 2.6, 6.4, 6.8, 7.2, 11.2, 11.5];

    let mut treatments = Vec::new();
    for &(h, carbs, insulin) in &meals {
        treatments.push(GraphTreatment {
            insulin: Some(insulin),
            carbs: Some(carbs),
            mbg: None,
            date: start + Duration::minutes((h * 60.0) as i64),
            is_isf: false,
        });
    }
    for &h in &smbs {
        treatments.push(GraphTreatment {
            insulin: Some(0.3),
            carbs: None,
            mbg: None,
            date: start + Duration::minutes((h * 60.0) as i64),
            is_isf: true,
        });
    }
    treatments.push(GraphTreatment {
        insulin: None,
        carbs: Some(15.0),
        mbg: Some(64.0),
        date: start + Duration::minutes(9 * 60 + 20),
        is_isf: false,
    });

    // Gamma-shaped rise for carbs, slower and longer fall for insulin.
    let bump = |t: f32, peak: f32| {
        if t <= 0.0 {
            0.0
        } else {
            (t / peak) * (1.0 - t / peak).exp()
        }
    };
    let entries = (0..=144)
        .map(|i| {
            let h = i as f32 * 5.0 / 60.0;
            let mut sgv = 120.0 + 6.0 * (h * 2.3).sin() + 3.0 * (h * 7.1).cos();
            for &(mh, carbs, insulin) in &meals {
                sgv += carbs * 2.2 * bump(h - mh, 1.0);
                sgv -= insulin * 14.0 * bump(h - mh, 2.0);
            }
            sgv -= 30.0 * bump(h - 7.6, 1.2);
            sgv += 25.0 * bump(h - 9.33, 0.7);
            GraphEntry {
                sgv,
                date: start + Duration::minutes(i * 5),
            }
        })
        .collect();

    (entries, treatments)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let (entries, treatments) = simulate(now);

    let image = GlucoseGraphBuilder::new()
        .with_layout(LayoutConfig {
            width: 1920,
            height: 1080,
            ..Default::default()
        })
        .with_theme(Theme::dark())
        .with_targets(70.0, 180.0)
        .with_microbolus_threshold(0.5)
        .with_trace(false)
        .with_entries(entries)
        .with_treatments(treatments)
        // Mini graphs are optional: add none, one or both, in the order they
        // should stack. Each treatment fades out over the given duration; with
        // an AID system, `MiniGraph::iob_reported` / `cob_reported` take its
        // reported values instead.
        .add_mini_graph(MiniGraph::iob(Duration::hours(4)))
        .add_mini_graph(MiniGraph::cob(Duration::hours(3)))
        .build()?;

    image.save("on_board.png")?;
    Ok(())
}
