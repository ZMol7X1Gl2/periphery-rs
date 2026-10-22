//! Periphery in the browser. Build with `trunk build` (or `trunk serve`) in
//! this directory; the page needs a Chromium browser for WebHID.

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    eframe::WebLogger::init(log_level()).ok();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document()).expect("browser document");
        let canvas = document
            .get_element_by_id("periphery")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("canvas #periphery");
        // `#test-devices` in the address adds made-up devices; see StartOptions.
        let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
        let start = periphery::ui::StartOptions { test_devices: hash == "#test-devices", ..Default::default() };
        let result = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(periphery::ui::App::new(cc, start)))))
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!("Periphery could not start: {e:?}")),
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn log_level() -> log::LevelFilter {
    log::LevelFilter::Warn
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("periphery-web runs in the browser: `trunk serve` in crates/web, then open it in Chrome or Edge.");
}
