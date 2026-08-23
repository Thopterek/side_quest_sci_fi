//! The reusable system view, as geometry rather than pixels.
//!
//! One function, [`layout`], turns a [`System`] into rings and dots at any
//! radius. Everything that draws a system goes through it: the 44 px thumbnails
//! in the vault, the live orreries inside the cube, the hover card, and the
//! 250 px view in the record panel. Detail tiers fall out of the radius, so
//! there is genuinely one implementation rather than four that drift apart.

use super::astro::{orbit_norm, phase_of, OrbitScale, REARTH_IN_AU, RSUN_IN_AU};
use super::model::System;

/// How much of a system to draw. Derived from radius, never passed in by hand.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Rings and dots only.
    Thumb,
    /// Adds the habitable zone band.
    Card,
    /// Adds planet labels and an AU scale bar.
    Full,
}

impl Detail {
    pub fn for_radius(r: f32) -> Detail {
        if r >= 100.0 {
            Detail::Full
        } else if r >= 40.0 {
            Detail::Card
        } else {
            Detail::Thumb
        }
    }
    pub fn shows_hz(self) -> bool {
        self != Detail::Thumb
    }
    pub fn shows_labels(self) -> bool {
        self == Detail::Full
    }
}

/// One planet, placed.
#[derive(Copy, Clone, Debug)]
pub struct Body {
    /// Index into `System::planets`.
    pub index: usize,
    /// Orbit radius in pixels.
    pub orbit_px: f32,
    /// Current true anomaly (circular approximation), radians.
    pub angle: f64,
    /// Drawn radius in pixels. Never physically true; see `exaggeration`.
    pub body_px: f32,
    pub in_hz: bool,
}

impl Body {
    /// Offset from the system centre, in the orbit plane's own basis.
    pub fn offset(&self) -> (f32, f32) {
        (self.orbit_px * self.angle.cos() as f32, self.orbit_px * self.angle.sin() as f32)
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub radius_px: f32,
    pub detail: Detail,
    /// Drawn stellar radius in pixels.
    pub star_px: f32,
    pub bodies: Vec<Body>,
    /// Habitable zone as (inner_px, outer_px), when it is wide enough to draw.
    pub hz_px: Option<(f32, f32)>,
    pub a_min: f64,
    pub a_max: f64,
}

impl Layout {
    /// Factor by which an Earth-sized body is drawn larger than truth at this
    /// size. Surfaced in the UI so the exaggeration is never silent.
    pub fn body_exaggeration(&self) -> f64 {
        let truthful = (REARTH_IN_AU / self.a_max) * self.radius_px as f64;
        let drawn = self.bodies.first().map(|b| b.body_px as f64).unwrap_or(3.0);
        if truthful <= 0.0 {
            f64::INFINITY
        } else {
            drawn / truthful
        }
    }

    /// Width in pixels this system would occupy if drawn at the cube's own
    /// scale. This is the number the honest-scale strip reports.
    pub fn true_width_px(&self, px_per_pc: f64) -> f64 {
        (self.a_max * 2.0 / super::astro::PC_IN_AU) * px_per_pc
    }

    /// Ratio between the innermost and outermost orbit, which is exactly what
    /// log compression hides and the "true" collapse reveals.
    pub fn dynamic_range(&self) -> f64 {
        if self.a_min > 0.0 { self.a_max / self.a_min } else { 1.0 }
    }
}

/// Place every planet in a system.
///
/// * `radius_px` — half-width of the drawing area.
/// * `clock_days` — shared simulation clock; planets move at true relative rates.
/// * `true_mix` — 0 for the selected `scale`, 1 for fully true orbits. The UI
///   tweens this so the collapse to true scale is visible rather than abrupt.
pub fn layout(
    sys: &System,
    radius_px: f32,
    clock_days: f64,
    scale: OrbitScale,
    true_mix: f64,
) -> Layout {
    let detail = Detail::for_radius(radius_px);
    let (a_min, a_max) = sys.axis_range();
    let mix = true_mix.clamp(0.0, 1.0);

    // Blend the chosen scale toward strictly proportional radii.
    let norm = |a: f64| -> f64 {
        let base = orbit_norm(a, a_min, a_max, scale);
        if mix == 0.0 {
            base
        } else {
            base * (1.0 - mix) + (a / a_max) * mix
        }
    };

    let hz = sys.hz();
    let hz_px = match (hz, detail.shows_hz()) {
        (Some(z), true) => {
            let inner = (norm(z.inner) * radius_px as f64) as f32;
            let outer = (norm(z.outer) * radius_px as f64) as f32;
            // Suppress a band too thin to read; better absent than misleading.
            if outer > inner && outer > 1.0 {
                Some((inner, outer))
            } else {
                None
            }
        }
        _ => None,
    };

    let bodies = sys
        .drawable_planets()
        .into_iter()
        .map(|(index, planet, a)| {
            let orbit_px = (norm(a) * radius_px as f64) as f32;

            let angle = match planet.orbper {
                Some(p) if p > 0.0 => {
                    phase_of(index) + (clock_days / p) * std::f64::consts::TAU
                }
                _ => phase_of(index),
            };

            // Symbolic size: disc area encodes radius on a log scale.
            let symbolic = if detail == Detail::Thumb {
                1.6
            } else {
                (2.1 + (planet.rade.unwrap_or(1.0) + 0.4).log10() * 5.2).clamp(2.0, 9.0)
            };
            // Truthful size, for the collapse. Almost always sub-pixel.
            let truthful =
                (planet.rade.unwrap_or(1.0) * REARTH_IN_AU / a_max) * radius_px as f64;
            let body_px = (symbolic * (1.0 - mix) + truthful * mix).max(0.35) as f32;

            Body {
                index,
                orbit_px,
                angle,
                body_px,
                in_hz: hz.map(|z| z.contains(a)).unwrap_or(false),
            }
        })
        .collect();

    // The star is the one body that can be drawn truthfully at full collapse:
    // for a compact M dwarf system it is a real fraction of the inner orbit.
    let star_true = (sys.radius_sun.unwrap_or(0.5) * RSUN_IN_AU / a_max) * radius_px as f64;
    let star_px = if mix > 0.5 {
        star_true.max(1.1) as f32
    } else {
        let sym = 9.0 + (sys.radius_sun.unwrap_or(0.3) + 0.05).log10() * 5.0;
        sym.clamp(2.4, (radius_px * 0.1).max(2.4) as f64) as f32
    };

    Layout { radius_px, detail, star_px, bodies, hz_px, a_min, a_max }
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Planet, System};

