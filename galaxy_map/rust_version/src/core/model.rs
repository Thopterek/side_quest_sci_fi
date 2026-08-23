//! The vault's data model.
//!
//! Two layers live side by side on every object and never mix:
//!   * **Archive** fields come from the NASA Exoplanet Archive and are replaced
//!     wholesale on refresh.
//!   * **Record** fields are the operator's own dossier and are never touched
//!     by a refresh.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::astro::{
    axis_from_period, habitable_zone, to_xyz, HabitableZone, Vec3,
};

/// Lowercase, hyphen-separated identifier. `"GJ 1061"` becomes `"gj-1061"`.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/* -------------------------------------------------------------------- arm -- */

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    Local,
    Perseus,
    Sagittarius,
    Scutum,
    Norma,
    Outer,
}

impl Arm {
    pub const ALL: [Arm; 6] =
        [Arm::Local, Arm::Perseus, Arm::Sagittarius, Arm::Scutum, Arm::Norma, Arm::Outer];

    pub fn name(self) -> &'static str {
        match self {
            Arm::Local => "Orion–Cygnus",
            Arm::Perseus => "Perseus",
            Arm::Sagittarius => "Sagittarius–Carina",
            Arm::Scutum => "Scutum–Centaurus",
            Arm::Norma => "Norma",
            Arm::Outer => "Outer",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Arm::Local => "Local Arm",
            Arm::Perseus => "outward",
            Arm::Sagittarius => "inward",
            Arm::Scutum => "inner",
            Arm::Norma => "innermost",
            Arm::Outer => "rim",
        }
    }

    /// Ink colour on the light plate theme.
    pub fn plate_rgb(self) -> [u8; 3] {
        match self {
            Arm::Local => [0x2E, 0x7D, 0x6B],
            Arm::Perseus => [0x8E, 0x32, 0x18],
            Arm::Sagittarius => [0x6B, 0x4C, 0x9A],
            Arm::Scutum => [0x9C, 0x6B, 0x12],
            Arm::Norma => [0x3F, 0x6E, 0x1F],
            Arm::Outer => [0x1C, 0x5C, 0x9E],
        }
    }

    /// Emissive colour on the dark negative theme.
    pub fn negative_rgb(self) -> [u8; 3] {
        match self {
            Arm::Local => [0x63, 0xC7, 0xB0],
            Arm::Perseus => [0xE3, 0x8B, 0x6C],
            Arm::Sagittarius => [0xAE, 0x93, 0xDD],
            Arm::Scutum => [0xE3, 0xB8, 0x5E],
            Arm::Norma => [0x95, 0xCF, 0x64],
            Arm::Outer => [0x74, 0xB2, 0xEA],
        }
    }
}

/* ----------------------------------------------------------------- source -- */

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Live from the archive.
    Nasa,
    /// Shipped with the binary; refresh replaces it.
    Seed,
    /// Sol, measured from the inside.
    Reference,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Nasa => "NASA pscomppars",
            Source::Seed => "seed values",
            Source::Reference => "reference",
        }
    }
}

/* ---------------------------------------------------------------- records -- */

/// The operator's dossier on a system.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub imperial_name: String,
    pub arm: Option<Arm>,
    pub population: String,
    pub notes: String,
}

/// The operator's dossier on one planet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanetRecord {
    pub imperial_name: String,
    pub population: String,
    /// Free text, comma separated.
    pub continents: String,
    pub notes: String,
}

impl PlanetRecord {
    pub fn continent_count(&self) -> usize {
        self.continents.split(',').filter(|s| !s.trim().is_empty()).count()
    }
}

/* ----------------------------------------------------------------- planet -- */

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Planet {
    pub name: String,
    /// Semi-major axis, AU. `None` when the archive only published a period.
    pub orbsmax: Option<f64>,
    /// Orbital period, days.
    pub orbper: Option<f64>,
    /// Radius, Earth radii.
    pub rade: Option<f64>,
    /// Best mass estimate, Earth masses.
    pub bmasse: Option<f64>,
    /// Measured equilibrium temperature, K.
    pub eqt: Option<f64>,
    pub orbeccen: Option<f64>,
    pub disc_year: Option<i64>,
    pub disc_method: Option<String>,
    pub disc_facility: Option<String>,
}

