//! Two themes, and the rule for turning astronomy into colour.
//!
//! The default is **plate**: a light, warm grey ground with near-black marks,
//! after the photographic glass plates that astrometry was actually measured
//! on, where stars appear as dark deposits rather than points of light.
//! **Negative** inverts it. The toggle is the pairing a plate and its negative
//! always came in, not a generic dark mode.

use egui::{Color32, FontFamily, FontId, TextStyle};

use crate::core::astro::{ink_blend, teff_rgb};
use crate::core::model::System;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Plate,
    Negative,
}

impl Mode {
    pub fn is_plate(self) -> bool {
        self == Mode::Plate
    }
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Plate => Mode::Negative,
            Mode::Negative => Mode::Plate,
        }
    }
    /// Label for the button, which names the state you would move *to*.
    pub fn other_label(self) -> &'static str {
        match self {
            Mode::Plate => "Negative",
            Mode::Negative => "Plate",
        }
    }
}

/// How star colour is assigned in the cube.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Blackbody appearance from effective temperature.
    Temperature,
    /// The galactic arm the operator assigned.
    Arm,
}

#[derive(Copy, Clone, Debug)]
pub struct Theme {
    pub mode: Mode,
    /// Page ground.
    pub plate: Color32,
    /// Panel ground, one step down.
    pub deep: Color32,
    /// Pressed/inset ground.
    pub sunk: Color32,
    pub ink: Color32,
    pub soft: Color32,
    pub dim: Color32,
    /// Structural hairlines: cube edges, grid, table rules.
    pub rule: Color32,
    /// Fainter hairlines still.
    pub hair: Color32,
    /// Annotation ink. Selection, measurement, habitable zone.
    pub accent: Color32,
    /// Reserved for compression warnings and destructive actions.
    pub warn: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Theme {
    pub fn new(mode: Mode) -> Self {
        match mode {
            Mode::Plate => Theme {
                mode,
                plate: rgb(0xE4, 0xE4, 0xDA),
                deep: rgb(0xDB, 0xDB, 0xD0),
                sunk: rgb(0xD0, 0xD0, 0xC4),
                ink: rgb(0x16, 0x18, 0x1A),
                soft: rgb(0x6D, 0x72, 0x76),
                dim: rgb(0x8A, 0x8F, 0x91),
                rule: rgb(0xB3, 0xB4, 0xA6),
                hair: rgb(0xC6, 0xC7, 0xBA),
                accent: rgb(0x1F, 0x3F, 0x9E),
                warn: rgb(0x8E, 0x32, 0x18),
            },
            Mode::Negative => Theme {
                mode,
                plate: rgb(0x0B, 0x0D, 0x10),
                deep: rgb(0x10, 0x13, 0x17),
                sunk: rgb(0x17, 0x1B, 0x20),
                ink: rgb(0xE7, 0xE8, 0xE3),
                soft: rgb(0x9A, 0xA1, 0xA8),
                dim: rgb(0x76, 0x7E, 0x86),
                rule: rgb(0x2B, 0x30, 0x37),
                hair: rgb(0x22, 0x26, 0x2C),
                accent: rgb(0x7F, 0xA0, 0xFF),
                warn: rgb(0xE0, 0x83, 0x63),
            },
        }
    }

    pub fn alpha(&self, c: Color32, a: f32) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
    }

    /// The colour of a star, under the current colour rule.
    ///
    /// On the plate, temperature colours are blended toward ink so a star reads
    /// as a deposit on glass rather than an emitter on black.
    pub fn star_color(&self, sys: &System, cm: ColorMode) -> Color32 {
        match cm {
            ColorMode::Arm => match sys.record.arm {
                Some(arm) => {
                    let c = if self.mode.is_plate() { arm.plate_rgb() } else { arm.negative_rgb() };
                    rgb(c[0], c[1], c[2])
                }
                None => self.dim,
            },
            ColorMode::Temperature => {
                let c = teff_rgb(sys.teff);
                let c = if self.mode.is_plate() { ink_blend(c, 0.52) } else { c };
                rgb(c[0], c[1], c[2])
            }
        }
    }

