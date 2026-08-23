//! Wiring. Layout, state, the shared orbital clock, and the honest-scale strip.

use egui::{RichText, Ui};

use crate::core::astro::{DistanceMode, OrbitScale, REARTH_IN_AU, RSUN_IN_AU};
use crate::core::camera::Camera;
use crate::core::index::VaultIndex;
use crate::core::model::System;
use crate::core::nasa::{host_adql, query_url, search_adql};
use crate::core::orrery::layout;
use crate::core::store::{MemoryStore, Settings, VaultStore};
use crate::core::vault::Vault;
use crate::ui::cube::{self, CubeSettings};
use crate::ui::http;
use crate::ui::record_panel;
use crate::ui::theme::{badge, eyebrow, ColorMode, Mode, Theme};
use crate::ui::vault_panel::{self, VaultState};

#[cfg(feature = "client")]
use crate::client::http_store::HttpStore;
#[cfg(feature = "db")]
use crate::db::PgStore;

#[cfg(feature = "db")]
use crate::db::{StoreHandle, StoreRequest, StoreUpdate};
#[cfg(not(feature = "db"))]
use crate::db_stub::{StoreHandle, StoreRequest, StoreUpdate};

pub struct Parallax {
    vault: Vault,
    /// Derived state, rebuilt only when `vault.revision()` changes.
    index: VaultIndex,
    /// Reused across frames so filtering allocates nothing.
    visible: Vec<usize>,
    store: Option<StoreHandle>,
    /// Kept alive for its side effect: dropping it stops the SSE thread.
    _changes: Option<crate::client::ChangeListener>,
    backend: String,
    /// Set when a dossier field changed this frame and needs persisting.
    dirty_record: bool,
    last_settings: Settings,
    cam: Camera,
    theme_mode: Mode,

    // Display settings.
    color_mode: ColorMode,
    distance: DistanceMode,
    system_scale: f32,
    show_links: bool,
    orbit_scale: OrbitScale,
    /// Tweens 0→1 when `orbit_scale` becomes `True`, so the collapse is visible.
    true_mix: f64,

    // The shared clock. Every orrery in the app reads it, so relative orbital
    // rates are correct across systems as well as within one.
    speed: f64,
    clock_days: f64,

    // Archive.
    slot: http::Slot,
    busy: bool,
    results: Vec<System>,
    error: Option<String>,
    refreshing: Option<String>,

    // Panels.
    vault_state: VaultState,
    scale_open: bool,
    status: Option<String>,
    px_per_pc: f64,
}

impl Default for Parallax {
    fn default() -> Self {
        Parallax {
            vault: Vault::seeded(),
            index: VaultIndex::new(),
            visible: Vec::new(),
            store: None,
            _changes: None,
            backend: "not connected".into(),
            dirty_record: false,
            last_settings: Settings::default(),
            cam: Camera::default(),
            theme_mode: Mode::Plate,
            color_mode: ColorMode::Temperature,
            distance: DistanceMode::Linear,
            system_scale: 1.0,
            show_links: true,
            orbit_scale: OrbitScale::Log,
            true_mix: 0.0,
            speed: 4.0,
            clock_days: 0.0,
            slot: http::slot(),
            busy: false,
            results: Vec::new(),
            error: None,
            refreshing: None,
            vault_state: VaultState::default(),
            scale_open: false,
            status: None,
            px_per_pc: 60.0,
        }
    }
}

impl Parallax {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Parallax::default();
        Theme::new(app.theme_mode).apply(&cc.egui_ctx);

        let ctx = cc.egui_ctx.clone();
        let wake = move || ctx.request_repaint();

        // Three backends, in descending order of preference.
        //
        // The server is first because it is the only one that is safe with more
        // than one operator: it does field-level writes, per-user settings and
        // change notification. Direct PostgreSQL is kept for a single-operator
        // desktop, and is only reached if PARALLAX_DATABASE_URL is set
        // explicitly. In-memory is the last resort so a missing backend
        // degrades to a usable session rather than a crash.
        let handle = Self::open_store(&mut app, wake);

        app.backend = handle.label().to_string();
        handle.send(StoreRequest::Load);

