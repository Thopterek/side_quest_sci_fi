//! The cube — Parallax's graph view.
//!
//! Every saved system sits at its real position and, above a threshold size,
//! renders as a live orrery lying flat in the equatorial plane. Painter's
//! algorithm for depth, drop lines to the base grid so height is readable, and
//! a camera that pivots on the current selection.

use egui::{Align2, FontId, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use crate::core::astro::{display_pos, separation, DistanceMode, OrbitScale, Vec3, PC_IN_LY};
use crate::core::camera::Camera;
use crate::core::index::VaultIndex;
use crate::core::orrery::layout;
use crate::core::vault::Vault;

use super::system_view::paint_into_plane;
use super::theme::{ColorMode, Theme};

/// What the cube wants the app to do after a frame.
#[derive(Default)]
pub struct CubeOutput {
    pub select: Option<String>,
    pub compare: Option<String>,
    pub hovered: Option<String>,
    /// Pixels per parsec at the pivot, for the honest-scale strip.
    pub px_per_pc: f64,
    /// The camera is still easing, so another frame is needed.
    pub animating: bool,
    /// Where to float the hover card, if anything is hovered.
    pub hover_anchor: Option<Pos2>,
}

/// Everything the caller needs to draw the hover card outside the canvas.
pub struct CubeSettings {
    pub distance: DistanceMode,
    pub color: ColorMode,
    /// 0 draws plain dots; otherwise a multiplier on the orrery radius.
    pub system_scale: f32,
    pub show_links: bool,
    pub clock_days: f64,
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    vault: &Vault,
    index: &VaultIndex,
    cam: &mut Camera,
    cfg: &CubeSettings,
) -> CubeOutput {
    let mut out = CubeOutput::default();
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let (w, h) = (rect.width(), rect.height());
    if w < 4.0 || h < 4.0 {
        return out;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.plate);

    // Positions, edges and extent all come from the cached index.
    let extent = index.extent();

    /* ------------------------------------------------------------ input -- */
    if response.dragged() {
        let d = response.drag_delta();
        cam.rotate(d.x, d.y);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > 0.5 {
            cam.zoom(-scroll, extent);
        }
    }
    out.animating = cam.ease(extent);

    // All mutation of the camera is done; reborrow immutably so the projection
    // closure and `plane_basis` can coexist.
    let cam: &Camera = &*cam;

    // The projection is relative to the widget, so shift into rect space.
    let to_screen = |p: Pos2| Pos2::new(rect.min.x + p.x, rect.min.y + p.y);
    let proj = |v: Vec3| cam.project(v, w, h).map(|p| (to_screen(Pos2::new(p.x, p.y)), p.depth, p.k));

    let seg = |a: Vec3, b: Vec3, stroke: Stroke| {
        if let (Some((pa, _, _)), Some((pb, _, _))) = (proj(a), proj(b)) {
            painter.line_segment([pa, pb], stroke);
        }
    };
    let dashed = |a: Vec3, b: Vec3, stroke: Stroke, dash: f32, gap: f32| {
        if let (Some((pa, _, _)), Some((pb, _, _))) = (proj(a), proj(b)) {
            painter.extend(Shape::dashed_line(&[pa, pb], stroke, dash, gap));
        }
    };

    /* ------------------------------------------------- grid and the cube -- */
    let faint = Stroke::new(0.6, theme.alpha(theme.rule, 0.3));
    let step = extent / 4.0;

    for i in -4i32..=4 {
        let t = i as f64 * step;
        let major = i == 0 || i.abs() == 4;
        let s = if major { Stroke::new(if i == 0 { 1.0 } else { 0.7 }, theme.alpha(theme.rule, 0.75)) } else { faint };
        seg(Vec3::new(t, -extent, 0.0), Vec3::new(t, extent, 0.0), s);
        seg(Vec3::new(-extent, t, 0.0), Vec3::new(extent, t, 0.0), s);
    }

    // Twelve edges of the bounding cube, dashed so they read as a frame rather
    // than as anything physical.
    let e = extent;
    // Slightly heavier than the floor grid: the cube is the frame of reference
    // and was disappearing into it.
    let box_stroke = Stroke::new(1.0, theme.alpha(theme.rule, 0.95));
    for (a, b) in cube_edges(e) {
        dashed(a, b, box_stroke, 4.0, 4.0);
    }

    // Parsec ticks up one vertical edge.
    for i in -4i32..=4 {
        if i == 0 {
            continue;
        }
        let t = i as f64 * step;
        if let Some((p, _, _)) = proj(Vec3::new(-e, -e, t)) {
            let label = if extent < 4.0 { format!("{t:.1}") } else { format!("{t:.0}") };
            painter.text(
                Pos2::new(p.x - 5.0, p.y),
                Align2::RIGHT_CENTER,
                label,
                FontId::monospace(9.0),
                theme.soft,
            );
        }
    }

    /* ------------------------------------------------------------ links -- */
    if cfg.show_links {
        let link_stroke = Stroke::new(1.2, theme.alpha(theme.accent, 0.5));
        for &(a, b) in index.edges() {
            if let (Some(ea), Some(eb)) = (index.get(a), index.get(b)) {
                seg(ea.display, eb.display, link_stroke);
            }
        }
    }

    /* ------------------------------------------------------ measurement -- */
    if let (Some(a), Some(b)) = (vault.selected(), vault.compared()) {
        let (pa, pb) = (display_pos(a.position(), cfg.distance), display_pos(b.position(), cfg.distance));
        seg(pa, pb, Stroke::new(1.6, theme.accent));
        if let (Some((sa, _, _)), Some((sb, _, _))) = (proj(pa), proj(pb)) {
            // Always the *true* separation, never the displayed one.
            let d = separation(a.position(), b.position());
            painter.text(
                Pos2::new((sa.x + sb.x) * 0.5, (sa.y + sb.y) * 0.5 - 9.0),
                Align2::CENTER_CENTER,
                format!("{d:.2} pc · {:.2} ly", d * PC_IN_LY),
                FontId::monospace(10.5),
                theme.accent,
            );
        }
    }

    /* ----------------------------------------------------------- system -- */
    struct Item<'a> {
        sys: &'a crate::core::model::System,
        world: Vec3,
        screen: Pos2,
        depth: f64,
        k: f64,
    }

    let mut items: Vec<Item> = Vec::with_capacity(vault.systems.len());
    for (i, sys) in vault.systems.iter().enumerate() {
        let Some(entry) = index.get(i) else { continue };
        if let Some((screen, depth, k)) = proj(entry.display) {
            items.push(Item { sys, world: entry.display, screen, depth, k });
        }
    }
    // Painter's algorithm: far to near.
    items.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));

    // Hit testing uses the same projected points, so picking always matches
    // what is on screen even mid-animation.
    let pointer = response.hover_pos();
    let mut hovered: Option<(f32, String)> = None;
    if let Some(pos) = pointer {
        for it in &items {
            let d = it.screen.distance(pos);
            if d < 22.0 && hovered.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
                hovered = Some((d, it.sys.id.clone()));
            }
        }
    }
    out.hovered = hovered.as_ref().map(|(_, id)| id.clone());

    let selected = vault.selected.clone();
    let compared = vault.compare.clone();
    let eps = extent * 0.015;

    // Labels are placed front to back, and one that would collide with an
    // already-placed label is dropped rather than overprinted. Nearer systems
    // are drawn last, so they win the space — which is the right precedence.
    let mut placed: Vec<egui::Rect> = Vec::new();

    for it in &items {
        let is_sel = selected.as_deref() == Some(it.sys.id.as_str());
        let is_cmp = compared.as_deref() == Some(it.sys.id.as_str());
        let is_hov = out.hovered.as_deref() == Some(it.sys.id.as_str());
        let color = theme.star_color(it.sys, cfg.color);

        // Drop line to the equatorial plane. Without this the cube reads flat.
        let foot = Vec3::new(it.world.x, it.world.y, 0.0);
        if let Some((fp, _, _)) = proj(foot) {
            painter.extend(Shape::dashed_line(
                &[it.screen, fp],
                Stroke::new(0.7, theme.alpha(theme.rule, 0.55)),
                2.0,
                2.0,
            ));
            painter.circle_filled(fp, 1.3, theme.alpha(theme.rule, 0.7));
        }

        let zoom_k = ((it.k / 190.0) as f32).clamp(0.4, 2.4);
        let orrery_radius = cfg.system_scale * 17.0 * zoom_k;
        let draw_orrery = cfg.system_scale > 0.0 && orrery_radius >= 7.0 && !it.sys.planets.is_empty();

        if draw_orrery {
            let basis = cam.plane_basis(it.world, w, h, eps);
            let lay = layout(it.sys, orrery_radius, cfg.clock_days, OrbitScale::Log, 0.0);
            paint_into_plane(&painter, theme, it.sys, &lay, it.screen, basis, cfg.color, None);
        }

        // Glyph size encodes apparent magnitude, not physical radius.
        let base = if it.sys.origin {
            6.5
        } else {
            (7.2 - it.sys.vmag.unwrap_or(4.0) * 0.22).max(2.6)
        } as f32;
        let star_px = star_radius(base, zoom_k, draw_orrery.then_some(orrery_radius));

        painter.circle_filled(it.screen, star_px * 3.1, theme.alpha(color, 0.16));
        painter.circle_filled(it.screen, star_px, color);
        painter.circle_stroke(it.screen, star_px, Stroke::new(0.7, theme.alpha(theme.ink, 0.55)));

        // Sol gets a surveyor's reticle: it is the origin of the coordinates.
        if it.sys.origin {
            let inner = if draw_orrery { orrery_radius + 4.0 } else { star_px + 3.0 };
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                painter.line_segment(
                    [
                        Pos2::new(it.screen.x + dx * inner, it.screen.y + dy * inner),
                        Pos2::new(it.screen.x + dx * (inner + 5.0), it.screen.y + dy * (inner + 5.0)),
                    ],
                    Stroke::new(0.9, theme.ink),
                );
            }
        }

        if is_sel || is_cmp || is_hov {
            let r = if draw_orrery { orrery_radius + 7.0 } else { star_px + 6.0 };
            painter.circle_stroke(it.screen, r, Stroke::new(if is_sel { 1.5 } else { 1.0 }, theme.accent));
            if is_cmp {
                painter.extend(Shape::dashed_line(
                    &ring_points(it.screen, r + 4.0),
                    Stroke::new(1.0, theme.accent),
                    2.0,
                    3.0,
                ));
            }
        }

        // Selection and hover always get a label; ambient ones are best effort.
        let insistent = is_sel || is_cmp || is_hov;
        if insistent || it.sys.origin || it.k > 120.0 {
            let off = if draw_orrery { orrery_radius } else { star_px } + 8.0;
            let at = Pos2::new(it.screen.x + off, it.screen.y);
            let galley = painter.layout_no_wrap(
                it.sys.display_name().to_owned(),
                FontId::proportional(10.5),
                if is_sel || is_cmp { theme.accent } else { theme.ink },
            );
            let bounds = egui::Rect::from_min_size(
                Pos2::new(at.x, at.y - galley.size().y * 0.5),
                galley.size(),
            )
            .expand(1.5);
            if insistent || !placed.iter().any(|r| r.intersects(bounds)) {
                painter.galley(bounds.min + egui::vec2(1.5, 1.5), galley, theme.ink);
                placed.push(bounds);
            }
        }
    }

    /* ------------------------------------------------------------ clicks -- */
    if response.clicked() {
        if let Some((_, id)) = hovered {
            if ui.input(|i| i.modifiers.shift) {
                out.compare = Some(id);
            } else {
                out.select = Some(id);
            }
        }
    }

    out.px_per_pc = cam.px_per_unit(w, h);
    out.hover_anchor = pointer.map(|p| Pos2::new(p.x + 14.0, p.y + 14.0));

    /* ------------------------------------------------------------ corner -- */
    let corner = rect.min + Vec2::new(11.0, 12.0);
    let mut y = corner.y;
    let line = |txt: String, size: f32, col: egui::Color32, mono: bool, y: &mut f32| {
        painter.text(
            Pos2::new(corner.x, *y),
            Align2::LEFT_TOP,
            txt,
            if mono { FontId::monospace(size) } else { FontId::proportional(size) },
            col,
        );
        *y += size + 4.0;
    };
    line("THE CUBE · EQUATORIAL CARTESIAN".into(), 9.5, theme.soft, false, &mut y);
    line(
        format!(
            "half-width {:.*} pc ({:.0} ly){}",
            if extent < 4.0 { 1 } else { 0 },
            extent,
            extent * PC_IN_LY,
            if cfg.distance == DistanceMode::Log { "  [log-radial]" } else { "" }
        ),
        10.5,
        theme.soft,
        true,
        &mut y,
    );
    line(
        "drag rotate · wheel zoom · click re-centre · shift-click measure".into(),
        10.5,
        theme.dim,
        true,
        &mut y,
    );
    if let Some(sel) = vault.selected() {
        line(format!("centred on {}", sel.display_name()), 10.5, theme.accent, true, &mut y);
    }

    /* ------------------------------------------------------------ legend -- */
    if cfg.color == ColorMode::Arm {
        let mut ly = rect.max.y - 12.0;
        for arm in crate::core::model::Arm::ALL.iter().rev() {
            let c = if theme.mode.is_plate() { arm.plate_rgb() } else { arm.negative_rgb() };
            let col = egui::Color32::from_rgb(c[0], c[1], c[2]);
            painter.circle_filled(Pos2::new(rect.min.x + 16.0, ly), 3.5, col);
            painter.text(
                Pos2::new(rect.min.x + 24.0, ly),
                Align2::LEFT_CENTER,
                arm.name(),
                FontId::monospace(10.0),
                theme.soft,
            );
            ly -= 14.0;
        }
        painter.text(
            Pos2::new(rect.min.x + 11.0, ly),
            Align2::LEFT_CENTER,
            "ARM",
            FontId::proportional(9.5),
            theme.soft,
        );
    }

    out
}

