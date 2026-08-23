//! Server-side data access.
//!
//! The important difference from [`crate::db::pg`] is that writes here are
//! *field level*. The desktop store wrote all four dossier columns on every
//! save, which is correct for one operator and loses data for two: if Alice
//! renames a system while Bob, working from a snapshot taken moments earlier,
//! records its population, Bob's write carries Alice's stale name and silently
//! reverts her. Sending only the fields that actually changed removes the
//! collision entirely for edits to different fields, and a version check
//! catches the genuine case where two people edited the same one.

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};

use crate::core::grant::{
    hash_token, mint_token, Capability, Grant, MintedGrant, Scope,
};
use serde::{Deserialize, Serialize};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Config, NoTls, Row};

use crate::core::model::{Arm, Planet, PlanetRecord, Record, Source, System};
use crate::core::patch::{PlanetRecordPatch, RecordPatch};
use crate::core::store::Settings;

const SCHEMA: &str = include_str!("../db/schema.sql");

#[derive(Debug)]
pub enum RepoError {
    Pool(String),
    Query(String),
    /// The row moved on since the client last read it.
    Conflict { current: i32 },
    NotFound,
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Pool(m) => write!(f, "database pool: {m}"),
            RepoError::Query(m) => write!(f, "query failed: {m}"),
            RepoError::Conflict { current } => {
                write!(f, "changed by someone else; current version is {current}")
            }
            RepoError::NotFound => write!(f, "no such system"),
        }
    }
}

impl std::error::Error for RepoError {}

type R<T> = Result<T, RepoError>;

fn q(e: tokio_postgres::Error) -> RepoError {
    RepoError::Query(e.to_string())
}

/// A system plus the version the client should quote on its next write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedSystem {
    #[serde(flatten)]
    pub system: System,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSnapshot {
    pub systems: Vec<VersionedSystem>,
    pub settings: Settings,
}

/* ---------------------------------------------------------------- repo -- */

