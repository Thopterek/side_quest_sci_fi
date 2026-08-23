//! Entry points. The same `Parallax` app runs natively and in the browser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([980.0, 640.0])
            .with_title("Parallax — a vault for star systems"),
        ..Default::default()
    };
    eframe::run_native(
        "parallax",
        options,
        Box::new(|cc| Ok(Box::new(parallax::app::Parallax::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();
    let options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id("parallax_canvas")
            .expect("missing <canvas id=\"parallax_canvas\">")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("that element is not a canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(|cc| Ok(Box::new(parallax::app::Parallax::new(cc)))),
            )
            .await
            .expect("failed to start parallax");
    });
}