/// The floating card shown next to a hovered system.
///
/// Drawn as a real egui area rather than painted into the canvas, so it can
/// reuse [`system_view`] at thumbnail size instead of approximating it.
pub fn peek_card(
    ui: &mut Ui,
    theme: &Theme,
    sys: &crate::core::model::System,
    at: Pos2,
    clock_days: f64,
    color: ColorMode,
) {
    egui::Area::new(egui::Id::new("cube-peek"))
        .order(egui::Order::Tooltip)
        .fixed_pos(at)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme.deep)
                .stroke(Stroke::new(1.0, theme.rule))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        super::system_view::thumb(ui, theme, sys, 92.0, clock_days, color);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(sys.display_name()).size(13.0).strong());
                            if !sys.record.imperial_name.trim().is_empty() {
                                ui.label(
                                    egui::RichText::new(&sys.hostname)
                                        .monospace().size(10.0).color(theme.dim),
                                );
                            }
                            let (_, a_max) = sys.axis_range();
                            for line in [
                                format!(
                                    "{} · {} pc",
                                    sys.spectype.as_deref().unwrap_or("—"),
                                    super::theme::num(sys.dist_pc, 2)
                                ),
                                format!("{} planets · outermost {:.3} AU", sys.planets.len(), a_max),
                            ] {
                                ui.label(
                                    egui::RichText::new(line)
                                        .monospace().size(10.0).color(theme.soft),
                                );
                            }
                            if let Some(arm) = sys.record.arm {
                                ui.label(
                                    egui::RichText::new(arm.name())
                                        .monospace().size(10.0).color(theme.soft),
                                );
                            }
                        });
                    });
                });
        });
}

