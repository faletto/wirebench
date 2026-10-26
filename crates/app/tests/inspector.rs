//! The packet inspector: selecting a frame, filtering the log, and stepping back.

use app::App;
use egui::{Context, Pos2, RawInput};

fn ready() -> (App, Context) {
    let mut app = App::new();
    let ctx = Context::default();
    for t in [0.0, 0.1] {
        let raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            time: Some(t),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
        out.textures_delta.clear();
    }
    (app, ctx)
}

fn ping(app: &mut App) {
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_run();
}

#[test]
fn every_hop_keeps_its_frame_layers_and_protocol_names() {
    let (mut app, _) = ready();
    ping(&mut app);
    let hops = app.sim_hops();
    assert!(hops.len() >= 4);
    for h in hops {
        assert!(h.frame.len() > 13);
        assert!(h.protocols.contains(&"ethernet"));
        assert_eq!(h.layers[0].range.start, 0);
    }
    assert!(hops[0].protocols.contains(&"arp"));
    assert!(hops.iter().any(|h| h.protocols.contains(&"icmp")));
}

#[test]
fn the_filter_hides_rows_without_removing_them() {
    let (mut app, _) = ready();
    ping(&mut app);
    let all = app.sim_visible();
    assert_eq!(all.len(), app.sim_hops().len());
    app.sim_set_filter("arp");
    let arp = app.sim_visible();
    assert!(!arp.is_empty() && arp.len() < all.len());
    assert!(arp
        .iter()
        .all(|i| app.sim_hops()[*i].protocols.contains(&"arp")));
    app.sim_set_filter("icmp -arp dev:pc1");
    let v = app.sim_visible();
    assert!(v.iter().all(|i| {
        let h = &app.sim_hops()[*i];
        h.device == "pc1" && h.protocols.contains(&"icmp")
    }));
    app.sim_set_filter("");
    assert_eq!(app.sim_visible(), all);
}

#[test]
fn selecting_a_row_and_leaving_the_range_clears_it() {
    let (mut app, _) = ready();
    ping(&mut app);
    app.sim_select(Some(1));
    assert_eq!(app.sim_selected(), Some(1));
    app.sim_select(Some(10_000));
    assert_eq!(app.sim_selected(), None);
}

#[test]
fn back_undoes_one_step_at_a_time_and_stepping_again_repeats_it() {
    let (mut app, _) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    for _ in 0..4 {
        assert!(app.sim_step_hop());
    }
    let before: Vec<String> = app.sim_hops().iter().map(|h| h.summary.clone()).collect();
    assert_eq!(before.len(), 4);
    app.sim_select(Some(3));
    assert!(app.sim_back());
    assert_eq!(app.sim_hops().len(), 3, "the undone frame leaves the log");
    assert_eq!(app.sim_selected(), None, "and so does its selection");
    assert!(app.sim_step_hop());
    let again: Vec<String> = app.sim_hops().iter().map(|h| h.summary.clone()).collect();
    assert_eq!(again, before, "the same frame comes next");
}

#[test]
fn back_can_undo_the_queued_ping_and_then_has_nothing_left() {
    let (mut app, _) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    assert!(app.sim_step_hop());
    assert!(app.sim_back());
    assert!(app.sim_back(), "the ping itself");
    assert!(!app.sim_can_back());
    assert!(!app.sim_back());
    assert!(!app.sim_step_hop(), "nothing is queued any more");
}

#[test]
fn a_console_command_ends_the_ability_to_step_back() {
    use egui::{Event, Key, Modifiers};
    let (mut app, ctx) = ready();
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_step_hop();
    assert!(app.sim_can_back());
    // Open pc1's console and type something.
    let at = app.node_centre("pc1").unwrap();
    let mut t = 1.0;
    let mut frame = |app: &mut App, events: Vec<Event>| {
        t += 0.05;
        let raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
        out.textures_delta.clear();
    };
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(&mut app, vec![Event::PointerMoved(at)]);
    frame(&mut app, vec![button(true)]);
    frame(&mut app, vec![button(false)]);
    frame(&mut app, vec![]);
    frame(&mut app, vec![Event::Text("ipconfig".into())]);
    frame(
        &mut app,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(!app.sim_can_back());
    assert!(!app.sim_back());
}

#[test]
fn drawing_a_selected_frame_with_its_tree_and_hex_does_not_panic() {
    let (mut app, ctx) = ready();
    ping(&mut app);
    for i in 0..app.sim_hops().len() {
        app.sim_select(Some(i));
        let raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            time: Some(5.0 + i as f64),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
        out.textures_delta.clear();
    }
}
