//! The face's storage backend: HTTP to `parallax-server`.
//!
//! Implements the same [`VaultStore`] trait as the direct-PostgreSQL backend, so
//! the existing worker thread, its write coalescing and the whole UI are
//! unchanged. What differs is what goes over the wire.
//!
//! The store keeps a **shadow copy** of every record as the server last
//! confirmed it. On save it sends the difference between the shadow and the
//! current value rather than the whole record. That is what makes concurrent
//! editing safe: two operators changing different fields of one system no longer
//! carry each other's stale values, because neither mentions the field they did
//! not touch.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::core::model::{PlanetRecord, Record, System};
use crate::core::store::{Settings, Snapshot, StoreError, StoreResult, VaultStore};

/// What the server returns for a write.
#[derive(Deserialize)]
struct VersionResponse {
    version: i32,
}

#[derive(Deserialize)]
struct ApiError {
    error: String,
    #[serde(default)]
    current_version: Option<i32>,
}

#[derive(Deserialize)]
struct VersionedSystem {
    #[serde(flatten)]
    system: System,
    version: i32,
}

#[derive(Deserialize)]
struct VaultResponse {
    systems: Vec<VersionedSystem>,
    settings: Settings,
}

pub struct HttpStore {
    base: String,
    user: String,
    agent: ureq::Agent,
    /// Records as the server last confirmed them, for diffing.
    shadow: BTreeMap<String, Record>,
    planet_shadow: BTreeMap<(String, String), PlanetRecord>,
    /// Versions as last seen, quoted on writes so a conflict is detectable.
    versions: BTreeMap<String, i32>,
}

impl HttpStore {
    pub fn new(base_url: &str, user: &str) -> Self {
        HttpStore {
            base: base_url.trim_end_matches('/').to_string(),
            user: user.to_string(),
            // A GUI must not stall on a slow server; the worker thread will
            // surface a timeout as an error the UI can show.
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .build(),
            shadow: BTreeMap::new(),
            planet_shadow: BTreeMap::new(),
            versions: BTreeMap::new(),
        }
    }

    pub fn from_env() -> Self {
        let base = std::env::var("PARALLAX_SERVER_URL")
            .unwrap_or_else(|_| "http://localhost:8080".into());
        let user = std::env::var("PARALLAX_USER").unwrap_or_else(|_| {
            std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "local".into())
        });
        HttpStore::new(&base, &user)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Turn a ureq failure into a `StoreError`, preserving a version conflict as
    /// something distinct from a transport problem.
    fn map_err(err: ureq::Error) -> StoreError {
        match err {
            ureq::Error::Status(code, response) => {
                let parsed: Option<ApiError> = response.into_json().ok();
                let message = match parsed.as_ref() {
                    Some(e) => match e.current_version {
                        // Naming the live version makes a conflict actionable
                        // rather than merely alarming.
                        Some(v) => format!("{} (server is at version {v})", e.error),
                        None => e.error.clone(),
                    },
                    None => format!("server returned {code}"),
                };
                match code {
                    409 => StoreError::Conflict(message),
                    404 => StoreError::Query(format!("not found: {message}")),
                    503 => StoreError::Connect(message),
                    _ => StoreError::Query(message),
                }
            }
            ureq::Error::Transport(t) => StoreError::Connect(t.to_string()),
        }
    }
}

impl VaultStore for HttpStore {
    /// The server owns migration. Asking a client to migrate would mean several
    /// clients racing to do it on startup.
    fn migrate(&mut self) -> StoreResult<()> {
        self.agent
            .get(&self.url("/health"))
            .call()
            .map_err(Self::map_err)?;
        Ok(())
    }

    fn load(&mut self) -> StoreResult<Snapshot> {
        let body: VaultResponse = self
            .agent
            .get(&self.url("/vault"))
            .set("X-Parallax-User", &self.user)
            .call()
            .map_err(Self::map_err)?
            .into_json()
            .map_err(|e| StoreError::Query(e.to_string()))?;

        // Reset the shadow to what the server just told us. Anything a local
        // edit had pending is superseded, which is the correct outcome for a
        // reload.
        self.shadow.clear();
        self.planet_shadow.clear();
        self.versions.clear();

        let mut systems = Vec::with_capacity(body.systems.len());
        for entry in body.systems {
            self.shadow.insert(entry.system.id.clone(), entry.system.record.clone());
            self.versions.insert(entry.system.id.clone(), entry.version);
            for (name, rec) in &entry.system.planet_records {
                self.planet_shadow
                    .insert((entry.system.id.clone(), name.clone()), rec.clone());
            }
            systems.push(entry.system);
        }
        Ok(Snapshot { systems, settings: body.settings })
    }

    fn upsert_system(&mut self, sys: &System) -> StoreResult<()> {
        let response: VersionResponse = self
            .agent
            .put(&self.url(&format!("/systems/{}", sys.id)))
            .set("X-Parallax-User", &self.user)
            .send_json(serde_json::to_value(sys).map_err(|e| StoreError::Query(e.to_string()))?)
            .map_err(Self::map_err)?
            .into_json()
            .map_err(|e| StoreError::Query(e.to_string()))?;
        self.versions.insert(sys.id.clone(), response.version);
        self.shadow.entry(sys.id.clone()).or_default();
        Ok(())
    }

