//! Pure astronomy. No rendering, no I/O, no dependencies beyond `std`.
//!
//! Every quantity here is in a named unit and every approximation is documented,
//! because the whole point of Parallax is that the reader can tell which numbers
//! are measured and which are inferred.

use serde::{Deserialize, Serialize};

/* ------------------------------------------------------------- constants -- */

/// Astronomical units in one parsec.
pub const PC_IN_AU: f64 = 206_264.806;
/// Light years in one parsec.
pub const PC_IN_LY: f64 = 3.261_563_8;
/// Solar radii expressed in AU.
pub const RSUN_IN_AU: f64 = 0.004_650_47;
/// Earth radii expressed in AU.
pub const REARTH_IN_AU: f64 = 4.263_52e-5;
/// Speed of light, km/s.
pub const C_KMS: f64 = 299_792.458;
/// Voyager 1 heliocentric speed, km/s.
pub const VOYAGER_KMS: f64 = 17.0;
/// Solar effective temperature, K.
pub const TEFF_SUN: f64 = 5772.0;

/* ------------------------------------------------------------------ vec3 -- */

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    pub fn scale(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }

    pub fn max_abs_component(self) -> f64 {
        self.x.abs().max(self.y.abs()).max(self.z.abs())
    }
}

/// Right ascension / declination / distance to equatorial Cartesian parsecs,
/// Sun at the origin, +Z toward the north celestial pole.
///
/// `ra` and `dec` are degrees, `dist` is parsecs.
pub fn to_xyz(ra: f64, dec: f64, dist: f64) -> Vec3 {
    let a = ra.to_radians();
    let d = dec.to_radians();
    Vec3::new(dist * d.cos() * a.cos(), dist * d.cos() * a.sin(), dist * d.sin())
}

/// True three-dimensional separation between two systems, in parsecs.
pub fn separation(a: Vec3, b: Vec3) -> f64 {
    a.sub(b).length()
}

/* -------------------------------------------------------- distance modes -- */

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMode {
    /// Radius drawn exactly as measured.
    Linear,
    /// Radius replaced by `ln(1 + r)`. Direction is untouched, so bearings stay
    /// exact while a system at 200 pc is pulled into frame beside one at 3 pc.
    Log,
}

impl DistanceMode {
    pub fn label(self) -> &'static str {
        match self {
            DistanceMode::Linear => "true",
            DistanceMode::Log => "log-radial",
        }
    }
    pub fn is_true(self) -> bool {
        matches!(self, DistanceMode::Linear)
    }
}

/// Where a system is *drawn*. Measurement always uses the untransformed vector.
pub fn display_pos(v: Vec3, mode: DistanceMode) -> Vec3 {
    match mode {
        DistanceMode::Linear => v,
        DistanceMode::Log => {
            let r = v.length();
            if r < 1e-9 {
                Vec3::ZERO
            } else {
                v.scale((1.0 + r).ln() / r)
            }
        }
    }
}

/* ---------------------------------------------------------- habitability -- */

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HabitableZone {
    /// Bolometric luminosity in solar units.
    pub l_sun: f64,
    /// Inner edge, AU (runaway greenhouse, S = 1.10 S⊕).
    pub inner: f64,
    /// Outer edge, AU (maximum greenhouse, S = 0.53 S⊕).
    pub outer: f64,
}

impl HabitableZone {
    pub fn contains(&self, a_au: f64) -> bool {
        a_au >= self.inner && a_au <= self.outer
    }
}

/// Conservative habitable zone from stellar radius (R☉) and effective
/// temperature (K), via `L = R²(T/T☉)⁴` and Kopparapu-style flux limits.
pub fn habitable_zone(radius_sun: Option<f64>, teff: Option<f64>) -> Option<HabitableZone> {
    let (r, t) = (radius_sun?, teff?);
    if r <= 0.0 || t <= 0.0 {
        return None;
    }
    let l = r * r * (t / TEFF_SUN).powi(4);
    Some(HabitableZone { l_sun: l, inner: (l / 1.10).sqrt(), outer: (l / 0.53).sqrt() })
}

