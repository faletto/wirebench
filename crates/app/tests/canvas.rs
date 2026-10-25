//! Headless tests for the topology canvas: real pointer events against the real UI.

use app::App;
use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput};

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
    // Frame 1 fits the lab to the canvas; frame 2 draws it with the final transform.
    frame(&ctx, app, vec![]);
    frame(&ctx, app, vec![]);
    ctx
}

fn click(ctx: &Context, app: &mut App, at: Pos2) {
    let btn = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(ctx, app, vec![Event::PointerMoved(at)]);
    frame(ctx, app, vec![btn(true)]);
    frame(ctx, app, vec![btn(false)]);
    frame(ctx, app, vec![]);
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

#[test]
fn every_device_is_drawn_inside_the_canvas() {
    let mut app = App::new();
    let _ctx = ready(&mut app);
    for name in ["r1", "r2", "pc1", "pc2"] {
        let c = app
            .node_centre(name)
            .unwrap_or_else(|| panic!("{name} not drawn"));
        assert!(
            c.x > 0.0 && c.x < 1000.0 && c.y > 0.0 && c.y < 700.0,
            "{name} at {c:?}"
        );
    }
    // The lab is laid out left to right: pc1, r1, r2, pc2.
    let x = |n| app.node_centre(n).unwrap().x;
    assert!(x("pc1") < x("r1") && x("r1") < x("r2") && x("r2") < x("pc2"));
}

#[test]
fn clicking_a_device_opens_its_terminal() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    assert_eq!(app.active_device(), "r1");
    let at = app.node_centre("pc2").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.active_device(), "pc2");
    assert_eq!(app.active_prompt(), "pc2>");
    let at = app.node_centre("r2").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.active_device(), "r2");
}

#[test]
fn the_clicked_terminal_receives_keyboard_input() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let at = app.node_centre("pc1").unwrap();
    click(&ctx, &mut app, at);
    type_line(&ctx, &mut app, "ping 10.0.2.2");
    let out = app.active_scrollback().join("\n");
    assert!(out.contains("Sent 4, received 4"), "{out}");
}

#[test]
fn clicking_empty_canvas_does_not_change_the_selection() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    // Far from any device, but inside the canvas panel.
    let r1 = app.node_centre("r1").unwrap();
    click(&ctx, &mut app, Pos2::new(r1.x, r1.y + 60.0));
    assert_eq!(app.active_device(), "r1");
}

#[test]
fn clicking_a_switch_opens_its_console() {
    let lab = r#"format = "labsim-lab"
version = 1
[[device]]
name = "sw"
template = "switch-l2-8"
pos = [100, 100]
[[device]]
name = "pc"
template = "host-pc"
pos = [300, 100]
[device.host]
ipv4 = "10.0.0.1/24"
[[link]]
a = "pc:eth0"
b = "sw:Fa0/1"
"#;
    let mut app = App::from_lab_text(lab).unwrap();
    let ctx = ready(&mut app);
    let at = app.node_centre("pc").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.active_device(), "pc");
    let at = app.node_centre("sw").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.active_device(), "sw");
    assert_eq!(app.active_prompt(), "sw>");
    type_line(&ctx, &mut app, "show mac address-table");
    let out = app.active_scrollback().join("\n");
    assert!(out.contains("(no addresses learned)"), "{out}");
}

#[test]
fn lab_problems_are_surfaced_not_fatal() {
    let bad = include_str!("../../../labs/m1_static_routes.lab.toml")
        .replace("ip address 10.0.1.1", "ip adress 10.0.1.1");
    let app = App::from_lab_text(&bad).unwrap();
    assert!(app
        .status_notice()
        .unwrap()
        .starts_with("lab: r1 config line"));
    assert!(App::from_lab_text("not a lab").is_err());
}

#[test]
fn bundled_lab_opens_cleanly() {
    let app = App::new();
    assert_eq!(app.status_notice(), None);
}

#[test]
fn deleting_every_device_leaves_an_empty_canvas_not_a_crash() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    while app.device_count() > 0 {
        app.delete_device(0);
        frame(&ctx, &mut app, vec![]);
    }
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.device_count(), 0);
}
