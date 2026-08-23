//! The right column: two layers on every object, never mixed.
//!
//! **Archive** is what NASA published and is replaced wholesale on refresh.
//! **Dossier** is the operator's own and survives every refresh.

use egui::{RichText, Ui};

use crate::core::astro::{equilibrium_temp, insolation, planet_class, OrbitScale, PC_IN_LY};
use crate::core::model::Arm;
use crate::core::vault::Vault;

use super::system_view;
use super::theme::{badge, eyebrow, num, ColorMode, Theme};

#[derive(Default)]
pub struct RecordOutput {
    pub select: Option<String>,
    pub focus_planet: Option<Option<String>>,
    pub refresh: bool,
    pub remove: bool,
    pub focus_in_cube: bool,
    pub set_scale: Option<OrbitScale>,
}

/// One archive row: label on the left, value right-aligned and monospaced.
fn fact(ui: &mut Ui, theme: &Theme, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(theme.soft));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).monospace().size(11.0));
        });
    });
    let r = ui.max_rect();
    ui.painter().hline(
        r.x_range(),
        ui.cursor().top() - 2.0,
        egui::Stroke::new(1.0, theme.hair),
    );
}

fn section(ui: &mut Ui, theme: &Theme, title: &str) {
    ui.add_space(12.0);
    eyebrow(ui, theme, title);
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top() + 2.0,
        egui::Stroke::new(1.0, theme.rule),
    );
    ui.add_space(6.0);
}