impl Planet {
    /// Name with the host prefix stripped: `"GJ 1061 d"` becomes `"d"`.
    pub fn short_name(&self, hostname: &str) -> String {
        let s = self.name.strip_prefix(hostname).unwrap_or(&self.name).trim();
        if s.is_empty() { self.name.clone() } else { s.to_string() }
    }
}

/* ----------------------------------------------------------------- system -- */

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct System {
    pub id: String,
    pub hostname: String,
    /// Right ascension, degrees.
    pub ra: f64,
    /// Declination, degrees.
    pub dec: f64,
    /// Distance, parsecs.
    pub dist_pc: Option<f64>,
    pub teff: Option<f64>,
    /// Stellar radius, R☉.
    pub radius_sun: Option<f64>,
    /// Stellar mass, M☉.
    pub mass_sun: Option<f64>,
    pub spectype: Option<String>,
    pub vmag: Option<f64>,
    pub planets: Vec<Planet>,
    pub record: Record,
    pub planet_records: BTreeMap<String, PlanetRecord>,
    pub source: Source,
    /// True only for Sol, which anchors the cube and draws a reticle.
    pub origin: bool,
}

impl Default for System {
    fn default() -> Self {
        System {
            id: String::new(),
            hostname: String::new(),
            ra: 0.0,
            dec: 0.0,
            dist_pc: None,
            teff: None,
            radius_sun: None,
            mass_sun: None,
            spectype: None,
            vmag: None,
            planets: Vec::new(),
            record: Record::default(),
            planet_records: BTreeMap::new(),
            source: Source::Nasa,
            origin: false,
        }
    }
}

impl System {
    /// What the operator called it, falling back to the catalog name.
    pub fn display_name(&self) -> &str {
        if self.record.imperial_name.trim().is_empty() {
            &self.hostname
        } else {
            &self.record.imperial_name
        }
    }

    pub fn position(&self) -> Vec3 {
        to_xyz(self.ra, self.dec, self.dist_pc.unwrap_or(0.0))
    }

    pub fn hz(&self) -> Option<HabitableZone> {
        habitable_zone(self.radius_sun, self.teff)
    }

    /// Semi-major axis of a planet, falling back to Kepler's third law.
    /// The `bool` is true when the value was derived rather than measured, and
    /// the UI marks those with an asterisk.
    pub fn axis_of(&self, p: &Planet) -> Option<(f64, bool)> {
        match p.orbsmax {
            Some(a) if a > 0.0 => Some((a, false)),
            _ => axis_from_period(p.orbper, self.mass_sun).map(|a| (a, true)),
        }
    }

    /// Every planet that can be placed on a ring, with its axis.
    pub fn drawable_planets(&self) -> Vec<(usize, &Planet, f64)> {
        self.planets
            .iter()
            .enumerate()
            .filter_map(|(i, p)| self.axis_of(p).map(|(a, _)| (i, p, a)))
            .collect()
    }

    /// Innermost and outermost drawable axis, in AU. Falls back to `(1.0, 1.0)`
    /// so callers never have to special-case an empty system.
    pub fn axis_range(&self) -> (f64, f64) {
        let axes: Vec<f64> = self.drawable_planets().iter().map(|(_, _, a)| *a).collect();
        if axes.is_empty() {
            return (1.0, 1.0);
        }
        let lo = axes.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = axes.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    }

    pub fn planet_record(&self, name: &str) -> PlanetRecord {
        self.planet_records.get(name).cloned().unwrap_or_default()
    }

    pub fn planet_record_mut(&mut self, name: &str) -> &mut PlanetRecord {
        self.planet_records.entry(name.to_string()).or_default()
    }

    /// Every note on the system and its planets, for tag and link scanning.
    pub fn all_notes(&self) -> String {
        let mut s = self.record.notes.clone();
        for r in self.planet_records.values() {
            s.push(' ');
            s.push_str(&r.notes);
        }
        s
    }

    /// Replace archive fields, keep the dossier. This is the single rule that
    /// makes "Refresh" safe to press.
    pub fn merge_archive_from(&mut self, fresh: System) {
        let record = std::mem::take(&mut self.record);
        let planet_records = std::mem::take(&mut self.planet_records);
        let origin = self.origin;
        *self = System { record, planet_records, origin, ..fresh };
    }
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_stable_and_url_safe() {
        assert_eq!(slug("GJ 1061"), "gj-1061");
        assert_eq!(slug("TRAPPIST-1"), "trappist-1");
        assert_eq!(slug("eps Eri"), "eps-eri");
        assert_eq!(slug("  Sol  "), "sol");
        assert_eq!(slug("Kepler-186"), "kepler-186");
    }

