#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    app::clock::mark_start();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("wirebench")
            .with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    // `wirebench-app [lab.toml]` opens that file instead of the bundled lab.
    let path = std::env::args().nth(1);
    eframe::run_native(
        "wirebench",
        options,
        Box::new(move |_cc| {
            let mut app = app::App::new();
            if let Some(p) = path {
                app.open_path(&p);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    app::start_web("the_canvas_id");
}