    /// Install palette and type scale. Called whenever the mode flips.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.mode.is_plate() {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.override_text_color = Some(self.ink);
        visuals.panel_fill = self.deep;
        visuals.window_fill = self.plate;
        visuals.extreme_bg_color = self.plate;
        visuals.faint_bg_color = self.sunk;
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.alpha(self.accent, 0.18);
        visuals.selection.stroke = egui::Stroke::new(1.0, self.accent);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.hair);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.rule);
        visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, self.soft);
        visuals.widgets.hovered.weak_bg_fill = self.sunk;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, self.accent);
        visuals.widgets.active.weak_bg_fill = self.sunk;
        visuals.window_stroke = egui::Stroke::new(1.0, self.rule);
        ctx.set_visuals(visuals);

        ctx.style_mut(|style| {
            use FontFamily::{Monospace, Proportional};
            style.text_styles = [
                (TextStyle::Heading, FontId::new(20.0, Proportional)),
                (TextStyle::Body, FontId::new(13.0, Proportional)),
                (TextStyle::Button, FontId::new(12.0, Proportional)),
                // Every number in the app is monospaced, catalog-card style.
                (TextStyle::Monospace, FontId::new(11.5, Monospace)),
                (TextStyle::Small, FontId::new(10.5, Proportional)),
            ]
            .into();
            style.spacing.item_spacing = egui::vec2(6.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.visuals.button_frame = true;
        });
    }
}

/* -------------------------------------------------------------- type helpers */

/// Letterspaced uppercase section label. The stencilled annotation on the plate.
pub fn eyebrow(ui: &mut egui::Ui, t: &Theme, text: &str) {
    let spaced: String = text.to_uppercase().chars().flat_map(|c| [c, '\u{2009}']).collect();
    ui.label(
        egui::RichText::new(spaced)
            .size(9.5)
            .color(t.soft)
            .strong(),
    );
}

/// Small monospaced value text.
pub fn mono(ui: &mut egui::Ui, t: &Theme, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).monospace().size(10.5).color(t.soft));
}

/// A pill that names which scale regime the neighbouring thing is drawn in.
pub fn badge(ui: &mut egui::Ui, t: &Theme, text: &str, truthful: bool) {
    let c = if truthful { t.accent } else { t.warn };
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, c))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text.to_uppercase())
                    .monospace()
                    .size(8.5)
                    .color(c)
                    .strong(),
            );
        });
}

/// Format a float the way the whole app does: em dash for missing, exponent
/// notation only where a fixed-point figure would be meaningless.
pub fn num(v: Option<f64>, decimals: usize) -> String {
    match v {
        None => "—".to_string(),
        Some(x) if x.is_nan() => "—".to_string(),
        Some(x) => {
            let a = x.abs();
            if a != 0.0 && (a < 1e-3 || a >= 1e6) {
                format!("{x:.1e}")
            } else {
                format!("{x:.decimals$}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_render_as_an_em_dash_not_zero() {
        assert_eq!(num(None, 2), "—");
        assert_eq!(num(Some(f64::NAN), 2), "—");
        assert_eq!(num(Some(0.0), 2), "0.00");
    }

    #[test]
    fn very_small_and_very_large_values_switch_to_exponent_form() {
        assert!(num(Some(1.4e-5), 2).contains('e'));
        assert!(num(Some(2.0e7), 2).contains('e'));
        assert_eq!(num(Some(3.67), 2), "3.67");
    }

    #[test]
    fn the_two_modes_are_actually_inverses() {
        let p = Theme::new(Mode::Plate);
        let n = Theme::new(Mode::Negative);
        assert!(p.plate.r() > p.ink.r(), "plate is light on dark ink");
        assert!(n.plate.r() < n.ink.r(), "negative is dark on light ink");
        assert_eq!(Mode::Plate.toggled(), Mode::Negative);
    }
}
