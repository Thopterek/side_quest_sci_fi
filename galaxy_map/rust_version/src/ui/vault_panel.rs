//! The left column: everything saved, and the way in from the archive.

use egui::{RichText, Ui};

use crate::core::vault::Vault;

use super::system_view;
use super::theme::{eyebrow, mono, num, ColorMode, Theme};

/// What the panel wants the app to do.
#[derive(Default)]
pub struct VaultOutput {
    pub select: Option<String>,
    pub search: Option<String>,
    pub save: Option<usize>,
    pub import: bool,
    pub copy_url: bool,
}

pub struct VaultState {
    pub filter: String,
    pub add_open: bool,
    pub query: String,
    pub paste: String,
    pub fallback_open: bool,
}

impl Default for VaultState {
    fn default() -> Self {
        VaultState {
            filter: String::new(),
            add_open: false,
            query: String::new(),
            paste: String::new(),
            fallback_open: false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    vault: &Vault,
    // `visible` is indices into vault.systems and `tags` is the tag list, both
    // precomputed from the cached index rather than rescanned per frame.
    visible: &[usize],
    tags: &[String],
    state: &mut VaultState,
    results: &[crate::core::model::System],
    busy: bool,
    error: Option<&str>,
    clock_days: f64,
    color_mode: ColorMode,
) -> VaultOutput {
    let mut out = VaultOutput::default();

    ui.horizontal(|ui| {
        eyebrow(ui, theme, "Vault");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if state.add_open { "Close" } else { "+ NASA" };
            if ui.small_button(label).clicked() {
                state.add_open = !state.add_open;
            }
        });
    });

    /* --------------------------------------------------- archive search -- */
    if state.add_open {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, theme.rule))
            .inner_margin(egui::Margin::same(9))
            .fill(theme.plate)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Search pscomppars — the same composite parameters behind NASA's catalog pages.",
                    )
                    .size(11.0)
                    .color(theme.soft),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .hint_text("GJ 1061, TRAPPIST-1, Kepler-186…")
                            .desired_width(ui.available_width() - 44.0),
                    );
                    let entered =
                        edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let clicked = ui.add_enabled(!busy, egui::Button::new(if busy { "…" } else { "Go" })).clicked();
                    if (entered || clicked) && !state.query.trim().is_empty() {
                        out.search = Some(state.query.clone());
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    for q in ["GJ 1061", "Kepler-186", "TOI-700", "K2-18", "HD 219134"] {
                        if ui.small_button(q).clicked() {
                            state.query = q.to_string();
                            out.search = Some(q.to_string());
                        }
                    }
                });

                if let Some(err) = error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(err).size(10.5).color(theme.warn));
                }

                if !results.is_empty() {
                    ui.add_space(6.0);
                    for (i, r) in results.iter().enumerate() {
                        ui.horizontal(|ui| {
                            system_view::thumb(ui, theme, r, 46.0, clock_days, color_mode);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&r.hostname).size(12.5).strong());
                                mono(
                                    ui,
                                    theme,
                                    format!(
                                        "{} pc · {}p · {}",
                                        num(r.dist_pc, 2),
                                        r.planets.len(),
                                        r.spectype.as_deref().unwrap_or("—")
                                    ),
                                );
                                // Naming the planets makes it obvious whether
                                // this is the system you meant before saving it.
                                let names: Vec<&str> =
                                    r.planets.iter().map(|p| p.name.as_str()).collect();
                                mono(ui, theme, names.join(", "));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let exists = vault.get(&r.id).is_some();
                                if ui.small_button(if exists { "Update" } else { "Save" }).clicked() {
                                    out.save = Some(i);
                                }
                            });
                        });
                    }
                }

                ui.add_space(4.0);
                let arrow = if state.fallback_open { "▾" } else { "▸" };
                if ui
                    .add(egui::Button::new(
                        RichText::new(format!("{arrow} Archive unreachable?")).size(9.5).color(theme.soft),
                    ).frame(false))
                    .clicked()
                {
                    state.fallback_open = !state.fallback_open;
                }
                if state.fallback_open {
                    ui.label(
                        RichText::new(
                            "Some networks block the archive. Open the query, copy the response, \
                             paste it here — identical result.",
                        )
                        .size(10.5)
                        .color(theme.soft),
                    );
                    if ui.small_button("Copy query URL").clicked() {
                        out.copy_url = true;
                    }
                    ui.add(
                        egui::TextEdit::multiline(&mut state.paste)
                            .hint_text("Paste JSON — starts with [{\"pl_name\":…")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    if ui
                        .add_enabled(!state.paste.trim().is_empty(), egui::Button::new("Import"))
                        .clicked()
                    {
                        out.import = true;
                    }
                }
            });
        ui.add_space(6.0);
    }

    /* ---------------------------------------------------------- filter -- */
    ui.add(
        egui::TextEdit::singleline(&mut state.filter)
            .hint_text("Filter name, dossier, tag")
            .desired_width(f32::INFINITY),
    );

    if !tags.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for tag in tags {
                let marker = format!("#{tag}");
                let on = state.filter == marker;
                if ui.selectable_label(on, RichText::new(&marker).monospace().size(10.0)).clicked() {
                    state.filter = if on { String::new() } else { marker };
                }
            }
        });
    }

    ui.add_space(4.0);

    /* ------------------------------------------------------------ list -- */
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if visible.is_empty() {
            ui.label(
                RichText::new("No match. Clear the filter, or pull a system from NASA.")
                    .size(11.5)
                    .color(theme.dim),
            );
            return;
        }
        for sys in visible.iter().filter_map(|&i| vault.systems.get(i)) {
            let selected = vault.selected.as_deref() == Some(sys.id.as_str());
            let bg = if selected { theme.alpha(theme.accent, 0.1) } else { egui::Color32::TRANSPARENT };
            let stroke = if selected {
                egui::Stroke::new(1.0, theme.accent)
            } else {
                egui::Stroke::NONE
            };

            let resp = egui::Frame::new()
                .fill(bg)
                .stroke(stroke)
                .inner_margin(egui::Margin::symmetric(6, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        system_view::thumb(ui, theme, sys, 44.0, clock_days, color_mode);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(sys.display_name()).size(12.5).strong());
                                if sys.origin {
                                    ui.label(
                                        RichText::new("YOU ARE HERE").size(8.5).monospace().color(theme.soft),
                                    );
                                }
                            });
                            let named = !sys.record.imperial_name.trim().is_empty();
                            mono(
                                ui,
                                theme,
                                format!(
                                    "{}{} pc · {}p",
                                    if named { format!("{} · ", sys.hostname) } else { String::new() },
                                    num(sys.dist_pc, 2),
                                    sys.planets.len()
                                ),
                            );
                        });
                        // Arm swatch, so the colour rule is legible in the list too.
                        if let Some(arm) = sys.record.arm {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let c = if theme.mode.is_plate() { arm.plate_rgb() } else { arm.negative_rgb() };
                                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                ui.painter().circle_filled(
                                    r.center(),
                                    3.5,
                                    egui::Color32::from_rgb(c[0], c[1], c[2]),
                                );
                            });
                        }
                    });
                })
                .response;

            if resp.interact(egui::Sense::click()).clicked() {
                out.select = Some(sys.id.clone());
            }
        }
    });

    out
}