        // Watch for other operators' edits. Without this the face is correct
        // but stale: someone renames a system and you keep the old name until
        // you restart.
        #[cfg(feature = "client")]
        {
            let base = std::env::var("PARALLAX_SERVER_URL").ok();
            if let Some(base) = base {
                let token = std::env::var("PARALLAX_TOKEN").ok().filter(|t| !t.is_empty());
                let ctx = cc.egui_ctx.clone();
                let reload = handle.sender();
                app._changes = Some(crate::client::ChangeListener::spawn(
                    &base,
                    token,
                    move || {
                        let _ = reload.send(StoreRequest::Load);
                        ctx.request_repaint();
                    },
                ));
            }
        }

        app.store = Some(handle);

        app.reframe(true);
        app
    }

    /// Pick a backend from configuration, without touching the network.
    ///
    /// This used to probe: construct an `HttpStore`, call `migrate()` — a
    /// blocking `GET /health` with a five second connect timeout — and fall
    /// through to PostgreSQL if it failed. That ran on the UI thread inside
    /// `new()`, so a server that was down or slow froze the window before it
    /// ever opened, for up to five seconds, every launch.
    ///
    /// It was also redundant: `StoreHandle::spawn` already calls `migrate()` on
    /// the worker thread and reports failure as a `StoreUpdate::Error` the next
    /// frame. So the choice is made from environment variables, which is
    /// instant, and reachability is discovered asynchronously exactly as every
    /// other backend error already is.
    fn open_store(app: &mut Parallax, wake: impl Fn() + Send + 'static) -> StoreHandle {
        #[cfg(feature = "client")]
        {
            if std::env::var("PARALLAX_SERVER_URL").is_ok() {
                let http = HttpStore::from_env();
                app.status = Some(format!("Connecting to {}…", http.describe()));
                return StoreHandle::spawn(http, wake);
            }
        }
        #[cfg(feature = "db")]
        {
            if std::env::var("PARALLAX_DATABASE_URL").is_ok() {
                // Connecting the driver is itself blocking, but it is a local
                // socket and only happens when the operator has explicitly
                // asked for the single-user path.
                match PgStore::from_env() {
                    Ok(store) => {
                        app.status = Some(
                            "Connected directly to PostgreSQL — single operator only.".into(),
                        );
                        return StoreHandle::spawn(store, wake);
                    }
                    Err(e) => app.status = Some(e.to_string()),
                }
            }
        }
        app.status = Some(
            "No vault backend configured — this session is in memory and will not be saved."
                .into(),
        );
        StoreHandle::spawn(MemoryStore::seeded(), wake)
    }

    /// Drain worker updates. Called once per frame; never blocks.
    fn poll_store(&mut self) {
        let Some(store) = &self.store else { return };
        let updates: Vec<StoreUpdate> = store.updates().collect();
        for update in updates {
            match update {
                StoreUpdate::Loaded(snap) => {
                    if snap.systems.is_empty() {
                        // First run against an empty database: write the shipped
                        // neighbourhood so there is something to look at.
                        let seeded = Vault::seeded();
                        for sys in &seeded.systems {
                            self.send(StoreRequest::InsertWithDossier(Box::new(sys.clone())));
                        }
                        self.vault = seeded;
                        self.status = Some("Empty vault — seeded the local neighbourhood.".into());
                    } else {
                        self.vault.systems = snap.systems;
                        self.vault.selected = snap.settings.selected.clone();
                        self.vault.compare = snap.settings.compare.clone();
                        self.vault.focus_planet = snap.settings.focus_planet.clone();
                        if self.vault.selected.is_none() {
                            self.vault.selected =
                                self.vault.systems.first().map(|s| s.id.clone());
                        }
                        self.last_settings = snap.settings;
                        self.vault.touch();
                    }
                    self.reframe(true);
                }
                StoreUpdate::Committed { .. } => {}
                StoreUpdate::Error(e) => self.status = Some(e),
            }
        }
    }

    fn send(&self, req: StoreRequest) {
        if let Some(s) = &self.store {
            s.send(req);
        }
    }

    /// Persist whatever changed this frame. Dossier edits are coalesced by the
    /// worker, so calling this every frame is cheap.
    fn persist_changes(&mut self) {
        if self.dirty_record {
            self.dirty_record = false;
            if let Some(sys) = self.vault.selected() {
                self.send(StoreRequest::SaveRecord {
                    system_id: sys.id.clone(),
                    record: sys.record.clone(),
                });
                if let Some(name) = self.vault.focus_planet.clone() {
                    let record = sys.planet_record(&name);
                    self.send(StoreRequest::SavePlanetRecord {
                        system_id: sys.id.clone(),
                        planet_name: name,
                        record,
                    });
                }
            }
        }
        let now = Settings {
            selected: self.vault.selected.clone(),
            compare: self.vault.compare.clone(),
            focus_planet: self.vault.focus_planet.clone(),
        };
        if now != self.last_settings {
            self.last_settings = now.clone();
            self.send(StoreRequest::SaveSettings(now));
        }
    }

    fn theme(&self) -> Theme {
        Theme::new(self.theme_mode)
    }

    /// Re-fit the cube and re-centre on the selection.
    fn reframe(&mut self, settle: bool) {
        let extent = self.index.extent();
        if let Some(sel) = self.vault.selected() {
            self.cam.look_at(self.vault.draw_pos(sel, self.distance));
        }
        self.cam.fit(extent);
        if settle {
            self.cam.settle();
        }
    }

    fn recentre(&mut self) {
        if let Some(sel) = self.vault.selected() {
            let p = self.vault.draw_pos(sel, self.distance);
            self.cam.look_at(p);
        }
    }

    fn start_fetch(&mut self, adql: String, ctx: &egui::Context) {
        self.busy = true;
        self.error = None;
        self.results.clear();
        *self.slot.lock().unwrap() = None;
        let ctx = ctx.clone();
        http::fetch(&adql, self.slot.clone(), move || ctx.request_repaint());
    }

    fn poll_fetch(&mut self) {
        let taken = self.slot.lock().unwrap().take();
        let Some(result) = taken else { return };
        self.busy = false;
        match result {
            Ok(systems) => {
                match self.refreshing.take() {
                    // A refresh targets one host: fold it straight in.
                    Some(host) => {
                        match systems.into_iter().find(|s| s.hostname == host) {
                            Some(fresh) => {
                                let name = fresh.hostname.clone();
                                self.send(StoreRequest::UpsertSystem(Box::new(fresh.clone())));
                                self.vault.upsert(fresh);
                                self.status = Some(format!("{name} refreshed. Your dossier was kept."));
                            }
                            None => {
                                self.status =
                                    Some(format!("{host} is not in pscomppars under that name."));
                            }
                        }
                    }
                    None => {
                        self.status = Some(format!("{} system(s) found.", systems.len()));
                        self.results = systems;
                    }
                }
            }
            Err(e) => {
                self.refreshing = None;
                self.error = Some(e.clone());
                self.vault_state.fallback_open = true;
                self.status = Some(e);
            }
        }
    }

    fn tick_clock(&mut self, ctx: &egui::Context) {
        if self.speed > 0.0 {
            let dt = ctx.input(|i| i.stable_dt).min(0.1) as f64;
            // Days of simulated time per second of wall clock.
            self.clock_days += dt * self.speed * 8.0;
            ctx.request_repaint();
        }
        let want = if self.orbit_scale.is_true() { 1.0 } else { 0.0 };
        if (self.true_mix - want).abs() > 1e-4 {
            let dt = ctx.input(|i| i.stable_dt).min(0.1) as f64;
            let step = dt / 1.3;
            self.true_mix += (want - self.true_mix).signum() * step;
            self.true_mix = self.true_mix.clamp(0.0, 1.0);
            ctx.request_repaint();
        }
    }
}

