//! Integration tests against a real PostgreSQL server.
//!
//! Skipped unless `PARALLAX_TEST_DATABASE_URL` is set, so `cargo test` stays
//! green on a machine with no database:
//!
//! ```sh
//! PARALLAX_TEST_DATABASE_URL="host=localhost user=postgres dbname=parallax" \
//!   cargo test --features db --test pg_integration -- --test-threads=1
//! ```
//!
//! These run single-threaded because they share one schema and truncate it.

#![cfg(feature = "db")]

use std::sync::Mutex;

use parallax::core::model::{slug, Arm, PlanetRecord, Record, Source};
use parallax::core::seed::seed_systems;
use parallax::core::store::{Settings, VaultStore};
use parallax::core::vault::Vault;
use parallax::db::PgStore;

/// Serialises access even if the harness is run multi-threaded by mistake.
static DB_LOCK: Mutex<()> = Mutex::new(());

fn store() -> Option<PgStore> {
    let url = std::env::var("PARALLAX_TEST_DATABASE_URL").ok()?;
    let mut s = PgStore::connect(&url).expect("connect");
    s.migrate().expect("migrate");
    s.truncate_all().expect("truncate");
    Some(s)
}

/// Every test body goes through here so a missing database skips rather than fails.
fn with_db(f: impl FnOnce(&mut PgStore)) {
    let _guard = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match store() {
        Some(mut s) => f(&mut s),
        None => eprintln!("skipped: PARALLAX_TEST_DATABASE_URL not set"),
    }
}

fn seeded(store: &mut PgStore) -> Vault {
    let vault = Vault::seeded();
    for s in &vault.systems {
        store.insert_with_dossier(s).expect("seed insert");
    }
    vault
}

#[test]
fn migration_is_idempotent() {
    with_db(|s| {
        s.migrate().expect("second migrate");
        s.migrate().expect("third migrate");
    });
}

#[test]
fn the_whole_seed_vault_round_trips() {
    with_db(|s| {
        let original = seeded(s);
        let snap = s.load().expect("load");

        assert_eq!(snap.systems.len(), original.systems.len());
        for want in &original.systems {
            let got = snap.systems.iter().find(|x| x.id == want.id).expect(&want.id);
            assert_eq!(got.hostname, want.hostname);
            assert_eq!(got.dist_pc, want.dist_pc);
            assert_eq!(got.teff, want.teff);
            assert_eq!(got.spectype, want.spectype);
            assert_eq!(got.source, want.source);
            assert_eq!(got.origin, want.origin);
            assert_eq!(got.record, want.record, "dossier for {}", want.id);
            assert_eq!(got.planets.len(), want.planets.len(), "planets for {}", want.id);
            // Planet order must survive, since the record panel reads outward.
            for (a, b) in got.planets.iter().zip(&want.planets) {
                assert_eq!(a.name, b.name);
                assert_eq!(a.orbsmax, b.orbsmax);
                assert_eq!(a.rade, b.rade);
            }
            assert_eq!(got.planet_records, want.planet_records);
        }
    });
}

#[test]
fn a_refresh_updates_the_archive_and_leaves_the_dossier_alone() {
    // The invariant the schema exists to enforce.
    with_db(|s| {
        seeded(s);
        s.save_record(
            "gj-1061",
            &Record {
                imperial_name: "Kestrel Reach".into(),
                arm: Some(Arm::Perseus),
                population: "4.1 billion".into(),
                notes: "seat of the outer marches #capital".into(),
            },
        )
        .unwrap();

        // A fresh archive pull carries no dossier at all.
        let mut fresh = seed_systems().into_iter().find(|x| x.id == "gj-1061").unwrap();
        fresh.dist_pc = Some(3.999);
        fresh.teff = Some(2999.0);
        fresh.source = Source::Nasa;
        fresh.record = Record::default();
        fresh.planet_records.clear();
        s.upsert_system(&fresh).unwrap();

        let snap = s.load().unwrap();
        let got = snap.systems.iter().find(|x| x.id == "gj-1061").unwrap();
        assert_eq!(got.dist_pc, Some(3.999), "archive column must update");
        assert_eq!(got.teff, Some(2999.0));
        assert_eq!(got.source, Source::Nasa);
        assert_eq!(got.record.imperial_name, "Kestrel Reach", "dossier must survive");
        assert_eq!(got.record.arm, Some(Arm::Perseus));
        assert_eq!(got.record.population, "4.1 billion");
    });
}

