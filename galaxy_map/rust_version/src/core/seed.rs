//! The catalog shipped with the binary: the solar neighbourhood out to about
//! 12.5 pc, plus TRAPPIST-1 as a far anchor.
//!
//! These exist so the vault is never empty and the app is useful with no
//! network. They are marked [`Source::Seed`] and pressing Refresh replaces
//! them with live archive values while keeping any dossier written on top.

use std::collections::BTreeMap;

use super::model::{slug, Arm, Planet, PlanetRecord, Record, Source, System};

/// `(name, semi-major axis AU, period days, radius R⊕, mass M⊕, eccentricity,
///   discovery year, method)`
type Row = (&'static str, f64, f64, f64, f64, f64, Option<i64>, &'static str);

fn planet(r: Row) -> Planet {
    Planet {
        name: r.0.to_string(),
        orbsmax: Some(r.1),
        orbper: Some(r.2),
        rade: Some(r.3),
        bmasse: Some(r.4),
        eqt: None,
        orbeccen: Some(r.5),
        disc_year: r.6,
        disc_method: Some(r.7.to_string()),
        disc_facility: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn system(
    hostname: &str,
    ra: f64,
    dec: f64,
    dist_pc: f64,
    teff: f64,
    radius_sun: f64,
    mass_sun: f64,
    spectype: &str,
    vmag: f64,
    notes: &str,
    rows: &[Row],
) -> System {
    System {
        id: slug(hostname),
        hostname: hostname.to_string(),
        ra,
        dec,
        dist_pc: Some(dist_pc),
        teff: Some(teff),
        radius_sun: Some(radius_sun),
        mass_sun: Some(mass_sun),
        spectype: Some(spectype.to_string()),
        vmag: Some(vmag),
        planets: rows.iter().copied().map(planet).collect(),
        record: Record {
            // Everything within about a kiloparsec of Sol genuinely is in the
            // Orion–Cygnus arm, so seeding that is a fact rather than a guess.
            arm: Some(Arm::Local),
            notes: notes.to_string(),
            ..Default::default()
        },
        planet_records: BTreeMap::new(),
        source: Source::Seed,
        origin: false,
    }
}

pub fn seed_systems() -> Vec<System> {
    let mut out = Vec::with_capacity(13);

    // ---- Sol, the only system measured from the inside ------------------
    let mut sol = system(
        "Sol", 0.0, 0.0, 0.0, 5772.0, 1.0, 1.0, "G2 V", -26.74,
        "Home. The only system here measured from the inside.\n\n\
         Every orbit you already recognise lives in this one, so it works as a \
         ruler for everything else in the vault.\n\n#reference",
        &[
            ("Mercury", 0.3871, 87.97, 0.383, 0.0553, 0.2056, None, "Direct Imaging"),
            ("Venus", 0.7233, 224.70, 0.949, 0.815, 0.0068, None, "Direct Imaging"),
            ("Earth", 1.0000, 365.26, 1.000, 1.000, 0.0167, None, "Direct Imaging"),
            ("Mars", 1.5237, 686.98, 0.532, 0.107, 0.0934, None, "Direct Imaging"),
            ("Jupiter", 5.2034, 4332.6, 11.209, 317.8, 0.0484, None, "Direct Imaging"),
            ("Saturn", 9.5371, 10759.0, 9.449, 95.16, 0.0539, None, "Direct Imaging"),
            ("Uranus", 19.191, 30685.0, 4.007, 14.54, 0.0473, None, "Direct Imaging"),
            ("Neptune", 30.069, 60190.0, 3.883, 17.15, 0.0086, None, "Direct Imaging"),
        ],
    );
    sol.origin = true;
    sol.source = Source::Reference;
    sol.record.imperial_name = "Sol Prime".into();
    sol.record.population = "8.2 billion".into();
    sol.planet_records.insert(
        "Earth".into(),
        PlanetRecord {
            imperial_name: "Terra".into(),
            population: "8.2 billion".into(),
            continents: "Africa, Antarctica, Asia, Australia, Europe, North America, South America".into(),
            notes: "The only confirmed biosphere in the catalog. Everything else here is inference.".into(),
        },
    );
    out.push(sol);

    out.push(system(
        "Proxima Centauri", 217.4289, -62.6795, 1.301, 3042.0, 0.1542, 0.1221, "M5.5 V", 11.13,
        "Closest star to Sol — everything else here is at least half again as far.\n\n\
         Proxima b sits in the habitable zone, but the star flares violently. \
         Compare with [[Ross 128]], which is quiet.\n\n#nearest #habitable-zone",
        &[
            ("Proxima Cen d", 0.02885, 5.122, 0.81, 0.26, 0.04, Some(2022), "Radial Velocity"),
            ("Proxima Cen b", 0.04857, 11.186, 1.03, 1.07, 0.02, Some(2016), "Radial Velocity"),
            ("Proxima Cen c", 1.489, 1928.0, 12.3, 7.0, 0.04, Some(2019), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "GJ 699", 269.4521, 4.6933, 1.828, 3195.0, 0.187, 0.162, "M4 V", 9.51,
        "Barnard's Star. Fastest proper motion of any star — it visibly crosses \
         the sky within a human lifetime.\n\nFour sub-Earth planets, all far too hot.\n\n#nearest",
        &[
            ("GJ 699 d", 0.0188, 2.340, 0.63, 0.19, 0.0, Some(2025), "Radial Velocity"),
            ("GJ 699 b", 0.0229, 3.154, 0.73, 0.30, 0.0, Some(2024), "Radial Velocity"),
            ("GJ 699 c", 0.0274, 4.124, 0.76, 0.34, 0.0, Some(2025), "Radial Velocity"),
            ("GJ 699 e", 0.0381, 6.739, 0.70, 0.26, 0.0, Some(2025), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "GJ 411", 165.8341, 35.9699, 2.546, 3601.0, 0.392, 0.389, "M2 V", 7.52,
        "Lalande 21185. Brightest red dwarf in the northern sky.\n\n\
         Wide spread — 0.08 AU out to 2.9 AU. A good system for seeing what log \
         compression is doing.\n\n#nearest",
        &[
            ("GJ 411 b", 0.079, 12.95, 1.36, 2.69, 0.06, Some(2019), "Radial Velocity"),
            ("GJ 411 c", 2.94, 2946.0, 12.0, 13.6, 0.14, Some(2021), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "eps Eri", 53.2327, -9.4583, 3.216, 5084.0, 0.735, 0.82, "K2 V", 3.73,
        "Epsilon Eridani. Young, active, with a debris disc. Naked-eye visible.\n\n\
         One of the two original targets of Project Ozma in 1960.\n\n#naked-eye #debris-disc",
        &[("eps Eri b", 3.53, 2692.0, 12.4, 254.0, 0.07, Some(2000), "Radial Velocity")],
    ));

    out.push(system(
        "GJ 887", 346.4667, -35.8533, 3.290, 3688.0, 0.47, 0.489, "M2 V", 7.34,
        "Unusually quiet for an M dwarf — few starspots, little flaring. That makes \
         it a good place to look for surviving atmospheres.\n\n#quiet-star",
        &[
            ("GJ 887 b", 0.0681, 9.262, 1.9, 4.2, 0.0, Some(2020), "Radial Velocity"),
            ("GJ 887 c", 0.1194, 21.789, 2.2, 7.6, 0.0, Some(2020), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "GJ 367", 145.2864, -45.7757, 9.413, 3522.0, 0.454, 0.455, "M1 V", 10.15,
        "GJ 367 b is an ultra-short-period iron planet — a bare metallic core on an \
         eight-hour year.\n\n#extreme",
        &[
            ("GJ 367 b", 0.00709, 0.3219, 0.699, 0.633, 0.0, Some(2021), "Transit"),
            ("GJ 367 c", 0.0596, 11.53, 1.5, 4.13, 0.0, Some(2023), "Radial Velocity"),
            ("GJ 367 d", 0.0982, 34.37, 2.0, 6.03, 0.0, Some(2023), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "Ross 128", 176.9375, 0.8003, 3.375, 3192.0, 0.1967, 0.168, "M4 V", 11.13,
        "Quiet M dwarf, temperate planet. The calm alternative to [[Proxima Centauri]].\n\n\
         #habitable-zone #quiet-star",
        &[("Ross 128 b", 0.0496, 9.866, 1.11, 1.40, 0.12, Some(2017), "Radial Velocity")],
    ));

    out.push(system(
        "GJ 1061", 53.9955, -44.5119, 3.670, 2953.0, 0.156, 0.120, "M5.5 V", 13.03,
        "Three super-Earths around a very small, very faint M dwarf.\n\n\
         d sits inside the conservative habitable zone; c is right on the inner edge. \
         The whole system is smaller than Mercury's orbit.\n\n\
         Compare with [[TRAPPIST-1]] — same idea, tighter packing.\n\n#habitable-zone #compact",
        &[
            ("GJ 1061 b", 0.021, 3.204, 1.04, 1.11, 0.05, Some(2020), "Radial Velocity"),
            ("GJ 1061 c", 0.035, 6.689, 1.18, 1.74, 0.03, Some(2020), "Radial Velocity"),
            ("GJ 1061 d", 0.054, 13.03, 1.16, 1.64, 0.05, Some(2020), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "YZ Cet", 17.9938, -16.9964, 3.712, 3151.0, 0.168, 0.142, "M4.5 V", 12.07,
        "Three planets inside 0.03 AU. First exoplanet with a possible detected \
         magnetic field, via radio emission.\n\n#compact",
        &[
            ("YZ Cet b", 0.01634, 2.021, 0.93, 0.70, 0.06, Some(2017), "Radial Velocity"),
            ("YZ Cet c", 0.02156, 3.060, 1.05, 1.14, 0.04, Some(2017), "Radial Velocity"),
            ("YZ Cet d", 0.02851, 4.656, 1.03, 1.09, 0.06, Some(2017), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "GJ 273", 111.8523, 5.2255, 3.786, 3382.0, 0.293, 0.290, "M3.5 V", 9.87,
        "Luyten's Star. A METI transmission was aimed here in 2017; it arrives in 2029.\n\n\
         #habitable-zone",
        &[
            ("GJ 273 c", 0.036, 4.723, 1.05, 1.18, 0.17, Some(2017), "Radial Velocity"),
            ("GJ 273 b", 0.09110, 18.650, 1.51, 2.89, 0.10, Some(2017), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "tau Cet", 26.0170, -15.9375, 3.603, 5344.0, 0.793, 0.783, "G8 V", 3.50,
        "The nearest single Sun-like star. Naked-eye visible. Thick debris disc, so \
         probably a heavy bombardment environment.\n\n\
         The other original Project Ozma target, alongside [[eps Eri]].\n\n\
         #naked-eye #sun-like #debris-disc",
        &[
            ("tau Cet g", 0.133, 20.00, 1.2, 1.75, 0.06, Some(2017), "Radial Velocity"),
            ("tau Cet h", 0.243, 49.41, 1.2, 1.83, 0.23, Some(2017), "Radial Velocity"),
            ("tau Cet e", 0.538, 162.87, 1.8, 3.93, 0.18, Some(2017), "Radial Velocity"),
            ("tau Cet f", 1.334, 636.13, 1.8, 3.93, 0.16, Some(2017), "Radial Velocity"),
        ],
    ));

    out.push(system(
        "TRAPPIST-1", 346.6266, -5.0414, 12.467, 2566.0, 0.1192, 0.0898, "M8 V", 18.80,
        "Seven Earth-sized transiting planets, all inside 0.07 AU. The whole system \
         would fit inside Mercury's orbit several times over.\n\n\
         The far anchor of this vault — nearly four times further out than anything \
         else here.\n\n#compact #habitable-zone #transit",
        &[
            ("TRAPPIST-1 b", 0.01154, 1.5109, 1.116, 1.374, 0.006, Some(2016), "Transit"),
            ("TRAPPIST-1 c", 0.01580, 2.4218, 1.097, 1.308, 0.007, Some(2016), "Transit"),
            ("TRAPPIST-1 d", 0.02227, 4.0496, 0.788, 0.388, 0.008, Some(2016), "Transit"),
            ("TRAPPIST-1 e", 0.02925, 6.1010, 0.920, 0.692, 0.005, Some(2017), "Transit"),
            ("TRAPPIST-1 f", 0.03849, 9.2075, 1.045, 1.039, 0.010, Some(2017), "Transit"),
            ("TRAPPIST-1 g", 0.04683, 12.352, 1.129, 1.321, 0.002, Some(2017), "Transit"),
            ("TRAPPIST-1 h", 0.06189, 18.773, 0.755, 0.326, 0.006, Some(2017), "Transit"),
        ],
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astro::PC_IN_LY;

    #[test]
    fn distances_match_the_published_parallaxes() {
        let s = seed_systems();
        let find = |h: &str| s.iter().find(|x| x.hostname == h).unwrap().dist_pc.unwrap();
        assert!((find("Proxima Centauri") * PC_IN_LY - 4.24).abs() < 0.02);
        assert!((find("GJ 699") * PC_IN_LY - 5.96).abs() < 0.03);
        assert!((find("TRAPPIST-1") * PC_IN_LY - 40.66).abs() < 0.1);
    }

    #[test]
    fn sol_is_the_only_origin_and_the_only_reference() {
        let s = seed_systems();
        assert_eq!(s.iter().filter(|x| x.origin).count(), 1);
        assert_eq!(s.iter().filter(|x| x.source == Source::Reference).count(), 1);
        let sol = s.iter().find(|x| x.origin).unwrap();
        assert_eq!(sol.dist_pc, Some(0.0));
        assert_eq!(sol.planets.len(), 8);
    }

    #[test]
    fn every_seed_system_is_assigned_the_local_arm() {
        // Factually correct for everything inside ~1 kpc.
        for s in seed_systems() {
            assert_eq!(s.record.arm, Some(Arm::Local), "{}", s.hostname);
        }
    }

    #[test]
    fn earth_is_prefilled_as_a_worked_example_of_the_dossier() {
        let s = seed_systems();
        let sol = s.iter().find(|x| x.origin).unwrap();
        let earth = sol.planet_record("Earth");
        assert_eq!(earth.imperial_name, "Terra");
        assert_eq!(earth.continent_count(), 7);
    }

    #[test]
    fn seed_planets_are_ordered_outward() {
        for s in seed_systems() {
            let axes: Vec<f64> = s.planets.iter().filter_map(|p| p.orbsmax).collect();
            assert!(axes.windows(2).all(|w| w[0] < w[1]), "{} is out of order", s.hostname);
        }
    }

    #[test]
    fn the_stated_habitable_zone_claims_in_the_notes_hold_up() {
        let s = seed_systems();
        let gj = s.iter().find(|x| x.hostname == "GJ 1061").unwrap();
        let hz = gj.hz().unwrap();
        assert!(hz.contains(0.054), "the note claims d is temperate");
        assert!(!hz.contains(0.021), "the note claims b is not");

        let sol = s.iter().find(|x| x.origin).unwrap();
        assert!(sol.hz().unwrap().contains(1.0), "Earth calibrates the whole scheme");
    }
}
