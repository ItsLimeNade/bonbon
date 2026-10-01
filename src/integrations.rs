use crate::models::{GraphEntry, GraphTreatment, SeriesPoint};
use cinnamon::model::{DeviceStatus, Glucose, Mbg, Sgv, Treatment, Units};

/// A treatment that has neither a readable `created_at` nor a `date`, so it
/// cannot be placed on a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("treatment has no readable timestamp")]
pub struct MissingTimestamp;

impl From<Sgv> for GraphEntry {
    fn from(entry: Sgv) -> Self {
        Self {
            sgv: entry.sgv.as_mgdl() as f32,
            date: entry.date.to_datetime(),
        }
    }
}

impl From<Mbg> for GraphTreatment {
    fn from(entry: Mbg) -> Self {
        Self {
            insulin: None,
            carbs: None,
            mbg: Some(entry.mbg.as_mgdl() as f32),
            date: entry.date.to_datetime(),
            is_isf: false,
        }
    }
}

impl TryFrom<Treatment> for GraphTreatment {
    type Error = MissingTimestamp;

    fn try_from(t: Treatment) -> Result<Self, Self::Error> {
        let date = t.time().ok_or(MissingTimestamp)?.to_datetime();
        // `glucose` is stored in the treatment's own units.
        let units = t
            .units
            .as_deref()
            .and_then(Units::parse)
            .unwrap_or_default();

        Ok(Self {
            insulin: t.insulin.map(|v| v as f32),
            carbs: t.carbs.map(|v| v as f32),
            mbg: t.glucose.map(|v| Glucose::new(v, units).as_mgdl() as f32),
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
    let mut iob = Vec::new();
    let mut cob = Vec::new();
    for status in statuses {
        let Some(date) = status.time().map(|t| t.to_datetime()) else {
            continue;
        };
        let openaps = status.openaps.as_ref();
        let suggested = openaps.and_then(|o| o.suggested.as_ref());
        let enacted = openaps.and_then(|o| o.enacted.as_ref());
        let loop_ = status.loop_status.as_ref();

        // oref0 uploads `iob` as a list of forecasts, the current one first.
        let iob_value = openaps
            .and_then(|o| o.iob_units())
            .or_else(|| suggested.and_then(|s| s.iob))
            .or_else(|| loop_.and_then(|l| l.iob.as_ref()).and_then(|v| v.iob));
        let cob_value = suggested
            .and_then(|s| s.cob)
            .or_else(|| enacted.and_then(|e| e.cob))
            .or_else(|| loop_.and_then(|l| l.cob.as_ref()).and_then(|v| v.cob));

        if let Some(value) = iob_value {
            iob.push(SeriesPoint {
                value: value as f32,
                date,
            });
        }
        if let Some(value) = cob_value {
            cob.push(SeriesPoint {
                value: value as f32,
                date,
            });
        }
    }
    (iob, cob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

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

    #[test]
    fn converts_entries_and_treatments() {
        let sgv: Sgv = serde_json::from_value(json!({
            "type": "sgv", "sgv": 120, "date": 1767261600000_i64
        }))
        .expect("sgv should deserialize");
        let entry = GraphEntry::from(sgv);
        assert_eq!(entry.sgv, 120.0);
        assert_eq!(entry.date.to_rfc3339(), "2026-01-01T10:00:00+00:00");

        // A mmol/L fingerprick lands on the graph in mg/dL.
        let check: Treatment = serde_json::from_value(json!({
            "eventType": "BG Check", "created_at": "2026-01-01T10:00:00Z",
            "glucose": 5.5, "units": "mmol"
        }))
        .expect("treatment should deserialize");
        let mbg = GraphTreatment::try_from(check).unwrap().mbg.unwrap();
        assert!((mbg - 99.1).abs() < 0.1);

        let undated: Treatment = serde_json::from_value(json!({ "insulin": 1.5 }))
            .expect("treatment should deserialize");
        assert_eq!(
            GraphTreatment::try_from(undated).unwrap_err(),
            MissingTimestamp
        );
    }
}
