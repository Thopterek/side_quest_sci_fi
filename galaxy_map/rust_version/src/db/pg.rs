//! PostgreSQL backend.
//!
//! Native only: a browser cannot open a Postgres connection, so the wasm build
//! uses [`MemoryStore`](crate::core::store::MemoryStore) instead.

use std::collections::BTreeMap;

use postgres::types::ToSql;
use postgres::{Client, NoTls, Row, Transaction};

use crate::core::model::{Arm, Planet, PlanetRecord, Record, Source, System};
use crate::core::store::{Settings, Snapshot, StoreError, StoreResult, VaultStore};

const SCHEMA: &str = include_str!("schema.sql");

/// Default connection string, overridable with `PARALLAX_DATABASE_URL`.
pub const DEFAULT_URL: &str = "host=localhost user=postgres dbname=parallax";

/// The identity this backend writes settings under. Direct PostgreSQL access is
/// single-operator by design; multi-user goes through `parallax-server`.
const LOCAL_USER: &str = "local";

pub fn connection_string() -> String {
    std::env::var("PARALLAX_DATABASE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

pub struct PgStore {
    client: Client,
    label: String,
}

impl PgStore {
    pub fn connect(url: &str) -> StoreResult<Self> {
        let client = Client::connect(url, NoTls).map_err(|e| StoreError::Connect(e.to_string()))?;
        // Never echo the connection string back; it may carry a password.
        let label = url
            .split_whitespace()
            .find(|t| t.starts_with("dbname="))
            .map(|t| t.trim_start_matches("dbname=").to_string())
            .unwrap_or_else(|| "postgres".into());
        Ok(PgStore { client, label })
    }

    pub fn from_env() -> StoreResult<Self> {
        Self::connect(&connection_string())
    }
}

/* ------------------------------------------------------------- mapping -- */

fn arm_to_sql(a: Option<Arm>) -> Option<&'static str> {
    a.map(|a| match a {
        Arm::Local => "local",
        Arm::Perseus => "perseus",
        Arm::Sagittarius => "sagittarius",
        Arm::Scutum => "scutum",
        Arm::Norma => "norma",
        Arm::Outer => "outer",
    })
}

fn arm_from_sql(s: Option<&str>) -> Option<Arm> {
    match s? {
        "local" => Some(Arm::Local),
        "perseus" => Some(Arm::Perseus),
        "sagittarius" => Some(Arm::Sagittarius),
        "scutum" => Some(Arm::Scutum),
        "norma" => Some(Arm::Norma),
        "outer" => Some(Arm::Outer),
        _ => None,
    }
}

fn source_to_sql(s: Source) -> &'static str {
    match s {
        Source::Nasa => "nasa",
        Source::Seed => "seed",
        Source::Reference => "reference",
    }
}

fn source_from_sql(s: &str) -> Source {
    match s {
        "seed" => Source::Seed,
        "reference" => Source::Reference,
        _ => Source::Nasa,
    }
}

fn system_from_row(row: &Row) -> System {
    System {
        id: row.get("id"),
        hostname: row.get("hostname"),
        ra: row.get("ra"),
        dec: row.get("dec"),
        dist_pc: row.get("dist_pc"),
        teff: row.get("teff"),
        radius_sun: row.get("radius_sun"),
        mass_sun: row.get("mass_sun"),
        spectype: row.get("spectype"),
        vmag: row.get("vmag"),
        planets: Vec::new(),
        record: Record {
            imperial_name: row.get("imperial_name"),
            arm: arm_from_sql(row.get::<_, Option<&str>>("arm")),
            population: row.get("population"),
            notes: row.get("notes"),
        },
        planet_records: BTreeMap::new(),
        source: source_from_sql(row.get("source")),
        origin: row.get("origin"),
    }
}

fn planet_from_row(row: &Row) -> Planet {
    Planet {
        name: row.get("name"),
        orbsmax: row.get("orbsmax"),
        orbper: row.get("orbper"),
        rade: row.get("rade"),
        bmasse: row.get("bmasse"),
        eqt: row.get("eqt"),
        orbeccen: row.get("orbeccen"),
        disc_year: row.get("disc_year"),
        disc_method: row.get("disc_method"),
        disc_facility: row.get("disc_facility"),
    }
}

fn q(e: postgres::Error) -> StoreError {
    StoreError::Query(e.to_string())
}