/// Columns for a grant, with timestamps rendered by the database.
///
/// Formatting in SQL avoids a date-formatting dependency for the sake of two
/// fields, and keeps `revoked` a boolean at the boundary rather than a
/// nullable instant the client would have to interpret.
const GRANT_COLS: &str = "id, label, capability, scope_kind, scope_tag, scope_systems, use_count,
     to_char(expires_at   at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as expires_at,
     to_char(last_used_at at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as last_used_at,
     (revoked_at is not null) as revoked";

/// Map a grant row. `None` for a row the model cannot represent, which is the
/// safe direction: an unreadable grant grants nothing.
fn grant_from_row(r: &tokio_postgres::Row) -> Option<Grant> {
    Some(Grant {
        id: r.get("id"),
        label: r.get("label"),
        capability: Capability::parse(r.get::<_, &str>("capability"))?,
        scope: Scope::from_columns(
            r.get::<_, &str>("scope_kind"),
            r.get::<_, Option<&str>>("scope_tag"),
            r.get::<_, Vec<String>>("scope_systems"),
        )?,
        expires_at: r.get("expires_at"),
        revoked: r.get("revoked"),
        use_count: r.get("use_count"),
        last_used_at: r.get("last_used_at"),
    })
}

#[derive(Clone)]
pub struct Repo {
    pool: Pool,
}

impl Repo {
    /// `max_size` caps concurrent database connections regardless of how many
    /// clients are attached — the point of putting a server in front at all.
    pub fn connect(url: &str, max_size: usize) -> R<Self> {
        let config: Config = url.parse().map_err(|e| RepoError::Pool(format!("{e}")))?;
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig { recycling_method: RecyclingMethod::Fast },
        );
        let pool = Pool::builder(manager)
            .max_size(max_size)
            .build()
            .map_err(|e| RepoError::Pool(e.to_string()))?;
        Ok(Repo { pool })
    }

    async fn client(&self) -> R<deadpool_postgres::Object> {
        self.pool.get().await.map_err(|e| RepoError::Pool(e.to_string()))
    }

    pub async fn migrate(&self) -> R<()> {
        self.client().await?.batch_execute(SCHEMA).await.map_err(q)
    }

    pub async fn health(&self) -> R<()> {
        self.client().await?.simple_query("select 1").await.map_err(q)?;
        Ok(())
    }

    /* ----------------------------------------------------------- reads -- */

    /// The vault as one grant may see it.
    ///
    /// Every one of the three queries below joins `parallax.visible_systems`,
    /// which is the single place the stage is decided. Filtering here rather
    /// than in the client is the difference between access control and merely
    /// not drawing something: a reader who inspects the response gets only the
    /// rows their link entitles them to.
    pub async fn snapshot(&self, grant_id: &str) -> R<VaultSnapshot> {
        let c = self.client().await?;

        // Resolve the stage once, then look rows up by primary key.
        //
        // Joining the three queries through `visible_systems` instead made the
        // planner re-run the whole boundary — grant lookup, scope test, and for
        // a tag scope the notes regexp over every system — once per query.
        // Measured on the seed vault: 6.06 ms of database time per vault read
        // that way, 1.55 ms this way, for identical results. The set is small
        // by construction, since it is one text id per system the caller may
        // see, so passing it back down as an array is cheap.
        let visible: Vec<String> = c
            .query("select system_id from parallax.visible_systems($1)", &[&grant_id])
            .await
            .map_err(q)?
            .iter()
            .map(|r| r.get(0))
            .collect();

        if visible.is_empty() {
            // A stage with nothing in it is a legitimate answer, and skipping
            // three round trips to discover that is worth the branch.
            return Ok(VaultSnapshot {
                systems: Vec::new(),
                settings: self.settings_for(&c, grant_id).await?,
            });
        }

        let rows = c
            .query(
                "select s.id, s.hostname, s.ra, s.dec, s.dist_pc, s.teff, s.radius_sun,
                        s.mass_sun, s.spectype, s.vmag, s.source, s.origin, s.imperial_name,
                        s.arm, s.population, s.notes, s.version
                   from parallax.systems s
                  where s.id = any($1)
                  order by s.origin desc, s.dist_pc nulls last, s.hostname",
                &[&visible],
            )
            .await
            .map_err(q)?;

        let mut systems: Vec<VersionedSystem> = rows
            .iter()
            .map(|r| VersionedSystem { system: system_from_row(r), version: r.get("version") })
            .collect();
        let index: std::collections::HashMap<String, usize> = systems
            .iter()
            .enumerate()
            .map(|(i, s)| (s.system.id.clone(), i))
            .collect();

        for row in c
            .query(
                "select p.system_id, p.name, p.orbsmax, p.orbper, p.rade, p.bmasse, p.eqt,
                        p.orbeccen, p.disc_year, p.disc_method, p.disc_facility
                   from parallax.planets p
                  where p.system_id = any($1)
                  order by p.system_id, p.ordinal",
                &[&visible],
            )
            .await
            .map_err(q)?
        {
            let sid: String = row.get("system_id");
            if let Some(&i) = index.get(&sid) {
                systems[i].system.planets.push(planet_from_row(&row));
            }
        }

        for row in c
            .query(
                "select r.system_id, r.planet_name, r.imperial_name, r.population,
                        r.continents, r.notes
                   from parallax.planet_records r
                  where r.system_id = any($1)",
                &[&visible],
            )
            .await
            .map_err(q)?
        {
            let sid: String = row.get("system_id");
            if let Some(&i) = index.get(&sid) {
                systems[i].system.planet_records.insert(
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

        let settings = self.settings_for(&c, grant_id).await?;
        Ok(VaultSnapshot { systems, settings })
    }

    /// Where this grant was last looking. Per grant, so a shared link shares a
    /// cursor and two separate links do not fight over one.
    async fn settings_for(
        &self,
        c: &deadpool_postgres::Object,
        grant_id: &str,
    ) -> R<Settings> {
        Ok(match c
            .query_opt(
                "select selected, compare, focus_planet from parallax.user_settings
                  where user_id = $1",
                &[&grant_id],
            )
            .await
            .map_err(q)?
        {
            Some(row) => Settings {
                selected: row.get("selected"),
                compare: row.get("compare"),
                focus_planet: row.get("focus_planet"),
            },
            None => Settings::default(),
        })
    }

    /* ---------------------------------------------------------- writes -- */

    /// Refresh archive columns and planets. Never touches a dossier.
    pub async fn upsert_system(&self, sys: &System) -> R<i32> {
        let mut c = self.client().await?;
        let tx = c.transaction().await.map_err(q)?;
        let arm = arm_to_sql(sys.record.arm);
        let source = source_to_sql(sys.source);
        let params: [&(dyn ToSql + Sync); 16] = [
            &sys.id, &sys.hostname, &sys.ra, &sys.dec, &sys.dist_pc, &sys.teff,
            &sys.radius_sun, &sys.mass_sun, &sys.spectype, &sys.vmag, &source, &sys.origin,
            &sys.record.imperial_name, &arm, &sys.record.population, &sys.record.notes,
        ];
        let row = tx
            .query_one(
                "insert into parallax.systems
                   (id, hostname, ra, dec, dist_pc, teff, radius_sun, mass_sun, spectype,
                    vmag, source, origin, imperial_name, arm, population, notes)
                 values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
                 on conflict (id) do update set
                   hostname = excluded.hostname, ra = excluded.ra, dec = excluded.dec,
                   dist_pc = excluded.dist_pc, teff = excluded.teff,
                   radius_sun = excluded.radius_sun, mass_sun = excluded.mass_sun,
                   spectype = excluded.spectype, vmag = excluded.vmag,
                   source = excluded.source
                 returning version",
                &params,
            )
            .await
            .map_err(q)?;

        tx.execute("delete from parallax.planets where system_id = $1", &[&sys.id])
            .await
            .map_err(q)?;
        for (i, p) in sys.planets.iter().enumerate() {
            let ordinal = i as i32;
            let params: [&(dyn ToSql + Sync); 12] = [
                &sys.id, &p.name, &ordinal, &p.orbsmax, &p.orbper, &p.rade, &p.bmasse,
                &p.eqt, &p.orbeccen, &p.disc_year, &p.disc_method, &p.disc_facility,
            ];
            tx.execute(
                "insert into parallax.planets
                   (system_id, name, ordinal, orbsmax, orbper, rade, bmasse, eqt,
                    orbeccen, disc_year, disc_method, disc_facility)
                 values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                &params,
            )
            .await
            .map_err(q)?;
        }
        tx.commit().await.map_err(q)?;
        Ok(row.get("version"))
    }

    /// Apply only the fields the client actually changed.
    ///
    /// Each column is written as `case when $flag then $value else column end`,
    /// so an untouched column keeps whatever another operator put there between
    /// this client's read and its write.
    pub async fn patch_record(&self, id: &str, patch: &RecordPatch) -> R<i32> {
        if patch.is_empty() {
            return self.current_version(id).await;
        }
        let c = self.client().await?;

        let set_name = patch.imperial_name.is_some();
        let name = patch.imperial_name.clone().unwrap_or_default();
        let set_arm = patch.arm.is_some();
        let arm = arm_to_sql(patch.arm.flatten());
        let set_pop = patch.population.is_some();
        let pop = patch.population.clone().unwrap_or_default();
        let set_notes = patch.notes.is_some();
        let notes = patch.notes.clone().unwrap_or_default();
        let expected = patch.expected_version;

        let params: [&(dyn ToSql + Sync); 10] = [
            &id, &set_name, &name, &set_arm, &arm, &set_pop, &pop, &set_notes, &notes,
            &expected,
        ];
        let row = c
            .query_opt(
                "update parallax.systems set
                   imperial_name = case when $2 then $3 else imperial_name end,
                   arm           = case when $4 then $5 else arm end,
                   population    = case when $6 then $7 else population end,
                   notes         = case when $8 then $9 else notes end
                 where id = $1 and ($10::int is null or version = $10)
                 returning version",
                &params,
            )
            .await
            .map_err(q)?;

        match row {
            Some(r) => Ok(r.get("version")),
            None => Err(self.explain_miss(id).await),
        }
    }

    pub async fn patch_planet_record(
        &self,
        id: &str,
        planet: &str,
        patch: &PlanetRecordPatch,
    ) -> R<i32> {
        let c = self.client().await?;
        let set_name = patch.imperial_name.is_some();
        let name = patch.imperial_name.clone().unwrap_or_default();
        let set_pop = patch.population.is_some();
        let pop = patch.population.clone().unwrap_or_default();
        let set_cont = patch.continents.is_some();
        let cont = patch.continents.clone().unwrap_or_default();
        let set_notes = patch.notes.is_some();
        let notes = patch.notes.clone().unwrap_or_default();

        let params: [&(dyn ToSql + Sync); 10] = [
            &id, &planet, &set_name, &name, &set_pop, &pop, &set_cont, &cont, &set_notes,
            &notes,
        ];
        let row = c
            .query_opt(
                "insert into parallax.planet_records
                   (system_id, planet_name, imperial_name, population, continents, notes)
                 values ($1, $2,
                         case when $3 then $4 else '' end,
                         case when $5 then $6 else '' end,
                         case when $7 then $8 else '' end,
                         case when $9 then $10 else '' end)
                 on conflict (system_id, planet_name) do update set
                   imperial_name = case when $3 then $4 else parallax.planet_records.imperial_name end,
                   population    = case when $5 then $6 else parallax.planet_records.population end,
                   continents    = case when $7 then $8 else parallax.planet_records.continents end,
                   notes         = case when $9 then $10 else parallax.planet_records.notes end
                 returning version",
                &params,
            )
            .await
            .map_err(q)?;
        row.map(|r| r.get("version")).ok_or(RepoError::NotFound)
    }

    pub async fn delete_system(&self, id: &str) -> R<()> {
        let n = self
            .client()
            .await?
            .execute("delete from parallax.systems where id = $1", &[&id])
            .await
            .map_err(q)?;
        if n == 0 {
            return Err(RepoError::NotFound);
        }
        Ok(())
    }

    pub async fn save_settings(&self, user: &str, s: &Settings) -> R<()> {
        let params: [&(dyn ToSql + Sync); 4] =
            [&user, &s.selected, &s.compare, &s.focus_planet];
        self.client()
            .await?
            .execute(
                "insert into parallax.user_settings
                   (user_id, selected, compare, focus_planet)
                 values ($1,$2,$3,$4)
                 on conflict (user_id) do update set
                   selected = excluded.selected,
                   compare = excluded.compare,
                   focus_planet = excluded.focus_planet,
                   updated_at = now()",
                &params,
            )
            .await
            .map_err(q)?;
        Ok(())
    }

    /// Load the shipped neighbourhood, dossiers included.
    ///
    /// Server-side because with several clients starting at once, each seeding
    /// independently would race. Returns how many systems were written, or
    /// `None` if the vault already had content and `force` was not set.
    pub async fn seed(&self, force: bool) -> R<Option<usize>> {
        let existing: i64 = self
            .client()
            .await?
            .query_one("select count(*) from parallax.systems", &[])
            .await
            .map_err(q)?
            .get(0);
        if existing > 0 && !force {
            return Ok(None);
        }

        let seeds = crate::core::seed::seed_systems();
        for sys in &seeds {
            self.upsert_system(sys).await?;
            // The archive path deliberately ignores the dossier, so seed
            // dossiers are written through the patch path.
            self.patch_record(
                &sys.id,
                &RecordPatch {
                    imperial_name: Some(sys.record.imperial_name.clone()),
                    arm: Some(sys.record.arm),
                    population: Some(sys.record.population.clone()),
                    notes: Some(sys.record.notes.clone()),
                    expected_version: None,
                },
            )
            .await?;
            for (name, rec) in &sys.planet_records {
                self.patch_planet_record(
                    &sys.id,
                    name,
                    &PlanetRecordPatch {
                        imperial_name: Some(rec.imperial_name.clone()),
                        population: Some(rec.population.clone()),
                        continents: Some(rec.continents.clone()),
                        notes: Some(rec.notes.clone()),
                        expected_version: None,
                    },
                )
                .await?;
            }
        }
        Ok(Some(seeds.len()))
    }

    /// Drop everything. Test support.
    /* ------------------------------------------------------------ grants */

    /// Resolve a token digest to a live grant. Expiry and revocation are
    /// applied by `parallax.live_grant`, not here.
    pub async fn grant_by_token_hash(&self, token_hash: &str) -> R<Option<Grant>> {
        let c = self.client().await?;
        let row = c
            .query_opt(&format!("select {GRANT_COLS} from parallax.live_grant($1)"), &[&token_hash])
            .await
            .map_err(q)?;
        Ok(row.as_ref().and_then(grant_from_row))
    }

    /// The stage a visitor gets before presenting anything.
    pub async fn anonymous_grant(&self) -> R<Grant> {
        let c = self.client().await?;
        let row = c
            .query_opt(&format!("select {GRANT_COLS} from parallax.grants where id = $1"), &[&Grant::ANONYMOUS_ID])
            .await
            .map_err(q)?;
        // A vault whose anonymous row was deleted should show nothing rather
        // than everything, so the fallback is the empty stage.
        Ok(row.as_ref().and_then(grant_from_row).unwrap_or_else(|| Grant {
            id: Grant::ANONYMOUS_ID.into(),
            label: "Anyone with the address".into(),
            capability: Capability::Read,
            scope: Scope::Systems { ids: Vec::new() },
            expires_at: None,
            revoked: false,
            use_count: 0,
            last_used_at: None,
        }))
    }

    /// How stale `last_used_at` may get before it is worth a write.
    ///
    /// Sixty seconds, because this exists so a host can tell a live link from a
    /// forgotten one — a resolution no finer than that is useful, and anything
    /// finer costs a write per request.
    const TOUCH_INTERVAL: &'static str = "60 seconds";

    /// Record that a link was picked up.
    ///
    /// Sampled, not counted. Updating on every request meant an UPDATE per read,
    /// which put a row lock on a single `grants` row: every holder of a shared
    /// link then serialised on it, and measured throughput was flat at ~120
    /// req/s whether one client was connected or thirty-two. The `where` clause
    /// below turns that into at most one write per grant per minute.
    ///
    /// `use_count` therefore counts *minutes in which the link was used*, not
    /// requests. That is the honest reading of the number and is what the host
    /// actually wants to know.
    pub async fn touch_grant(&self, id: &str) -> R<()> {
        let c = self.client().await?;
        c.execute(
            &format!(
                "update parallax.grants
                    set last_used_at = now(), use_count = use_count + 1
                  where id = $1
                    and (last_used_at is null
                         or last_used_at < now() - interval '{}')",
                Self::TOUCH_INTERVAL
            ),
            &[&id],
        )
        .await
        .map_err(q)?;
        Ok(())
    }

    pub async fn list_grants(&self) -> R<Vec<Grant>> {
        let c = self.client().await?;
        let rows = c
            .query(&format!("select {GRANT_COLS} from parallax.grants order by created_at"), &[])
            .await
            .map_err(q)?;
        Ok(rows.iter().filter_map(grant_from_row).collect())
    }

    /// Mint a link. The plaintext token is returned exactly once and is not
    /// recoverable afterwards, because only its digest is stored.
    pub async fn mint_grant(
        &self,
        label: &str,
        capability: Capability,
        scope: &Scope,
        expires_in_days: Option<i32>,
    ) -> R<MintedGrant> {
        scope
            .validate(capability)
            .map_err(|e| RepoError::Query(e.to_string()))?;

        let token = mint_token();
        let hash = hash_token(&token);
        let id = format!("g-{}", &hash[..16]);
        let ids: Vec<String> = scope.system_ids().to_vec();

        let c = self.client().await?;
        let sql = format!(
            "insert into parallax.grants
                   (id, token_hash, label, capability, scope_kind, scope_tag, scope_systems,
                    expires_at)
                 values ($1,$2,$3,$4,$5,$6,$7,
                         case when $8::int is null then null
                              else now() + make_interval(days => $8::int) end)
                 returning {GRANT_COLS}"
        );
        let row = c
            .query_one(
                sql.as_str(),
                &[
                    &id,
                    &hash,
                    &label,
                    &capability.as_str(),
                    &scope.kind(),
                    &scope.tag(),
                    &ids,
                    &expires_in_days,
                ],
            )
            .await
            .map_err(q)?;

        let grant = grant_from_row(&row)
            .ok_or_else(|| RepoError::Query("minted an unreadable grant".into()))?;
        Ok(MintedGrant { grant, token })
    }

    /// Revocation takes effect on the next request: `live_grant` filters on it.
    pub async fn revoke_grant(&self, id: &str) -> R<bool> {
        if id == Grant::ANONYMOUS_ID {
            // Revoking it would lock the public out with no way back through
            // the API. Narrow the anonymous scope instead.
            return Err(RepoError::Query(
                "the anonymous grant cannot be revoked; edit its scope instead".into(),
            ));
        }
        let c = self.client().await?;
        let n = c
            .execute(
                "update parallax.grants set revoked_at = now()
                  where id = $1 and revoked_at is null",
                &[&id],
            )
            .await
            .map_err(q)?;
        Ok(n > 0)
    }

    /// Is this system inside the grant's stage? Write paths call this so a
    /// scoped link cannot edit a system it cannot see.
    pub async fn grant_can_see(&self, grant_id: &str, system_id: &str) -> R<bool> {
        let c = self.client().await?;
        let row = c
            .query_one(
                "select exists (select 1 from parallax.visible_systems($1) where system_id = $2)",
                &[&grant_id, &system_id],
            )
            .await
            .map_err(q)?;
        Ok(row.get(0))
    }

    /// Create or replace a grant with a known id and no token.
    ///
    /// Test support only: real grants are minted with a random token whose
    /// digest is stored, and there is deliberately no way to choose the id or
    /// to create one that is reachable without a token.
    #[doc(hidden)]
    pub async fn upsert_grant_for_test(
        &self,
        id: &str,
        capability: Capability,
        scope: &Scope,
    ) -> R<()> {
        let c = self.client().await?;
        let ids: Vec<String> = scope.system_ids().to_vec();
        c.execute(
            "insert into parallax.grants
               (id, token_hash, label, capability, scope_kind, scope_tag, scope_systems)
             values ($1, '', 'test', $2, $3, $4, $5)
             on conflict (id) do update set
               capability = excluded.capability,
               scope_kind = excluded.scope_kind,
               scope_tag = excluded.scope_tag,
               scope_systems = excluded.scope_systems,
               revoked_at = null",
            &[&id, &capability.as_str(), &scope.kind(), &scope.tag(), &ids],
        )
        .await
        .map_err(q)?;
        Ok(())
    }

    /// Run arbitrary SQL. Test support only.
    #[doc(hidden)]
    pub async fn raw_batch(&self, sql: &str) -> R<()> {
        let c = self.client().await?;
        c.batch_execute(sql).await.map_err(q)?;
        Ok(())
    }

    /// Call the access boundary with a deliberately hostile `search_path`.
    /// Test support only; the application never sets one.
    #[doc(hidden)]
    pub async fn visible_ids_with_search_path(
        &self,
        grant_id: &str,
        search_path: &str,
    ) -> R<Vec<String>> {
        let c = self.client().await?;
        c.batch_execute(&format!("set search_path to {search_path}"))
            .await
            .map_err(q)?;
        let rows = c
            .query(
                "select system_id from parallax.visible_systems($1) order by 1",
                &[&grant_id],
            )
            .await
            .map_err(q)?;
        // Leave the connection as it was found: it goes back to the pool.
        c.batch_execute("reset search_path").await.map_err(q)?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn truncate_all(&self) -> R<()> {
        self.client()
            .await?
            .batch_execute(
                "truncate parallax.planet_records, parallax.planets, parallax.systems cascade;
                 truncate parallax.user_settings;
                 delete from parallax.grants where id <> 'anonymous';
                 insert into parallax.user_settings (user_id) values ('local')
                   on conflict do nothing;",
            )
            .await
            .map_err(q)
    }

    async fn current_version(&self, id: &str) -> R<i32> {
        self.client()
            .await?
            .query_opt("select version from parallax.systems where id = $1", &[&id])
            .await
            .map_err(q)?
            .map(|r| r.get("version"))
            .ok_or(RepoError::NotFound)
    }

    /// An update that matched nothing is either a gone row or a stale version.
    /// Telling them apart is the difference between "reload" and "it's deleted".
    async fn explain_miss(&self, id: &str) -> RepoError {
        match self.current_version(id).await {
            Ok(current) => RepoError::Conflict { current },
            Err(_) => RepoError::NotFound,
        }
    }
}

/* -------------------------------------------------------------- mapping -- */

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
        planet_records: Default::default(),
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