/// Kepler's third law, used only when the archive has a period but no axis.
/// `period` in days, `star_mass` in M☉, result in AU.
pub fn axis_from_period(period_days: Option<f64>, star_mass: Option<f64>) -> Option<f64> {
    let (p, m) = (period_days?, star_mass?);
    if p <= 0.0 || m <= 0.0 {
        return None;
    }
    Some((m * (p / 365.25).powi(2)).cbrt())
}

/// Stellar flux at a given orbit, in Earth units.
pub fn insolation(l_sun: f64, a_au: f64) -> Option<f64> {
    if a_au <= 0.0 {
        return None;
    }
    Some(l_sun / (a_au * a_au))
}

/// Equilibrium temperature in K, assuming Bond albedo 0.3 and even
/// redistribution. Only used when the archive has no measured value.
pub fn equilibrium_temp(l_sun: f64, a_au: f64) -> Option<f64> {
    if a_au <= 0.0 || l_sun <= 0.0 {
        return None;
    }
    Some(255.0 * l_sun.powf(0.25) / a_au.sqrt())
}

/// NASA-style size bucket from radius in Earth radii.
pub fn planet_class(rade: Option<f64>) -> &'static str {
    match rade {
        None => "unclassified",
        Some(r) if r < 1.25 => "Terrestrial",
        Some(r) if r < 2.0 => "Super-Earth",
        Some(r) if r < 6.0 => "Neptune-like",
        Some(_) => "Gas giant",
    }
}

/* ----------------------------------------------------------- orbit scale -- */

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrbitScale {
    /// Radius proportional to `log a`. Legible for any dynamic range.
    Log,
    /// Radius proportional to `√a`. Mildly compressed.
    Sqrt,
    /// Radius proportional to `a`. Honest, and usually unreadable.
    True,
}

impl OrbitScale {
    pub fn label(self) -> &'static str {
        match self {
            OrbitScale::Log => "log",
            OrbitScale::Sqrt => "√",
            OrbitScale::True => "true",
        }
    }
    pub fn is_true(self) -> bool {
        matches!(self, OrbitScale::True)
    }
}

/// Map a semi-major axis onto 0..1 of the drawing radius.
///
/// Guaranteed monotonic in `a` and inside `0..=1` for `a` in `a_min..=a_max`,
/// which is what keeps orbit rings from crossing.
pub fn orbit_norm(a: f64, a_min: f64, a_max: f64, scale: OrbitScale) -> f64 {
    if a <= 0.0 || a_max <= 0.0 {
        return 0.0;
    }
    match scale {
        OrbitScale::True => a / a_max,
        _ if (a_max - a_min).abs() < f64::EPSILON => 0.62,
        OrbitScale::Sqrt => {
            let lo = a_min * 0.55;
            ((a - lo).max(0.0)).sqrt() / (a_max - lo).sqrt() * 0.86 + 0.1
        }
        OrbitScale::Log => {
            let lo = (a_min * 0.55).ln();
            let hi = (a_max * 1.25).ln();
            ((a.ln() - lo) / (hi - lo)) * 0.9 + 0.08
        }
    }
}

/// Stable, irrational-ratio starting angle so systems never line up.
pub fn phase_of(index: usize) -> f64 {
    (index as f64 * 2.399_963_229) % std::f64::consts::TAU
}

/* ---------------------------------------------------------------- colour -- */

const TEFF_STOPS: [(f64, [u8; 3]); 9] = [
    (2300.0, [255, 122, 74]),
    (3000.0, [255, 150, 88]),
    (3900.0, [255, 194, 137]),
    (5200.0, [255, 226, 184]),
    (5900.0, [255, 246, 232]),
    (6600.0, [242, 241, 255]),
    (8000.0, [214, 226, 255]),
    (12000.0, [168, 195, 255]),
    (30000.0, [141, 175, 255]),
];