fn field(ui: &mut Ui, theme: &Theme, label: &str, value: &mut String, hint: &str, multiline: bool) {
    ui.add_space(8.0);
    eyebrow(ui, theme, label);
    if multiline {
        ui.add(
            egui::TextEdit::multiline(value)
                .hint_text(hint)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
    } else {
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    vault: &mut Vault,
    clock_days: f64,
    scale: OrbitScale,
    true_mix: f64,
    color_mode: ColorMode,
    busy: bool,
) -> RecordOutput {
    let mut out = RecordOutput::default();

    let Some(sel_id) = vault.selected.clone() else {
        ui.label(RichText::new("The vault is empty.").color(theme.dim));
        return out;
    };
    let focus_planet = vault.focus_planet.clone();
    // Resolve link targets to display names here, while the whole vault is
    // still borrowable.
    let links: Vec<(String, String)> = vault
        .links_of(&sel_id)
        .into_iter()
        .map(|id| {
            let label = vault
                .get(&id)
                .map(|s| s.display_name().to_string())
                .unwrap_or_else(|| id.clone());
            (id, label)
        })
        .collect();
    let measurement = vault.measurement();
    let compared_name = vault.compared().map(|s| s.hostname.clone());

    let Some(sys) = vault.get_mut(&sel_id) else { return out };

    ui.horizontal(|ui| {
        eyebrow(ui, theme, "Record");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !sys.origin && ui.small_button("Remove").clicked() {
                out.remove = true;
            }
            if ui.add_enabled(!busy, egui::Button::new(if busy { "…" } else { "Refresh" }).small()).clicked() {
                out.refresh = true;
            }
        });
    });

    ui.label(RichText::new(sys.display_name()).size(20.0).strong());
    ui.horizontal(|ui| {
        if !sys.record.imperial_name.trim().is_empty() {
            ui.label(RichText::new(&sys.hostname).monospace().size(10.5).color(theme.dim));
        }
        ui.label(
            RichText::new(sys.source.label())
                .monospace()
                .size(10.5)
                .color(if sys.source == crate::core::model::Source::Nasa { theme.accent } else { theme.soft }),
        );
    });

    /* ------------------------------------------------- entity selector -- */
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(focus_planet.is_none(), RichText::new("★ system").monospace().size(10.5)).clicked() {
            out.focus_planet = Some(None);
        }
        for p in &sys.planets {
            let rec = sys.planet_records.get(&p.name);
            let label = rec
                .filter(|r| !r.imperial_name.trim().is_empty())
                .map(|r| r.imperial_name.clone())
                .unwrap_or_else(|| p.short_name(&sys.hostname));
            let on = focus_planet.as_deref() == Some(p.name.as_str());
            if ui.selectable_label(on, RichText::new(label).monospace().size(10.5)).clicked() {
                out.focus_planet = Some(Some(p.name.clone()));
            }
        }
    });

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        match focus_planet.clone() {
            /* ================================================== SYSTEM == */
            None => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    let (_, hit) = system_view::show(
                        ui, theme, sys, 300.0, clock_days, scale, true_mix, color_mode, None, true,
                    );
                    if let Some(name) = hit {
                        out.focus_planet = Some(Some(name));
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Focus in cube").clicked() {
                        out.focus_in_cube = true;
                    }
                    eyebrow(ui, theme, "orbit scale");
                    for s in [OrbitScale::Log, OrbitScale::Sqrt, OrbitScale::True] {
                        if ui.selectable_label(scale == s, s.label()).clicked() {
                            out.set_scale = Some(s);
                        }
                    }
                });

                let (a_min, a_max) = sys.axis_range();
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if true_mix > 0.55 {
                        format!(
                            "True relative orbits. The innermost is {:.1}% of the outermost.",
                            a_min / a_max * 100.0
                        )
                    } else {
                        format!(
                            "Compressed so every orbit stays legible — they really differ by {:.1}×.",
                            a_max / a_min
                        )
                    })
                    .size(11.0)
                    .color(theme.soft),
                );

                section(ui, theme, "Archive · NASA");
                let hz = sys.hz();
                fact(ui, theme, "distance", format!(
                    "{} pc / {} ly",
                    num(sys.dist_pc, 3),
                    num(sys.dist_pc.map(|d| d * PC_IN_LY), 2)
                ));
                fact(ui, theme, "RA / Dec", format!("{:.4}° {:.4}°", sys.ra, sys.dec));
                fact(ui, theme, "spectral type", format!(
                    "{} · {} K",
                    sys.spectype.as_deref().unwrap_or("—"),
                    num(sys.teff, 0)
                ));
                fact(ui, theme, "radius / mass", format!(
                    "{} R☉ · {} M☉",
                    num(sys.radius_sun, 3),
                    num(sys.mass_sun, 3)
                ));
                fact(ui, theme, "luminosity", hz.map(|z| format!("{} L☉", num(Some(z.l_sun), 4))).unwrap_or("—".into()));
                fact(ui, theme, "habitable zone", hz
                    .map(|z| format!("{} – {} AU", num(Some(z.inner), 3), num(Some(z.outer), 3)))
                    .unwrap_or("—".into()));
                fact(ui, theme, "V magnitude", num(sys.vmag, 2));
                fact(ui, theme, "planets", sys.planets.len().to_string());

                // A whole-system table at a glance, so the operator does not
                // have to click through every planet to compare them.
                if !sys.planets.is_empty() {
                    section(ui, theme, "Planets");
                    egui::Grid::new("planet-table")
                        .num_columns(5)
                        .striped(true)
                        .spacing([10.0, 3.0])
                        .show(ui, |ui| {
                            for h in ["planet", "a (AU)", "P (d)", "R⊕", "M⊕"] {
                                ui.label(RichText::new(h).size(9.0).color(theme.soft).strong());
                            }
                            ui.end_row();

                            let mut any_derived = false;
                            for p in &sys.planets {
                                let axis = sys.axis_of(p);
                                let a = axis.map(|(a, _)| a);
                                let derived = axis.map(|(_, d)| d).unwrap_or(false);
                                any_derived |= derived;
                                let in_hz = match (hz, a) {
                                    (Some(z), Some(a)) => z.contains(a),
                                    _ => false,
                                };

                                let mut name = RichText::new(p.short_name(&sys.hostname))
                                    .monospace()
                                    .size(10.5);
                                if in_hz {
                                    name = name.color(theme.accent).strong();
                                }
                                if ui.selectable_label(false, name).clicked() {
                                    out.focus_planet = Some(Some(p.name.clone()));
                                }
                                for v in [
                                    format!(
                                        "{}{}",
                                        num(a, if a.unwrap_or(1.0) < 0.1 { 4 } else { 3 }),
                                        if derived { "*" } else { "" }
                                    ),
                                    num(p.orbper, if p.orbper.unwrap_or(0.0) > 100.0 { 0 } else { 2 }),
                                    num(p.rade, 2),
                                    num(p.bmasse, 2),
                                ] {
                                    ui.label(RichText::new(v).monospace().size(10.5));
                                }
                                ui.end_row();
                            }
                            if any_derived {
                                ui.label(
                                    RichText::new("* axis from Kepler's third law")
                                        .size(9.0)
                                        .color(theme.dim),
                                );
                                ui.end_row();
                            }
                        });
                }

                section(ui, theme, "Dossier · yours");
                field(ui, theme, "Imperial name", &mut sys.record.imperial_name, "unnamed", false);

                ui.add_space(8.0);
                eyebrow(ui, theme, "Galactic arm");
                let current = sys.record.arm;
                egui::ComboBox::from_id_salt("arm-select")
                    .width(ui.available_width())
                    .selected_text(current.map(|a| a.name()).unwrap_or("unassigned"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sys.record.arm, None, "unassigned");
                        for arm in Arm::ALL {
                            ui.selectable_value(
                                &mut sys.record.arm,
                                Some(arm),
                                format!("{} — {}", arm.name(), arm.subtitle()),
                            );
                        }
                    });
                ui.label(
                    RichText::new(
                        "Colours the cube when colour is set to arm. Everything within about a \
                         kiloparsec of Sol really is in Orion–Cygnus; the rest of the list is for \
                         systems of your own.",
                    )
                    .size(10.0)
                    .color(theme.dim),
                );

                field(ui, theme, "Population", &mut sys.record.population, "e.g. 4.1 billion, 12 stations, uninhabited", false);
                field(ui, theme, "Notes", &mut sys.record.notes,
                    "#tags become filters. [[System name]] draws a link in the cube.", true);

                if !links.is_empty() {
                    ui.add_space(8.0);
                    eyebrow(ui, theme, "Links");
                    ui.horizontal_wrapped(|ui| {
                        // Labelled by display name, not by slug: `gj-1061` is an
                        // internal key, not something to show the operator.
                        for (id, label) in &links {
                            if ui.small_button(label.as_str()).clicked() {
                                out.select = Some(id.clone());
                            }
                        }
                    });
                }

                if let (Some(m), Some(other)) = (measurement, compared_name) {
                    ui.add_space(14.0);
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0, theme.accent))
                        .fill(theme.alpha(theme.accent, 0.08))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            eyebrow(ui, theme, "Measured separation");
                            ui.label(RichText::new(format!("{:.3} pc", m.pc)).monospace().size(20.0).strong());
                            ui.label(
                                RichText::new(format!("{} ↔ {}", sys.hostname, other))
                                    .monospace().size(10.5).color(theme.soft),
                            );
                            for line in [
                                format!("{:.3} light years", m.ly),
                                format!("{:.2e} AU", m.au),
                                format!("{:.0} years at Voyager 1's speed", m.voyager_years),
                            ] {
                                ui.label(RichText::new(line).monospace().size(10.5).color(theme.dim));
                            }
                        });
                }
            }

            /* ================================================== PLANET == */
            Some(planet_name) => {
                let Some(idx) = sys.planets.iter().position(|p| p.name == planet_name) else {
                    out.focus_planet = Some(None);
                    return;
                };
                let planet = sys.planets[idx].clone();
                let axis = sys.axis_of(&planet);
                let hz = sys.hz();
                let in_hz = match (hz, axis) {
                    (Some(z), Some((a, _))) => z.contains(a),
                    _ => false,
                };
                let pr = sys.planet_record(&planet_name);
                let display = if pr.imperial_name.trim().is_empty() {
                    planet.name.clone()
                } else {
                    pr.imperial_name.clone()
                };

                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(display).size(17.0).strong());
                    if in_hz {
                        badge(ui, theme, "habitable zone", true);
                    }
                });
                if !pr.imperial_name.trim().is_empty() {
                    ui.label(RichText::new(&planet.name).monospace().size(10.5).color(theme.dim));
                }

                section(ui, theme, "Archive · NASA");
                let derived = axis.map(|(_, d)| d).unwrap_or(false);
                let a = axis.map(|(a, _)| a);
                let s = hz.zip(a).and_then(|(z, a)| insolation(z.l_sun, a));
                let teq = planet.eqt.or_else(|| hz.zip(a).and_then(|(z, a)| equilibrium_temp(z.l_sun, a)));

                fact(ui, theme, "class", planet_class(planet.rade).to_string());
                fact(ui, theme, "semi-major axis", format!(
                    "{} AU{}",
                    num(a, if a.unwrap_or(1.0) < 0.1 { 4 } else { 3 }),
                    if derived { " *" } else { "" }
                ));
                fact(ui, theme, "orbital period", format!(
                    "{} days",
                    num(planet.orbper, if planet.orbper.unwrap_or(0.0) > 100.0 { 1 } else { 3 })
                ));
                fact(ui, theme, "radius", format!("{} R⊕", num(planet.rade, 3)));
                fact(ui, theme, "mass", format!("{} M⊕", num(planet.bmasse, 3)));
                fact(ui, theme, "eccentricity", num(planet.orbeccen, 3));
                fact(ui, theme, "insolation", s.map(|v| format!("{} S⊕", num(Some(v), 2))).unwrap_or("—".into()));
                fact(ui, theme, "equilibrium temp", teq
                    .map(|v| format!("{} K{}", num(Some(v), 0), if planet.eqt.is_some() { "" } else { " *" }))
                    .unwrap_or("—".into()));
                fact(ui, theme, "discovery", format!(
                    "{}{}",
                    planet.disc_method.as_deref().unwrap_or("—"),
                    planet.disc_year.map(|y| format!(", {y}")).unwrap_or_default()
                ));
                fact(ui, theme, "facility", planet.disc_facility.clone().unwrap_or("—".into()));

                ui.add_space(5.0);
                ui.label(
                    RichText::new(
                        "* derived, not measured — axis from Kepler's third law, temperature from \
                         luminosity at albedo 0.3.",
                    )
                    .monospace().size(10.0).color(theme.dim),
                );

                section(ui, theme, "Dossier · yours");
                let rec = sys.planet_record_mut(&planet_name);
                field(ui, theme, "Imperial name", &mut rec.imperial_name, "unnamed", false);
                field(ui, theme, "Population", &mut rec.population, "e.g. 900 million, orbital only, none", false);
                field(ui, theme, "Continents", &mut rec.continents, "comma separated", false);
                let count = rec.continent_count();
                if count > 0 {
                    ui.label(RichText::new(format!("{count} listed")).monospace().size(10.0).color(theme.dim));
                }
                field(ui, theme, "Notes", &mut rec.notes, "#tags and [[links]] work here too.", true);
            }
        }
    });

    out
}
