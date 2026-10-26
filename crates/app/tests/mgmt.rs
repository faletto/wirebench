//! Headless tests for the management plane in the terminal: passwords, banner, reload.

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

fn open_r1(ctx: &Context, app: &mut App) {
    let at = app.node_centre("r1").unwrap();
    click(ctx, app, at);
    assert_eq!(app.active_device(), "r1");
}

fn lines(app: &App) -> String {
    app.active_scrollback().join("\n")
}

#[test]
fn a_typed_password_is_not_echoed_remembered_or_recalled() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    open_r1(&ctx, &mut app);
    for l in [
        "enable",
        "configure terminal",
        "enable secret hunter2",
        "end",
        "disable",
        "enable",
    ] {
        type_line(&ctx, &mut app, l);
    }
    assert_eq!(app.active_prompt(), "Password: ");
    // `?` is an ordinary character in a password, not a request for help.
    let before = app.active_scrollback().len();
    type_line(&ctx, &mut app, "wrong?pw");
    assert_eq!(
        app.active_scrollback().len(),
        before + 1,
        "one line for the prompt, and no help output"
    );
    assert_eq!(
        app.active_prompt(),
        "Password: ",
        "still asking after a wrong one"
    );
    let screen = lines(&app);
    // The command that set the secret is echoed like any command; the attempts are not.
    assert!(!screen.contains("wrong?pw"), "{screen}");
    assert_eq!(screen.matches("hunter2").count(), 1, "{screen}");
    // Up-arrow finds nothing to recall at a password prompt.
    key(&ctx, &mut app, Key::ArrowUp, Modifiers::NONE);
    type_line(&ctx, &mut app, "hunter2");
    assert_eq!(app.active_prompt(), "r1#");
    let screen = lines(&app);
    assert_eq!(screen.matches("hunter2").count(), 1, "{screen}");
    // Back at a normal prompt, history holds commands but never the passwords.
    key(&ctx, &mut app, Key::ArrowUp, Modifiers::NONE);
    type_line(&ctx, &mut app, "");
    assert!(!lines(&app).contains("wrong?pw"));
}

#[test]
fn the_banner_and_login_prompt_greet_a_reopened_terminal() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    open_r1(&ctx, &mut app);
    for l in [
        "enable",
        "configure terminal",
        "banner motd #Authorised use only#",
        "line console 0",
        "password lab",
        "login",
        "end",
        "write",
        "reload",
        "",
    ] {
        type_line(&ctx, &mut app, l);
    }
    let screen = lines(&app);
    assert!(screen.contains("System restarted."), "{screen}");
    assert!(screen.contains("Authorised use only"), "{screen}");
    assert!(screen.contains("User Access Verification"), "{screen}");
    assert_eq!(app.active_prompt(), "Password: ");
    type_line(&ctx, &mut app, "lab");
    assert_eq!(app.active_prompt(), "r1>");
}

#[test]
fn write_puts_the_startup_config_into_the_saved_lab() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    open_r1(&ctx, &mut app);
    let before = app.export_lab();
    for l in [
        "enable",
        "configure terminal",
        "hostname kept",
        "end",
        "write",
    ] {
        type_line(&ctx, &mut app, l);
    }
    let saved = app.export_lab();
    assert_ne!(saved, before);
    assert!(saved.contains("startup_config"), "{saved}");
    // Opening that file gives a router that remembers what was saved.
    let again = App::from_lab_text(&saved).unwrap();
    assert_eq!(again.active_prompt(), "kept>");
}