    fn gj1061() -> System {
        System {
            id: "gj-1061".into(),
            hostname: "GJ 1061".into(),
            ra: 53.9955,
            dec: -44.5119,
            dist_pc: Some(3.670),
            teff: Some(2953.0),
            radius_sun: Some(0.156),
            mass_sun: Some(0.120),
            planets: vec![
                Planet { name: "GJ 1061 b".into(), orbsmax: Some(0.021), orbper: Some(3.204), rade: Some(1.04), ..Default::default() },
                Planet { name: "GJ 1061 c".into(), orbsmax: Some(0.035), orbper: Some(6.689), rade: Some(1.18), ..Default::default() },
                Planet { name: "GJ 1061 d".into(), orbsmax: Some(0.054), orbper: Some(13.03), rade: Some(1.16), ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn axis_range_spans_the_system() {
        let (lo, hi) = gj1061().axis_range();
        assert_eq!((lo, hi), (0.021, 0.054));
    }

    #[test]
    fn empty_system_has_a_safe_axis_range() {
        assert_eq!(System::default().axis_range(), (1.0, 1.0));
    }

    #[test]
    fn missing_axis_falls_back_to_kepler_and_is_flagged() {
        let mut s = gj1061();
        s.planets[0].orbsmax = None;
        let (a, derived) = s.axis_of(&s.planets[0]).unwrap();
        assert!(derived, "a Kepler-derived axis must be marked");
        // 3.204 d around 0.12 M☉ lands close to the published 0.021 AU.
        assert!((a - 0.021).abs() < 0.002, "got {a}");
        let (_, measured) = s.axis_of(&s.planets[1]).unwrap();
        assert!(!measured, "a published axis must not be marked derived");
    }

    #[test]
    fn short_names_strip_the_host_prefix() {
        let s = gj1061();
        assert_eq!(s.planets[2].short_name(&s.hostname), "d");
        let odd = Planet { name: "Earth".into(), ..Default::default() };
        assert_eq!(odd.short_name("Sol"), "Earth");
    }

    #[test]
    fn refresh_replaces_archive_fields_but_never_the_dossier() {
        let mut mine = gj1061();
        mine.record = Record {
            imperial_name: "Kestrel Reach".into(),
            arm: Some(Arm::Perseus),
            population: "4.1 billion".into(),
            notes: "#capital".into(),
        };
        mine.planet_records.insert(
            "GJ 1061 d".into(),
            PlanetRecord { imperial_name: "Anvil".into(), continents: "North, South".into(), ..Default::default() },
        );

        let mut fresh = gj1061();
        fresh.dist_pc = Some(3.671); // archive nudged the parallax
        fresh.teff = Some(2960.0);
        fresh.source = Source::Nasa;

        mine.merge_archive_from(fresh);

        assert_eq!(mine.dist_pc, Some(3.671), "archive fields must update");
        assert_eq!(mine.teff, Some(2960.0));
        assert_eq!(mine.record.imperial_name, "Kestrel Reach", "dossier must survive");
        assert_eq!(mine.record.arm, Some(Arm::Perseus));
        assert_eq!(mine.planet_record("GJ 1061 d").imperial_name, "Anvil");
    }

    #[test]
    fn display_name_prefers_the_imperial_name() {
        let mut s = gj1061();
        assert_eq!(s.display_name(), "GJ 1061");
        s.record.imperial_name = "  ".into();
        assert_eq!(s.display_name(), "GJ 1061", "whitespace is not a name");
        s.record.imperial_name = "Kestrel Reach".into();
        assert_eq!(s.display_name(), "Kestrel Reach");
    }

    #[test]
    fn continents_are_counted_not_just_stored() {
        let r = PlanetRecord { continents: "Africa, Asia, , Europe".into(), ..Default::default() };
        assert_eq!(r.continent_count(), 3);
        assert_eq!(PlanetRecord::default().continent_count(), 0);
    }

    #[test]
    fn position_length_is_the_catalogued_distance() {
        assert!((gj1061().position().length() - 3.670).abs() < 1e-9);
    }

    #[test]
    fn records_round_trip_through_serde() {
        let mut s = gj1061();
        s.record.arm = Some(Arm::Scutum);
        s.planet_record_mut("GJ 1061 b").population = "none".into();
        let json = serde_json::to_string(&s).unwrap();
        let back: System = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