/// Replace every planet row for a host. Used by both insert paths.
fn write_planets(tx: &mut Transaction<'_>, sys: &System) -> StoreResult<()> {
    tx.execute("delete from parallax.planets where system_id = $1", &[&sys.id]).map_err(q)?;
    for (i, p) in sys.planets.iter().enumerate() {
        let ordinal = i as i32;
        let params: [&(dyn ToSql + Sync); 12] = [
            &sys.id,
            &p.name,
            &ordinal,
            &p.orbsmax,
            &p.orbper,
            &p.rade,
            &p.bmasse,
            &p.eqt,
            &p.orbeccen,
            &p.disc_year,
            &p.disc_method,
            &p.disc_facility,
        ];
        tx.execute(
            "insert into parallax.planets
               (system_id, name, ordinal, orbsmax, orbper, rade, bmasse, eqt,
                orbeccen, disc_year, disc_method, disc_facility)
             values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
            &params,
        )
        .map_err(q)?;
    }
    Ok(())
}

impl VaultStore for PgStore {
    fn migrate(&mut self) -> StoreResult<()> {
        self.client.batch_execute(SCHEMA).map_err(|e| StoreError::Migrate(e.to_string()))
    }

    fn load(&mut self) -> StoreResult<Snapshot> {
        let rows = self
            .client
            .query(
                "select id, hostname, ra, dec, dist_pc, teff, radius_sun, mass_sun,
                        spectype, vmag, source, origin, imperial_name, arm, population, notes
                   from parallax.systems
                  order by origin desc, dist_pc nulls last, hostname",
                &[],
            )
            .map_err(q)?;

        let mut systems: Vec<System> = rows.iter().map(system_from_row).collect();
        let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
        for (i, s) in systems.iter().enumerate() {
            by_id.insert(s.id.clone(), i);
        }

        // Two flat queries rather than one per system: the vault is loaded once
        // at startup and this keeps it to three round trips regardless of size.
        for row in self
            .client
            .query(
                "select system_id, name, orbsmax, orbper, rade, bmasse, eqt, orbeccen,
                        disc_year, disc_method, disc_facility
                   from parallax.planets order by system_id, ordinal",
                &[],
            )
            .map_err(q)?
        {
            let sid: String = row.get("system_id");
            if let Some(&i) = by_id.get(&sid) {
                systems[i].planets.push(planet_from_row(&row));
            }
        }

        for row in self
            .client
            .query(
                "select system_id, planet_name, imperial_name, population, continents, notes
                   from parallax.planet_records",
                &[],
            )
            .map_err(q)?
        {
            let sid: String = row.get("system_id");
            if let Some(&i) = by_id.get(&sid) {
                systems[i].planet_records.insert(
                    row.get("planet_name"),
                    PlanetRecord {
                        imperial_name: row.get("imperial_name"),
                        population: row.get("population"),
                        continents: row.get("continents"),
                        notes: row.get("notes"),
                    },
                );
            }
        }

        let settings = match self
            .client
            .query_opt(
                "select selected, compare, focus_planet from parallax.user_settings
                  where user_id = $1",
                &[&LOCAL_USER],
            )
            .map_err(q)?
        {
            Some(row) => Settings {
                selected: row.get("selected"),
                compare: row.get("compare"),
                focus_planet: row.get("focus_planet"),
            },
            None => Settings::default(),
        };

        Ok(Snapshot { systems, settings })
    }

    /// A refresh. The `do update` clause lists archive columns and nothing else,
    /// so the dossier cannot be clobbered even by a caller that wants to.
    fn upsert_system(&mut self, sys: &System) -> StoreResult<()> {
        let mut tx = self.client.transaction().map_err(q)?;
        let arm = arm_to_sql(sys.record.arm);
        let source = source_to_sql(sys.source);
        let params: [&(dyn ToSql + Sync); 16] = [
            &sys.id,
            &sys.hostname,
            &sys.ra,
            &sys.dec,
            &sys.dist_pc,
            &sys.teff,
            &sys.radius_sun,
            &sys.mass_sun,
            &sys.spectype,
            &sys.vmag,
            &source,
            &sys.origin,
            &sys.record.imperial_name,
            &arm,
            &sys.record.population,
            &sys.record.notes,
        ];
        tx.execute(
            "insert into parallax.systems
               (id, hostname, ra, dec, dist_pc, teff, radius_sun, mass_sun, spectype,
                vmag, source, origin, imperial_name, arm, population, notes)
             values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
             on conflict (id) do update set
               hostname   = excluded.hostname,
               ra         = excluded.ra,
               dec        = excluded.dec,
               dist_pc    = excluded.dist_pc,
               teff       = excluded.teff,
               radius_sun = excluded.radius_sun,
               mass_sun   = excluded.mass_sun,
               spectype   = excluded.spectype,
               vmag       = excluded.vmag,
               source     = excluded.source,
               updated_at = now()",
            &params,
        )
        .map_err(q)?;
        write_planets(&mut tx, sys)?;
        tx.commit().map_err(q)
    }

