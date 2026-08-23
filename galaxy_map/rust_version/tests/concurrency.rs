//! Concurrent-operator behaviour, against a real PostgreSQL server.
//!
//! ```sh
//! PARALLAX_TEST_DATABASE_URL="host=localhost user=postgres dbname=parallax" \
//!   cargo test --features server,db --test concurrency -- --test-threads=1
//! ```
//!
//! Each test names a failure that the single-user design actually had, and the
//! first one demonstrates it still exists on the old code path — so the fix is
//! measured against something real rather than asserted against nothing.

#![cfg(all(feature = "server", feature = "db"))]

use std::sync::Arc;

use parallax::core::model::{Arm, PlanetRecord, Record};
use parallax::core::patch::{PlanetRecordPatch, RecordPatch};
use parallax::core::store::{Settings, VaultStore};
use parallax::db::PgStore;
use parallax::core::grant::{Capability, Scope};
use parallax::server::{Repo, RepoError};

fn url() -> Option<String> {
    std::env::var("PARALLAX_TEST_DATABASE_URL").ok()
}

/// A migrated, empty vault with the seed catalog loaded.
///
/// All async: the sync `postgres` driver starts a runtime of its own, which
/// cannot nest inside `#[tokio::test]`.
async fn fresh() -> Option<(Repo, String)> {
    let url = url()?;
    let repo = Repo::connect(&url, 16).expect("pool");
    repo.migrate().await.expect("migrate");
    repo.truncate_all().await.expect("truncate");
    repo.seed(true).await.expect("seed");

    // Reads are scoped to a grant, so the operators in these tests need one.
    // Each stands for a person holding a share link over the whole vault —
    // these tests are about edits colliding, not about what a stage contains.
    for who in ["alice", "bob", "carol"] {
        repo.upsert_grant_for_test(who, Capability::Admin, &Scope::All)
            .await
            .expect("grant");
    }
    Some((repo, url))
}

macro_rules! db_test {
    ($name:ident, |$repo:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some(($repo, _url)) = fresh().await else {
                eprintln!("skipped: PARALLAX_TEST_DATABASE_URL not set");
                return;
            };
            $body
        }
    };
}

/* ------------------------------------------------- the bug, demonstrated -- */

/// Not a `#[tokio::test]`: this one deliberately uses the *sync* desktop store,
/// whose driver builds its own runtime and cannot be nested inside one.
#[test]
fn whole_row_writes_lose_a_concurrent_edit() {
    // Documents why the server exists. Two operators on one system, editing
    // different fields, through the original desktop write path.
    let Some(url) = url() else { return };
    let mut alice = PgStore::connect(&url).unwrap();
    let mut bob = PgStore::connect(&url).unwrap();
    alice.migrate().unwrap();
    alice.truncate_all().unwrap();
    for s in parallax::core::seed::seed_systems() {
        alice.insert_with_dossier(&s).unwrap();
    }

    let base = alice
        .load()
        .unwrap()
        .systems
        .into_iter()
        .find(|s| s.id == "gj-1061")
        .unwrap()
        .record;

    let mut a = base.clone();
    a.imperial_name = "Kestrel Reach".into();
    alice.save_record("gj-1061", &a).unwrap();

    // Bob writes from the snapshot he took before Alice's edit.
    let mut b = base.clone();
    b.population = "4.1 billion".into();
    bob.save_record("gj-1061", &b).unwrap();

    let after = bob
        .load()
        .unwrap()
        .systems
        .into_iter()
        .find(|s| s.id == "gj-1061")
        .unwrap()
        .record;

    assert_eq!(after.population, "4.1 billion", "Bob's edit lands");
    assert_ne!(
        after.imperial_name, "Kestrel Reach",
        "and Alice's is silently gone — this is the bug the server fixes"
    );
}

/* ------------------------------------------------------------ the fix -- */