/// Approximate blackbody appearance for a stellar effective temperature.
pub fn teff_rgb(teff: Option<f64>) -> [u8; 3] {
    let t = teff.unwrap_or(4000.0).clamp(1800.0, 40_000.0);
    for w in TEFF_STOPS.windows(2) {
        let (ta, ca) = w[0];
        let (tb, cb) = w[1];
        if t >= ta && t <= tb {
            let k = (t - ta) / (tb - ta);
            return [
                (ca[0] as f64 + (cb[0] as f64 - ca[0] as f64) * k) as u8,
                (ca[1] as f64 + (cb[1] as f64 - ca[1] as f64) * k) as u8,
                (ca[2] as f64 + (cb[2] as f64 - ca[2] as f64) * k) as u8,
            ];
        }
    }
    if t < TEFF_STOPS[0].0 { TEFF_STOPS[0].1 } else { TEFF_STOPS[8].1 }
}

/// Blend toward near-black. On the light "plate" theme a star reads as an ink
/// deposit rather than an emitter, so temperature colours are darkened.
pub fn ink_blend(c: [u8; 3], k: f64) -> [u8; 3] {
    const INK: [u8; 3] = [22, 24, 26];
    [
        (c[0] as f64 * (1.0 - k) + INK[0] as f64 * k) as u8,
        (c[1] as f64 * (1.0 - k) + INK[1] as f64 * k) as u8,
        (c[2] as f64 * (1.0 - k) + INK[2] as f64 * k) as u8,
    ]
}

/* ----------------------------------------------------------- measurement -- */

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Measurement {
    pub pc: f64,
    pub ly: f64,
    pub au: f64,
    /// Travel time at Voyager 1's speed, in years.
    pub voyager_years: f64,
    /// Travel time at light speed, in years.
    pub light_years_time: f64,
}