    /// Seeding and import: the dossier travels with the system.
    fn insert_with_dossier(&mut self, sys: &System) -> StoreResult<()> {
        let mut tx = self.client.transaction().map_err(q)?;
        let arm = arm_to_sql(sys.record.arm);
        let source = source_to_sql(sys.source);
        let params: [&(dyn ToSql + Sync); 16] = [
            &sys.id,
            &sys.hostname,
            &sys.ra,
            &sys.dec,
            &sys.dist_pc,
            &sys.teff,
            &sys.radius_sun,
            &sys.mass_sun,
            &sys.spectype,
            &sys.vmag,
            &source,
            &sys.origin,
            &sys.record.imperial_name,
            &arm,
            &sys.record.population,
            &sys.record.notes,
        ];
        tx.execute(
            "insert into parallax.systems
               (id, hostname, ra, dec, dist_pc, teff, radius_sun, mass_sun, spectype,
                vmag, source, origin, imperial_name, arm, population, notes)
             values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
             on conflict (id) do update set
               hostname      = excluded.hostname,
               ra            = excluded.ra,
               dec           = excluded.dec,
               dist_pc       = excluded.dist_pc,
               teff          = excluded.teff,
               radius_sun    = excluded.radius_sun,
               mass_sun      = excluded.mass_sun,
               spectype      = excluded.spectype,
               vmag          = excluded.vmag,
               source        = excluded.source,
               imperial_name = excluded.imperial_name,
               arm           = excluded.arm,
               population    = excluded.population,
               notes         = excluded.notes,
               updated_at    = now()",
            &params,
        )
        .map_err(q)?;
        write_planets(&mut tx, sys)?;

        for (name, rec) in &sys.planet_records {
            let params: [&(dyn ToSql + Sync); 6] = [
                &sys.id,
                name,
                &rec.imperial_name,
                &rec.population,
                &rec.continents,
                &rec.notes,
            ];
            tx.execute(
                "insert into parallax.planet_records
                   (system_id, planet_name, imperial_name, population, continents, notes)
                 values ($1,$2,$3,$4,$5,$6)
                 on conflict (system_id, planet_name) do update set
                   imperial_name = excluded.imperial_name,
                   population    = excluded.population,
                   continents    = excluded.continents,
                   notes         = excluded.notes,
                   updated_at    = now()",
                &params,
            )
            .map_err(q)?;
        }
        tx.commit().map_err(q)
    }

    fn save_record(&mut self, system_id: &str, record: &Record) -> StoreResult<()> {
        let arm = arm_to_sql(record.arm);
        let params: [&(dyn ToSql + Sync); 5] = [
            &record.imperial_name,
            &arm,
            &record.population,
            &record.notes,
            &system_id,
        ];
        self.client
            .execute(
                "update parallax.systems
                    set imperial_name = $1, arm = $2, population = $3, notes = $4,
                        updated_at = now()
                  where id = $5",
                &params,
            )
            .map_err(q)?;
        Ok(())
    }

    fn save_planet_record(
        &mut self,
        system_id: &str,
        planet_name: &str,
        record: &PlanetRecord,
    ) -> StoreResult<()> {
        let params: [&(dyn ToSql + Sync); 6] = [
            &system_id,
            &planet_name,
            &record.imperial_name,
            &record.population,
            &record.continents,
            &record.notes,
        ];
        self.client
            .execute(
                "insert into parallax.planet_records
                   (system_id, planet_name, imperial_name, population, continents, notes)
                 values ($1,$2,$3,$4,$5,$6)
                 on conflict (system_id, planet_name) do update set
                   imperial_name = excluded.imperial_name,
                   population    = excluded.population,
                   continents    = excluded.continents,
                   notes         = excluded.notes,
                   updated_at    = now()",
                &params,
            )
            .map_err(q)?;
        Ok(())
    }

