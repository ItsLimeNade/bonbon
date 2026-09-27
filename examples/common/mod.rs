//! Realistic sample data shared by the examples.

use bonbon::prelude::*;
use chrono::{Duration, Utc};
use chrono_tz::Tz;

/// A small seeded random generator, so the example needs no extra crates and
/// draws the same graph every run.
struct Rng(u64);

impl Rng {
    /// Uniform in [0, 1).
    fn next(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [lo, hi).
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    /// Standard normal (Box-Muller).
    fn normal(&mut self) -> f32 {
        let (u, v) = (self.next().max(1e-7), self.next());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.next() < p
    }
}

/// Rise-then-fall shape peaking at `peak` hours after `t = 0`.
fn bump(t: f32, peak: f32) -> f32 {
    if t <= 0.0 {
        0.0
    } else {
        (t / peak) * (1.0 - t / peak).exp()
    }
}

/// Something that moves glucose: `amount` mg/dL at its peak, `peak` hours
/// after `at` (in hours since the first day's midnight).
struct Effect {
    at: f32,
    peak: f32,
    amount: f32,
}

/// 14 days of 5-minute readings that behave like a real type 1 diabetes
/// fortnight: meals at different times and sizes, boluses that are sometimes
/// late or forgotten, nights that stay flat, drop low or run high, dawn
/// rises, the odd afternoon exercise low, day-to-day drift, a slow random
/// wander and sensor noise, clamped to the sensor's 40–400 mg/dL range, with
/// a sensor change leaving a 2-hour gap. Everything runs on one timeline, so
/// a late dinner carries on past midnight.
///
/// `fortnights_ago` picks the period: 0 for the last 14 days, 1 for the 14
/// before them. `seed` decides everything else, so each run draws the same.
pub fn fortnight(tz: Tz, fortnights_ago: i64, seed: u64) -> Vec<GraphEntry> {
    const DAYS: usize = 14;
    let mut rng = Rng(seed);
    // Start at local midnight, so meals land at real clock times.
    let days_back = DAYS as i64 * (fortnights_ago + 1);
    let start = (Utc::now().with_timezone(&tz).date_naive() - Duration::days(days_back))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(tz)
        .unwrap()
        .with_timezone(&Utc);

    // Each day's baseline, reached at noon and blended into the next.
    let bases: Vec<f32> = (0..=DAYS).map(|_| 138.0 + 24.0 * rng.normal()).collect();
    let base = |t: f32| {
        let f = ((t - 12.0) / 24.0).clamp(0.0, (DAYS - 1) as f32);
        let d = f.floor() as usize;
        bases[d] + (bases[d + 1] - bases[d]) * (f - d as f32)
    };

    let mut effects = Vec::new();
    for day in 0..DAYS {
        let day0 = day as f32 * 24.0;
        let mut add = |at: f32, peak: f32, amount: f32| {
            effects.push(Effect {
                at: day0 + at,
                peak,
                amount,
            })
        };

        // Dawn phenomenon.
        add(rng.range(3.5, 5.0), 2.0, rng.range(10.0, 35.0));

        // Breakfast, lunch and dinner: carbs raise, insulin brings it back
        // down later. A late or missed bolus leaves a long high until the
        // correction that eventually follows.
        for (time, spread, carbs) in [(7.5, 0.75, 110.0), (12.75, 1.0, 120.0), (19.75, 1.0, 140.0)]
        {
            if rng.chance(0.08) {
                continue; // Skipped meal.
            }
            let at = time + spread * rng.normal();
            let rise = carbs * rng.range(0.6, 1.6);
            let dosed = if rng.chance(0.2) {
                rng.range(0.1, 0.45)
            } else {
                rng.range(0.75, 1.15)
            };
            add(at, 1.0, rise);
            // The slower insulin curve covers 2.2 times the carbs' area for
            // the same height, so a full bolus cancels the meal.
            add(at + rng.range(0.0, 0.4), 2.2, -rise * dosed / 2.2);
            if dosed < 0.5 {
                add(at + rng.range(2.5, 4.0), 2.0, -rise * 0.3);
            }
        }

        // Afternoon snack, exercise, and how the night goes.
        if rng.chance(0.35) {
            add(rng.range(15.5, 17.0), 1.0, rng.range(25.0, 60.0));
        }
        if rng.chance(0.25) {
            add(rng.range(16.0, 19.0), 1.3, -rng.range(45.0, 85.0));
        }
        match rng.next() {
            n if n < 0.15 => add(rng.range(1.0, 3.0), 1.5, -rng.range(50.0, 80.0)),
            n if n < 0.4 => add(rng.range(-1.0, 1.0), 3.0, rng.range(40.0, 90.0)),
            _ => {}
        }
    }

    let mut entries = Vec::new();
    let mut wander = 0.0f32;
    for i in 0..DAYS * 288 {
        // A sensor change on day 9 leaves a 2-hour gap.
        if (9 * 288 + 132..9 * 288 + 156).contains(&i) {
            continue;
        }
        let t = i as f32 / 12.0;
        wander = 0.97 * wander + 5.0 * rng.normal();
        let moved: f32 = effects
            .iter()
            .filter(|e| (0.0..14.0).contains(&(t - e.at)))
            .map(|e| e.amount * bump(t - e.at, e.peak))
            .sum();
        let sgv = base(t) + moved + wander + 4.0 * rng.normal();
        entries.push(GraphEntry {
            sgv: sgv.clamp(40.0, 400.0).round(),
            date: start + Duration::minutes(i as i64 * 5),
        });
    }
    entries
}
