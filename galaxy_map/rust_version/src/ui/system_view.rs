//! Painting a [`Layout`] into a rectangle.
//!
//! This is the reuse point. The same function draws the 44 px thumbnails in the
//! vault, the 92 px hover card, the 250 px view in the record panel, and — via
//! [`paint_into_plane`] — every live orrery inside the cube. Detail comes from
//! the radius, so there is one implementation rather than four.

use egui::{Align2, FontId, Painter, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use crate::core::astro::OrbitScale;
use crate::core::model::System;
use crate::core::orrery::{layout, Detail, Layout};

use super::theme::{ColorMode, Theme};

/// A unit basis for the orbit plane in screen space.
///
/// `(1,0),(0,1)` draws circles facing the viewer. Inside the cube the camera
/// supplies a foreshortened basis instead, so orbits lie flat in the equatorial
/// plane and tilt with the view.
pub type PlaneBasis = ((f32, f32), (f32, f32));

pub const FACING: PlaneBasis = ((1.0, 0.0), (0.0, 1.0));

fn at(centre: Pos2, basis: PlaneBasis, r: f32, angle: f64) -> Pos2 {
    let (c, s) = (angle.cos() as f32, angle.sin() as f32);
    let ((ux, uy), (vx, vy)) = basis;
    Pos2::new(centre.x + r * (c * ux + s * vx), centre.y + r * (c * uy + s * vy))
}

thread_local! {
    /// Painting happens on one thread, and every ring is consumed by the
    /// painter before the next is built, so a single reusable buffer removes
    /// what was roughly sixty-five Vec allocations per frame.
    static RING: std::cell::RefCell<Vec<Pos2>> = std::cell::RefCell::new(Vec::with_capacity(64));
}

/// Build a ring into the shared scratch buffer and hand it to `f`.
fn with_ring<R>(
    centre: Pos2,
    basis: PlaneBasis,
    r: f32,
    segments: usize,
    f: impl FnOnce(&[Pos2]) -> R,
) -> R {
    RING.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.reserve(segments + 1);
        for i in 0..=segments {
            buf.push(at(centre, basis, r, i as f64 / segments as f64 * std::f64::consts::TAU));
        }
        f(&buf)
    })
}

