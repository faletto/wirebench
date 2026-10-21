#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("wirebench")
            .with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "wirebench",
        options,
        Box::new(|_cc| Ok(Box::new(app::App::new()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    app::start_web("the_canvas_id");
}
