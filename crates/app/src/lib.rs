//! egui front end. The same `App` runs natively (`ptclone-app`) and in the browser
//! (built with `trunk`, see `index.html`).

mod app;
mod canvas;
mod term;
pub mod widget;

pub use app::App;
pub use term::TermModel;

/// Entry point for the web build: attach to the canvas with this element id.
#[cfg(target_arch = "wasm32")]
pub fn start_web(canvas_id: &str) {
    use eframe::wasm_bindgen::JsCast as _;
    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(canvas_id))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("canvas element missing");
    wasm_bindgen_futures::spawn_local(async move {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(App::new()))),
            )
            .await;
        if let Some(loading) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("loading"))
        {
            // Remove the loading text whether or not startup worked.
            loading.remove();
        }
        if let Err(e) = result {
            web_sys::console::error_1(&e);
        }
    });
}
