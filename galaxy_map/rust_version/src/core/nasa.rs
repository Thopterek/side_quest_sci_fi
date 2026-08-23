//! NASA Exoplanet Archive access.
//!
//! Query construction and response parsing live here and are pure, so they can
//! be tested without a network. The actual HTTP call is left to the UI layer,
//! which has to differ between native and wasm anyway.
//!
//! Table: `pscomppars`, the Planetary Systems Composite Parameters set — the
//! same one behind the archive's own catalog pages, which is why a system
//! pulled here matches what NASA shows for it.

use std::collections::BTreeMap;

use serde_json::Value;

use super::model::{slug, Planet, Record, Source, System};

pub const TAP_ENDPOINT: &str = "https://exoplanetarchive.ipac.caltech.edu/TAP/sync";

/// Columns requested. Kept in one place so the query and the parser cannot
/// drift apart.
pub const COLUMNS: &[&str] = &[
    "pl_name", "hostname", "sy_snum", "sy_pnum", "ra", "dec", "sy_dist",
    "pl_orbsmax", "pl_orbper", "pl_rade", "pl_bmasse", "pl_eqt", "pl_orbeccen",
    "st_teff", "st_rad", "st_mass", "st_spectype", "sy_vmag",
    "discoverymethod", "disc_year", "disc_facility",
];

#[derive(Debug)]
pub enum NasaError {
    Http(String),
    BadJson(String),
    Empty,
}

impl std::fmt::Display for NasaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NasaError::Http(m) => write!(f, "could not reach the archive: {m}"),
            NasaError::BadJson(m) => write!(f, "the archive returned something unexpected: {m}"),
            NasaError::Empty => write!(f, "no rows matched"),
        }
    }
}

impl std::error::Error for NasaError {}

/// ADQL escaping: in string literals a single quote is doubled.
fn escape(term: &str) -> String {
    term.replace('\'', "''").to_uppercase()
}

/// Fuzzy search across host star and planet names.
pub fn search_adql(term: &str) -> String {
    let t = escape(term);
    format!(
        "select {} from pscomppars where upper(hostname) like '%{}%' or upper(pl_name) like '%{}%'",
        COLUMNS.join(","),
        t,
        t
    )
}

/// Exact host lookup, used by Refresh.
pub fn host_adql(hostname: &str) -> String {
    format!(
        "select {} from pscomppars where upper(hostname) = '{}'",
        COLUMNS.join(","),
        escape(hostname)
    )
}