/// Drawn radius of a star glyph, in pixels.
///
/// Inside an orrery the star must not crowd the innermost orbit, so it is
/// capped at a fraction of the orrery radius — but it must also stay large
/// enough to see. Those two bounds can invert: an orrery drawn just above the
/// 7 px threshold wants a cap below the 2 px floor, and expressing that as
/// `clamp(2.0, r * 0.16)` panics with `min > max`. Applying the cap first and
/// the floor second is order-independent, and is what the tests below pin.
fn star_radius(base: f32, zoom_k: f32, orrery_radius: Option<f32>) -> f32 {
    let unbounded = base * zoom_k;
    match orrery_radius {
        Some(r) => (unbounded * 0.8).min(r * 0.16).max(2.0),
        None => unbounded.max(2.0),
    }
}

fn ring_points(centre: Pos2, r: f32) -> Vec<Pos2> {
    (0..=32)
        .map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            Pos2::new(centre.x + r * a.cos(), centre.y + r * a.sin())
        })
        .collect()
}

/// Unit-cube edges as sign triples. Enumerated once at compile time rather
/// than rediscovered by an O(n²) vertex comparison every frame.
const UNIT_EDGES: [([f64; 3], [f64; 3]); 12] = [
    ([-1., -1., -1.], [1., -1., -1.]),
    ([-1., 1., -1.], [1., 1., -1.]),
    ([-1., -1., 1.], [1., -1., 1.]),
    ([-1., 1., 1.], [1., 1., 1.]),
    ([-1., -1., -1.], [-1., 1., -1.]),
    ([1., -1., -1.], [1., 1., -1.]),
    ([-1., -1., 1.], [-1., 1., 1.]),
    ([1., -1., 1.], [1., 1., 1.]),
    ([-1., -1., -1.], [-1., -1., 1.]),
    ([1., -1., -1.], [1., -1., 1.]),
    ([-1., 1., -1.], [-1., 1., 1.]),
    ([1., 1., -1.], [1., 1., 1.]),
];