#[test]
fn planet_dossiers_survive_the_planet_rows_being_replaced() {
    // A refresh deletes and reinserts every planet row. If planet_records
    // cascaded from planets rather than from systems, this would wipe them.
    with_db(|s| {
        seeded(s);
        s.save_planet_record(
            "gj-1061",
            "GJ 1061 d",
            &PlanetRecord {
                imperial_name: "Anvil".into(),
                population: "900 million".into(),
                continents: "North, South, Verge".into(),
                notes: "capital world #terraformed".into(),
            },
        )
        .unwrap();

        let fresh = seed_systems().into_iter().find(|x| x.id == "gj-1061").unwrap();
        s.upsert_system(&fresh).unwrap();

        let snap = s.load().unwrap();
        let got = snap.systems.iter().find(|x| x.id == "gj-1061").unwrap();
        let rec = got.planet_record("GJ 1061 d");
        assert_eq!(rec.imperial_name, "Anvil");
        assert_eq!(rec.continent_count(), 3);
        assert_eq!(got.planets.len(), 3, "planets should still be present");
    });
}

#[test]
fn a_dossier_outlives_a_planet_the_archive_retracts() {
    with_db(|s| {
        seeded(s);
        s.save_planet_record(
            "gj-1061",
            "GJ 1061 d",
            &PlanetRecord { imperial_name: "Anvil".into(), ..Default::default() },
        )
        .unwrap();

        // The archive drops planet d in a later release.
        let mut fresh = seed_systems().into_iter().find(|x| x.id == "gj-1061").unwrap();
        fresh.planets.retain(|p| p.name != "GJ 1061 d");
        s.upsert_system(&fresh).unwrap();

        let snap = s.load().unwrap();
        let got = snap.systems.iter().find(|x| x.id == "gj-1061").unwrap();
        assert_eq!(got.planets.len(), 2, "the archive's retraction applies");
        assert_eq!(
            got.planet_record("GJ 1061 d").imperial_name,
            "Anvil",
            "but the operator's note is theirs to delete, not NASA's"
        );
    });
}

#[test]
fn sql_slugify_agrees_with_the_rust_implementation() {
    // Two implementations of the same rule; a divergence would silently break
    // wikilink resolution in the views.
    with_db(|s| {
        seeded(s);
        for sys in &Vault::seeded().systems {
            let sql: String = s
                .raw_query_one_text("select parallax.slugify($1)", &sys.hostname)
                .expect("slugify");
            assert_eq!(sql, slug(&sys.hostname), "diverged on {:?}", sys.hostname);
            assert_eq!(sql, sys.id);
        }
        for odd in ["  Sol  ", "Kepler-186", "eps Eri", "HD 219134", "K2-18", "TOI-700"] {
            let sql: String = s.raw_query_one_text("select parallax.slugify($1)", odd).unwrap();
            assert_eq!(sql, slug(odd), "diverged on {odd:?}");
        }
    });
}

#[test]
fn the_tags_view_matches_the_rust_scanner() {
    with_db(|s| {
        let vault = seeded(s);
        let mut from_sql = s.tags().unwrap();
        let mut from_rust = vault.tags();
        from_sql.sort();
        from_rust.sort();
        assert_eq!(from_sql, from_rust);
        assert!(from_rust.contains(&"habitable-zone".to_string()));
    });
}

#[test]
fn the_tags_view_sees_planet_notes_too() {
    with_db(|s| {
        seeded(s);
        s.save_planet_record(
            "gj-1061",
            "GJ 1061 d",
            &PlanetRecord { notes: "#terraformed".into(), ..Default::default() },
        )
        .unwrap();
        assert!(s.tags().unwrap().contains(&"terraformed".to_string()));
        assert_eq!(s.by_tag("terraformed").unwrap(), vec!["gj-1061".to_string()]);
    });
}

#[test]
fn the_link_view_matches_the_rust_resolver() {
    with_db(|s| {
        let vault = seeded(s);
        let mut from_sql = s.link_edges().unwrap();
        let mut from_rust = vault.link_edges();
        from_sql.sort();
        from_rust.sort();
        assert_eq!(from_sql, from_rust);
        assert!(from_sql.contains(&("gj-1061".to_string(), "trappist-1".to_string())));
    });
}