db_test!(field_level_writes_do_not_lose_a_concurrent_edit, |repo| {
    // The same scenario through the server's patch path.
    let base = repo.snapshot("alice").await.unwrap();
    let before = base
        .systems
        .iter()
        .find(|s| s.system.id == "gj-1061")
        .unwrap();
    let version = before.version;

    // Alice renames. Bob sets the population, from the same starting version.
    let mut after_alice = before.system.record.clone();
    after_alice.imperial_name = "Kestrel Reach".into();
    let alice_patch = RecordPatch::between(&before.system.record, &after_alice);

    let mut after_bob = before.system.record.clone();
    after_bob.population = "4.1 billion".into();
    let bob_patch = RecordPatch::between(&before.system.record, &after_bob);

    // Neither patch mentions the other's field, so neither carries a stale value.
    assert!(alice_patch.population.is_none());
    assert!(bob_patch.imperial_name.is_none());

    repo.patch_record("gj-1061", &alice_patch).await.unwrap();
    repo.patch_record("gj-1061", &bob_patch).await.unwrap();

    let after = repo.snapshot("alice").await.unwrap();
    let record = &after
        .systems
        .iter()
        .find(|s| s.system.id == "gj-1061")
        .unwrap()
        .system
        .record;

    assert_eq!(record.imperial_name, "Kestrel Reach", "Alice's edit survives");
    assert_eq!(record.population, "4.1 billion", "and so does Bob's");
    // Version advanced twice, so the history is visible.
    let now = after
        .systems
        .iter()
        .find(|s| s.system.id == "gj-1061")
        .unwrap()
        .version;
    assert_eq!(now, version + 2);
});

