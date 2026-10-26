//! Headless tests for undo and redo of canvas edits.

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

fn key(ctx: &Context, app: &mut App, key: Key, modifiers: Modifiers) {
    let ev = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    frame(ctx, app, vec![Event::ModifiersChanged(modifiers), ev(true)]);
    frame(
        ctx,
        app,
        vec![ev(false), Event::ModifiersChanged(Modifiers::NONE)],
    );
}

fn type_line(ctx: &Context, app: &mut App, text: &str) {
    frame(ctx, app, vec![Event::Text(text.into())]);
    frame(
        ctx,
        app,
        vec![
            Event::ModifiersChanged(Modifiers::NONE),
            Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
    );
}

fn empty_spot(app: &App) -> Pos2 {
    let r1 = app.node_centre("r1").unwrap();
    Pos2::new(r1.x, r1.y + 62.0)
}

#[test]
fn undo_and_redo_an_added_device() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    assert!(!app.can_undo());
    let at = empty_spot(&app);
    app.set_tool(Tool::Place(LabKind::Host));
    click(&ctx, &mut app, at);
    assert_eq!(app.device_names().len(), 5);
    assert!(app.can_undo());
    app.undo();
    assert_eq!(app.device_names(), ["r1", "r2", "pc1", "pc2"]);
    assert!(app.status_notice().unwrap().contains("undid add pc3"));
    assert!(app.can_redo() && !app.can_undo());
    app.redo();
    assert_eq!(app.device_names(), ["r1", "r2", "pc1", "pc2", "pc3"]);
}

#[test]
fn undo_brings_back_a_deleted_device_with_its_cables() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let before = app.export_lab();
    app.set_tool(Tool::Delete);
    let r2 = app.node_centre("r2").unwrap();
    click(&ctx, &mut app, r2);
    assert_eq!(app.cables(), [("pc1", "r1")]);
    app.undo();
    assert_eq!(app.device_names(), ["r1", "r2", "pc1", "pc2"]);
    assert_eq!(app.export_lab(), before, "the lab is exactly as it was");
    assert!(app.can_redo());
    app.redo();
    assert_eq!(app.device_names(), ["r1", "pc1", "pc2"]);
}

#[test]
fn undo_removes_a_cable_that_was_just_drawn() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let before = app.cables().len();
    let at = empty_spot(&app);
    app.set_tool(Tool::Place(LabKind::Host));
    click(&ctx, &mut app, at);
    app.set_tool(Tool::Place(LabKind::Switch));
    click(&ctx, &mut app, at + egui::vec2(120.0, 0.0));
    app.set_tool(Tool::Link);
    let (a, b) = (
        app.node_centre("pc3").unwrap(),
        app.node_centre("sw1").unwrap(),
    );
    click(&ctx, &mut app, a);
    click(&ctx, &mut app, b);
    assert_eq!(app.cables().len(), before + 1);
    app.undo();
    assert_eq!(app.cables().len(), before);
    assert_eq!(app.device_names().len(), 6, "only the cable went");
    app.undo();
    app.undo();
    assert_eq!(app.device_names().len(), 4);
}

#[test]
fn a_whole_drag_is_one_undo_step() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let before = app.export_lab();
    let from = app.node_centre("r1").unwrap();
    drag(&ctx, &mut app, from, from + egui::vec2(0.0, 40.0));
    assert_ne!(app.export_lab(), before);
    app.undo();
    assert_eq!(app.export_lab(), before, "one undo undoes the whole drag");
    assert!(!app.can_undo(), "the drag left a single entry");
}

#[test]
fn a_click_without_movement_records_nothing() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let at = app.node_centre("r1").unwrap();
    click(&ctx, &mut app, at);
    assert!(!app.can_undo());
}

#[test]
fn configuration_typed_after_an_edit_survives_undoing_it() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.set_tool(Tool::Place(LabKind::Host));
    let at = empty_spot(&app);
    click(&ctx, &mut app, at);
    app.set_tool(Tool::Select);
    let r1 = app.node_centre("r1").unwrap();
    click(&ctx, &mut app, r1);
    for line in [
        "enable",
        "configure terminal",
        "hostname typed-later",
        "end",
    ] {
        type_line(&ctx, &mut app, line);
    }
    assert!(app.export_lab().contains("typed-later"));
    app.undo();
    assert_eq!(app.device_names().len(), 4, "the added device is gone");
    assert!(
        app.export_lab().contains("typed-later"),
        "the hostname typed after the edit is kept"
    );
}

#[test]
fn ctrl_z_undoes_and_ctrl_shift_z_and_ctrl_y_redo() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.set_tool(Tool::Place(LabKind::Host));
    let at = empty_spot(&app);
    click(&ctx, &mut app, at);
    assert_eq!(app.device_names().len(), 5);
    key(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(app.device_names().len(), 4);
    key(
        &ctx,
        &mut app,
        Key::Z,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert_eq!(app.device_names().len(), 5);
    key(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    key(&ctx, &mut app, Key::Y, Modifiers::COMMAND);
    assert_eq!(app.device_names().len(), 5);
}

#[test]
fn undo_with_nothing_to_undo_says_so() {
    let mut app = App::new();
    let _ctx = ready(&mut app);
    app.undo();
    assert_eq!(app.status_notice(), Some("nothing to undo"));
    app.redo();
    assert_eq!(app.status_notice(), Some("nothing to redo"));
}
