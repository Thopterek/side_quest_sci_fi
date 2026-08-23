//! The cube's camera.
//!
//! An orbit camera that pivots on whatever the operator last selected, so
//! rotating and zooming both act on that system rather than on the Sun.
//! Deliberately free of any rendering dependency: it turns `Vec3` into screen
//! coordinates and nothing else, which is what makes it testable.

use super::astro::Vec3;

/// A world point after projection.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Projected {
    pub x: f32,
    pub y: f32,
    /// Distance into the screen. Used for painter's-algorithm sorting.
    pub depth: f64,
    /// Pixels per world unit at this depth. Drives size attenuation.
    pub k: f64,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub yaw: f64,
    pub pitch: f64,
    /// Current distance from the pivot, world units (parsecs).
    pub dist: f64,
    /// Focal length in pixels.
    pub focal: f64,
    /// The point the camera orbits and zooms toward.
    pub pivot: Vec3,
    want_dist: f64,
    want_pivot: Vec3,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            yaw: 0.6,
            pitch: -0.62,
            dist: 22.0,
            focal: 620.0,
            pivot: Vec3::ZERO,
            want_dist: 22.0,
            want_pivot: Vec3::ZERO,
        }
    }
}

impl Camera {
    /// World point to screen. `None` when the point is behind the camera.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<Projected> {
        let (px, py, pz) = (p.x - self.pivot.x, p.y - self.pivot.y, p.z - self.pivot.z);

        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let x1 = px * cy - py * sy;
        let y1 = px * sy + py * cy;

        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let y2 = y1 * cp - pz * sp;
        let z2 = y1 * sp + pz * cp;

        let depth = z2 + self.dist;
        if depth <= 0.2 {
            return None;
        }
        let k = self.focal / depth;
        Some(Projected {
            x: w / 2.0 + (x1 * k) as f32,
            y: h / 2.0 - (y2 * k) as f32,
            depth,
            k,
        })
    }

    /// Screen-space basis vectors for the world plane `z = pivot.z` at `p`.
    ///
    /// Returned as `(u, v)` where `u` is unit length and `v` carries the
    /// foreshortening. Drawing an orbit as `cos θ·u + sin θ·v` makes it lie
    /// flat in the cube and tilt with the camera, instead of always facing it.
    pub fn plane_basis(&self, p: Vec3, w: f32, h: f32, eps: f64) -> ((f32, f32), (f32, f32)) {
        let origin = self.project(p, w, h);
        let along_x = self.project(Vec3::new(p.x + eps, p.y, p.z), w, h);
        let along_y = self.project(Vec3::new(p.x, p.y + eps, p.z), w, h);

        match (origin, along_x, along_y) {
            (Some(o), Some(a), Some(b)) => {
                let (mut ux, mut uy) = (a.x - o.x, a.y - o.y);
                let (mut vx, mut vy) = (b.x - o.x, b.y - o.y);
                let len = (ux * ux + uy * uy).sqrt();
                if len > f32::EPSILON {
                    ux /= len;
                    uy /= len;
                    vx /= len;
                    vy /= len;
                }
                ((ux, uy), (vx, vy))
            }
            // Degenerate view: fall back to a screen-facing circle.
            _ => ((1.0, 0.0), (0.0, 1.0)),
        }
    }

    /// Pixels per parsec at the pivot. Feeds the honest-scale strip.
    pub fn px_per_unit(&self, w: f32, h: f32) -> f64 {
        match (self.project(self.pivot, w, h), self.project(Vec3::new(self.pivot.x + 1.0, self.pivot.y, self.pivot.z), w, h)) {
            (Some(a), Some(b)) => ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt() as f64,
            _ => 0.0,
        }
    }

    pub fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw += dx as f64 * 0.007;
        self.pitch = (self.pitch - dy as f64 * 0.007).clamp(-1.53, 1.53);
    }

    /// Wheel zoom. Travels toward the pivot and cancels any in-flight easing.
    pub fn zoom(&mut self, notches: f32, extent: f64) {
        let factor = 1.0 + notches.signum() as f64 * 0.13;
        self.dist = (self.dist * factor).clamp(extent * 0.02, extent * 22.0);
        self.want_dist = self.dist;
    }

    /// Re-centre on a system without changing zoom.
    pub fn look_at(&mut self, p: Vec3) {
        self.want_pivot = p;
    }

    /// Frame the whole cube.
    ///
    /// The multiplier is derived, not guessed: at distance `d` a half-width `E`
    /// projects to `focal * E / d` pixels, and we want that to be a bit under
    /// half the viewport so the cube's corners sit just inside the frame. With
    /// the default focal length that lands near 2.2, which fills the view
    /// instead of leaving the vault as a speck in the middle.
    pub fn fit(&mut self, extent: f64) {
        self.want_dist = extent * Self::FIT_RATIO;
    }

    /// Distance-to-half-width ratio used by [`fit`](Camera::fit).
    pub const FIT_RATIO: f64 = 2.2;

    /// Pull in until the pivot's orbits are readable.
    pub fn focus(&mut self, extent: f64) {
        self.want_dist = (extent * 0.1).max(0.8);
    }

    /// Snap without animating. Used on load, so the first frame is already right.
    pub fn settle(&mut self) {
        self.dist = self.want_dist;
        self.pivot = self.want_pivot;
    }

    /// Advance the easing by one frame. Returns `true` while still moving, which
    /// the UI uses to decide whether to request another repaint.
    pub fn ease(&mut self, extent: f64) -> bool {
        let gap_d = self.want_dist - self.dist;
        let gap_p = self.want_pivot.sub(self.pivot);
        let moving = gap_d.abs() > extent * 0.003 || gap_p.length() > extent * 0.002;
        if moving {
            self.dist += gap_d * 0.14;
            self.pivot.x += gap_p.x * 0.16;
            self.pivot.y += gap_p.y * 0.16;
            self.pivot.z += gap_p.z * 0.16;
        } else {
            self.settle();
        }
        moving
    }
}

