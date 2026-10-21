//! Drive the real egui UI with synthetic input, no window. This is the "headless egui test"
//! from the plan: it proves keyboard events reach the terminal and the prompt changes.

use app::App;
use egui::{Context, Event, Key, Modifiers, RawInput};

fn frame(ctx: &Context, app: &mut App, events: Vec<Event>) {
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    // Backends report modifier state with ModifiersChanged ahead of the key event itself.
    for e in events {
        if let Event::Key { modifiers, .. } = &e {
            raw.events.push(Event::ModifiersChanged(*modifiers));
        }
        raw.events.push(e);
    }
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    // No renderer here; egui panics if texture uploads are dropped unhandled.
    out.textures_delta.clear();
}

fn type_line(ctx: &Context, app: &mut App, text: &str) {
    frame(ctx, app, vec![Event::Text(text.to_string())]);
    frame(ctx, app, vec![key(Key::Enter, Modifiers::NONE)]);
}

fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn ready() -> (Context, App) {
    let ctx = Context::default();
    let mut app = App::new();
    // First frame lets the terminal take focus (the app requests it on startup).
    frame(&ctx, &mut app, vec![]);
    frame(&ctx, &mut app, vec![]);
    (ctx, app)
}

#[test]
fn typing_enable_and_conf_t_changes_the_prompt() {
    let (ctx, mut app) = ready();
    assert_eq!(app.active_prompt(), "r1>");
    type_line(&ctx, &mut app, "enable");
    assert_eq!(app.active_prompt(), "r1#");
    type_line(&ctx, &mut app, "conf t");
    assert_eq!(app.active_prompt(), "r1(config)#");
    type_line(&ctx, &mut app, "hostname core");
    assert_eq!(app.active_prompt(), "core(config)#");
}

#[test]
fn tab_completes_and_enter_runs_the_completed_command() {
    let (ctx, mut app) = ready();
    frame(&ctx, &mut app, vec![Event::Text("ena".into())]);
    frame(&ctx, &mut app, vec![key(Key::Tab, Modifiers::NONE)]);
    frame(&ctx, &mut app, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(app.active_prompt(), "r1#");
}

#[test]
fn pasted_script_runs_every_line_in_order() {
    let (ctx, mut app) = ready();
    frame(
        &ctx,
        &mut app,
        vec![Event::Paste(
            "enable\nconfigure terminal\nhostname pasted\n".into(),
        )],
    );
    assert_eq!(app.active_prompt(), "pasted(config)#");
}

#[test]
fn alt_number_switches_device_and_keeps_the_other_session() {
    let (ctx, mut app) = ready();
    type_line(&ctx, &mut app, "enable");
    frame(&ctx, &mut app, vec![key(Key::Num3, Modifiers::ALT)]);
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.active_prompt(), "pc1>");
    type_line(&ctx, &mut app, "ipconfig set 10.0.1.2 255.255.255.0");
    frame(&ctx, &mut app, vec![key(Key::Num1, Modifiers::ALT)]);
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.active_prompt(), "r1#", "r1 kept its privileged session");
}

#[test]
fn ready_time_is_recorded_on_the_first_frame_and_then_frozen() {
    let ctx = Context::default();
    let mut app = App::new();
    assert_eq!(
        app.ready_ms(),
        None,
        "nothing is measured before a frame is drawn"
    );
    frame(&ctx, &mut app, vec![]);
    let first = app.ready_ms().expect("set on the first frame");
    assert!(first.is_finite() && first >= 0.0);
    std::thread::sleep(std::time::Duration::from_millis(5));
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.ready_ms(), Some(first), "later frames do not move it");
}

#[test]
fn terminal_takes_keyboard_focus_on_the_first_frame() {
    // No warm-up: the app asks for focus on startup, so typing works immediately.
    let ctx = Context::default();
    let mut app = App::new();
    frame(&ctx, &mut app, vec![Event::Text("enable".into())]);
    frame(&ctx, &mut app, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(app.active_prompt(), "r1#");
}