/// The twelve edges of an axis-aligned cube of half-width `e`.
fn cube_edges(e: f64) -> impl Iterator<Item = (Vec3, Vec3)> {
    UNIT_EDGES.into_iter().map(move |(a, b)| {
        (Vec3::new(a[0] * e, a[1] * e, a[2] * e), Vec3::new(b[0] * e, b[1] * e, b[2] * e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_orrery_does_not_invert_the_star_size_bounds() {
        // Regression: the cap and the floor cross over for any orrery between
        // the 7 px draw threshold and 12.5 px, which used to panic on the first
        // frame at default zoom.
        for r in [7.0_f32, 8.0, 10.6, 12.4, 12.5, 20.0, 60.0, 400.0] {
            let px = star_radius(6.5, 1.0, Some(r));
            assert!(px.is_finite() && px >= 2.0, "orrery radius {r} gave {px}");
        }
    }

    #[test]
    fn the_star_never_crowds_the_innermost_orbit_once_there_is_room() {
        // Above the crossover the cap governs, so the glyph stays a small
        // fraction of the orrery rather than swallowing it.
        let r = 400.0;
        assert!(star_radius(6.5, 2.4, Some(r)) <= r * 0.16);
        // And a bare dot with no orrery is unaffected by the cap.
        assert!(star_radius(6.5, 2.4, None) > star_radius(6.5, 1.0, None));
        assert_eq!(star_radius(0.0, 0.0, None), 2.0, "always visible");
    }

    #[test]
    fn star_size_is_monotonic_in_zoom_and_brightness() {
        let dim = star_radius(2.6, 1.5, None);
        let bright = star_radius(6.5, 1.5, None);
        assert!(bright > dim, "a brighter star draws larger");
        assert!(star_radius(6.5, 2.4, None) > star_radius(6.5, 0.4, None));
    }

    #[test]
    fn a_cube_has_exactly_twelve_axis_aligned_edges() {
        let edges: Vec<_> = cube_edges(4.0).collect();
        assert_eq!(edges.len(), 12);
        for (a, b) in edges {
            // Each edge spans one axis only, with length 2e.
            let differing = (a.x != b.x) as u8 + (a.y != b.y) as u8 + (a.z != b.z) as u8;
            assert_eq!(differing, 1);
            assert!((a.sub(b).length() - 8.0).abs() < 1e-9);
        }
    }
}
