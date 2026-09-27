mod common;

use bonbon::prelude::*;
use chrono::NaiveTime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tz = chrono_tz::Europe::Paris;
    let previous = common::fortnight(tz, 1, 0xfeed_cafe);
    let current = common::fortnight(tz, 0, 0x5eed_b0b0);

    // One builder, two views: by hour of the day and by day of the week.
    let breakdown = |grouping| {
        BreakdownGraphBuilder::new()
            .with_layout(LayoutConfig {
                width: 1920,
                height: 1080,
                ..Default::default()
            })
            .with_theme(Theme::dark())
            .with_timezone(tz)
            .with_targets(80.0, 180.0)
            // A higher low target overnight, from 23:00 to 07:00.
            .with_night_targets(NightTargets {
                low: 90.0,
                high: 180.0,
                from: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
                until: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
            })
            .with_grouping(grouping)
            .with_entries(current.clone())
            .with_previous(previous.clone())
    };

    breakdown(Grouping::Hour)
        .build()?
        .save("breakdown_hourly.png")?;
    breakdown(Grouping::Weekday)
        .build()?
        .save("breakdown_weekly.png")?;
    Ok(())
}
