//! Partial edits.
//!
//! Shared by the server and the face, which is why they live in `core` rather
//! than behind either feature: the client computes a patch and the server
//! applies it, and if the two disagreed about the wire format the concurrency
//! guarantees would be fiction.
//!
//! Every field is `Option`: absent means "leave alone", present means "set to
//! this". `arm` is doubly wrapped because `None` is itself a meaningful value —
//! absent leaves the arm alone, `null` clears it. Getting that distinction
//! wrong is how a partial update quietly becomes a whole-row overwrite again.

use serde::{Deserialize, Deserializer, Serialize};

use super::model::{Arm, PlanetRecord, Record};

/// Deserialize a doubly-optional field so that `null` survives as `Some(None)`.
///
/// Serde collapses `null` into `None` for any `Option<T>`, including the outer
/// one — which would make an explicit "clear this field" indistinguishable from
/// "field absent", quietly turning a clear into a no-op. Wrapping the result in
/// `Some` restores the distinction: absent never reaches this function at all,
/// because `#[serde(default)]` handles it.
fn present_even_if_null<'de, T, D>(de: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(de).map(Some)
}

/// A partial edit to a system's dossier.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RecordPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imperial_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_even_if_null"
    )]
    pub arm: Option<Option<Arm>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub population: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Version the client last read. `None` skips the check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<i32>,
}

impl RecordPatch {
    /// The diff between what the client last saw and what it has now.
    pub fn between(before: &Record, after: &Record) -> Self {
        RecordPatch {
            imperial_name: (before.imperial_name != after.imperial_name)
                .then(|| after.imperial_name.clone()),
            arm: (before.arm != after.arm).then_some(after.arm),
            population: (before.population != after.population)
                .then(|| after.population.clone()),
            notes: (before.notes != after.notes).then(|| after.notes.clone()),
            expected_version: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.imperial_name.is_none()
            && self.arm.is_none()
            && self.population.is_none()
            && self.notes.is_none()
    }

    /// Apply locally, so the client can keep its shadow copy in step.
    pub fn apply_to(&self, record: &mut Record) {
        if let Some(v) = &self.imperial_name {
            record.imperial_name = v.clone();
        }
        if let Some(v) = self.arm {
            record.arm = v;
        }
        if let Some(v) = &self.population {
            record.population = v.clone();
        }
        if let Some(v) = &self.notes {
            record.notes = v.clone();
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PlanetRecordPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imperial_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub population: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continents: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<i32>,
}

impl PlanetRecordPatch {
    pub fn between(before: &PlanetRecord, after: &PlanetRecord) -> Self {
        PlanetRecordPatch {
            imperial_name: (before.imperial_name != after.imperial_name)
                .then(|| after.imperial_name.clone()),
            population: (before.population != after.population)
                .then(|| after.population.clone()),
            continents: (before.continents != after.continents)
                .then(|| after.continents.clone()),
            notes: (before.notes != after.notes).then(|| after.notes.clone()),
            expected_version: None,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.imperial_name.is_none()
            && self.population.is_none()
            && self.continents.is_none()
            && self.notes.is_none()
    }
    pub fn apply_to(&self, record: &mut PlanetRecord) {
        if let Some(v) = &self.imperial_name {
            record.imperial_name = v.clone();
        }
        if let Some(v) = &self.population {
            record.population = v.clone();
        }
        if let Some(v) = &self.continents {
            record.continents = v.clone();
        }
        if let Some(v) = &self.notes {
            record.notes = v.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_carries_only_what_changed() {
        let before = Record {
            imperial_name: "Kestrel Reach".into(),
            arm: Some(Arm::Perseus),
            population: "4.1 billion".into(),
            notes: "#capital".into(),
        };
        let mut after = before.clone();
        after.population = "4.2 billion".into();

        let patch = RecordPatch::between(&before, &after);
        assert!(patch.imperial_name.is_none(), "an untouched field must not travel");
        assert!(patch.arm.is_none());
        assert!(patch.notes.is_none());
        assert_eq!(patch.population.as_deref(), Some("4.2 billion"));
        assert!(!patch.is_empty());
    }

    #[test]
    fn an_unchanged_record_produces_no_patch_at_all() {
        let r = Record { notes: "same".into(), ..Default::default() };
        assert!(RecordPatch::between(&r, &r).is_empty());
    }

    #[test]
    fn clearing_the_arm_is_distinguishable_from_leaving_it_alone() {
        let before = Record { arm: Some(Arm::Perseus), ..Default::default() };
        let cleared = Record { arm: None, ..Default::default() };
        // Absent means leave alone; Some(None) means set to null.
        assert_eq!(RecordPatch::between(&before, &before).arm, None);
        assert_eq!(RecordPatch::between(&before, &cleared).arm, Some(None));
    }

    #[test]
    fn patches_round_trip_through_json_preserving_absent_versus_null() {
        let patch = RecordPatch {
            arm: Some(None),
            population: Some("none".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&patch).unwrap();
        assert!(!json.contains("imperial_name"), "absent fields must not serialise");
        assert!(json.contains("\"arm\":null"), "an explicit clear must survive: {json}");

        let back: RecordPatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.arm, Some(None));
        assert!(back.imperial_name.is_none());
        assert!(back.notes.is_none());
    }

    #[test]
    fn applying_a_patch_locally_matches_what_the_server_will_store() {
        let mut record = Record {
            imperial_name: "Kestrel Reach".into(),
            arm: Some(Arm::Perseus),
            population: "4.1 billion".into(),
            notes: "#capital".into(),
        };
        let patch = RecordPatch {
            population: Some("4.2 billion".into()),
            arm: Some(None),
            ..Default::default()
        };
        patch.apply_to(&mut record);
        assert_eq!(record.population, "4.2 billion");
        assert_eq!(record.arm, None);
        assert_eq!(record.imperial_name, "Kestrel Reach", "untouched fields stand");
        assert_eq!(record.notes, "#capital");
    }

    #[test]
    fn planet_patches_behave_the_same_way() {
        let before = PlanetRecord {
            imperial_name: "Anvil".into(),
            continents: "North, South".into(),
            ..Default::default()
        };
        let mut after = before.clone();
        after.continents = "North, South, Verge".into();
        let patch = PlanetRecordPatch::between(&before, &after);
        assert!(patch.imperial_name.is_none());
        assert_eq!(patch.continents.as_deref(), Some("North, South, Verge"));

        let mut target = before.clone();
        patch.apply_to(&mut target);
        assert_eq!(target, after);
    }
}
