//! Native entry point: one window, software-rendered viewport.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Brush Studio")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Brush Studio",
        options,
        Box::new(|cc| Ok(Box::new(brush_studio::StudioApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;
    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = eframe::web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("brush-studio-canvas")
            .expect("no #brush-studio-canvas")
            .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
            .expect("canvas element is not a canvas");
        if let Some(loading) = document.get_element_by_id("loading") {
            loading.remove();
        }
        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(brush_studio::StudioApp::new(cc)))),
            )
            .await
            .expect("failed to start Brush Studio");
    });
}
