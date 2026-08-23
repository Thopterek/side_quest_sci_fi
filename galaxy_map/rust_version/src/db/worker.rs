//! The store, on its own thread.
//!
//! Two problems this solves.
//!
//! **Blocking.** A Postgres round trip is tens of milliseconds. Doing that on
//! the UI thread would drop frames every time a system is saved. The worker
//! owns the connection; the UI sends requests and picks up results next frame.
//!
//! **Write amplification.** Dossier fields are bound directly to text boxes, so
//! a naive implementation issues an UPDATE per keystroke. The worker coalesces
//! instead: dirty records are held in a map keyed by target and flushed once the
//! operator stops typing, so a sentence becomes one write rather than forty.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryIter};
use std::sync::mpsc::{self};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::model::{PlanetRecord, Record, System};
use crate::core::store::{Settings, Snapshot, StoreResult, VaultStore};

/// How long to wait for typing to stop before flushing a dossier edit.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// Upper bound on how long an edit can sit unwritten while typing continues.
const MAX_DEFER: Duration = Duration::from_millis(2000);

pub enum StoreRequest {
    Load,
    /// Refresh: archive columns only.
    UpsertSystem(Box<System>),
    /// Seed or import: dossier travels with it.
    InsertWithDossier(Box<System>),
    SaveRecord { system_id: String, record: Record },
    SavePlanetRecord { system_id: String, planet_name: String, record: PlanetRecord },
    DeleteSystem(String),
    SaveSettings(Settings),
    /// Write everything pending immediately. Sent on shutdown.
    Flush,
    Shutdown,
}

pub enum StoreUpdate {
    Loaded(Box<Snapshot>),
    /// A write reached the database. Carries the number coalesced into it.
    Committed { writes: usize },
    Error(String),
}

/// UI-side handle. Cloneable, non-blocking.
pub struct StoreHandle {
    tx: Sender<StoreRequest>,
    rx: Receiver<StoreUpdate>,
    label: String,
}

impl StoreHandle {
    /// Move `store` onto a worker thread and return a handle to it.
    ///
    /// `wake` is called whenever an update is ready, so an idle UI repaints.
    pub fn spawn<S: VaultStore + 'static>(
        mut store: S,
        wake: impl Fn() + Send + 'static,
    ) -> StoreHandle {
        let label = store.describe();
        let (req_tx, req_rx) = mpsc::channel::<StoreRequest>();
        let (up_tx, up_rx) = mpsc::channel::<StoreUpdate>();

        thread::spawn(move || {
            let mut pending = Pending::default();
            if let Err(e) = store.migrate() {
                let _ = up_tx.send(StoreUpdate::Error(e.to_string()));
                wake();
            }
            loop {
                // Block until there is work, or until a deferred write is due.
                let timeout = pending.time_until_flush().unwrap_or(Duration::from_millis(500));
                match req_rx.recv_timeout(timeout) {
                    Ok(req) => {
                        if matches!(req, StoreRequest::Shutdown) {
                            pending.flush(&mut store, &up_tx, &wake);
                            return;
                        }
                        let stop = handle(req, &mut store, &mut pending, &up_tx, &wake);
                        // Drain anything else already queued before touching the
                        // database, so a burst collapses into one flush.
                        let drained: Vec<StoreRequest> = req_rx.try_iter().collect();
                        for r in drained {
                            if matches!(r, StoreRequest::Shutdown) {
                                pending.flush(&mut store, &up_tx, &wake);
                                return;
                            }
                            handle(r, &mut store, &mut pending, &up_tx, &wake);
                        }
                        if stop {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if pending.time_until_flush() == Some(Duration::ZERO) {
                            pending.flush(&mut store, &up_tx, &wake);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        pending.flush(&mut store, &up_tx, &wake);
                        return;
                    }
                }
            }
        });

        StoreHandle { tx: req_tx, rx: up_rx, label }
    }

    pub fn send(&self, req: StoreRequest) {
        let _ = self.tx.send(req);
    }

    /// A sender that can be moved onto another thread, for callers that need to
    /// post work from outside the UI loop — the SSE listener, chiefly.
    pub fn sender(&self) -> Sender<StoreRequest> {
        self.tx.clone()
    }

    /// Non-blocking. Call once per frame.
    pub fn updates(&self) -> TryIter<'_, StoreUpdate> {
        self.rx.try_iter()
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

impl Drop for StoreHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(StoreRequest::Shutdown);
    }
}

/* ------------------------------------------------------------- pending -- */

