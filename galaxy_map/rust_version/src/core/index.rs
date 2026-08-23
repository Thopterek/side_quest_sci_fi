//! Derived state, computed once per change rather than once per frame.
//!
//! The cube and the vault list are immediate-mode: everything they read is
//! recomputed sixty times a second. Several of those reads were doing real
//! work — four trig calls per system for its position, a fresh lowercase
//! haystack string per system for the filter, an O(n²) wikilink resolution with
//! a string allocation per candidate edge, and a `BTreeSet` rebuild for tags.
//!
//! None of it changes between frames unless the vault does. [`VaultIndex`]
//! caches all of it behind a revision counter, so a frame that changes nothing
//! costs nothing.

use crate::core::astro::{display_pos, habitable_zone, DistanceMode, HabitableZone, Vec3};
use crate::core::camera::nice_extent;
use crate::core::model::{slug, System};
use crate::core::vault::{extract_links, extract_tags, Vault};

/// Everything derived from one system that the render loop wants.
#[derive(Clone, Debug)]
pub struct SystemIndex {
    /// True position, parsecs. Used for measurement.
    pub position: Vec3,
    /// Position under the current [`DistanceMode`]. Used for drawing.
    pub display: Vec3,
    pub a_min: f64,
    pub a_max: f64,
    pub hz: Option<HabitableZone>,
    /// Lowercased, pre-joined search haystack.
    pub haystack: String,
}

#[derive(Clone, Debug, Default)]
pub struct VaultIndex {
    systems: Vec<SystemIndex>,
    tags: Vec<String>,
    /// Edges as index pairs rather than id strings, so the cube can look up
    /// endpoints without hashing or comparing strings per frame.
    edges: Vec<(usize, usize)>,
    extent: f64,
    /// The `(revision, mode)` this index was built for.
    built_for: Option<(u64, DistanceMode)>,
}

impl VaultIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild only if the vault or the distance mode has changed.
    /// Returns `true` if work was actually done.
    pub fn sync(&mut self, vault: &Vault, mode: DistanceMode) -> bool {
        if self.built_for == Some((vault.revision(), mode)) {
            return false;
        }
        self.rebuild(vault, mode);
        self.built_for = Some((vault.revision(), mode));
        true
    }

    fn rebuild(&mut self, vault: &Vault, mode: DistanceMode) {
        self.systems.clear();
        self.systems.reserve(vault.systems.len());

        let mut reach: f64 = 0.4;
        for sys in &vault.systems {
            let position = sys.position();
            let display = display_pos(position, mode);
            reach = reach.max(display.max_abs_component());
            let (a_min, a_max) = sys.axis_range();
            self.systems.push(SystemIndex {
                position,
                display,
                a_min,
                a_max,
                hz: habitable_zone(sys.radius_sun, sys.teff),
                haystack: build_haystack(sys),
            });
        }
        self.extent = nice_extent(reach);

        // Tags: one pass, sorted and deduplicated in place rather than through
        // a BTreeSet allocation per frame.
        self.tags.clear();
        for sys in &vault.systems {
            collect_tags(sys, &mut self.tags);
        }
        self.tags.sort_unstable();
        self.tags.dedup();

        // Links: resolve names to indices once. Was O(n²) with an allocation
        // per candidate; now one slug allocation per literal link written.
        self.edges.clear();
        let mut notes = String::new();
        for (i, sys) in vault.systems.iter().enumerate() {
            notes.clear();
            append_notes(sys, &mut notes);
            for name in extract_links(&notes) {
                let target = slug(&name);
                if let Some(j) = vault.systems.iter().position(|s| s.id == target) {
                    if i == j {
                        continue;
                    }
                    let pair = if i < j { (i, j) } else { (j, i) };
                    if !self.edges.contains(&pair) {
                        self.edges.push(pair);
                    }
                }
            }
        }
    }

    pub fn get(&self, i: usize) -> Option<&SystemIndex> {
        self.systems.get(i)
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn edges(&self) -> &[(usize, usize)] {
        &self.edges
    }

    pub fn extent(&self) -> f64 {
        self.extent
    }

    pub fn len(&self) -> usize {
        self.systems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }

    /// Indices of systems matching a filter, using the cached haystacks.
    /// `out` is reused across frames so the filter allocates nothing.
    pub fn filter_into(&self, query: &str, out: &mut Vec<usize>) {
        out.clear();
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            out.extend(0..self.systems.len());
            return;
        }
        for (i, s) in self.systems.iter().enumerate() {
            if s.haystack.contains(&q) {
                out.push(i);
            }
        }
    }
}

fn append_notes(sys: &System, out: &mut String) {
    out.push_str(&sys.record.notes);
    for r in sys.planet_records.values() {
        out.push(' ');
        out.push_str(&r.notes);
    }
}