impl eframe::App for Parallax {
    /// The vault lives in PostgreSQL, not in eframe's blob store. All this does
    /// is make sure nothing is still sitting in the worker's coalescing buffer.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.send(StoreRequest::Flush);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_store();
        self.poll_fetch();
        self.tick_clock(ctx);
        // One rebuild per edit, not one per frame.
        self.index.sync(&self.vault, self.distance);
        let theme = self.theme();

        /* ------------------------------------------------------- header -- */
        egui::TopBottomPanel::top("bar").exact_height(48.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.5, theme.ink);
                ui.label(RichText::new("P A R A L L A X").size(16.0).strong());
                ui.label(RichText::new("a vault for star systems").size(11.0).color(theme.soft));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(self.theme_mode.other_label()).clicked() {
                        self.theme_mode = self.theme_mode.toggled();
                        Theme::new(self.theme_mode).apply(ctx);
                    }
                    if ui.button("Scale").clicked() {
                        self.scale_open = true;
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} systems · {} planets · furthest {:.1} pc · {}",
                            self.vault.systems.len(),
                            self.vault.planet_count(),
                            self.vault.furthest_pc(),
                            self.backend
                        ))
                        .monospace()
                        .size(10.5)
                        .color(theme.soft),
                    );
                });
            });
        });

        /* -------------------------------------------------- scale strip -- */
        let strip_data = self.strip_data();
        egui::TopBottomPanel::bottom("strip").exact_height(44.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);
                eyebrow(ui, &theme, "cube");
                badge(ui, &theme, self.distance.label(), self.distance.is_true());
                ui.label(RichText::new(format!("1 pc = {:.1} px", self.px_per_pc)).monospace().size(10.5));

                ui.add_space(16.0);
                eyebrow(ui, &theme, "orbits");
                badge(
                    ui,
                    &theme,
                    if self.orbit_scale.is_true() { "true" } else { "compressed" },
                    self.orbit_scale.is_true(),
                );
                ui.label(RichText::new(strip_data.0).monospace().size(10.5));

                ui.add_space(16.0);
                eyebrow(ui, &theme, "honest number");
                ui.label(RichText::new(strip_data.1).monospace().size(10.5).color(theme.soft));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(RichText::new("WHY ↗").size(10.0).color(theme.accent)).clicked() {
                        self.scale_open = true;
                    }
                });
            });
        });

        /* -------------------------------------------------------- vault -- */
        egui::SidePanel::left("vault").resizable(true).default_width(268.0).show(ctx, |ui| {
            self.index.filter_into(&self.vault_state.filter, &mut self.visible);
            let out = vault_panel::show(
                ui, &theme, &self.vault, &self.visible, self.index.tags(),
                &mut self.vault_state, &self.results,
                self.busy, self.error.as_deref(), self.clock_days, self.color_mode,
            );
            if let Some(id) = out.select {
                self.vault.select(&id);
                self.recentre();
            }
            if let Some(term) = out.search {
                self.start_fetch(search_adql(&term), ctx);
            }
            if let Some(i) = out.save {
                if let Some(sys) = self.results.get(i).cloned() {
                    let name = sys.hostname.clone();
                    self.send(StoreRequest::UpsertSystem(Box::new(sys.clone())));
                    self.vault.upsert(sys);
                    self.status = Some(format!("{name} saved. The cube re-framed to fit it."));
                    self.reframe(false);
                    self.vault_state.add_open = false;
                }
            }
            if out.copy_url {
                let q = if self.vault_state.query.trim().is_empty() { "GJ 1061" } else { &self.vault_state.query };
                ctx.copy_text(query_url(&search_adql(q)));
                self.status = Some("Query URL copied.".into());
            }
            if out.import {
                match crate::core::nasa::parse_rows(&self.vault_state.paste) {
                    Ok(systems) => {
                        let n = systems.len();
                        for s in systems {
                            self.send(StoreRequest::UpsertSystem(Box::new(s.clone())));
                            self.vault.upsert(s);
                        }
                        self.vault_state.paste.clear();
                        self.vault_state.add_open = false;
                        self.error = None;
                        self.status = Some(format!("Imported {n} system(s)."));
                        self.reframe(false);
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        });

        /* ------------------------------------------------------- record -- */
        egui::SidePanel::right("record").resizable(true).default_width(340.0).show(ctx, |ui| {
            let before = self.vault.revision();
            let out = record_panel::show(
                ui, &theme, &mut self.vault, self.clock_days, self.orbit_scale,
                self.true_mix, self.color_mode, self.busy,
            );
            // The panel binds text boxes straight to the model, so any revision
            // bump here means a dossier field was edited.
            if self.vault.revision() != before {
                self.dirty_record = true;
            }
            if let Some(id) = out.select {
                self.vault.select(&id);
                self.recentre();
            }
            if let Some(fp) = out.focus_planet {
                self.vault.focus_planet = fp;
            }
            if let Some(s) = out.set_scale {
                self.orbit_scale = s;
            }
            if out.focus_in_cube {
                let e = self.index.extent();
                self.recentre();
                self.cam.focus(e);
            }
            if out.refresh {
                if let Some(sel) = self.vault.selected() {
                    let host = sel.hostname.clone();
                    self.refreshing = Some(host.clone());
                    self.start_fetch(host_adql(&host), ctx);
                }
            }
            if out.remove {
                if let Some(id) = self.vault.selected.clone() {
                    self.vault.remove(&id);
                    self.send(StoreRequest::DeleteSystem(id));
                    self.reframe(false);
                }
            }
        });

        /* --------------------------------------------------- the centre -- */
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme.plate).inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                self.controls(ui, &theme);
                ui.add_space(6.0);
                let cfg = CubeSettings {
                    distance: self.distance,
                    color: self.color_mode,
                    system_scale: self.system_scale,
                    show_links: self.show_links,
                    clock_days: self.clock_days,
                };
                let out = cube::show(ui, &theme, &self.vault, &self.index, &mut self.cam, &cfg);
                self.px_per_pc = out.px_per_pc;
                // The peek card is drawn outside the canvas so it can host a
                // real SystemView rather than a hand-painted approximation.
                if let (Some(id), Some(at)) = (&out.hovered, out.hover_anchor) {
                    if let Some(sys) = self.vault.get(id) {
                        cube::peek_card(ui, &theme, sys, at, self.clock_days, self.color_mode);
                    }
                }
                if out.animating {
                    ctx.request_repaint();
                }
                if let Some(id) = out.select {
                    self.vault.select(&id);
                    self.recentre();
                }
                if let Some(id) = out.compare {
                    self.vault.toggle_compare(&id);
                }
            });

        self.persist_changes();
        self.scale_sheet(ctx, &theme);

        if let Some(msg) = self.status.clone() {
            egui::Window::new("status")
                .title_bar(false)
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -56.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(msg).size(11.5));
                        if ui.small_button("dismiss").clicked() {
                            self.status = None;
                        }
                    });
                });
        }
    }
}

