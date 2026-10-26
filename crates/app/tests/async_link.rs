//! The app with an engine that answers late, as one in a Web Worker will: nothing may assume an
//! answer has come by the time the command that asked for it returns.

use app::widget::TermAction;
use app::App;
use egui::{Context, Pos2, RawInput};

const LAB: &str = include_str!("../../../labs/m1_static_routes.lab.toml");

struct Driver {
    ctx: Context,
    t: f64,
}

impl Driver {
    fn new() -> Driver {
        Driver {
            ctx: Context::default(),
            t: 0.0,
        }
    }

    fn frames(&mut self, app: &mut App, n: usize) {
        for _ in 0..n {
            self.t += 0.05;
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(self.t),
                ..Default::default()
            };
            let mut out = self.ctx.run_ui(raw, |ui| app.ui(ui));
            out.textures_delta.clear();
        }
    }
}

/// The tab of the PC named `pc1`, selected.
fn pc(app: &mut App) -> usize {
    let i = app.device_names().iter().position(|n| *n == "pc1").unwrap();
    app.select(i);
    i
}

fn screen(app: &App) -> String {
    app.active_scrollback().join("\n")
}

#[test]
fn a_console_opens_when_the_engine_gets_round_to_it() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    assert_eq!(
        app.active_prompt(),
        "",
        "no prompt yet: the console is not open"
    );
    // Typing at a console that is not open is ignored, not a crash.
    app.apply(0, TermAction::Submit("ipconfig".into()));
    d.frames(&mut app, 5);
    assert!(
        app.active_prompt().ends_with('>'),
        "{:?}",
        app.active_prompt()
    );
    assert!(
        !screen(&app).contains("Address"),
        "the ignored line did not run later"
    );
}

#[test]
fn a_typed_line_is_echoed_at_once_and_its_output_comes_when_it_arrives() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 5);
    let p = pc(&mut app);
    app.apply(p, TermAction::Submit("ipconfig".into()));
    assert!(screen(&app).contains("ipconfig"), "echoed straight away");
    assert!(
        !screen(&app).contains("Address"),
        "but nothing has answered yet"
    );
    d.frames(&mut app, 5);
    assert!(screen(&app).contains("Address"), "{}", screen(&app));
}

#[test]
fn answers_land_in_order_and_the_prompt_follows_them() {
    let mut app = App::from_lab_text_delayed(LAB, 4).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 6);
    // Router console is the third device.
    let r1 = app.device_names().iter().position(|n| *n == "r1").unwrap();
    app.select(r1);
    d.frames(&mut app, 6);
    for line in ["enable", "configure terminal", "hostname later"] {
        app.apply(r1, TermAction::Submit(line.into()));
    }
    assert!(!app.active_prompt().starts_with("later"), "not yet");
    d.frames(&mut app, 10);
    assert_eq!(app.active_prompt(), "later(config)#");
    let s = screen(&app);
    let (a, b, c) = (
        s.find("enable"),
        s.find("configure terminal"),
        s.find("hostname later"),
    );
    assert!(a < b && b < c, "{s}");
}

#[test]
fn help_and_completion_arrive_late_too() {
    let mut app = App::from_lab_text_delayed(LAB, 2).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 6);
    let p = pc(&mut app);
    app.apply(p, TermAction::Help("".into()));
    d.frames(&mut app, 4);
    assert!(
        screen(&app).to_lowercase().contains("ipconfig"),
        "{}",
        screen(&app)
    );
}

#[test]
fn a_change_made_by_a_late_answer_still_dirties_the_lab() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 6);
    assert!(!app.is_dirty());
    let p = pc(&mut app);
    app.apply(
        p,
        TermAction::Submit("ipconfig set 10.0.1.55 255.255.255.0".into()),
    );
    assert!(
        !app.is_dirty(),
        "not until the engine has said what the lab is now"
    );
    d.frames(&mut app, 8);
    assert!(app.is_dirty());
    assert!(app.export_lab().contains("10.0.1.55"));
}
