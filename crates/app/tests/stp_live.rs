//! Spanning tree keeps its own time, so the app lets simulated time pass while frames are drawn.

use app::App;
use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput};

const LAB: &str = r#"
format = "labsim-lab"
version = 1

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [300, 100]
running_config = """
spanning-tree mode rapid-pvst
spanning-tree vlan 1 priority 4096
"""

[[device]]
name = "sw2"
template = "switch-l2-8"
pos = [150, 300]
running_config = "spanning-tree mode rapid-pvst"

[[device]]
name = "sw3"
template = "switch-l2-8"
pos = [450, 300]
running_config = "spanning-tree mode rapid-pvst"

[[link]]
a = "sw1:Fa0/1"
b = "sw2:Fa0/1"

[[link]]
a = "sw1:Fa0/2"
b = "sw3:Fa0/1"

[[link]]
a = "sw2:Fa0/2"
b = "sw3:Fa0/2"
"#;

fn frame_at(ctx: &Context, app: &mut App, time: f64, events: Vec<Event>) {
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        time: Some(time),
        ..Default::default()
    };
    raw.events = events;
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn simulated_time_passes_while_spanning_tree_runs_and_the_tree_settles() {
    let mut app = App::from_lab_text(LAB).unwrap();
    let ctx = Context::default();
    let mut t = 0.0;
    let mut step = |app: &mut App, events: Vec<Event>| {
        t += 0.05;
        frame_at(&ctx, app, t, events);
    };
    step(&mut app, vec![]);
    step(&mut app, vec![]);
    assert!(
        app.sim_time_ns() < 200_000_000,
        "only a frame's worth so far"
    );
    // Ten seconds of frames at 20 per second.
    for _ in 0..200 {
        step(&mut app, vec![]);
    }
    let secs = app.sim_time_ns() as f64 / 1e9;
    assert!((9.0..=11.0).contains(&secs), "{secs} simulated seconds");
    // Ask sw2 what it thinks.
    let at = app.node_centre("sw2").unwrap();
    step(&mut app, vec![Event::PointerMoved(at)]);
    step(&mut app, vec![button(at, true)]);
    step(&mut app, vec![button(at, false)]);
    step(&mut app, vec![]);
    assert_eq!(app.active_device(), "sw2");
    for line in ["enable", "show spanning-tree"] {
        step(&mut app, vec![Event::Text(line.into())]);
        step(
            &mut app,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    let screen = app.active_scrollback().join("\n");
    assert!(screen.contains("protocol rstp"), "{screen}");
    assert!(screen.contains("Root FWD"), "{screen}");
    assert!(screen.contains("Desg FWD"), "{screen}");
}

#[test]
fn a_lab_without_spanning_tree_leaves_the_clock_alone() {
    let mut app = App::new();
    let ctx = Context::default();
    for i in 0..100 {
        frame_at(&ctx, &mut app, f64::from(i) * 0.05, vec![]);
    }
    assert_eq!(app.sim_time_ns(), 0);
}

const REDUNDANT: &str = include_str!("../../../testkit/throttle/m2-lab.toml");

/// Found in a real browser: with spanning tree running, the live clock ran a queued ping to the
/// end between frames, so Step found nothing to step through.
#[test]
fn a_ping_being_stepped_through_is_not_run_ahead_by_the_live_clock() {
    let mut app = App::from_lab_text(REDUNDANT).unwrap();
    let ctx = Context::default();
    let mut t = 0.0;
    let mut frames = |app: &mut App, n: usize| {
        for _ in 0..n {
            t += 0.05;
            frame_at(&ctx, app, t, vec![]);
        }
    };
    frames(&mut app, 200);
    let before = app.sim_time_ns();
    assert!(before > 9_000_000_000, "time ran while nothing was queued");
    app.sim_queue_ping("pc1", [10, 0, 0, 2]);
    frames(&mut app, 40);
    assert_eq!(
        app.sim_hops().len(),
        0,
        "nothing was delivered behind the stepper's back"
    );
    assert!(
        // (a round of the clock already out when the ping was queued may still land)
        app.sim_time_ns() - before < 300_000_000,
        "and time stood still while frames were queued"
    );
    assert!(app.sim_step_hop());
    assert_eq!(app.sim_hops().len(), 1);
    // Back to before the ping: nothing is queued, so time runs again.
    assert!(app.sim_back());
    assert!(app.sim_back());
    let at = app.sim_time_ns();
    frames(&mut app, 40);
    assert!(app.sim_time_ns() > at + 1_000_000_000);
    // Stepped to the end, the simulation is idle and the clock resumes.
    app.sim_queue_ping("pc1", [10, 0, 0, 2]);
    app.sim_run();
    let at = app.sim_time_ns();
    frames(&mut app, 40);
    assert!(app.sim_time_ns() > at + 1_000_000_000);
}

/// A hidden tab draws no frames. When the next one comes, the time that went by is made up for, in
/// bounded rounds, so the lab is where it would have been had the clock kept running.
#[test]
fn time_that_passed_while_no_frames_were_drawn_is_made_up_for() {
    let mut app = App::from_lab_text(REDUNDANT).unwrap();
    let ctx = Context::default();
    let mut t = 0.0;
    for _ in 0..40 {
        t += 0.05;
        frame_at(&ctx, &mut app, t, vec![]);
    }
    let before = app.sim_time_ns();
    // Half an hour with the tab hidden.
    t += 1800.0;
    for _ in 0..60 {
        t += 0.1;
        frame_at(&ctx, &mut app, t, vec![]);
    }
    let gained = (app.sim_time_ns() - before) as f64 / 1e9;
    assert!(
        gained >= 1800.0,
        "only {gained} s of the half hour was made up"
    );
    assert!(gained < 1820.0, "{gained} s: more than passed");
    // Beyond an hour the lab carries on from where it was rather than racing to catch up.
    let before = app.sim_time_ns();
    t += 5.0 * 3600.0;
    for _ in 0..100 {
        t += 0.1;
        frame_at(&ctx, &mut app, t, vec![]);
    }
    let gained = (app.sim_time_ns() - before) as f64 / 1e9;
    assert!((3600.0..3640.0).contains(&gained), "{gained}");
}