impl Parallax {
    fn controls(&mut self, ui: &mut Ui, theme: &Theme) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                eyebrow(ui, theme, "systems");
                ui.horizontal(|ui| {
                    for (v, l) in [(0.0, "dots"), (1.0, "×1"), (2.0, "×2"), (3.5, "×4")] {
                        if ui.selectable_label((self.system_scale - v).abs() < 0.01, l).clicked() {
                            self.system_scale = v;
                        }
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                eyebrow(ui, theme, "motion");
                ui.horizontal(|ui| {
                    for (v, l) in [(0.0, "❚❚"), (1.0, "1×"), (4.0, "4×"), (30.0, "30×")] {
                        if ui.selectable_label((self.speed - v).abs() < 0.01, l).clicked() {
                            self.speed = v;
                        }
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                eyebrow(ui, theme, "colour");
                ui.horizontal(|ui| {
                    for (m, l) in [(ColorMode::Temperature, "temp"), (ColorMode::Arm, "arm")] {
                        if ui.selectable_label(self.color_mode == m, l).clicked() {
                            self.color_mode = m;
                        }
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                eyebrow(ui, theme, "distance");
                ui.horizontal(|ui| {
                    for (m, l) in [(DistanceMode::Linear, "true"), (DistanceMode::Log, "log")] {
                        if ui.selectable_label(self.distance == m, l).clicked() && self.distance != m {
                            self.distance = m;
                            self.reframe(false);
                        }
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                eyebrow(ui, theme, "framing");
                ui.horizontal(|ui| {
                    let e = self.index.extent();
                    if ui.small_button("Focus").clicked() {
                        self.recentre();
                        self.cam.focus(e);
                    }
                    if ui.small_button("Fit all").clicked() {
                        self.cam.fit(e);
                    }
                    ui.checkbox(&mut self.show_links, "[[links]]");
                    if self.vault.compare.is_some() && ui.small_button("clear measure").clicked() {
                        self.vault.compare = None;
                    }
                });
            });
        });
    }

    /// The two live figures in the strip: the outermost orbit, and how badly
    /// the orreries are exaggerating the system relative to the cube's scale.
    fn strip_data(&self) -> (String, String) {
        let Some(sys) = self.vault.selected() else {
            return ("—".into(), "no system selected".into());
        };
        let lay = layout(sys, 235.0, self.clock_days, self.orbit_scale, self.true_mix);
        let true_width = lay.true_width_px(self.px_per_pc);
        let exaggeration = if true_width > 0.0 { 34.0 / true_width } else { f64::INFINITY };
        (
            format!("outermost {:.*} AU", if lay.a_max < 0.1 { 3 } else { 2 }, lay.a_max),
            format!(
                "drawn truthfully in the cube, {} would be {:.1e} px wide — the orreries exaggerate it {:.1e}×",
                sys.hostname, true_width, exaggeration
            ),
        )
    }

    fn scale_sheet(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.scale_open {
            return;
        }
        let sys_name = self.vault.selected().map(|s| s.hostname.clone()).unwrap_or_default();
        let (star_pct, earth_pct) = match self.vault.selected() {
            Some(s) => {
                let (_, a_max) = s.axis_range();
                (
                    s.radius_sun.unwrap_or(0.0) * RSUN_IN_AU / a_max * 100.0,
                    REARTH_IN_AU / a_max * 100.0,
                )
            }
            None => (0.0, 0.0),
        };
        let px = self.px_per_pc;

        let mut open = true;
        egui::Window::new(RichText::new("How scale works here").size(13.0))
            .open(&mut open)
            .collapsible(false)
            .default_width(600.0)
            .show(ctx, |ui| {
                let para = |ui: &mut Ui, s: String| {
                    ui.label(RichText::new(s).size(12.0).color(theme.soft));
                    ui.add_space(6.0);
                };
                para(ui, "One scale cannot hold a galaxy and a planet at once — the gap is about \
                          seventeen orders of magnitude. Parallax uses four, and labels which one \
                          you are looking at wherever it matters.".into());

                ui.separator();
                ui.horizontal(|ui| { badge(ui, theme, "true", true); ui.label(RichText::new("Where systems sit — the cube").strong()); });
                para(ui, format!(
                    "Real right ascension, declination and distance, converted to Cartesian parsecs \
                     with the Sun at the origin. Shift-click any two stars and the number you get is \
                     the real three-dimensional separation, never a projection. Currently {px:.1} px \
                     per parsec. The camera pivots on whatever you last clicked, so rotating and \
                     zooming both act on that system rather than on the Sun."
                ));

                ui.separator();
                ui.horizontal(|ui| { badge(ui, theme, "optional", false); ui.label(RichText::new("Distance compression — log-radial").strong()); });
                para(ui, "Add something at 200 pc and the local neighbourhood collapses to a dot, \
                          because it truthfully is one. Switching distance to log replaces each radius \
                          r with ln(1 + r), keeping every direction exact while pulling far systems \
                          inward. Measurements stay true regardless, and the strip flags the mode.".into());

                ui.separator();
                ui.horizontal(|ui| { badge(ui, theme, "compressed", false); ui.label(RichText::new("Orbits — inside every system").strong()); });
                para(ui, "The orreries in the cube and in the record panel place orbits on a log \
                          radius, so a 0.02 AU orbit and a 30 AU orbit share one frame. Planets move \
                          at their real relative rates from a single shared clock, so a TRAPPIST-1 \
                          planet really does lap Neptune thousands of times. Set orbit scale to true \
                          and watch the inner planets fall in — the size of that collapse is the size \
                          of the distortion.".into());

                ui.separator();
                ui.horizontal(|ui| { badge(ui, theme, "symbolic", false); ui.label(RichText::new("Bodies — always exaggerated").strong()); });
                para(ui, format!(
                    "Star and planet discs are never to scale, in any mode. For {sys_name} the star's \
                     radius is about {star_pct:.3}% of the outermost orbit, and an Earth-sized planet \
                     about {earth_pct:.1e}%. Drawn faithfully they would be well under a pixel. Disc \
                     area encodes radius on a log scale; colour encodes either surface temperature or \
                     the arm you assigned."
                ));

                ui.separator();
                ui.label(RichText::new("Sources and method").strong());
                ui.label(RichText::new(
                    "Data: NASA Exoplanet Archive, Planetary Systems Composite Parameters \
                     (pscomppars), via TAP.\n\
                     Position: x = d·cos δ·cos α, y = d·cos δ·sin α, z = d·sin δ.\n\
                     Luminosity: L = R²·(T/5772)⁴. Habitable zone: conservative limits at S = 1.10 \
                     and 0.53 S⊕.\n\
                     Missing axes from Kepler's third law; missing T_eq from luminosity at albedo 0.3.\n\
                     Arm assignment is yours, not the archive's. 1 pc = 3.26156 ly = 206,265 AU.",
                ).monospace().size(10.0).color(theme.dim));
            });
        self.scale_open = open;
    }
}