db_test!(editing_the_same_field_is_reported_as_a_conflict, |repo| {
    // Different fields merge. The same field genuinely cannot, so the second
    // writer is told rather than allowed to overwrite blindly.
    let snap = repo.snapshot("alice").await.unwrap();
    let entry = snap.systems.iter().find(|s| s.system.id == "gj-1061").unwrap();
    let stale_version = entry.version;

    let first = RecordPatch {
        notes: Some("Alice was here".into()),
        expected_version: Some(stale_version),
        ..Default::default()
    };
    repo.patch_record("gj-1061", &first).await.unwrap();

    let second = RecordPatch {
        notes: Some("Bob was here".into()),
        expected_version: Some(stale_version),
        ..Default::default()
    };
    match repo.patch_record("gj-1061", &second).await {
        Err(RepoError::Conflict { current }) => {
            assert_eq!(current, stale_version + 1, "the live version is reported back");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }

    // Alice's text stands; Bob's was refused rather than silently dropped.
    let after = repo.snapshot("alice").await.unwrap();
    let notes = &after
        .systems
        .iter()
        .find(|s| s.system.id == "gj-1061")
        .unwrap()
        .system
        .record
        .notes;
    assert_eq!(notes, "Alice was here");
});

db_test!(a_write_that_quotes_the_current_version_succeeds, |repo| {
    let snap = repo.snapshot("alice").await.unwrap();
    let entry = snap.systems.iter().find(|s| s.system.id == "sol").unwrap();
    let patch = RecordPatch {
        population: Some("8.2 billion".into()),
        expected_version: Some(entry.version),
        ..Default::default()
    };
    let new_version = repo.patch_record("sol", &patch).await.unwrap();
    assert_eq!(new_version, entry.version + 1);
});

db_test!(settings_are_per_user_not_shared, |repo| {
    // Was a singleton row: whoever clicked last decided what everyone selected.
    repo.save_settings(
        "alice",
        &Settings {
            selected: Some("gj-1061".into()),
            compare: Some("trappist-1".into()),
            focus_planet: Some("GJ 1061 d".into()),
        },
    )
    .await
    .unwrap();
    repo.save_settings(
        "bob",
        &Settings { selected: Some("sol".into()), compare: None, focus_planet: None },
    )
    .await
    .unwrap();

    let alice = repo.snapshot("alice").await.unwrap().settings;
    let bob = repo.snapshot("bob").await.unwrap().settings;

    assert_eq!(alice.selected.as_deref(), Some("gj-1061"));
    assert_eq!(alice.focus_planet.as_deref(), Some("GJ 1061 d"));
    assert_eq!(bob.selected.as_deref(), Some("sol"), "Bob's view is his own");
    assert_eq!(bob.compare, None);

    // And an operator who has never connected gets an empty view, not someone
    // else's.
    assert_eq!(repo.snapshot("carol").await.unwrap().settings, Settings::default());
});

db_test!(planet_dossiers_merge_field_by_field_too, |repo| {
    let a = PlanetRecordPatch {
        imperial_name: Some("Anvil".into()),
        ..Default::default()
    };
    let b = PlanetRecordPatch {
        continents: Some("North, South, Verge".into()),
        ..Default::default()
    };
    repo.patch_planet_record("gj-1061", "GJ 1061 d", &a).await.unwrap();
    repo.patch_planet_record("gj-1061", "GJ 1061 d", &b).await.unwrap();

    let snap = repo.snapshot("alice").await.unwrap();
    let rec = snap
        .systems
        .iter()
        .find(|s| s.system.id == "gj-1061")
        .unwrap()
        .system
        .planet_record("GJ 1061 d");
    assert_eq!(rec.imperial_name, "Anvil");
    assert_eq!(rec.continent_count(), 3);
});

db_test!(a_refresh_still_never_touches_a_dossier, |repo| {
    // The invariant has to hold through the new write path as well.
    repo.patch_record(
        "gj-1061",
        &RecordPatch {
            imperial_name: Some("Kestrel Reach".into()),
            arm: Some(Some(Arm::Perseus)),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let mut fresh = parallax::core::seed::seed_systems()
        .into_iter()
        .find(|s| s.id == "gj-1061")
        .unwrap();
    fresh.dist_pc = Some(3.999);
    fresh.record = Record::default();
    repo.upsert_system(&fresh).await.unwrap();

    let snap = repo.snapshot("alice").await.unwrap();
    let s = &snap.systems.iter().find(|s| s.system.id == "gj-1061").unwrap().system;
    assert_eq!(s.dist_pc, Some(3.999));
    assert_eq!(s.record.imperial_name, "Kestrel Reach");
    assert_eq!(s.record.arm, Some(Arm::Perseus));
});

db_test!(deleting_a_system_while_someone_edits_it_reports_gone_not_silence, |repo| {
    // The desktop path issued an UPDATE that matched zero rows and returned
    // success, so the operator's text vanished with no indication why.
    repo.delete_system("yz-cet").await.unwrap();
    match repo
        .patch_record("yz-cet", &RecordPatch { notes: Some("late".into()), ..Default::default() })
        .await
    {
        Err(RepoError::NotFound) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
    // And deleting it twice is also reported rather than passing quietly.
    assert!(matches!(repo.delete_system("yz-cet").await, Err(RepoError::NotFound)));
});

db_test!(an_empty_patch_is_a_no_op_that_does_not_bump_the_version, |repo| {
    // The UI marks a dossier dirty on any revision bump, including ones that
    // changed nothing. Those must not churn the version and provoke conflicts
    // for everybody else.
    let before = repo.snapshot("alice").await.unwrap();
    let v = before.systems.iter().find(|s| s.system.id == "sol").unwrap().version;
    let after = repo.patch_record("sol", &RecordPatch::default()).await.unwrap();
    assert_eq!(after, v, "an empty patch must leave the version alone");
});

/* -------------------------------------------------------- real contention -- */

db_test!(fifty_concurrent_writers_all_land, |repo| {
    // The pool is capped at 16, so this also exercises queueing rather than
    // connection exhaustion.
    let repo = Arc::new(repo);
    let mut tasks = Vec::new();
    for i in 0..50 {
        let repo = Arc::clone(&repo);
        tasks.push(tokio::spawn(async move {
            // Unconditional writes: no expected_version, so contention shows up
            // as serialisation rather than conflicts.
            let patch = RecordPatch {
                population: Some(format!("writer {i}")),
                ..Default::default()
            };
            repo.patch_record("sol", &patch).await
        }));
    }
    let mut ok = 0;
    for t in tasks {
        if t.await.unwrap().is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 50, "every write should be accepted");

    let snap = repo.snapshot("alice").await.unwrap();
    let entry = snap.systems.iter().find(|s| s.system.id == "sol").unwrap();
    assert!(
        entry.version >= 50,
        "each write bumps the version; got {}",
        entry.version
    );
    assert!(
        entry.system.record.population.starts_with("writer "),
        "the last writer wins a same-field race, which is expected"
    );
});

db_test!(concurrent_writers_to_different_systems_do_not_serialise_incorrectly, |repo| {
    let repo = Arc::new(repo);
    let ids: Vec<String> = repo
        .snapshot("alice")
        .await
        .unwrap()
        .systems
        .iter()
        .map(|s| s.system.id.clone())
        .collect();

    let mut tasks = Vec::new();
    for id in ids.clone() {
        let repo = Arc::clone(&repo);
        tasks.push(tokio::spawn(async move {
            let patch = RecordPatch {
                notes: Some(format!("touched {id}")),
                ..Default::default()
            };
            repo.patch_record(&id, &patch).await
        }));
    }
    for t in tasks {
        t.await.unwrap().expect("independent systems must not collide");
    }

    let snap = repo.snapshot("alice").await.unwrap();
    for entry in &snap.systems {
        assert_eq!(
            entry.system.record.notes,
            format!("touched {}", entry.system.id),
            "every system got its own write"
        );
    }
});

db_test!(the_pool_survives_more_concurrent_readers_than_it_has_connections, |repo| {
    let repo = Arc::new(repo);
    let mut tasks = Vec::new();
    for _ in 0..64 {
        let repo = Arc::clone(&repo);
        tasks.push(tokio::spawn(async move { repo.snapshot("alice").await.map(|s| s.systems.len()) }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap().unwrap(), 13);
    }
});

/* ------------------------------------------------------------ notify -- */

#[tokio::test]
async fn a_change_notifies_listeners() {
    let Some((repo, url)) = fresh().await else { return };
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    parallax::server::listen::spawn(url, tx);

    // Give the listener a moment to establish its session.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    repo.patch_record(
        "gj-1061",
        &RecordPatch { notes: Some("changed".into()), ..Default::default() },
    )
    .await
    .unwrap();

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("no notification arrived within 5s")
        .expect("channel closed");
    assert_eq!(received, "gj-1061");
}

#[tokio::test]
async fn a_planet_dossier_change_also_notifies() {
    let Some((repo, url)) = fresh().await else { return };
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    parallax::server::listen::spawn(url, tx);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    repo.patch_planet_record(
        "gj-1061",
        "GJ 1061 d",
        &PlanetRecordPatch { notes: Some("noted".into()), ..Default::default() },
    )
    .await
    .unwrap();

    // The payload is the *system* id even for a planet change, because that is
    // the unit a client re-reads.
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("no notification within 5s")
        .expect("channel closed");
    assert_eq!(received, "gj-1061");
}

/* ------------------------------------------------------- shadow diffing -- */

#[test]
fn the_client_only_sends_fields_it_changed() {
    // The client-side half of the guarantee: if the diff were computed wrongly,
    // the server's field-level writes would not help.
    let server_state = Record {
        imperial_name: "Kestrel Reach".into(),
        arm: Some(Arm::Perseus),
        population: "4.1 billion".into(),
        notes: "#capital".into(),
    };
    let mut local = server_state.clone();
    local.notes = "#capital #besieged".into();

    let patch = RecordPatch::between(&server_state, &local);
    assert!(patch.imperial_name.is_none());
    assert!(patch.arm.is_none());
    assert!(patch.population.is_none());
    assert_eq!(patch.notes.as_deref(), Some("#capital #besieged"));

    // Applying it to a copy that someone else has since renamed keeps the rename.
    let mut theirs = server_state.clone();
    theirs.imperial_name = "Renamed By Someone Else".into();
    patch.apply_to(&mut theirs);
    assert_eq!(theirs.imperial_name, "Renamed By Someone Else");
    assert_eq!(theirs.notes, "#capital #besieged");
}

#[test]
fn planet_record_diffs_behave_the_same() {
    let server_state = PlanetRecord {
        imperial_name: "Anvil".into(),
        population: "900 million".into(),
        continents: "North, South".into(),
        notes: String::new(),
    };
    let mut local = server_state.clone();
    local.population = "1.1 billion".into();

    let patch = PlanetRecordPatch::between(&server_state, &local);
    assert!(patch.continents.is_none());
    assert!(patch.imperial_name.is_none());
    assert_eq!(patch.population.as_deref(), Some("1.1 billion"));
}