    fn delete_system(&mut self, system_id: &str) -> StoreResult<()> {
        // Planets and planet_records cascade; settings references null out.
        self.client
            .execute("delete from parallax.systems where id = $1", &[&system_id])
            .map_err(q)?;
        Ok(())
    }

    fn save_settings(&mut self, settings: &Settings) -> StoreResult<()> {
        let params: [&(dyn ToSql + Sync); 4] =
            [&LOCAL_USER, &settings.selected, &settings.compare, &settings.focus_planet];
        // An upsert, not an update: `TRUNCATE systems CASCADE` reaches this
        // table through its foreign key and removes the row, so it cannot be
        // assumed to exist.
        //
        // Writes under the fixed `local` identity. This backend is for one
        // operator on one desktop by definition — anything multi-user goes
        // through the server, which is where per-user settings live.
        self.client
            .execute(
                "insert into parallax.user_settings
                   (user_id, selected, compare, focus_planet)
                 values ($1, $2, $3, $4)
                 on conflict (user_id) do update set
                   selected     = excluded.selected,
                   compare      = excluded.compare,
                   focus_planet = excluded.focus_planet,
                   updated_at   = now()",
                &params,
            )
            .map_err(q)?;
        Ok(())
    }

    fn describe(&self) -> String {
        format!("PostgreSQL · {}", self.label)
    }
}

/* -------------------------------------------------------- query helpers -- */

impl PgStore {
    /// Tags, from the `system_tags` view rather than by scanning notes in Rust.
    pub fn tags(&mut self) -> StoreResult<Vec<String>> {
        Ok(self
            .client
            .query("select distinct tag from parallax.system_tags order by tag", &[])
            .map_err(q)?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }

    /// Undirected `[[link]]` edges, already deduplicated and resolved.
    pub fn link_edges(&mut self) -> StoreResult<Vec<(String, String)>> {
        Ok(self
            .client
            .query("select a, b from parallax.link_edges order by a, b", &[])
            .map_err(q)?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect())
    }

    /// Indexed full-text search across catalog names and dossiers.
    pub fn search(&mut self, term: &str) -> StoreResult<Vec<String>> {
        Ok(self
            .client
            .query(
                "select id from parallax.systems
                  where search @@ plainto_tsquery('simple', $1)
                  order by dist_pc nulls last",
                &[&term],
            )
            .map_err(q)?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }

    /// Systems carrying a given tag, via the view.
    pub fn by_tag(&mut self, tag: &str) -> StoreResult<Vec<String>> {
        Ok(self
            .client
            .query(
                "select distinct system_id from parallax.system_tags where tag = $1 order by 1",
                &[&tag.to_lowercase()],
            )
            .map_err(q)?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }

    /// Drop everything. Test support.
    pub fn truncate_all(&mut self) -> StoreResult<()> {
        self.client
            .batch_execute(
                "truncate parallax.planet_records, parallax.planets, parallax.systems cascade;
                 truncate parallax.user_settings;
                 insert into parallax.user_settings (user_id) values ('local')
                   on conflict do nothing;",
            )
            .map_err(q)
    }
}

/* ------------------------------------------------------- test support -- */

impl PgStore {
    /// Run a statement and return its single text column. Used by the schema
    /// tests to compare SQL `slugify` against the Rust `slug`.
    pub fn raw_query_one_text(&mut self, sql: &str, arg: &str) -> StoreResult<String> {
        let row = self.client.query_one(sql, &[&arg]).map_err(q)?;
        Ok(row.get(0))
    }

    /// Execute arbitrary SQL. Returns `Err` when a constraint rejects it, which
    /// is what the constraint tests assert on.
    pub fn raw_execute(&mut self, sql: &str) -> StoreResult<u64> {
        self.client.execute(sql, &[]).map_err(q)
    }

    /// Count rows in `table` belonging to a system.
    pub fn count(&mut self, table: &str, system_id: &str) -> StoreResult<i64> {
        let sql = format!("select count(*) from {table} where system_id = $1");
        let row = self.client.query_one(sql.as_str(), &[&system_id]).map_err(q)?;
        Ok(row.get(0))
    }
}