    fn gj1061() -> System {
        System {
            hostname: "GJ 1061".into(),
            teff: Some(2953.0),
            radius_sun: Some(0.156),
            mass_sun: Some(0.120),
            planets: vec![
                Planet { name: "b".into(), orbsmax: Some(0.021), orbper: Some(3.204), rade: Some(1.04), ..Default::default() },
                Planet { name: "c".into(), orbsmax: Some(0.035), orbper: Some(6.689), rade: Some(1.18), ..Default::default() },
                Planet { name: "d".into(), orbsmax: Some(0.054), orbper: Some(13.03), rade: Some(1.16), ..Default::default() },
            ],
            ..Default::default()
        }
    }

    fn sol() -> System {
        let axes = [0.387, 0.723, 1.0, 1.524, 5.203, 9.537, 19.19, 30.07];
        let pers = [87.97, 224.7, 365.26, 686.98, 4332.6, 10759.0, 30685.0, 60190.0];
        System {
            hostname: "Sol".into(),
            teff: Some(5772.0),
            radius_sun: Some(1.0),
            mass_sun: Some(1.0),
            planets: axes
                .iter()
                .zip(pers)
                .map(|(&a, p)| Planet {
                    name: format!("p{a}"),
                    orbsmax: Some(a),
                    orbper: Some(p),
                    rade: Some(1.0),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn trappist() -> System {
        let axes = [0.01154, 0.0158, 0.02227, 0.02925, 0.03849, 0.04683, 0.06189];
        let pers = [1.5109, 2.4218, 4.0496, 6.1010, 9.2075, 12.352, 18.773];
        System {
            hostname: "TRAPPIST-1".into(),
            teff: Some(2566.0),
            radius_sun: Some(0.1192),
            mass_sun: Some(0.0898),
            planets: axes
                .iter()
                .zip(pers)
                .enumerate()
                .map(|(i, (&a, p))| Planet {
                    name: format!("TRAPPIST-1 {}", (b'b' + i as u8) as char),
                    orbsmax: Some(a),
                    orbper: Some(p),
                    rade: Some(1.0),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn detail_tiers_come_from_size_alone() {
        // The three call sites, by the radius each actually uses.
        assert_eq!(Detail::for_radius(22.0), Detail::Thumb); // 44 px vault row
        assert_eq!(Detail::for_radius(46.0), Detail::Card); //  92 px hover card
        assert_eq!(Detail::for_radius(104.0), Detail::Full); // 300 px record panel
        assert!(!Detail::Thumb.shows_hz());
        assert!(Detail::Card.shows_hz() && !Detail::Card.shows_labels());
        assert!(Detail::Full.shows_labels());
    }

    #[test]
    fn rings_never_cross_at_any_size_or_scale() {
        for sys in [gj1061(), trappist()] {
            for r in [22.0_f32, 46.0, 125.0, 235.0] {
                for scale in [OrbitScale::Log, OrbitScale::Sqrt, OrbitScale::True] {
                    let l = layout(&sys, r, 0.0, scale, 0.0);
                    let mut prev = -1.0;
                    for b in &l.bodies {
                        assert!(b.orbit_px > prev, "crossed at r={r} {scale:?}");
                        assert!(b.orbit_px <= r + 0.001, "escaped the frame");
                        prev = b.orbit_px;
                    }
                }
            }
        }
    }

    #[test]
    fn planets_move_at_true_relative_rates() {
        // One TRAPPIST-1 year for planet b is 1.5109 days. After exactly that
        // long, b must be back where it started and h must not be.
        let sys = trappist();
        let a = layout(&sys, 235.0, 0.0, OrbitScale::Log, 0.0);
        let b = layout(&sys, 235.0, 1.5109, OrbitScale::Log, 0.0);
        let wrap = |x: f64| x.rem_euclid(std::f64::consts::TAU);
        assert!((wrap(a.bodies[0].angle) - wrap(b.bodies[0].angle)).abs() < 1e-6);
        assert!((wrap(a.bodies[6].angle) - wrap(b.bodies[6].angle)).abs() > 0.4);
    }

    #[test]
    fn the_inner_planet_laps_the_outer_one() {
        let sys = trappist();
        let turns = |i: usize, days: f64| {
            let l = layout(&sys, 235.0, days, OrbitScale::Log, 0.0);
            (l.bodies[i].angle - phase_of(i)) / std::f64::consts::TAU
        };
        let year = 365.0;
        assert!(turns(0, year) > turns(6, year) * 10.0, "b must lap h many times");
    }

    #[test]
    fn true_collapse_scales_with_how_much_was_being_hidden() {
        // The collapse is only dramatic for systems with a wide dynamic range.
        // Sol spans 78x, so Mercury falls a long way inward.
        let compressed = layout(&sol(), 235.0, 0.0, OrbitScale::Log, 0.0);
        let collapsed = layout(&sol(), 235.0, 0.0, OrbitScale::Log, 1.0);
        assert!(
            collapsed.bodies[0].orbit_px < compressed.bodies[0].orbit_px * 0.15,
            "Mercury should fall into the Sun"
        );
        // The outermost planet is the anchor and stays put in every mode.
        let last = collapsed.bodies.len() - 1;
        assert!((collapsed.bodies[last].orbit_px - 235.0).abs() < 0.01);
        // Bodies become invisible, which is the truth about them.
        assert!(collapsed.bodies[0].body_px < 1.0);
    }

    #[test]
    fn a_compact_system_barely_collapses_and_that_is_the_point() {
        // GJ 1061 spans only 2.6x, so log compression was never distorting it
        // much. Pressing "true" should honestly reveal almost no change.
        let sys = gj1061();
        let compressed = layout(&sys, 235.0, 0.0, OrbitScale::Log, 0.0);
        let collapsed = layout(&sys, 235.0, 0.0, OrbitScale::Log, 1.0);
        let ratio = collapsed.bodies[0].orbit_px / compressed.bodies[0].orbit_px;
        assert!((0.9..1.1).contains(&ratio), "expected little movement, got {ratio}");
        assert!(compressed.dynamic_range() < 3.0);
    }

    #[test]
    fn habitable_zone_lands_on_the_right_planet() {
        let l = layout(&gj1061(), 235.0, 0.0, OrbitScale::Log, 0.0);
        assert!(!l.bodies[0].in_hz, "b is too hot");
        assert!(l.bodies[2].in_hz, "d is the temperate one");
        let (inner, outer) = l.hz_px.expect("band should be drawable at this size");
        assert!(outer > inner);
        assert!(l.bodies[2].orbit_px >= inner && l.bodies[2].orbit_px <= outer);
    }

    #[test]
    fn thumbnails_omit_the_habitable_zone_but_keep_the_rings() {
        let l = layout(&gj1061(), 22.0, 0.0, OrbitScale::Log, 0.0);
        assert_eq!(l.detail, Detail::Thumb);
        assert!(l.hz_px.is_none());
        assert_eq!(l.bodies.len(), 3);
    }

    #[test]
    fn dynamic_range_reports_what_compression_hides() {
        let l = layout(&gj1061(), 235.0, 0.0, OrbitScale::Log, 0.0);
        assert!((l.dynamic_range() - 0.054 / 0.021).abs() < 1e-9);
    }

    #[test]
    fn body_exaggeration_is_reported_and_scales_with_the_system() {
        // A compact M dwarf system needs about 15x; the Solar System, spread
        // over 30 AU, needs thousands. Both are surfaced rather than hidden.
        let compact = layout(&gj1061(), 235.0, 0.0, OrbitScale::Log, 0.0);
        let wide = layout(&sol(), 235.0, 0.0, OrbitScale::Log, 0.0);
        assert!((10.0..40.0).contains(&compact.body_exaggeration()));
        assert!(wide.body_exaggeration() > 1000.0);
        assert!(wide.body_exaggeration() > compact.body_exaggeration());
        // At the cube's own scale the whole system is far under one pixel.
        assert!(compact.true_width_px(60.0) < 1e-3);
    }

    #[test]
    fn empty_systems_lay_out_without_panicking() {
        let l = layout(&System::default(), 100.0, 12.0, OrbitScale::Log, 0.4);
        assert!(l.bodies.is_empty());
        assert!(l.star_px > 0.0);
    }
}