/// Round a required reach up to a value whose quarters are readable numbers.
/// Keeps the four axis ticks on the cube from landing on things like 11.25 pc.
pub fn nice_extent(max_component: f64) -> f64 {
    const LADDER: [f64; 19] = [
        0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 2.5, 4.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 250.0,
        500.0, 1000.0, 2500.0, 5000.0,
    ];
    let need = max_component.max(0.4) * 1.06;
    for step in LADDER {
        if step * 4.0 >= need {
            return step * 4.0;
        }
    }
    (need / 4.0).ceil() * 4.0
}

/* ------------------------------------------------------------------ tests -- */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astro::to_xyz;

    const W: f32 = 900.0;
    const H: f32 = 650.0;

    fn centred(p: &Projected) -> bool {
        (p.x - W / 2.0).abs() < 1e-3 && (p.y - H / 2.0).abs() < 1e-3
    }

    #[test]
    fn the_pivot_sits_exactly_at_screen_centre() {
        let target = to_xyz(346.6266, -5.0414, 12.467); // TRAPPIST-1
        let mut cam = Camera { dist: 52.8, ..Default::default() };
        cam.look_at(target);
        cam.settle();
        assert!(centred(&cam.project(target, W, H).unwrap()));
    }

    #[test]
    fn the_pivot_stays_pinned_through_every_rotation() {
        // This is the whole point of the pivot rework: selecting a system must
        // not leave the camera swinging around the Sun.
        let target = to_xyz(346.6266, -5.0414, 12.467);
        let mut cam = Camera { dist: 52.8, ..Default::default() };
        cam.look_at(target);
        cam.settle();
        for yaw_step in 0..12 {
            for pitch_step in -3..=3 {
                cam.yaw = yaw_step as f64 * 0.5;
                cam.pitch = pitch_step as f64 * 0.4;
                let p = cam.project(target, W, H).expect("pivot must never clip");
                assert!(centred(&p), "drifted at yaw {yaw_step} pitch {pitch_step}");
            }
        }
    }

    #[test]
    fn zooming_travels_toward_the_pivot() {
        let target = to_xyz(346.6266, -5.0414, 12.467);
        let other = to_xyz(53.9955, -44.5119, 3.670);
        let mut cam = Camera { dist: 52.8, ..Default::default() };
        cam.look_at(target);
        cam.settle();

        let radius = |c: &Camera| {
            let p = c.project(other, W, H).unwrap();
            ((p.x - W / 2.0).powi(2) + (p.y - H / 2.0).powi(2)).sqrt()
        };
        let before = radius(&cam);
        for _ in 0..5 {
            cam.zoom(-1.0, 16.0);
        }
        assert!(radius(&cam) > before, "zooming in must push other stars outward");
        assert!(centred(&cam.project(target, W, H).unwrap()), "pivot must not drift");
    }

    #[test]
    fn positive_z_draws_upward_and_x_separates_horizontally() {
        let cam = Camera::default();
        let o = cam.project(Vec3::ZERO, W, H).unwrap();
        let up = cam.project(Vec3::new(0.0, 0.0, 5.0), W, H).unwrap();
        let side = cam.project(Vec3::new(5.0, 0.0, 0.0), W, H).unwrap();
        assert!(up.y < o.y, "north celestial pole must be up on screen");
        assert!((side.x - o.x).abs() > 1.0, "the X axis must separate horizontally");
    }

    #[test]
    fn nearer_points_project_larger() {
        // At the default yaw/pitch, +Y swings toward the viewer.
        let cam = Camera::default();
        let near = cam.project(Vec3::new(0.0, 8.0, 0.0), W, H).unwrap();
        let far = cam.project(Vec3::new(0.0, -8.0, 0.0), W, H).unwrap();
        assert!(near.depth < far.depth, "depth must sort front to back");
        assert!(near.k > far.k, "nearer points must attenuate larger");
    }

    #[test]
    fn points_behind_the_camera_are_dropped() {
        let cam = Camera { dist: 5.0, ..Default::default() };
        assert!(cam.project(Vec3::new(0.0, 0.0, -500.0), W, H).is_none());
    }

    #[test]
    fn plane_basis_foreshortens_but_never_collapses() {
        let cam = Camera { dist: 52.8, ..Default::default() };
        let p = to_xyz(53.9955, -44.5119, 3.670);
        let ((ux, uy), (vx, vy)) = cam.plane_basis(p, W, H, 0.24);
        let ulen = (ux * ux + uy * uy).sqrt();
        let vlen = (vx * vx + vy * vy).sqrt();
        assert!((ulen - 1.0).abs() < 1e-4, "u must be unit length, got {ulen}");
        assert!(vlen > 0.05 && vlen <= 1.05, "v foreshortens, got {vlen}");
    }

    #[test]
    fn plane_basis_flattens_as_the_view_goes_edge_on() {
        let p = Vec3::new(1.0, 1.0, 0.0);
        let mut flat = Camera { dist: 30.0, pitch: -1.52, ..Default::default() };
        let mut top = Camera { dist: 30.0, pitch: 0.0, ..Default::default() };
        flat.settle();
        top.settle();
        let vlen = |c: &Camera| {
            let (_, (vx, vy)) = c.plane_basis(p, W, H, 0.2);
            (vx * vx + vy * vy).sqrt()
        };
        assert!(vlen(&flat) < vlen(&top), "orbits must squash when seen edge-on");
    }

    #[test]
    fn easing_converges_and_then_reports_done() {
        let target = to_xyz(53.9955, -44.5119, 3.670);
        let mut cam = Camera::default();
        cam.look_at(target);
        cam.fit(16.0);
        let mut frames = 0;
        while cam.ease(16.0) {
            frames += 1;
            assert!(frames < 400, "easing never settled");
        }
        assert!(frames > 3, "easing should animate, not snap");
        assert!(cam.pivot.sub(target).length() < 1e-9);
        assert!((cam.dist - 16.0 * Camera::FIT_RATIO).abs() < 1e-9);
    }

    #[test]
    fn fit_fills_the_frame_rather_than_leaving_a_speck() {
        // Regression: the first build framed a 16 pc cube at 11 px/pc, crushing
        // the whole solar neighbourhood into about a hundred pixels.
        //
        // Measured as the cube's half-width in pixels at the pivot plane, which
        // is the honest figure: the near corner is magnified by perspective and
        // clipping it is normal, so corner reach says little about occupancy.
        let (w, h) = (1000.0_f32, 1000.0_f32);
        let mut cam = Camera::default();
        cam.fit(16.0);
        while cam.ease(16.0) {}

        let half_width_px = cam.px_per_unit(w, h) * 16.0;
        let half_frame = (h / 2.0) as f64;
        assert!(
            half_width_px > half_frame * 0.45,
            "cube occupies only {half_width_px:.0} px of {half_frame:.0}"
        );
        assert!(
            half_width_px < half_frame * 0.9,
            "cube overflows the frame at {half_width_px:.0} px"
        );
    }

    #[test]
    fn fit_is_scale_invariant() {
        // A 2 pc vault and a 2000 pc vault must frame identically; only the
        // tick labels should differ.
        let (w, h) = (1000.0_f32, 1000.0_f32);
        let mut occupancy = Vec::new();
        for extent in [2.0, 16.0, 200.0, 2000.0] {
            let mut cam = Camera::default();
            cam.fit(extent);
            while cam.ease(extent) {}
            occupancy.push(cam.px_per_unit(w, h) * extent);
        }
        // Not exactly invariant, and correctly so: the probe point used by
        // `px_per_unit` sits one world unit from the pivot, which at 2 pc is a
        // measurable fraction of the camera distance and so is foreshortened
        // slightly differently than at 2000 pc. A few percent is perspective;
        // orders of magnitude would mean `fit` had stopped tracking extent.
        let mean = occupancy.iter().sum::<f64>() / occupancy.len() as f64;
        for o in &occupancy {
            assert!(
                (o - mean).abs() / mean < 0.12,
                "framing drifted with scale: {occupancy:?}"
            );
        }
    }

    #[test]
    fn focus_is_closer_than_fit() {
        let mut a = Camera::default();
        let mut b = Camera::default();
        a.fit(16.0);
        b.focus(16.0);
        while a.ease(16.0) {}
        while b.ease(16.0) {}
        assert!(b.dist < a.dist);
        // Close enough that the orrery is drawn at better than 1:1 zoom scale.
        assert!(b.focal / b.dist / 190.0 > 1.5);
    }

    #[test]
    fn zoom_is_clamped_both_ways() {
        let mut cam = Camera::default();
        for _ in 0..200 {
            cam.zoom(1.0, 16.0);
        }
        assert!(cam.dist <= 16.0 * 22.0 + 1e-9);
        for _ in 0..400 {
            cam.zoom(-1.0, 16.0);
        }
        assert!(cam.dist >= 16.0 * 0.02 - 1e-9);
    }

    #[test]
    fn pitch_cannot_flip_over_the_pole() {
        let mut cam = Camera::default();
        for _ in 0..500 {
            cam.rotate(0.0, -100.0);
        }
        assert!(cam.pitch <= 1.53);
        for _ in 0..1000 {
            cam.rotate(0.0, 100.0);
        }
        assert!(cam.pitch >= -1.53);
    }

    #[test]
    fn extent_ladder_gives_readable_quarter_ticks() {
        // 12.08 is TRAPPIST-1's largest coordinate in the seed vault.
        assert_eq!(nice_extent(12.08), 16.0);
        assert_eq!(nice_extent(3.6), 4.0);
        assert_eq!(nice_extent(1.2), 2.0);
        // An empty vault still needs a cube to draw.
        assert!(nice_extent(0.0) > 0.0);
        for reach in [0.3, 1.0, 3.67, 12.47, 178.0, 2000.0] {
            let e = nice_extent(reach);
            assert!(e >= reach, "extent {e} must contain reach {reach}");
            let tick = e / 4.0;
            assert!(tick.is_finite() && tick > 0.0);
        }
    }
}
