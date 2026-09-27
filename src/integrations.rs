use crate::models::{GraphEntry, GraphTreatment, SeriesPoint};
use chrono::{DateTime, Utc};
use cinnamon::models::{
    devicestatus::DeviceStatus,
    entries::{MbgEntry, SgvEntry},
    treatments::Treatment,
};
use serde_json::Value;

impl From<SgvEntry> for GraphEntry {
    fn from(entry: SgvEntry) -> Self {
        let date = DateTime::from_timestamp_millis(entry.date).unwrap_or_else(|| Utc::now());

        Self {
            sgv: entry.sgv as f32,
            date,
        }
    }
}

impl From<MbgEntry> for GraphTreatment {
    fn from(entry: MbgEntry) -> Self {
        let date = DateTime::from_timestamp_millis(entry.date).unwrap_or_else(|| Utc::now());

        Self {
            insulin: None,
            carbs: None,
            mbg: Some(entry.mbg as f32),
            date,
            is_isf: false,
        }
    }
}

impl TryFrom<Treatment> for GraphTreatment {
    type Error = chrono::ParseError;

    fn try_from(t: Treatment) -> Result<Self, Self::Error> {
        let date = DateTime::parse_from_rfc3339(&t.created_at)?.with_timezone(&Utc);

        Ok(Self {
            insulin: t.insulin.map(|v| v as f32),
            carbs: t.carbs.map(|v| v as f32),
            mbg: t.glucose.map(|v| v as f32),
            date,
            is_isf: false,
        })
    }
}

/// Splits Nightscout device statuses into insulin on board and carbs on board
/// samples, ready for
/// [`MiniGraph::iob_reported`](crate::charts::glucose::MiniGraph::iob_reported) and
/// [`MiniGraph::cob_reported`](crate::charts::glucose::MiniGraph::cob_reported).
///
/// Reads the `openaps` block (AAPS, Trio, OpenAPS) and the `loop` block
/// (Loop). Statuses without a readable date or without a value are skipped.
pub fn on_board_from_device_statuses(
    statuses: &[DeviceStatus],
) -> (Vec<SeriesPoint>, Vec<SeriesPoint>) {
    let number = |v: Option<&Value>, key: &str| {
        v.and_then(|v| v.get(key))
            .and_then(Value::as_f64)
            .map(|n| n as f32)
    };

    let mut iob = Vec::new();
    let mut cob = Vec::new();
    for status in statuses {
        let Ok(date) = DateTime::parse_from_rfc3339(&status.created_at) else {
            continue;
        };
        let date = date.with_timezone(&Utc);
        let openaps = status.openaps.as_ref();
        let suggested = openaps.and_then(|o| o.get("suggested"));
        let enacted = openaps.and_then(|o| o.get("enacted"));
        // Loop's block is keyed `loop`, which lands in `extra`.
        let loop_ = status.loop_.as_ref().or_else(|| status.extra.get("loop"));

        // oref0 uploads `iob` as a list of forecasts, the current one first.
        let openaps_iob = openaps
            .and_then(|o| o.get("iob"))
            .map(|v| v.get(0).unwrap_or(v));
        let iob_value = number(openaps_iob, "iob")
            .or_else(|| number(suggested, "IOB"))
            .or_else(|| number(loop_.and_then(|l| l.get("iob")), "iob"));
        let cob_value = number(suggested, "COB")
            .or_else(|| number(enacted, "COB"))
            .or_else(|| number(loop_.and_then(|l| l.get("cob")), "cob"));

        if let Some(value) = iob_value {
            iob.push(SeriesPoint { value, date });
        }
        if let Some(value) = cob_value {
            cob.push(SeriesPoint { value, date });
        }
    }
    (iob, cob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(body: Value) -> DeviceStatus {
        serde_json::from_value(body).expect("device status should deserialize")
    }

    #[test]
    fn reads_openaps_and_loop_statuses() {
        let statuses = [
            // AAPS / Trio: `iob` is an object, COB sits in `suggested`.
            status(json!({
                "created_at": "2026-01-01T10:00:00.000Z",
                "openaps": { "iob": { "iob": 1.25 }, "suggested": { "COB": 20 } }
            })),
            // oref0: `iob` is a list of forecasts, COB only in `enacted`.
            status(json!({
                "created_at": "2026-01-01T10:05:00.000Z",
                "openaps": { "iob": [{ "iob": -0.4 }, { "iob": -0.3 }], "enacted": { "COB": 0 } }
            })),
            // Loop.
            status(json!({
                "created_at": "2026-01-01T10:10:00Z",
                "loop": { "iob": { "iob": 2.5 }, "cob": { "cob": 12.0 } }
            })),
            // Pump-only status and an unreadable date: skipped.
            status(json!({ "created_at": "2026-01-01T10:15:00Z", "pump": { "reservoir": 80 } })),
            status(json!({ "created_at": "yesterday", "loop": { "iob": { "iob": 9.0 } } })),
        ];

        let (iob, cob) = on_board_from_device_statuses(&statuses);
        let values = |points: &[SeriesPoint]| points.iter().map(|p| p.value).collect::<Vec<_>>();
        assert_eq!(values(&iob), vec![1.25, -0.4, 2.5]);
        assert_eq!(values(&cob), vec![20.0, 0.0, 12.0]);
        assert_eq!(iob[2].date.to_rfc3339(), "2026-01-01T10:10:00+00:00");
    }
}
