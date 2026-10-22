//! Headless tests for canvas editing: real pointer events against the real UI.

use app::{App, Tool};
use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput};
use engine::lab::LabKind;

fn frame(ctx: &Context, app: &mut App, events: Vec<Event>) {
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    raw.events = events;
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
}

fn ready(app: &mut App) -> Context {
    let ctx = Context::default();
    frame(&ctx, app, vec![]);
    frame(&ctx, app, vec![]);
    ctx
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn click(ctx: &Context, app: &mut App, at: Pos2) {
    frame(ctx, app, vec![Event::PointerMoved(at)]);
    frame(ctx, app, vec![button(at, true)]);
    frame(ctx, app, vec![button(at, false)]);
    frame(ctx, app, vec![]);
}

fn drag(ctx: &Context, app: &mut App, from: Pos2, to: Pos2) {
    frame(ctx, app, vec![Event::PointerMoved(from)]);
    frame(ctx, app, vec![button(from, true)]);
    // Move in steps so egui sees a drag, not a jump.
    for i in 1..=4 {
        let p = from + (to - from) * (i as f32 / 4.0);
        frame(ctx, app, vec![Event::PointerMoved(p)]);
    }
    frame(ctx, app, vec![button(to, false)]);
    frame(ctx, app, vec![]);
}

fn empty_spot(app: &App) -> Pos2 {
    let r1 = app.node_centre("r1").unwrap();
    Pos2::new(r1.x, r1.y + 62.0)
}

#[test]
fn place_tool_adds_a_device_where_you_click() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let at = empty_spot(&app);
    app.set_tool(Tool::Place(LabKind::Host));
    frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, at);
    assert_eq!(app.device_names(), ["r1", "r2", "pc1", "pc2", "pc3"]);
    assert_eq!(app.active_device(), "pc3", "the new device's console opens");
    frame(&ctx, &mut app, vec![]);
    let c = app.node_centre("pc3").unwrap();
    assert!((c - at).length() < 2.0, "placed at {c:?}, clicked {at:?}");
    // The tool stays active so several devices can be dropped in a row.
    assert_eq!(app.tool(), Tool::Place(LabKind::Host));
}

#[test]
fn dragging_a_device_moves_it_and_the_saved_lab_follows() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let from = app.node_centre("r1").unwrap();
    let to = from + egui::vec2(0.0, 40.0);
    drag(&ctx, &mut app, from, to);
    let moved = app.node_centre("r1").unwrap();
    assert!(
        (moved - to).length() < 2.0,
        "r1 at {moved:?}, wanted {to:?}"
    );
    assert_eq!(app.active_device(), "r1", "a drag is not a click");
    let saved = app.export_lab();
    let reloaded = App::from_lab_text(&saved).unwrap();
    assert_eq!(reloaded.device_names(), app.device_names());
    assert_ne!(
        saved,
        App::new().export_lab(),
        "the new position is in the saved file"
    );
}

#[test]
fn cable_tool_connects_two_free_devices_and_reports_full_ports() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.add_device(LabKind::Switch, [350, 185]);
    app.add_device(LabKind::Host, [150, 185]);
    frame(&ctx, &mut app, vec![]);
    frame(&ctx, &mut app, vec![]);
    app.set_tool(Tool::Link);
    let (a, b) = (
        app.node_centre("sw1").unwrap(),
        app.node_centre("pc3").unwrap(),
    );
    click(&ctx, &mut app, a);
    click(&ctx, &mut app, b);
    assert!(app.cables().contains(&("sw1", "pc3")), "{:?}", app.cables());
    // pc1 already has its one cable; r1 has both ports used.
    let (p1, r1) = (
        app.node_centre("pc1").unwrap(),
        app.node_centre("r1").unwrap(),
    );
    click(&ctx, &mut app, p1);
    click(&ctx, &mut app, r1);
    assert_eq!(app.cables().len(), 4, "no cable added");
    assert!(app.status_notice().unwrap().contains("no free port"));
}

#[test]
fn delete_tool_removes_a_device_and_its_cables_and_the_lab_still_works() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.set_tool(Tool::Delete);
    let r2 = app.node_centre("r2").unwrap();
    click(&ctx, &mut app, r2);
    assert_eq!(app.device_names(), ["r1", "pc1", "pc2"]);
    assert_eq!(app.cables(), [("pc1", "r1")], "cables to r2 are gone");
    assert!(app.status_notice().unwrap().contains("removed r2"));
    // The remaining devices still have working consoles after the rebuild.
    app.set_tool(Tool::Select);
    let pc = app.node_centre("pc1").unwrap();
    click(&ctx, &mut app, pc);
    assert_eq!(app.active_device(), "pc1");
    assert_eq!(app.active_prompt(), "pc1>");
}

#[test]
fn delete_tool_removes_a_cable_by_clicking_on_it() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.set_tool(Tool::Delete);
    let (a, b) = (
        app.node_centre("r1").unwrap(),
        app.node_centre("r2").unwrap(),
    );
    click(&ctx, &mut app, a + (b - a) * 0.5);
    assert_eq!(app.device_names().len(), 4, "devices stay");
    assert!(!app.cables().contains(&("r1", "r2")), "{:?}", app.cables());
    assert_eq!(app.cables().len(), 2);
}

#[test]
fn escape_returns_to_select_and_cancels_a_half_drawn_cable() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.set_tool(Tool::Link);
    let a = app.node_centre("pc1").unwrap();
    click(&ctx, &mut app, a);
    let esc = |pressed| Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    frame(&ctx, &mut app, vec![esc(true)]);
    frame(&ctx, &mut app, vec![esc(false)]);
    assert_eq!(app.tool(), Tool::Select);
    // The next click must not complete the old half-drawn cable.
    let b = app.node_centre("pc2").unwrap();
    click(&ctx, &mut app, b);
    assert_eq!(app.cables().len(), 3);
}

#[test]
fn an_edited_lab_survives_export_and_reload_with_working_traffic() {
    let mut app = App::new();
    let _ctx = ready(&mut app);
    app.add_device(LabKind::Host, [100, 150]);
    app.add_device(LabKind::Switch, [250, 150]);
    // pc3 (index 4) - sw1 (index 5); pc3 gets an address through its console.
    app.connect(4, 5);
    let saved = app.export_lab();
    let mut again = App::from_lab_text(&saved).unwrap();
    assert_eq!(again.cables(), app.cables());
    assert_eq!(
        again.export_lab(),
        saved,
        "export is stable across a reload"
    );
    again.sim_queue_ping("pc1", [10, 0, 1, 1]);
    again.sim_run();
    assert!(!again.sim_hops().is_empty());
}
