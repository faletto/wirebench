//! UI frame time on a 100-device lab, native. The plan's budget is p99 under 8 ms native (16 ms in
//! the browser). Run by hand, in release, since debug builds say nothing about it:
//!
//! ```sh
//! cargo test -p app --release --test ui_bench -- --ignored --nocapture
//! ```

use app::App;
use egui::{Context, Pos2, RawInput};
use std::time::Instant;

const LAB: &str = include_str!("../../../testkit/throttle/hundred-lab.toml");

fn frame(ctx: &Context, app: &mut App, t: f64) -> f64 {
    let raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1366.0, 768.0),
        )),
        time: Some(t),
        ..Default::default()
    };
    let began = Instant::now();
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    // The CPU half of drawing, as eframe does it; the GPU's share is not in this number.
    let meshes = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
    std::hint::black_box(meshes);
    out.textures_delta.clear();
    began.elapsed().as_secs_f64() * 1000.0
}

#[test]
fn the_hundred_device_lab_opens() {
    let app = App::from_lab_text(LAB).unwrap();
    assert_eq!(app.device_count(), 100);
}

#[test]
#[ignore = "a measurement, not a check: run it by hand in release"]
fn frame_time_with_a_hundred_devices() {
    let mut app = App::from_lab_text(LAB).unwrap();
    let ctx = Context::default();
    let mut t = 0.0;
    for _ in 0..30 {
        t += 0.016;
        frame(&ctx, &mut app, t);
    }
    // The lab running live (spanning tree is on), then with the frame being redrawn from scratch.
    let mut ms: Vec<f64> = (0..400)
        .map(|_| {
            t += 0.016;
            frame(&ctx, &mut app, t)
        })
        .collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| ms[((ms.len() as f64 * p) as usize).min(ms.len() - 1)];
    println!(
        "100 devices, native, {} frames: p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        ms.len(),
        q(0.5),
        q(0.99),
        ms[ms.len() - 1]
    );
}
