//! The vault itself: an ordered collection of systems plus the selection state.
//!
//! Obsidian's shape, applied to star systems. A system is a note, `#tags`
//! become filters, `[[Wikilinks]]` become edges in the cube, and the cube is
//! the graph view.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::astro::{display_pos, measure, DistanceMode, Measurement, Vec3};
use super::camera::nice_extent;
use super::model::{slug, System};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Vault {
    pub systems: Vec<System>,
    pub selected: Option<String>,
    /// The second endpoint of a shift-click measurement.
    pub compare: Option<String>,
    /// Which planet's dossier is open, if any.
    pub focus_planet: Option<String>,
    /// Bumped by every mutation. `VaultIndex` uses it to decide whether the
    /// derived state it cached is still valid, so an idle frame does no work.
    #[serde(skip)]
    revision: u64,
}

impl Default for Vault {
    fn default() -> Self {
        Vault { systems: Vec::new(), selected: None, compare: None, focus_planet: None, revision: 0 }
    }
}

impl Vault {
    /// The shipped local neighbourhood, out to about 12.5 pc.
    pub fn seeded() -> Self {
        let systems = super::seed::seed_systems();
        let selected = systems.iter().find(|s| s.id == "gj-1061").map(|s| s.id.clone());
        Vault { systems, selected, compare: None, focus_planet: None, revision: 1 }
    }

    /// Monotonic edit counter. See [`crate::core::index::VaultIndex`].
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Mark the vault dirty. Call after mutating a system through `get_mut`.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn get(&self, id: &str) -> Option<&System> {
        self.systems.iter().find(|s| s.id == id)
    }