fn collect_tags(sys: &System, out: &mut Vec<String>) {
    out.extend(extract_tags(&sys.record.notes));
    for r in sys.planet_records.values() {
        out.extend(extract_tags(&r.notes));
    }
}

fn build_haystack(sys: &System) -> String {
    let mut h = String::with_capacity(128);
    let mut push = |s: &str| {
        h.push_str(s);
        h.push(' ');
    };
    push(&sys.hostname);
    push(&sys.record.imperial_name);
    push(&sys.record.population);
    push(&sys.record.notes);
    if let Some(t) = &sys.spectype {
        push(t);
    }
    for p in &sys.planets {
        push(&p.name);
    }
    for r in sys.planet_records.values() {
        push(&r.imperial_name);
        push(&r.continents);
        push(&r.notes);
    }
    h.make_ascii_lowercase();
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Record, System};

    fn big_vault(n: usize) -> Vault {
        let mut v = Vault::seeded();
        for i in 0..n {
            let hostname = format!("Synthetic {i}");
            v.upsert(System {
                id: slug(&hostname),
                hostname,
                ra: (i as f64 * 7.3) % 360.0,
                dec: ((i as f64 * 3.1) % 160.0) - 80.0,
                dist_pc: Some(1.0 + (i % 90) as f64),
                teff: Some(3000.0 + (i % 30) as f64 * 100.0),
                radius_sun: Some(0.3),
                mass_sun: Some(0.3),
                record: Record { notes: format!("#bulk [[GJ 1061]] entry {i}"), ..Default::default() },
                ..Default::default()
            });
        }
        v
    }

    #[test]
    fn the_index_matches_what_the_vault_computes_directly() {
        let vault = Vault::seeded();
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);

        assert_eq!(idx.tags(), vault.tags().as_slice());
        assert_eq!(idx.extent(), vault.extent(DistanceMode::Linear));

        for (i, sys) in vault.systems.iter().enumerate() {
            let e = idx.get(i).unwrap();
            assert_eq!(e.position, sys.position());
            assert_eq!((e.a_min, e.a_max), sys.axis_range());
        }

        // Edge sets must agree, once index pairs are mapped back to ids.
        let mut from_index: Vec<(String, String)> = idx
            .edges()
            .iter()
            .map(|&(a, b)| (vault.systems[a].id.clone(), vault.systems[b].id.clone()))
            .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
            .collect();
        let mut direct = vault.link_edges();
        from_index.sort();
        direct.sort();
        assert_eq!(from_index, direct);
    }

    #[test]
    fn filtering_through_the_index_matches_the_direct_filter() {
        let vault = Vault::seeded();
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);
        let mut out = Vec::new();

        for query in ["", "gj", "#habitable-zone", "trappist", "ZZZZ", "Terra"] {
            idx.filter_into(query, &mut out);
            let direct: Vec<String> =
                vault.filter(query).into_iter().map(|s| s.id.clone()).collect();
            let via: Vec<String> = out.iter().map(|&i| vault.systems[i].id.clone()).collect();
            assert_eq!(via, direct, "diverged on {query:?}");
        }
    }

    #[test]
    fn a_frame_that_changes_nothing_rebuilds_nothing() {
        let vault = Vault::seeded();
        let mut idx = VaultIndex::new();
        assert!(idx.sync(&vault, DistanceMode::Linear), "first sync must build");
        for _ in 0..120 {
            assert!(!idx.sync(&vault, DistanceMode::Linear), "steady state must be free");
        }
    }

    #[test]
    fn editing_the_vault_invalidates_the_index() {
        let mut vault = Vault::seeded();
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);

        vault.get_mut("gj-1061").unwrap().record.imperial_name = "Kestrel Reach".into();
        assert!(idx.sync(&vault, DistanceMode::Linear), "an edit must invalidate");

        let mut out = Vec::new();
        idx.filter_into("kestrel", &mut out);
        assert_eq!(out.len(), 1, "the cached haystack must have been rebuilt");
    }

    #[test]
    fn changing_distance_mode_invalidates_the_index() {
        let vault = Vault::seeded();
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);
        let linear = idx.get(12).unwrap().display;
        assert!(idx.sync(&vault, DistanceMode::Log));
        let logged = idx.get(12).unwrap().display;
        assert!(logged.length() < linear.length(), "log mode must compress");
        // True positions are mode-independent and must not have moved.
        assert_eq!(idx.get(12).unwrap().position.length(), linear.length());
    }

    #[test]
    fn the_index_scales_to_a_large_vault() {
        let vault = big_vault(500);
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);
        assert_eq!(idx.len(), vault.systems.len());
        // Every synthetic system links to GJ 1061, so the edge count should be
        // one per synthetic entry plus the seed vault's own links.
        assert!(idx.edges().len() >= 500);
        let mut out = Vec::new();
        idx.filter_into("#bulk", &mut out);
        assert_eq!(out.len(), 500);
    }
}
