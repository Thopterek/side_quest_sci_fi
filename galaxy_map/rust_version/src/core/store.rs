//! Persistence, as a trait.
//!
//! The vault does not know whether it is backed by PostgreSQL, by a JSON blob,
//! or by nothing at all. That matters for more than tidiness: the wasm build
//! cannot open a TCP connection to a database, so it needs a different backend
//! from the native build, and the tests want a third.

use serde::{Deserialize, Serialize};

use super::model::{PlanetRecord, Record, System};

/// The parts of vault state that outlive a session but are not systems.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub selected: Option<String>,
    pub compare: Option<String>,
    pub focus_planet: Option<String>,
}

/// Everything needed to reconstitute a vault.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub systems: Vec<System>,
    pub settings: Settings,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Connect(String),
    Migrate(String),
    Query(String),
    /// Someone else changed this record since it was read. Only the HTTP
    /// backend can raise it; the single-user backends have no one to conflict
    /// with.
    Conflict(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Connect(m) => write!(f, "could not reach the vault database: {m}"),
            StoreError::Migrate(m) => write!(f, "schema migration failed: {m}"),
            StoreError::Query(m) => write!(f, "vault query failed: {m}"),
            StoreError::Conflict(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type StoreResult<T> = Result<T, StoreError>;

/// A backend for the vault.
///
/// The split between [`upsert_system`](VaultStore::upsert_system) and
/// [`save_record`](VaultStore::save_record) is the whole design: the first
/// writes what NASA published, the second writes what the operator wrote, and
/// no implementation is permitted to let the first touch the second.
pub trait VaultStore: Send {
    /// Create or update the schema. Must be idempotent.
    fn migrate(&mut self) -> StoreResult<()>;

    fn load(&mut self) -> StoreResult<Snapshot>;

    /// Insert a system, or refresh only its archive columns and planets.
    /// An existing dossier must survive untouched.
    fn upsert_system(&mut self, sys: &System) -> StoreResult<()>;

    /// Insert a system *including* its dossier. Only for seeding and import,
    /// never for a refresh.
    fn insert_with_dossier(&mut self, sys: &System) -> StoreResult<()>;

    fn save_record(&mut self, system_id: &str, record: &Record) -> StoreResult<()>;

    fn save_planet_record(
        &mut self,
        system_id: &str,
        planet_name: &str,
        record: &PlanetRecord,
    ) -> StoreResult<()>;

    fn delete_system(&mut self, system_id: &str) -> StoreResult<()>;

    fn save_settings(&mut self, settings: &Settings) -> StoreResult<()>;

    /// Human-readable backend name, shown in the header.
    fn describe(&self) -> String;
}

/* ------------------------------------------------------------ in memory -- */

/// Backend for wasm, for `--no-default-features`, and for tests.
///
/// Mirrors the PostgreSQL semantics exactly, including the rule that
/// `upsert_system` never overwrites a dossier, so the same test suite can be
/// run against both and compared.
#[derive(Default)]
pub struct MemoryStore {
    systems: Vec<System>,
    settings: Settings,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seeded() -> Self {
        MemoryStore { systems: super::seed::seed_systems(), settings: Settings::default() }
    }
}

impl VaultStore for MemoryStore {
    fn migrate(&mut self) -> StoreResult<()> {
        Ok(())
    }

    fn load(&mut self) -> StoreResult<Snapshot> {
        Ok(Snapshot { systems: self.systems.clone(), settings: self.settings.clone() })
    }

    fn upsert_system(&mut self, sys: &System) -> StoreResult<()> {
        match self.systems.iter_mut().find(|s| s.id == sys.id) {
            Some(existing) => existing.merge_archive_from(sys.clone()),
            None => self.systems.push(sys.clone()),
        }
        Ok(())
    }

    fn insert_with_dossier(&mut self, sys: &System) -> StoreResult<()> {
        match self.systems.iter_mut().find(|s| s.id == sys.id) {
            Some(existing) => *existing = sys.clone(),
            None => self.systems.push(sys.clone()),
        }
        Ok(())
    }

    fn save_record(&mut self, system_id: &str, record: &Record) -> StoreResult<()> {
        if let Some(s) = self.systems.iter_mut().find(|s| s.id == system_id) {
            s.record = record.clone();
        }
        Ok(())
    }

    fn save_planet_record(
        &mut self,
        system_id: &str,
        planet_name: &str,
        record: &PlanetRecord,
    ) -> StoreResult<()> {
        if let Some(s) = self.systems.iter_mut().find(|s| s.id == system_id) {
            s.planet_records.insert(planet_name.to_string(), record.clone());
        }
        Ok(())
    }

    fn delete_system(&mut self, system_id: &str) -> StoreResult<()> {
        self.systems.retain(|s| s.id != system_id);
        if self.settings.selected.as_deref() == Some(system_id) {
            self.settings.selected = None;
        }
        if self.settings.compare.as_deref() == Some(system_id) {
            self.settings.compare = None;
        }
        Ok(())
    }

    fn save_settings(&mut self, settings: &Settings) -> StoreResult<()> {
        self.settings = settings.clone();
        Ok(())
    }

    fn describe(&self) -> String {
        format!("in memory ({} systems)", self.systems.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Arm;

    #[test]
    fn memory_upsert_preserves_the_dossier_like_postgres_does() {
        let mut store = MemoryStore::seeded();
        store
            .save_record(
                "gj-1061",
                &Record {
                    imperial_name: "Kestrel Reach".into(),
                    arm: Some(Arm::Perseus),
                    population: "4.1 billion".into(),
                    notes: "#capital".into(),
                },
            )
            .unwrap();

        let mut fresh = crate::core::seed::seed_systems()
            .into_iter()
            .find(|s| s.id == "gj-1061")
            .unwrap();
        fresh.dist_pc = Some(3.999);
        fresh.record = Record::default(); // as a fresh archive pull would be
        store.upsert_system(&fresh).unwrap();

        let snap = store.load().unwrap();
        let s = snap.systems.iter().find(|s| s.id == "gj-1061").unwrap();
        assert_eq!(s.dist_pc, Some(3.999), "archive must update");
        assert_eq!(s.record.imperial_name, "Kestrel Reach", "dossier must survive");
        assert_eq!(s.record.arm, Some(Arm::Perseus));
    }

    #[test]
    fn deleting_a_system_clears_it_from_settings() {
        let mut store = MemoryStore::seeded();
        store
            .save_settings(&Settings {
                selected: Some("gj-1061".into()),
                compare: Some("gj-1061".into()),
                focus_planet: None,
            })
            .unwrap();
        store.delete_system("gj-1061").unwrap();
        let snap = store.load().unwrap();
        assert!(snap.settings.selected.is_none());
        assert!(snap.settings.compare.is_none());
    }
}