/// Paint a system into an arbitrary plane. Everything else here delegates to it.
#[allow(clippy::too_many_arguments)]
pub fn paint_into_plane(
    painter: &Painter,
    theme: &Theme,
    sys: &System,
    lay: &Layout,
    centre: Pos2,
    basis: PlaneBasis,
    color_mode: ColorMode,
    picked: Option<&str>,
) {
    let star = theme.star_color(sys, color_mode);
    let detail = lay.detail;
    let segments = if detail == Detail::Thumb { 24 } else { 48 };

    // Habitable zone, drawn as a wide stroke down the middle of the band so it
    // reads as an annulus without needing a filled path.
    if let Some((inner, outer)) = lay.hz_px {
        let mid = (inner + outer) * 0.5;
        let width = (outer - inner).max(0.6);
        let band = Stroke::new(width, theme.alpha(theme.accent, 0.15));
        with_ring(centre, basis, mid, segments, |pts| {
            painter.add(Shape::line(pts.to_vec(), band));
        });
        if detail.shows_labels() {
            let edge_stroke = Stroke::new(0.6, theme.alpha(theme.accent, 0.4));
            for edge in [inner, outer] {
                with_ring(centre, basis, edge, segments, |pts| {
                    painter.add(Shape::line(pts.to_vec(), edge_stroke));
                });
            }
        }
    }

    // Orbit rings.
    let ring_stroke = Stroke::new(
        if detail == Detail::Thumb { 0.5 } else { 0.75 },
        theme.alpha(theme.rule, 0.85),
    );
    for b in &lay.bodies {
        if b.orbit_px < 0.35 {
            continue;
        }
        with_ring(centre, basis, b.orbit_px, segments, |pts| {
            painter.add(Shape::line(pts.to_vec(), ring_stroke));
        });
    }

    // The star, with a soft halo so it reads through the rings.
    painter.circle_filled(centre, lay.star_px * 2.6, theme.alpha(star, 0.12));
    painter.circle_filled(centre, lay.star_px, star);

    // Planets.
    for b in &lay.bodies {
        let p = at(centre, basis, b.orbit_px, b.angle);
        let name = &sys.planets[b.index].name;
        let is_picked = picked == Some(name.as_str());

        if (b.in_hz || is_picked) && b.orbit_px > 1.0 {
            let c = if is_picked { theme.accent } else { theme.alpha(theme.accent, 0.55) };
            painter.circle_stroke(p, b.body_px + 3.5, Stroke::new(if is_picked { 1.4 } else { 1.0 }, c));
        }
        painter.circle_filled(p, b.body_px, theme.ink);

        if detail.shows_labels() {
            painter.text(
                Pos2::new(p.x, p.y - b.body_px - 8.0),
                Align2::CENTER_CENTER,
                sys.planets[b.index].short_name(&sys.hostname),
                FontId::monospace(9.5),
                if is_picked { theme.accent } else { theme.soft },
            );
        }
    }

    // An AU rule along the bottom, so the compressed radii still have a unit.
    if detail.shows_labels() {
        let y = centre.y + lay.radius_px + 16.0;
        let (x0, x1) = (centre.x, centre.x + lay.radius_px);
        let s = Stroke::new(0.75, theme.rule);
        painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], s);
        for x in [x0, x1] {
            painter.line_segment([Pos2::new(x, y - 4.0), Pos2::new(x, y + 4.0)], s);
        }
        painter.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, "0", FontId::monospace(9.5), theme.soft);
        painter.text(
            Pos2::new(x1, y + 12.0),
            Align2::RIGHT_CENTER,
            format!("{:.*} AU", if lay.a_max < 0.1 { 3 } else { 2 }, lay.a_max),
            FontId::monospace(9.5),
            theme.soft,
        );
    }
}

/// Allocate a square of the given side and paint a system into it.
///
/// Returns the click response, and `Some(planet_name)` when a planet was hit,
/// which is how the record panel's orrery doubles as a planet selector.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    sys: &System,
    side: f32,
    clock_days: f64,
    scale: OrbitScale,
    true_mix: f64,
    color_mode: ColorMode,
    picked: Option<&str>,
    interactive: bool,
) -> (egui::Response, Option<String>) {
    let sense = if interactive { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), sense);

    // Labels need room outside the orbit radius, so the drawing radius is
    // inset. The inset is the only place detail tiers are hard-coded.
    // Full detail draws labels above each planet and an AU rule below, so the
    // inset has to grow before the detail tier does.
    let pad = if side >= 280.0 { 46.0 } else if side >= 110.0 { 12.0 } else { 5.0 };
    let radius = (side * 0.5 - pad).max(4.0);
    let centre = rect.center();

    let lay = layout(sys, radius, clock_days, scale, true_mix);
    if ui.is_rect_visible(rect) {
        paint_into_plane(ui.painter(), theme, sys, &lay, centre, FACING, color_mode, picked);
    }

    let mut hit = None;
    if interactive {
        if let Some(pos) = response.interact_pointer_pos() {
            if response.clicked() {
                let mut best = f32::MAX;
                for b in &lay.bodies {
                    let p = at(centre, FACING, b.orbit_px, b.angle);
                    let d = p.distance(pos);
                    if d < (b.body_px + 7.0).max(10.0) && d < best {
                        best = d;
                        hit = Some(sys.planets[b.index].name.clone());
                    }
                }
            }
        }
    }
    (response, hit)
}

/// Convenience for the many read-only thumbnails.
pub fn thumb(ui: &mut Ui, theme: &Theme, sys: &System, side: f32, clock_days: f64, color_mode: ColorMode) {
    show(ui, theme, sys, side, clock_days, OrbitScale::Log, 0.0, color_mode, None, false);
}