/// Edits waiting to be written, keyed so repeats overwrite instead of queueing.
#[derive(Default)]
struct Pending {
    records: BTreeMap<String, Record>,
    planet_records: BTreeMap<(String, String), PlanetRecord>,
    settings: Option<Settings>,
    /// When the oldest unwritten edit arrived.
    oldest: Option<Instant>,
    /// When the most recent edit arrived.
    newest: Option<Instant>,
    /// How many individual edits have been folded into this batch.
    coalesced: usize,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.records.is_empty() && self.planet_records.is_empty() && self.settings.is_none()
    }

    fn touch(&mut self) {
        let now = Instant::now();
        self.oldest.get_or_insert(now);
        self.newest = Some(now);
        self.coalesced += 1;
    }

    /// `Some(ZERO)` when a flush is due now, `Some(d)` to wait, `None` if idle.
    fn time_until_flush(&self) -> Option<Duration> {
        if self.is_empty() {
            return None;
        }
        let (Some(oldest), Some(newest)) = (self.oldest, self.newest) else {
            return Some(Duration::ZERO);
        };
        let quiet_for = newest.elapsed();
        let waited = oldest.elapsed();
        if quiet_for >= DEBOUNCE || waited >= MAX_DEFER {
            Some(Duration::ZERO)
        } else {
            // Wake at whichever deadline comes first.
            let until_quiet = DEBOUNCE.saturating_sub(quiet_for);
            let until_max = MAX_DEFER.saturating_sub(waited);
            Some(until_quiet.min(until_max))
        }
    }

    fn flush<S: VaultStore>(
        &mut self,
        store: &mut S,
        up: &Sender<StoreUpdate>,
        wake: &impl Fn(),
    ) {
        if self.is_empty() {
            return;
        }
        let writes = self.coalesced;
        let mut result: StoreResult<()> = Ok(());

        for (id, rec) in std::mem::take(&mut self.records) {
            if result.is_ok() {
                result = store.save_record(&id, &rec);
            }
        }
        for ((id, planet), rec) in std::mem::take(&mut self.planet_records) {
            if result.is_ok() {
                result = store.save_planet_record(&id, &planet, &rec);
            }
        }
        if let Some(s) = self.settings.take() {
            if result.is_ok() {
                result = store.save_settings(&s);
            }
        }
        self.oldest = None;
        self.newest = None;
        self.coalesced = 0;

        let _ = match result {
            Ok(()) => up.send(StoreUpdate::Committed { writes }),
            Err(e) => up.send(StoreUpdate::Error(e.to_string())),
        };
        wake();
    }
}

