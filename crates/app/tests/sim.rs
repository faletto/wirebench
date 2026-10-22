use app::App;
use egui::{Context, Pos2, RawInput};

fn frame(ctx: &Context, app: &mut App) {
    let raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1100.0, 700.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
}

#[test]
fn stepping_logs_named_hops_until_idle() {
    let mut app = App::new();
    let ctx = Context::default();
    frame(&ctx, &mut app);
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    assert!(app.sim_hops().is_empty(), "queueing must not advance time");
    let mut steps = 0;
    while app.sim_step() {
        steps += 1;
        frame(&ctx, &mut app);
        assert!(steps < 100, "simulation never went idle");
    }
    let log: Vec<String> = app
        .sim_hops()
        .iter()
        .map(|h| format!("{} {}", h.device, h.summary))
        .collect();
    assert_eq!(log.len(), 4, "{log:?}");
    assert!(log[0].starts_with("r1 ARP request"), "{log:?}");
    assert!(log[3].starts_with("pc1 ICMP echo reply"), "{log:?}");
}

#[test]
fn unknown_or_wrong_device_is_reported_not_fatal() {
    let mut app = App::new();
    app.sim_queue_ping("nope", [1, 2, 3, 4]);
    assert_eq!(app.status_notice(), Some("no device named nope"));
    app.sim_queue_ping("r1", [1, 2, 3, 4]);
    assert!(app.status_notice().unwrap().contains("only a PC"));
    assert!(app.sim_hops().is_empty());
}

#[test]
fn run_goes_straight_to_idle_and_panel_renders() {
    let mut app = App::new();
    let ctx = Context::default();
    app.sim_queue_ping("pc1", [10, 0, 2, 2]);
    app.sim_run();
    assert!(app.sim_hops().len() >= 4, "Run still logs hops");
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(!app.sim_step(), "nothing left after Run");
}
