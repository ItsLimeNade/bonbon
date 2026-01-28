use crate::models::{GraphEntry, GraphTreatment};
use chrono::{DateTime, Utc};
use cinnamon::models::{
    entries::{MbgEntry, SgvEntry},
    treatments::Treatment,
};

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