/// Returns true if the worker should stop.
fn handle<S: VaultStore>(
    req: StoreRequest,
    store: &mut S,
    pending: &mut Pending,
    up: &Sender<StoreUpdate>,
    wake: &impl Fn(),
) -> bool {
    let immediate: StoreResult<()> = match req {
        StoreRequest::Load => match store.load() {
            Ok(snap) => {
                let _ = up.send(StoreUpdate::Loaded(Box::new(snap)));
                wake();
                return false;
            }
            Err(e) => Err(e),
        },

        // Structural changes are written straight through: they are rare, and
        // deferring them would let the UI and the database disagree about which
        // systems exist.
        StoreRequest::UpsertSystem(sys) => store.upsert_system(&sys),
        StoreRequest::InsertWithDossier(sys) => store.insert_with_dossier(&sys),
        StoreRequest::DeleteSystem(id) => {
            pending.records.remove(&id);
            pending.planet_records.retain(|(sid, _), _| sid != &id);
            store.delete_system(&id)
        }

        // Field edits are coalesced.
        StoreRequest::SaveRecord { system_id, record } => {
            pending.records.insert(system_id, record);
            pending.touch();
            return false;
        }
        StoreRequest::SavePlanetRecord { system_id, planet_name, record } => {
            pending.planet_records.insert((system_id, planet_name), record);
            pending.touch();
            return false;
        }
        StoreRequest::SaveSettings(s) => {
            pending.settings = Some(s);
            pending.touch();
            return false;
        }

        StoreRequest::Flush => {
            pending.flush(store, up, wake);
            return false;
        }
        StoreRequest::Shutdown => return true,
    };

    let _ = match immediate {
        Ok(()) => up.send(StoreUpdate::Committed { writes: 1 }),
        Err(e) => up.send(StoreUpdate::Error(e.to_string())),
    };
    wake();
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Counts how many times each store method is actually invoked, so the
    /// coalescing can be measured rather than assumed.
    #[derive(Default)]
    struct CountingStore {
        inner: MemoryStore,
        record_writes: Arc<AtomicUsize>,
    }

    impl VaultStore for CountingStore {
        fn migrate(&mut self) -> StoreResult<()> {
            Ok(())
        }
        fn load(&mut self) -> StoreResult<Snapshot> {
            self.inner.load()
        }
        fn upsert_system(&mut self, s: &System) -> StoreResult<()> {
            self.inner.upsert_system(s)
        }
        fn insert_with_dossier(&mut self, s: &System) -> StoreResult<()> {
            self.inner.insert_with_dossier(s)
        }
        fn save_record(&mut self, id: &str, r: &Record) -> StoreResult<()> {
            self.record_writes.fetch_add(1, Ordering::SeqCst);
            self.inner.save_record(id, r)
        }
        fn save_planet_record(
            &mut self,
            id: &str,
            p: &str,
            r: &PlanetRecord,
        ) -> StoreResult<()> {
            self.inner.save_planet_record(id, p, r)
        }
        fn delete_system(&mut self, id: &str) -> StoreResult<()> {
            self.inner.delete_system(id)
        }
        fn save_settings(&mut self, s: &Settings) -> StoreResult<()> {
            self.inner.save_settings(s)
        }
        fn describe(&self) -> String {
            "counting".into()
        }
    }

    #[test]
    fn a_burst_of_keystrokes_becomes_one_write() {
        let writes = Arc::new(AtomicUsize::new(0));
        let store = CountingStore {
            inner: MemoryStore::seeded(),
            record_writes: writes.clone(),
        };
        let handle = StoreHandle::spawn(store, || {});

        // Forty keystrokes, as fast as typing.
        for i in 0..40 {
            handle.send(StoreRequest::SaveRecord {
                system_id: "gj-1061".into(),
                record: Record { notes: "x".repeat(i + 1), ..Default::default() },
            });
            thread::sleep(Duration::from_millis(5));
        }
        // Let the debounce expire.
        thread::sleep(DEBOUNCE + Duration::from_millis(250));

        let n = writes.load(Ordering::SeqCst);
        assert!(n >= 1, "the edit must eventually be written");
        assert!(n <= 2, "40 keystrokes should collapse to about one write, got {n}");
    }

    #[test]
    fn the_last_value_typed_is_the_one_stored() {
        let store = CountingStore { inner: MemoryStore::seeded(), ..Default::default() };
        let handle = StoreHandle::spawn(store, || {});
        for text in ["K", "Ke", "Kes", "Kestrel Reach"] {
            handle.send(StoreRequest::SaveRecord {
                system_id: "gj-1061".into(),
                record: Record { imperial_name: text.into(), ..Default::default() },
            });
        }
        thread::sleep(DEBOUNCE + Duration::from_millis(250));
        handle.send(StoreRequest::Load);
        thread::sleep(Duration::from_millis(150));

        let mut found = None;
        for up in handle.updates() {
            if let StoreUpdate::Loaded(snap) = up {
                found = snap
                    .systems
                    .iter()
                    .find(|s| s.id == "gj-1061")
                    .map(|s| s.record.imperial_name.clone());
            }
        }
        assert_eq!(found.as_deref(), Some("Kestrel Reach"));
    }

    #[test]
    fn continuous_typing_still_gets_written_within_the_deferral_ceiling() {
        let writes = Arc::new(AtomicUsize::new(0));
        let store = CountingStore {
            inner: MemoryStore::seeded(),
            record_writes: writes.clone(),
        };
        let handle = StoreHandle::spawn(store, || {});
        // Never quiet for a full DEBOUNCE, for longer than MAX_DEFER.
        let start = Instant::now();
        while start.elapsed() < MAX_DEFER + Duration::from_millis(400) {
            handle.send(StoreRequest::SaveRecord {
                system_id: "gj-1061".into(),
                record: Record { notes: "typing".into(), ..Default::default() },
            });
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            writes.load(Ordering::SeqCst) >= 1,
            "an edit must not be deferred forever by continued typing"
        );
    }

    #[test]
    fn deleting_a_system_discards_its_queued_edits() {
        let store = CountingStore { inner: MemoryStore::seeded(), ..Default::default() };
        let handle = StoreHandle::spawn(store, || {});
        handle.send(StoreRequest::SaveRecord {
            system_id: "gj-1061".into(),
            record: Record { imperial_name: "doomed".into(), ..Default::default() },
        });
        handle.send(StoreRequest::DeleteSystem("gj-1061".into()));
        thread::sleep(DEBOUNCE + Duration::from_millis(250));
        handle.send(StoreRequest::Load);
        thread::sleep(Duration::from_millis(150));

        for up in handle.updates() {
            if let StoreUpdate::Loaded(snap) = up {
                assert!(snap.systems.iter().all(|s| s.id != "gj-1061"));
            }
        }
    }
}
