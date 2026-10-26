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

#[test]
fn stepping_waits_for_the_engine_and_the_hops_arrive_in_order() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 8);
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_step_hop();
    assert_eq!(app.sim_hops().len(), 0, "nothing yet");
    d.frames(&mut app, 8);
    assert_eq!(app.sim_hops().len(), 1);
    // A second request while one is out is not started on top of it.
    app.sim_step_hop();
    app.sim_step_hop();
    d.frames(&mut app, 8);
    assert_eq!(app.sim_hops().len(), 2);
    assert!(app.sim_hops()[0].ns <= app.sim_hops()[1].ns);
}

#[test]
fn run_goes_to_the_end_by_itself_and_back_then_undoes_it_step_by_step() {
    let mut app = App::from_lab_text_delayed(LAB, 2).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 6);
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_run();
    d.frames(&mut app, 60);
    let all = app.sim_hops().len();
    assert!(all >= 4, "{all}");
    assert!(app.sim_can_back());
    // Each Back undoes one step, which may be a timer with no frame in it; frames only go away.
    let mut last = all;
    for _ in 0..all + 6 {
        app.sim_back();
        d.frames(&mut app, 6);
        assert!(app.sim_hops().len() <= last);
        last = app.sim_hops().len();
    }
    assert!(last < all, "stepping back removed frames from the log");
}

#[test]
fn play_steps_through_a_ping_at_its_own_pace_and_stops_when_idle() {
    let mut app = App::from_lab_text_delayed(LAB, 2).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 6);
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.set_playing(true);
    d.frames(&mut app, 400);
    assert!(!app.is_playing(), "it ran out of things to show");
    assert!(app.sim_hops().len() >= 4);
}

#[test]
fn the_live_clock_keeps_running_with_a_slow_engine() {
    let lab = include_str!("../../../testkit/throttle/m2-lab.toml");
    let mut app = App::from_lab_text_delayed(lab, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 40);
    let at = app.sim_time_ns();
    d.frames(&mut app, 100);
    assert!(
        app.sim_time_ns() > at + 3_000_000_000,
        "{} then {}",
        at,
        app.sim_time_ns()
    );
}

#[test]
fn an_added_device_appears_when_the_engine_confirms_it_and_undo_follows_suit() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 8);
    let n = app.device_count();
    app.add_device(engine::lab::LabKind::Switch, [10, 10]);
    assert_eq!(app.device_count(), n, "not until the engine has said so");
    d.frames(&mut app, 8);
    assert_eq!(app.device_count(), n + 1);
    assert!(app.can_undo());
    assert!(app.is_dirty());
    // The new device's console opens a little after it appears.
    d.frames(&mut app, 8);
    assert!(
        app.active_prompt().ends_with('>'),
        "{:?}",
        app.active_prompt()
    );
    app.undo();
    d.frames(&mut app, 12);
    assert_eq!(app.device_count(), n, "undone");
    assert!(app.can_redo());
}

#[test]
fn cabling_and_deleting_wait_for_the_engine_too() {
    let mut app = App::from_lab_text_delayed(LAB, 2).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 8);
    let cables = app.cables().len();
    app.add_device(engine::lab::LabKind::Switch, [0, 0]);
    app.add_device(engine::lab::LabKind::Switch, [50, 0]);
    d.frames(&mut app, 12);
    let n = app.device_count();
    app.connect(n - 2, n - 1);
    assert_eq!(app.cables().len(), cables, "not yet");
    d.frames(&mut app, 8);
    assert_eq!(app.cables().len(), cables + 1);
    app.delete_link(cables);
    d.frames(&mut app, 12);
    assert_eq!(app.cables().len(), cables);
    app.delete_device(n - 1);
    d.frames(&mut app, 12);
    assert_eq!(app.device_count(), n - 1);
}

#[test]
fn a_lab_opened_late_replaces_the_old_one_and_a_bad_one_changes_nothing() {
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 8);
    let before = app.export_lab();
    app.load_lab_text("this is not a lab", "junk");
    d.frames(&mut app, 8);
    assert_eq!(app.export_lab(), before);
    assert!(app
        .status_notice()
        .unwrap_or("")
        .contains("could not open junk"));
    let other = include_str!("../../../testkit/scenarios/m2_stp.lab.toml");
    app.load_lab_text(other, "stp");
    assert_eq!(
        app.export_lab(),
        before,
        "still the old lab until the answer"
    );
    d.frames(&mut app, 12);
    assert_ne!(app.export_lab(), before);
    assert!(app.device_names().contains(&"sw1"));
    assert!(app.status_notice().unwrap_or("").contains("opened stp"));
}

#[test]
fn the_settings_panel_shows_what_the_engine_says_when_it_says_it() {
    use engine::panel::DeviceState;
    let mut app = App::from_lab_text_delayed(LAB, 3).unwrap();
    let mut d = Driver::new();
    d.frames(&mut app, 10);
    pc(&mut app);
    d.frames(&mut app, 10);
    let said = app.configure_active(&[
        "ipconfig set 10.0.1.66 255.255.255.0",
        "ipconfig set 300.0.0.1 255.255.255.0",
    ]);
    assert!(said.is_empty(), "nothing has been said yet");
    d.frames(&mut app, 12);
    app.active_state();
    d.frames(&mut app, 8);
    let Some(DeviceState::Host(h)) = app.active_state() else {
        panic!()
    };
    assert_eq!(h.ip, "10.0.1.66");
    assert!(app.is_dirty());
}