pub fn measure(a: Vec3, b: Vec3) -> Measurement {
    let pc = separation(a, b);
    let ly = pc * PC_IN_LY;
    Measurement {
        pc,
        ly,
        au: pc * PC_IN_AU,
        voyager_years: ly / (VOYAGER_KMS / C_KMS),
        light_years_time: ly,
    }
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn xyz_round_trips_to_the_catalogued_distance() {
        // The length of the position vector must reproduce sy_dist exactly,
        // otherwise every measurement in the app is quietly wrong.
        for &(ra, dec, d) in &[
            (217.4289, -62.6795, 1.301),   // Proxima Centauri
            (269.4521, 4.6933, 1.828),     // Barnard's Star
            (53.9955, -44.5119, 3.670),    // GJ 1061
            (346.6266, -5.0414, 12.467),   // TRAPPIST-1
        ] {
            assert!(approx(to_xyz(ra, dec, d).length(), d, 1e-9));
        }
    }

    #[test]
    fn separation_matches_published_values() {
        // eps Eri to tau Cet is about 5.4 ly in the literature.
        let e = to_xyz(53.2327, -9.4583, 3.216);
        let t = to_xyz(26.0170, -15.9375, 3.603);
        let ly = separation(e, t) * PC_IN_LY;
        assert!((5.0..5.8).contains(&ly), "got {ly} ly");
    }

    #[test]
    fn log_distance_preserves_direction_exactly() {
        let v = to_xyz(53.9955, -44.5119, 3.670);
        let d = display_pos(v, DistanceMode::Log);
        let cos = (v.x * d.x + v.y * d.y + v.z * d.z) / (v.length() * d.length());
        assert!(approx(cos, 1.0, 1e-12), "direction drifted, cos = {cos}");
        assert!(d.length() < v.length(), "log mode must compress");
    }

    #[test]
    fn log_distance_is_monotonic_and_leaves_the_origin_fixed() {
        assert_eq!(display_pos(Vec3::ZERO, DistanceMode::Log), Vec3::ZERO);
        let mut last = 0.0;
        for r in [0.5, 1.3, 3.7, 12.5, 178.0, 1000.0] {
            let len = display_pos(to_xyz(0.0, 0.0, r), DistanceMode::Log).length();
            assert!(len > last, "not monotonic at {r}");
            last = len;
        }
    }

    #[test]
    fn gj_1061_habitable_zone_contains_planet_d() {
        // Published result: d sits in the conservative HZ, c is on the inner edge,
        // b is well inside it. This pins the luminosity and flux-limit maths.
        let hz = habitable_zone(Some(0.156), Some(2953.0)).unwrap();
        assert!(approx(hz.inner, 0.0389, 5e-4), "inner {}", hz.inner);
        assert!(approx(hz.outer, 0.0561, 5e-4), "outer {}", hz.outer);
        assert!(hz.contains(0.054), "planet d should be in the zone");
        assert!(!hz.contains(0.021), "planet b should be too hot");
    }

    #[test]
    fn sun_earth_is_the_calibration_case() {
        let hz = habitable_zone(Some(1.0), Some(TEFF_SUN)).unwrap();
        assert!(approx(hz.l_sun, 1.0, 1e-9));
        assert!(hz.contains(1.0), "Earth must be in the Sun's habitable zone");
        assert!(approx(insolation(1.0, 1.0).unwrap(), 1.0, 1e-9));
    }

    #[test]
    fn kepler_third_law_recovers_known_axes() {
        // Earth: 365.25 d around 1 M☉ is 1 AU by construction.
        assert!(approx(axis_from_period(Some(365.25), Some(1.0)).unwrap(), 1.0, 1e-9));
        // Jupiter, to within the eccentricity-driven slop.
        let j = axis_from_period(Some(4332.6), Some(1.0)).unwrap();
        assert!(approx(j, 5.203, 0.01), "got {j}");
    }

    #[test]
    fn orbit_norm_is_monotonic_and_bounded_for_real_systems() {
        let systems: [&[f64]; 5] = [
            &[0.021, 0.035, 0.054],                                          // GJ 1061
            &[0.01154, 0.0158, 0.02227, 0.02925, 0.03849, 0.04683, 0.06189], // TRAPPIST-1
            &[0.387, 0.723, 1.0, 1.524, 5.203, 9.537, 19.19, 30.07],         // Sol
            &[0.079, 2.94],                                                  // GJ 411, 37x range
            &[0.00709, 0.0596, 0.0982],                                      // GJ 367
        ];
        for scale in [OrbitScale::Log, OrbitScale::Sqrt, OrbitScale::True] {
            for axes in systems {
                let (lo, hi) = (axes[0], axes[axes.len() - 1]);
                let mut prev = f64::NEG_INFINITY;
                for &a in axes {
                    let n = orbit_norm(a, lo, hi, scale);
                    assert!(n > prev, "rings crossed: {scale:?} {a}");
                    assert!((0.0..=1.0).contains(&n), "out of range: {scale:?} {a} -> {n}");
                    prev = n;
                }
            }
        }
    }

    #[test]
    fn single_planet_systems_do_not_divide_by_zero() {
        let n = orbit_norm(3.53, 3.53, 3.53, OrbitScale::Log);
        assert!(n.is_finite() && (0.0..=1.0).contains(&n));
    }

    #[test]
    fn planet_class_boundaries() {
        assert_eq!(planet_class(Some(1.0)), "Terrestrial");
        assert_eq!(planet_class(Some(1.5)), "Super-Earth");
        assert_eq!(planet_class(Some(3.883)), "Neptune-like"); // Neptune
        assert_eq!(planet_class(Some(11.209)), "Gas giant"); // Jupiter
        assert_eq!(planet_class(None), "unclassified");
    }

    #[test]
    fn voyager_takes_about_seventeen_thousand_years_per_light_year() {
        let m = measure(Vec3::ZERO, to_xyz(0.0, 0.0, 1.0 / PC_IN_LY));
        assert!(approx(m.ly, 1.0, 1e-9));
        assert!(approx(m.voyager_years, 17_635.0, 5.0), "got {}", m.voyager_years);
    }

    #[test]
    fn hotter_stars_are_bluer() {
        let m_dwarf = teff_rgb(Some(3000.0));
        let a_star = teff_rgb(Some(9000.0));
        assert!(m_dwarf[0] > m_dwarf[2], "M dwarf should be red-dominant");
        assert!(a_star[2] > a_star[0], "A star should be blue-dominant");
    }

    #[test]
    fn phases_are_spread_not_aligned() {
        let a = phase_of(0);
        let b = phase_of(1);
        assert!((a - b).abs() > 0.5);
    }
}