#[test]
fn the_link_view_drops_self_and_unresolved_targets() {
    with_db(|s| {
        seeded(s);
        s.save_record(
            "gj-1061",
            &Record {
                notes: "[[GJ 1061]] [[Nowhere At All]] [[TRAPPIST-1]]".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let edges = s.link_edges().unwrap();
        assert!(!edges.iter().any(|(a, b)| a == b), "a system must not link to itself");
        // Of the three targets written, only TRAPPIST-1 resolves: the self link
        // and the nonexistent one are dropped by the view.
        let from_gj: Vec<_> = edges
            .iter()
            .filter(|(a, b)| a == "gj-1061" || b == "gj-1061")
            .collect();
        assert_eq!(from_gj.len(), 1, "expected one resolvable edge, got {from_gj:?}");
        assert!(edges.contains(&("gj-1061".to_string(), "trappist-1".to_string())));
    });
}

#[test]
fn full_text_search_finds_a_system_by_its_imperial_name() {
    with_db(|s| {
        seeded(s);
        assert!(s.search("kestrel").unwrap().is_empty());
        s.save_record(
            "gj-1061",
            &Record { imperial_name: "Kestrel Reach".into(), ..Default::default() },
        )
        .unwrap();
        assert_eq!(s.search("kestrel").unwrap(), vec!["gj-1061".to_string()]);
        // The generated column is maintained by the database, not by us.
        assert_eq!(s.search("Reach").unwrap(), vec!["gj-1061".to_string()]);
    });
}

#[test]
fn deleting_a_system_cascades_and_clears_the_selection() {
    with_db(|s| {
        seeded(s);
        s.save_planet_record(
            "gj-1061",
            "GJ 1061 d",
            &PlanetRecord { imperial_name: "Anvil".into(), ..Default::default() },
        )
        .unwrap();
        s.save_settings(&Settings {
            selected: Some("gj-1061".into()),
            compare: Some("trappist-1".into()),
            focus_planet: Some("GJ 1061 d".into()),
        })
        .unwrap();

        s.delete_system("gj-1061").unwrap();
        let snap = s.load().unwrap();

        assert!(snap.systems.iter().all(|x| x.id != "gj-1061"));
        assert_eq!(snap.settings.selected, None, "a dangling selection must be nulled");
        assert_eq!(snap.settings.compare.as_deref(), Some("trappist-1"), "the other endpoint stands");
        assert_eq!(s.count("parallax.planets", "gj-1061").unwrap(), 0);
        assert_eq!(s.count("parallax.planet_records", "gj-1061").unwrap(), 0);
    });
}

#[test]
fn settings_round_trip() {
    with_db(|s| {
        seeded(s);
        let want = Settings {
            selected: Some("trappist-1".into()),
            compare: Some("sol".into()),
            focus_planet: Some("TRAPPIST-1 e".into()),
        };
        s.save_settings(&want).unwrap();
        assert_eq!(s.load().unwrap().settings, want);
    });
}

#[test]
fn the_schema_rejects_data_the_model_would_never_produce() {
    with_db(|s| {
        seeded(s);
        // Declination outside ±90.
        assert!(s
            .raw_execute(
                "insert into parallax.systems (id,hostname,ra,dec) values ('bad','Bad',10,999)"
            )
            .is_err());
        // An arm that is not one of the six.
        assert!(s
            .raw_execute(
                "update parallax.systems set arm = 'andromeda' where id = 'gj-1061'"
            )
            .is_err());
        // An id that does not match its hostname would break every lookup.
        assert!(s
            .raw_execute(
                "insert into parallax.systems (id,hostname,ra,dec) values ('wrong','GJ 1061',1,1)"
            )
            .is_err());
        // Only one system may be the coordinate origin.
        assert!(s
            .raw_execute("update parallax.systems set origin = true where id = 'gj-1061'")
            .is_err());
        // A negative orbital period is not a period.
        assert!(s
            .raw_execute(
                "insert into parallax.planets (system_id,name,ordinal,orbper)
                 values ('gj-1061','GJ 1061 z',9,-5)"
            )
            .is_err());
    });
}

#[test]
fn loading_orders_sol_first_then_by_distance() {
    with_db(|s| {
        seeded(s);
        let snap = s.load().unwrap();
        assert!(snap.systems[0].origin, "Sol anchors the list");
        let rest: Vec<f64> = snap.systems[1..].iter().filter_map(|x| x.dist_pc).collect();
        assert!(
            rest.windows(2).all(|w| w[0] <= w[1]),
            "the rest should read outward: {rest:?}"
        );
    });
}