/// Percent-encode everything that is not unreserved, so the ADQL survives the
/// query string intact.
pub fn url_encode(s: &str) -> String {
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

pub fn query_url(adql: &str) -> String {
    format!("{TAP_ENDPOINT}?query={}&format=json", url_encode(adql))
}

/* ---------------------------------------------------------------- parsing -- */

fn num(row: &Value, key: &str) -> Option<f64> {
    match row.get(key)? {
        Value::Number(n) => n.as_f64(),
        // The archive occasionally serialises a figure as a string.
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn int(row: &Value, key: &str) -> Option<i64> {
    num(row, key).map(|v| v as i64)
}

fn text(row: &Value, key: &str) -> Option<String> {
    match row.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Group flat `pscomppars` rows, which are one row per *planet*, into systems.
pub fn parse_rows(body: &str) -> Result<Vec<System>, NasaError> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| NasaError::BadJson(e.to_string()))?;
    let rows = parsed
        .as_array()
        .ok_or_else(|| NasaError::BadJson("expected a JSON array of rows".into()))?;

    // Insertion-ordered grouping, so results appear in the archive's own order.
    let mut order: Vec<String> = Vec::new();
    let mut by_host: BTreeMap<String, System> = BTreeMap::new();

    for row in rows {
        let Some(hostname) = text(row, "hostname") else { continue };
        let id = slug(&hostname);

        let entry = by_host.entry(id.clone()).or_insert_with(|| {
            order.push(id.clone());
            System {
                id: id.clone(),
                hostname: hostname.clone(),
                ra: num(row, "ra").unwrap_or(0.0),
                dec: num(row, "dec").unwrap_or(0.0),
                dist_pc: num(row, "sy_dist"),
                teff: num(row, "st_teff"),
                radius_sun: num(row, "st_rad"),
                mass_sun: num(row, "st_mass"),
                spectype: text(row, "st_spectype"),
                vmag: num(row, "sy_vmag"),
                planets: Vec::new(),
                record: Record::default(),
                planet_records: BTreeMap::new(),
                source: Source::Nasa,
                origin: false,
            }
        });

        let Some(name) = text(row, "pl_name") else { continue };
        if entry.planets.iter().any(|p| p.name == name) {
            continue; // defensive: pscomppars should be one row per planet
        }
        entry.planets.push(Planet {
            name,
            orbsmax: num(row, "pl_orbsmax"),
            orbper: num(row, "pl_orbper"),
            rade: num(row, "pl_rade"),
            bmasse: num(row, "pl_bmasse"),
            eqt: num(row, "pl_eqt"),
            orbeccen: num(row, "pl_orbeccen"),
            disc_year: int(row, "disc_year"),
            disc_method: text(row, "discoverymethod"),
            disc_facility: text(row, "disc_facility"),
        });
    }

    if by_host.is_empty() {
        return Err(NasaError::Empty);
    }

    let mut out: Vec<System> = order
        .into_iter()
        .filter_map(|id| by_host.remove(&id))
        .collect();

    // Innermost planet first, so the record panel reads outward.
    for s in &mut out {
        s.planets.sort_by(|a, b| {
            let ka = a.orbsmax.or(a.orbper).unwrap_or(f64::MAX);
            let kb = b.orbsmax.or(b.orbper).unwrap_or(f64::MAX);
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    Ok(out)
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
      {"pl_name":"GJ 1061 d","hostname":"GJ 1061","sy_snum":1,"sy_pnum":3,
       "ra":53.9955,"dec":-44.5119,"sy_dist":3.67,
       "pl_orbsmax":0.054,"pl_orbper":13.03,"pl_rade":1.16,"pl_bmasse":1.64,
       "pl_eqt":null,"pl_orbeccen":0.05,"st_teff":2953.0,"st_rad":0.156,
       "st_mass":0.12,"st_spectype":"M5.5 V","sy_vmag":13.03,
       "discoverymethod":"Radial Velocity","disc_year":2020,"disc_facility":"La Silla Observatory"},
      {"pl_name":"GJ 1061 b","hostname":"GJ 1061","sy_snum":1,"sy_pnum":3,
       "ra":53.9955,"dec":-44.5119,"sy_dist":3.67,
       "pl_orbsmax":0.021,"pl_orbper":3.204,"pl_rade":1.04,"pl_bmasse":1.11,
       "pl_eqt":null,"pl_orbeccen":0.05,"st_teff":2953.0,"st_rad":0.156,
       "st_mass":0.12,"st_spectype":"M5.5 V","sy_vmag":13.03,
       "discoverymethod":"Radial Velocity","disc_year":2020,"disc_facility":"La Silla Observatory"},
      {"pl_name":"Ross 128 b","hostname":"Ross 128","sy_snum":1,"sy_pnum":1,
       "ra":176.9375,"dec":0.8003,"sy_dist":"3.375",
       "pl_orbsmax":null,"pl_orbper":9.866,"pl_rade":1.11,"pl_bmasse":1.40,
       "pl_eqt":null,"pl_orbeccen":0.12,"st_teff":3192.0,"st_rad":0.1967,
       "st_mass":0.168,"st_spectype":"  ","sy_vmag":11.13,
       "discoverymethod":"Radial Velocity","disc_year":2017,"disc_facility":"La Silla Observatory"}
    ]"#;

    #[test]
    fn adql_escapes_quotes_and_upcases() {
        let q = search_adql("o'brien");
        assert!(q.contains("'%O''BRIEN%'"), "quote not doubled: {q}");
        assert!(!q.contains("';"), "no statement break should survive");
    }

    #[test]
    fn adql_asks_for_every_column_the_parser_reads() {
        let q = search_adql("GJ 1061");
        for c in COLUMNS {
            assert!(q.contains(c), "query is missing {c}");
        }
    }

    #[test]
    fn url_encoding_survives_a_round_trip_of_the_awkward_characters() {
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("x='y'"), "x%3D%27y%27");
        assert_eq!(url_encode("safe-_.~"), "safe-_.~");
        let url = query_url(&search_adql("GJ 1061"));
        assert!(url.starts_with(TAP_ENDPOINT));
        assert!(url.ends_with("&format=json"));
        assert!(!url.contains(' '), "a raw space would break the request");
    }

    #[test]
    fn rows_group_into_systems_by_host() {
        let out = parse_rows(FIXTURE).unwrap();
        assert_eq!(out.len(), 2);
        let gj = &out[0];
        assert_eq!(gj.id, "gj-1061");
        assert_eq!(gj.planets.len(), 2);
        assert_eq!(gj.source, Source::Nasa);
        assert_eq!(gj.dist_pc, Some(3.67));
        assert_eq!(gj.spectype.as_deref(), Some("M5.5 V"));
    }

    #[test]
    fn planets_are_ordered_from_the_inside_out() {
        let out = parse_rows(FIXTURE).unwrap();
        // The fixture lists d before b, deliberately.
        assert_eq!(out[0].planets[0].name, "GJ 1061 b");
        assert_eq!(out[0].planets[1].name, "GJ 1061 d");
    }

    #[test]
    fn nulls_become_none_rather_than_zero() {
        let out = parse_rows(FIXTURE).unwrap();
        assert_eq!(out[0].planets[0].eqt, None, "a null must not read as 0 K");
        assert_eq!(out[1].planets[0].orbsmax, None);
    }

    #[test]
    fn a_missing_axis_is_recovered_from_the_period() {
        let out = parse_rows(FIXTURE).unwrap();
        let ross = &out[1];
        let (a, derived) = ross.axis_of(&ross.planets[0]).unwrap();
        assert!(derived);
        assert!((a - 0.0496).abs() < 0.002, "got {a} AU");
    }

    #[test]
    fn numeric_strings_are_accepted() {
        // sy_dist arrives as "3.375" in the fixture.
        assert_eq!(parse_rows(FIXTURE).unwrap()[1].dist_pc, Some(3.375));
    }

    #[test]
    fn blank_strings_become_none_not_empty_labels() {
        assert_eq!(parse_rows(FIXTURE).unwrap()[1].spectype, None);
    }

    #[test]
    fn parsed_systems_are_immediately_usable() {
        let gj = &parse_rows(FIXTURE).unwrap()[0];
        assert!((gj.position().length() - 3.67).abs() < 1e-9);
        let hz = gj.hz().unwrap();
        assert!(hz.contains(0.054), "planet d should land in the zone");
    }

    #[test]
    fn empty_and_malformed_responses_are_errors_not_panics() {
        assert!(matches!(parse_rows("[]"), Err(NasaError::Empty)));
        assert!(matches!(parse_rows("not json"), Err(NasaError::BadJson(_))));
        assert!(matches!(parse_rows(r#"{"error":"nope"}"#), Err(NasaError::BadJson(_))));
        // A row with no hostname is skipped rather than crashing.
        assert!(matches!(parse_rows(r#"[{"pl_name":"orphan"}]"#), Err(NasaError::Empty)));
    }
}
