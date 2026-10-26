//! Headless tests for packet animation: frames are driven with explicit wall-clock times.

use app::App;
use egui::{Context, Pos2, RawInput, ViewportId};
use std::time::Duration;

/// Run one frame at wall time `t` seconds; returns how long egui was asked to wait before
/// the next repaint (`Duration::MAX` = never, i.e. the app is idle).
fn frame(ctx: &Context, app: &mut App, t: f64) -> Duration {
    let raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1100.0, 700.0),
        )),
        time: Some(t),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
    out.viewport_output[&ViewportId::ROOT].repaint_delay
}

fn ready() -> (App, Context) {
    let mut app = App::new();
    let ctx = Context::default();
    frame(&ctx, &mut app, 0.0);
    frame(&ctx, &mut app, 0.01);
    // The settings panel asks the engine for the open device's state and draws it a frame later.
    frame(&ctx, &mut app, 0.02);
    frame(&ctx, &mut app, 0.03);
    (app, ctx)
}

#[test]
fn a_step_launches_a_dot_from_sender_to_receiver_and_it_lands() {
    let (mut app, ctx) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    assert!(app.sim_step_hop());
    assert_eq!(app.flights(), [("pc1", "r1", "ARP request")]);
    assert_eq!(app.sim_hops().len(), 1);
    frame(&ctx, &mut app, 0.2);
    assert_eq!(app.flights().len(), 1, "still in the air partway through");
    frame(&ctx, &mut app, 1.6);
    assert!(app.flights().is_empty(), "landed after the hop duration");
}

#[test]
fn replies_fly_back_the_other_way() {
    let (mut app, ctx) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_step_hop();
    app.sim_step_hop();
    frame(&ctx, &mut app, 0.02);
    let f = app.flights();
    assert_eq!(f[1], ("r1", "pc1", "ARP reply"), "{f:?}");
}

#[test]
fn play_paces_one_hop_per_interval_then_stops_when_idle() {
    let (mut app, ctx) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.set_playing(true);
    let mut counts = Vec::new();
    for i in 0..=40 {
        let t = 10.0 + f64::from(i) * 0.1;
        frame(&ctx, &mut app, t);
        counts.push(app.sim_hops().len());
    }
    // First hop immediately, then none until ~0.45 s later.
    assert_eq!(counts[0], 1);
    assert_eq!(counts[3], 1, "{counts:?}");
    assert_eq!(counts[5], 2, "{counts:?}");
    assert_eq!(*counts.last().unwrap(), 4, "{counts:?}");
    assert!(counts.windows(2).all(|w| w[0] <= w[1]));
    assert!(!app.is_playing(), "stops when the simulation is idle");
    assert_eq!(app.status_notice(), Some("simulation is idle"));
}

#[test]
fn pause_stops_stepping() {
    let (mut app, ctx) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.set_playing(true);
    frame(&ctx, &mut app, 10.0);
    app.set_playing(false);
    for i in 1..20 {
        frame(&ctx, &mut app, 10.0 + f64::from(i) * 0.2);
    }
    assert_eq!(app.sim_hops().len(), 1);
}

#[test]
fn speed_changes_the_pace() {
    let slow = {
        let (mut app, ctx) = ready();
        app.sim_queue_ping("pc1", [10, 0, 1, 1]);
        app.set_speed(0); // 0.5x
        app.set_playing(true);
        for i in 0..=10 {
            frame(&ctx, &mut app, 10.0 + f64::from(i) * 0.1);
        }
        app.sim_hops().len()
    };
    let fast = {
        let (mut app, ctx) = ready();
        app.sim_queue_ping("pc1", [10, 0, 1, 1]);
        app.set_speed(3); // 4x
        app.set_playing(true);
        for i in 0..=10 {
            frame(&ctx, &mut app, 10.0 + f64::from(i) * 0.1);
        }
        app.sim_hops().len()
    };
    assert!(fast > slow, "fast {fast} vs slow {slow}");
    assert_eq!(slow, 2, "0.5x: a hop every 0.9 s");
}

#[test]
fn animation_can_be_switched_off_and_run_never_animates() {
    let (mut app, ctx) = ready();
    app.set_animation_enabled(false);
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    assert!(app.sim_step_hop());
    assert!(app.flights().is_empty());
    assert_eq!(
        app.sim_hops().len(),
        1,
        "the log is the accessible path and stays on"
    );
    app.set_animation_enabled(true);
    app.sim_run();
    frame(&ctx, &mut app, 1.0);
    assert!(app.flights().is_empty(), "fast-forward has nothing to draw");
    assert_eq!(app.sim_hops().len(), 4);
}

#[test]
fn an_idle_app_asks_for_no_repaints() {
    let (mut app, ctx) = ready();
    assert_eq!(frame(&ctx, &mut app, 5.0), Duration::MAX, "nothing moving");
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_step_hop();
    assert_eq!(
        frame(&ctx, &mut app, 5.1),
        Duration::ZERO,
        "a dot is moving: repaint now"
    );
    // The frame that retires the dot still reports the repaint asked for by the frame
    // before; the next frame, with nothing moving, must ask for none.
    frame(&ctx, &mut app, 7.0);
    assert!(app.flights().is_empty());
    assert_eq!(
        frame(&ctx, &mut app, 7.1),
        Duration::MAX,
        "landed: idle again"
    );
    // Playing with animation off: no dot to follow, so between steps the app sleeps until
    // the next one is due instead of spinning.
    app.set_animation_enabled(false);
    app.set_playing(true);
    frame(&ctx, &mut app, 8.0);
    frame(&ctx, &mut app, 8.05); // let the previous repaint request drain
    let wait = frame(&ctx, &mut app, 8.2);
    assert!(
        wait > Duration::from_millis(100) && wait < Duration::from_millis(400),
        "should sleep about until 8.45: {wait:?}"
    );
}

#[test]
fn flights_through_a_switch_follow_the_cables() {
    let lab = r#"format = "labsim-lab"
version = 1
[[device]]
name = "a"
template = "host-pc"
pos = [0, 0]
[device.host]
ipv4 = "10.0.0.1/24"
[[device]]
name = "sw"
template = "switch-l2-8"
pos = [100, 0]
[[device]]
name = "b"
template = "host-pc"
pos = [200, 0]
[device.host]
ipv4 = "10.0.0.2/24"
[[link]]
a = "a:eth0"
b = "sw:Fa0/1"
[[link]]
a = "b:eth0"
b = "sw:Fa0/2"
"#;
    let mut app = App::from_lab_text(lab).unwrap();
    let ctx = Context::default();
    frame(&ctx, &mut app, 0.0);
    app.sim_queue_ping("a", [10, 0, 0, 2]);
    app.sim_step_hop(); // a -> sw
    frame(&ctx, &mut app, 0.01);
    app.sim_step_hop(); // sw -> b (ARP flood)
    frame(&ctx, &mut app, 0.02);
    let f = app.flights();
    assert_eq!(f[0], ("a", "sw", "ARP request"), "{f:?}");
    assert_eq!(f[1], ("sw", "b", "ARP request"), "{f:?}");
}