    /// Seeding is a server operation, for the same reason migration is.
    fn insert_with_dossier(&mut self, sys: &System) -> StoreResult<()> {
        self.upsert_system(sys)?;
        self.save_record(&sys.id.clone(), &sys.record.clone())?;
        for (name, rec) in sys.planet_records.clone() {
            self.save_planet_record(&sys.id, &name, &rec)?;
        }
        Ok(())
    }

    fn save_record(&mut self, system_id: &str, record: &Record) -> StoreResult<()> {
        let before = self.shadow.get(system_id).cloned().unwrap_or_default();
        let mut patch = crate::core::patch::RecordPatch::between(&before, record);
        if patch.is_empty() {
            return Ok(());
        }
        patch.expected_version = self.versions.get(system_id).copied();

        let result = self
            .agent
            .patch(&self.url(&format!("/systems/{system_id}/record")))
            .set("X-Parallax-User", &self.user)
            .send_json(
                serde_json::to_value(&patch).map_err(|e| StoreError::Query(e.to_string()))?,
            );

        match result {
            Ok(response) => {
                let v: VersionResponse = response
                    .into_json()
                    .map_err(|e| StoreError::Query(e.to_string()))?;
                self.versions.insert(system_id.to_string(), v.version);
                self.shadow.insert(system_id.to_string(), record.clone());
                Ok(())
            }
            Err(e) => {
                let mapped = Self::map_err(e);
                // On a conflict the shadow is stale by definition. Forgetting the
                // version means the retry is unconditional rather than looping on
                // a check that can never pass.
                if matches!(mapped, StoreError::Conflict(_)) {
                    self.versions.remove(system_id);
                }
                Err(mapped)
            }
        }
    }

    fn save_planet_record(
        &mut self,
        system_id: &str,
        planet_name: &str,
        record: &PlanetRecord,
    ) -> StoreResult<()> {
        let key = (system_id.to_string(), planet_name.to_string());
        let before = self.planet_shadow.get(&key).cloned().unwrap_or_default();
        let patch = crate::core::patch::PlanetRecordPatch::between(&before, record);
        if patch.is_empty() {
            return Ok(());
        }

        let encoded =
            urlencode(planet_name);
        self.agent
            .patch(&self.url(&format!("/systems/{system_id}/planets/{encoded}/record")))
            .set("X-Parallax-User", &self.user)
            .send_json(
                serde_json::to_value(&patch).map_err(|e| StoreError::Query(e.to_string()))?,
            )
            .map_err(Self::map_err)?;
        self.planet_shadow.insert(key, record.clone());
        Ok(())
    }

    fn delete_system(&mut self, system_id: &str) -> StoreResult<()> {
        self.agent
            .delete(&self.url(&format!("/systems/{system_id}")))
            .set("X-Parallax-User", &self.user)
            .call()
            .map_err(Self::map_err)?;
        self.shadow.remove(system_id);
        self.versions.remove(system_id);
        self.planet_shadow.retain(|(sid, _), _| sid != system_id);
        Ok(())
    }

    fn save_settings(&mut self, settings: &Settings) -> StoreResult<()> {
        self.agent
            .put(&self.url("/settings"))
            .set("X-Parallax-User", &self.user)
            .send_json(
                serde_json::to_value(settings).map_err(|e| StoreError::Query(e.to_string()))?,
            )
            .map_err(Self::map_err)?;
        Ok(())
    }

    fn describe(&self) -> String {
        format!("{} as {}", self.base, self.user)
    }
}

/// Percent-encode a path segment. Planet names contain spaces.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planet_names_survive_the_url() {
        assert_eq!(urlencode("GJ 1061 d"), "GJ%201061%20d");
        assert_eq!(urlencode("Proxima Cen b"), "Proxima%20Cen%20b");
        assert_eq!(urlencode("Earth"), "Earth");
    }

    #[test]
    fn the_base_url_is_normalised() {
        let s = HttpStore::new("http://example.test:8080/", "alice");
        assert_eq!(s.url("/vault"), "http://example.test:8080/vault");
    }

    #[test]
    fn describe_names_the_server_and_the_user_but_carries_no_secret() {
        let s = HttpStore::new("http://example.test:8080", "alice");
        let d = s.describe();
        assert!(d.contains("example.test"));
        assert!(d.contains("alice"));
        assert!(!d.contains("password"));
    }

    #[test]
    fn an_unchanged_record_sends_nothing() {
        // The worker calls save_record on any revision bump, including ones that
        // did not actually alter a dossier field. Diffing against the shadow is
        // what keeps that from becoming a request per frame.
        let mut store = HttpStore::new("http://127.0.0.1:1", "alice");
        let record = Record { notes: "same".into(), ..Default::default() };
        store.shadow.insert("gj-1061".into(), record.clone());
        // No server is listening on port 1; if this tried to send, it would fail.
        assert!(store.save_record("gj-1061", &record).is_ok());
    }
}
