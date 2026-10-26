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
#[cfg(target_arch = "wasm32")]
mod worker;

pub use app::{App, Choice, Pending};
pub use canvas::Tool;
pub use files::{Inbox, Incoming};
pub use sim::HopRow;
pub use term::TermModel;

/// Entry point for the web build: attach to the canvas with this element id.
#[cfg(target_arch = "wasm32")]
pub fn start_web(canvas_id: &str) {
    use eframe::wasm_bindgen::JsCast as _;
    // The same wasm module is started inside the engine's worker; there is no page to draw.
    if worker::in_worker() {
        return;
    }
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
                    let mut app = web_app();
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

/// The app for the page: its engine in a worker unless the address asks for `?engine=inline` or a
/// worker cannot be started, in which case it runs in the page as before.
#[cfg(target_arch = "wasm32")]
fn web_app() -> App {
    let inline = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.contains("engine=inline"));
    if inline {
        return App::new();
    }
    match worker::WorkerTransport::spawn() {
        Ok(t) => {
            web_sys::console::log_1(&"wirebench: engine in a Web Worker".into());
            App::with_transport(t)
        }
        Err(why) => {
            web_sys::console::warn_1(
                &format!("wirebench: no worker ({why}); engine in the page").into(),
            );
            App::new()
        }
    }
}

/// Keeps the worker's entry point in the bundle.
#[cfg(target_arch = "wasm32")]
pub use worker::worker_start;
