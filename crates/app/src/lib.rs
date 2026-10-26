//! egui front end. The same `App` runs natively (`wirebench-app`) and in the browser
//! (built with `trunk`, see `index.html`).

mod anim;
mod app;
pub mod autosave;
mod canvas;
mod changes;
pub mod clock;
mod files;
mod history;
mod inspector;
mod link;
mod settings;
mod sim;
mod term;
pub mod widget;

pub use app::{App, Choice, Pending};
pub use canvas::Tool;
pub use files::{Inbox, Incoming};
pub use sim::HopRow;
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
    files::install_page_guards();
    wasm_bindgen_futures::spawn_local(async move {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| {
                    let mut app = App::new();
                    app.set_store(Box::new(autosave::LocalStorageStore));
                    Ok(Box::new(app))
                }),
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