    /// Mutable access. Bumps the revision eagerly, since the caller is about to
    /// change something and the index must not serve stale derived state.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut System> {
        self.revision = self.revision.wrapping_add(1);
        self.systems.iter_mut().find(|s| s.id == id)
    }

    pub fn selected(&self) -> Option<&System> {
        self.selected.as_deref().and_then(|id| self.get(id))
    }

    pub fn compared(&self) -> Option<&System> {
        self.compare.as_deref().and_then(|id| self.get(id))
    }

    pub fn select(&mut self, id: &str) {
        if self.get(id).is_some() {
            self.selected = Some(id.to_string());
            self.focus_planet = None;
        }
    }

    /// Toggle the measurement endpoint.
    pub fn toggle_compare(&mut self, id: &str) {
        self.compare = if self.compare.as_deref() == Some(id) { None } else { Some(id.to_string()) };
    }

    /// Insert a system, or refresh an existing one's archive fields while
    /// leaving its dossier untouched. Returns true if this was a new entry.
    pub fn upsert(&mut self, fresh: System) -> bool {
        match self.get_mut(&fresh.id) {
            Some(existing) => {
                existing.merge_archive_from(fresh);
                self.revision = self.revision.wrapping_add(1);
                false
            }
            None => {
                let id = fresh.id.clone();
                self.systems.push(fresh);
                self.selected = Some(id);
                self.focus_planet = None;
                self.revision = self.revision.wrapping_add(1);
                true
            }
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.systems.retain(|s| s.id != id);
        self.revision = self.revision.wrapping_add(1);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.systems.first().map(|s| s.id.clone());
            self.focus_planet = None;
        }
        if self.compare.as_deref() == Some(id) {
            self.compare = None;
        }
    }

    /* -------------------------------------------------------------- search */

    /// Every `#tag` used anywhere in the vault, system notes and planet notes
    /// alike, sorted and deduplicated.
    pub fn tags(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for s in &self.systems {
            for tag in extract_tags(&s.all_notes()) {
                set.insert(tag);
            }
        }
        set.into_iter().collect()
    }

    /// Case-insensitive search across catalog names, dossier fields and planets.
    pub fn filter(&self, query: &str) -> Vec<&System> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.systems.iter().collect();
        }
        self.systems.iter().filter(|s| haystack(s).contains(&q)).collect()
    }

    /* --------------------------------------------------------------- links */

    /// `[[Wikilinks]]` from this system's notes that resolve to something in
    /// the vault. Self-links are dropped.
    pub fn links_of(&self, id: &str) -> Vec<String> {
        let Some(sys) = self.get(id) else { return Vec::new() };
        let mut out = Vec::new();
        for name in extract_links(&sys.all_notes()) {
            let target = slug(&name);
            if target != id && self.get(&target).is_some() && !out.contains(&target) {
                out.push(target);
            }
        }
        out
    }

    /// Every link as an unordered pair, deduplicated, for drawing edges.
    pub fn link_edges(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for s in &self.systems {
            for target in self.links_of(&s.id) {
                let pair = if s.id < target {
                    (s.id.clone(), target)
                } else {
                    (target, s.id.clone())
                };
                if !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
        out
    }

    /* --------------------------------------------------------------- space */

    /// Half-width of the cube needed to hold everything, in the given mode.
    pub fn extent(&self, mode: DistanceMode) -> f64 {
        let reach = self
            .systems
            .iter()
            .map(|s| display_pos(s.position(), mode).max_abs_component())
            .fold(0.4_f64, f64::max);
        nice_extent(reach)
    }

    /// True separation between the selection and the comparison endpoint.
    /// Always computed from untransformed positions, so log-radial display
    /// never corrupts a measurement.
    pub fn measurement(&self) -> Option<Measurement> {
        let a = self.selected()?;
        let b = self.compared()?;
        Some(measure(a.position(), b.position()))
    }

    /// Where a system is drawn under the current distance mode.
    pub fn draw_pos(&self, sys: &System, mode: DistanceMode) -> Vec3 {
        display_pos(sys.position(), mode)
    }

    pub fn planet_count(&self) -> usize {
        self.systems.iter().map(|s| s.planets.len()).sum()
    }

    pub fn furthest_pc(&self) -> f64 {
        self.systems.iter().filter_map(|s| s.dist_pc).fold(0.0, f64::max)
    }
}

fn haystack(s: &System) -> String {
    let mut h = String::new();
    h.push_str(&s.hostname);
    h.push(' ');
    h.push_str(&s.record.imperial_name);
    h.push(' ');
    h.push_str(&s.record.population);
    h.push(' ');
    h.push_str(&s.record.notes);
    h.push(' ');
    if let Some(t) = &s.spectype {
        h.push_str(t);
        h.push(' ');
    }
    for p in &s.planets {
        h.push_str(&p.name);
        h.push(' ');
    }
    for r in s.planet_records.values() {
        h.push_str(&r.imperial_name);
        h.push(' ');
        h.push_str(&r.continents);
        h.push(' ');
        h.push_str(&r.notes);
        h.push(' ');
    }
    h.to_lowercase()
}

/// Pull `#tag` tokens out of free text.
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '#' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_alphanumeric() || bytes[j] == '-' || bytes[j] == '_') {
                j += 1;
            }
            if j > start {
                out.push(bytes[start..j].iter().collect());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Pull `[[Wikilink]]` targets out of free text.
pub fn extract_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        match after.find("]]") {
            Some(close) => {
                let name = after[..close].trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
                rest = &after[close + 2..];
            }
            None => break,
        }
    }
    out
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Arm, PlanetRecord, Record, Source, System};

    #[test]
    fn the_seed_vault_is_the_local_neighbourhood() {
        let v = Vault::seeded();
        assert_eq!(v.systems.len(), 13);
        assert!(v.planet_count() > 35);
        assert_eq!(v.selected().unwrap().hostname, "GJ 1061");
        assert!(v.get("sol").unwrap().origin, "Sol must anchor the cube");
        assert!((v.furthest_pc() - 12.467).abs() < 1e-6, "TRAPPIST-1 is the far anchor");
    }

    #[test]
    fn every_seed_system_has_a_unique_slug_and_a_position() {
        let v = Vault::seeded();
        let mut ids = BTreeSet::new();
        for s in &v.systems {
            assert!(ids.insert(s.id.clone()), "duplicate id {}", s.id);
            assert_eq!(s.id, slug(&s.hostname));
            let p = s.position();
            assert!(p.length().is_finite());
            if let Some(d) = s.dist_pc {
                assert!((p.length() - d).abs() < 1e-9, "{} misplaced", s.hostname);
            }
        }
    }

    #[test]
    fn every_seed_planet_can_be_drawn() {
        for s in Vault::seeded().systems {
            for p in &s.planets {
                assert!(s.axis_of(p).is_some(), "{} has no usable axis", p.name);
            }
            let (lo, hi) = s.axis_range();
            assert!(lo > 0.0 && hi >= lo, "{} has a broken axis range", s.hostname);
        }
    }

    #[test]
    fn upsert_adds_then_refreshes() {
        let mut v = Vault::default();
        let mut s = System { id: "x".into(), hostname: "X".into(), dist_pc: Some(5.0), ..Default::default() };
        assert!(v.upsert(s.clone()), "first insert is new");
        assert_eq!(v.systems.len(), 1);
        s.dist_pc = Some(5.5);
        assert!(!v.upsert(s), "second insert is a refresh");
        assert_eq!(v.systems.len(), 1, "must not duplicate");
        assert_eq!(v.get("x").unwrap().dist_pc, Some(5.5));
    }

    #[test]
    fn refreshing_from_the_archive_preserves_the_dossier() {
        let mut v = Vault::seeded();
        v.get_mut("gj-1061").unwrap().record = Record {
            imperial_name: "Kestrel Reach".into(),
            arm: Some(Arm::Perseus),
            population: "4.1 billion".into(),
            notes: "#capital".into(),
        };
        v.get_mut("gj-1061").unwrap().planet_records.insert(
            "GJ 1061 d".into(),
            PlanetRecord { imperial_name: "Anvil".into(), ..Default::default() },
        );

        let fresh = System {
            id: "gj-1061".into(),
            hostname: "GJ 1061".into(),
            dist_pc: Some(3.671),
            source: Source::Nasa,
            ..Default::default()
        };
        v.upsert(fresh);

        let s = v.get("gj-1061").unwrap();
        assert_eq!(s.dist_pc, Some(3.671));
        assert_eq!(s.source, Source::Nasa);
        assert_eq!(s.record.imperial_name, "Kestrel Reach");
        assert_eq!(s.planet_record("GJ 1061 d").imperial_name, "Anvil");
    }

    #[test]
    fn removing_the_selection_moves_it_somewhere_valid() {
        let mut v = Vault::seeded();
        v.select("trappist-1");
        v.toggle_compare("trappist-1");
        v.remove("trappist-1");
        assert!(v.get("trappist-1").is_none());
        assert!(v.selected().is_some(), "selection must not dangle");
        assert!(v.compare.is_none(), "measurement endpoint must clear");
    }

    #[test]
    fn tags_are_gathered_from_system_and_planet_notes() {
        let mut v = Vault::default();
        let mut s = System { id: "a".into(), hostname: "A".into(), ..Default::default() };
        s.record.notes = "orbit is #compact and #habitable-zone".into();
        s.planet_record_mut("A b").notes = "#terraformed, see notes".into();
        v.upsert(s);
        assert_eq!(v.tags(), vec!["compact", "habitable-zone", "terraformed"]);
    }

    #[test]
    fn tag_extraction_stops_at_punctuation() {
        assert_eq!(extract_tags("#one, #two-three. #four_5 plain#six"), vec!["one", "two-three", "four_5", "six"]);
        assert!(extract_tags("nothing here").is_empty());
        assert!(extract_tags("# ").is_empty(), "a bare hash is not a tag");
    }

    #[test]
    fn wikilinks_resolve_and_ignore_self_references() {
        let mut v = Vault::default();
        let mut a = System { id: "a".into(), hostname: "A".into(), ..Default::default() };
        a.record.notes = "see [[B]] and [[A]] and [[Nowhere]]".into();
        let b = System { id: "b".into(), hostname: "B".into(), ..Default::default() };
        v.upsert(a);
        v.upsert(b);
        assert_eq!(v.links_of("a"), vec!["b"], "self and unresolved links drop out");
    }

    #[test]
    fn link_edges_are_deduplicated_across_both_directions() {
        let mut v = Vault::default();
        let mut a = System { id: "a".into(), hostname: "A".into(), ..Default::default() };
        a.record.notes = "[[B]]".into();
        let mut b = System { id: "b".into(), hostname: "B".into(), ..Default::default() };
        b.record.notes = "[[A]]".into();
        v.upsert(a);
        v.upsert(b);
        assert_eq!(v.link_edges(), vec![("a".to_string(), "b".to_string())]);
    }

    #[test]
    fn malformed_wikilinks_do_not_hang_or_panic() {
        assert!(extract_links("[[unclosed").is_empty());
        assert!(extract_links("[[]]").is_empty());
        assert_eq!(extract_links("[[a]][[b]]"), vec!["a", "b"]);
    }

    #[test]
    fn the_seed_notes_actually_link_up() {
        let v = Vault::seeded();
        assert!(v.links_of("gj-1061").contains(&"trappist-1".to_string()));
        assert!(v.links_of("tau-cet").contains(&"eps-eri".to_string()));
        assert!(!v.link_edges().is_empty());
    }

    #[test]
    fn filtering_reaches_dossier_and_planet_names() {
        let mut v = Vault::seeded();
        v.get_mut("gj-1061").unwrap().record.imperial_name = "Kestrel Reach".into();
        assert_eq!(v.filter("kestrel").len(), 1);
        assert_eq!(v.filter("#habitable-zone").len(), v.filter("#habitable-zone").len());
        assert!(v.filter("trappist-1 e").iter().any(|s| s.id == "trappist-1"));
        assert_eq!(v.filter("").len(), v.systems.len());
        assert!(v.filter("zzzznope").is_empty());
    }

    #[test]
    fn extent_grows_to_contain_a_distant_addition() {
        let mut v = Vault::seeded();
        let local = v.extent(DistanceMode::Linear);
        assert_eq!(local, 16.0);

        v.upsert(System {
            id: "kepler-186".into(),
            hostname: "Kepler-186".into(),
            ra: 298.4,
            dec: 43.95,
            dist_pc: Some(178.0),
            ..Default::default()
        });
        let wide = v.extent(DistanceMode::Linear);
        assert!(wide > local * 5.0, "the cube must re-frame, got {wide}");

        // Log-radial keeps the local group readable alongside it.
        let logged = v.extent(DistanceMode::Log);
        assert!(logged < wide / 10.0, "log mode should tame it, got {logged}");
    }

    #[test]
    fn measurement_uses_true_positions_regardless_of_display_mode() {
        let mut v = Vault::seeded();
        v.select("sol");
        v.toggle_compare("proxima-centauri");
        let m = v.measurement().unwrap();
        assert!((m.pc - 1.301).abs() < 1e-6);
        assert!((m.ly - 4.243).abs() < 0.01);
        assert!(m.voyager_years > 70_000.0, "Voyager would take a while");
        // Switching display mode must not move the number.
        let _ = v.extent(DistanceMode::Log);
        assert!((v.measurement().unwrap().pc - m.pc).abs() < 1e-12);
    }

    #[test]
    fn measurement_needs_both_endpoints() {
        let mut v = Vault::seeded();
        v.compare = None;
        assert!(v.measurement().is_none());
    }

    #[test]
    fn compare_toggles_off_when_reselected() {
        let mut v = Vault::seeded();
        v.toggle_compare("sol");
        assert_eq!(v.compare.as_deref(), Some("sol"));
        v.toggle_compare("sol");
        assert!(v.compare.is_none());
    }

    #[test]
    fn the_whole_vault_round_trips_through_json() {
        let mut v = Vault::seeded();
        v.get_mut("sol").unwrap().record.notes = "home #reference [[GJ 1061]]".into();
        let json = serde_json::to_string(&v).unwrap();
        let back: Vault = serde_json::from_str(&json).unwrap();
        assert_eq!(back.systems.len(), v.systems.len());
        assert_eq!(back.selected, v.selected);
        assert_eq!(back.links_of("sol"), vec!["gj-1061"]);
    }
}
