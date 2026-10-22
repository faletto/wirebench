//! Save and open, headless. Native paths are real files in a per-test temp directory; the
//! browser's file picker is simulated by pushing into the app's inbox.

use app::widget::TermAction;
use app::{App, Incoming};
use egui::{Context, Pos2, RawInput};
use engine::lab::LabKind;
use std::path::PathBuf;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("wirebench-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_string()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn frame(ctx: &Context, app: &mut App) {
    let raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1100.0, 700.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
}

fn run(app: &mut App, tab: usize, lines: &[&str]) {
    for l in lines {
        app.apply(tab, TermAction::Submit((*l).to_string()));
    }
}

#[test]
fn save_then_open_restores_structure_positions_and_cli_changes() {
    let dir = Dir::new("roundtrip");
    let path = dir.file("mine.lab.toml");
    let mut a = App::new();
    a.add_device(LabKind::Host, [10, 200]);
    a.move_node(0, [333, 77]); // r1
                               // Tab 0 is r1 (lab order). Configure something new through its console.
    run(
        &mut a,
        0,
        &[
            "enable",
            "configure terminal",
            "ip route 172.16.0.0 255.255.0.0 10.0.12.2",
        ],
    );
    assert!(a.save_to_path(&path));
    assert_eq!(a.status_notice(), Some(format!("saved {path}").as_str()));

    let mut b = App::new();
    assert!(b.open_path(&path));
    assert_eq!(b.device_names(), a.device_names());
    assert_eq!(b.cables(), a.cables());
    let saved = b.export_lab();
    assert!(
        saved.contains("ip route 172.16.0.0 255.255.0.0 10.0.12.2"),
        "{saved}"
    );
    assert!(saved.contains("pos = [333, 77]"), "{saved}");
    assert_eq!(
        saved,
        a.export_lab(),
        "what you open is exactly what you saved"
    );
}

#[test]
fn a_failed_open_keeps_the_current_lab_and_says_why() {
    let dir = Dir::new("bad");
    let junk = dir.file("junk.toml");
    std::fs::write(&junk, "this is [not a lab").unwrap();
    let mut app = App::new();
    let before = app.export_lab();
    assert!(!app.open_path(&junk));
    assert!(
        app.status_notice().unwrap().starts_with("could not open"),
        "{:?}",
        app.status_notice()
    );
    assert!(!app.open_path(&dir.file("missing.toml")));
    assert!(!app.open_path(""));
    assert!(app.status_notice().unwrap().contains("file name"));
    assert_eq!(app.export_lab(), before, "nothing changed");
    // A parseable file that is not a lab is rejected too.
    let wrong = dir.file("wrong.toml");
    std::fs::write(&wrong, "format = \"something-else\"\nversion = 1\n").unwrap();
    assert!(!app.open_path(&wrong));
    assert_eq!(app.export_lab(), before);
}

#[test]
fn unwritable_save_is_reported_not_fatal() {
    let mut app = App::new();
    assert!(!app.save_to_path("/definitely/not/a/dir/lab.toml"));
    assert!(app.status_notice().unwrap().starts_with("could not save"));
    assert!(!app.save_to_path("   "));
}

#[test]
fn opening_keeps_your_session_preferences_and_refits_the_view() {
    let dir = Dir::new("session");
    let path = dir.file("small.lab.toml");
    let small = "format = \"labsim-lab\"\nversion = 1\n[meta]\ntitle = \"tiny\"\n[[device]]\nname = \"solo\"\ntemplate = \"host-pc\"\npos = [5000, 5000]\n";
    std::fs::write(&path, small).unwrap();
    let ctx = Context::default();
    let mut app = App::new();
    frame(&ctx, &mut app);
    app.set_animation_enabled(false);
    app.set_speed(3);
    assert!(app.open_path(&path));
    assert_eq!(app.device_names(), ["solo"]);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    // The view was refitted: a device at (5000, 5000) is on screen, not lost off-canvas.
    let c = app.node_centre("solo").unwrap();
    assert!(
        c.x > 0.0 && c.x < 1100.0 && c.y > 0.0 && c.y < 700.0,
        "{c:?}"
    );
    // Animation preference survived: queue-step produces no flight.
    app.sim_queue_ping("solo", [1, 2, 3, 4]);
    app.sim_step_hop();
    assert!(app.flights().is_empty());
}

#[test]
fn lab_problems_are_shown_alongside_the_open_notice() {
    let dir = Dir::new("warn");
    let path = dir.file("warn.lab.toml");
    let bad = include_str!("../../../labs/m1_static_routes.lab.toml")
        .replace("ip address 10.0.1.1", "ip adress 10.0.1.1");
    std::fs::write(&path, bad).unwrap();
    let mut app = App::new();
    assert!(app.open_path(&path), "problems are warnings, not failures");
    let n = app.status_notice().unwrap();
    assert!(
        n.starts_with("opened ") && n.contains("lab: r1 config line"),
        "{n}"
    );
}

#[test]
fn files_picked_in_the_browser_arrive_through_the_inbox() {
    let ctx = Context::default();
    let mut app = App::new();
    let inbox = app.inbox();
    let lab = include_str!("../../../labs/m1_static_routes.lab.toml")
        .replace("Two routers, static routes", "Picked");
    inbox.push(Incoming::Lab {
        name: "picked.lab.toml".into(),
        text: lab,
    });
    frame(&ctx, &mut app);
    assert_eq!(app.status_notice(), Some("opened picked.lab.toml"));
    inbox.push(Incoming::Failed("big.toml is too large to be a lab".into()));
    frame(&ctx, &mut app);
    assert_eq!(
        app.status_notice(),
        Some("big.toml is too large to be a lab")
    );
    inbox.push(Incoming::Lab {
        name: "x".into(),
        text: "nope [".into(),
    });
    frame(&ctx, &mut app);
    assert!(app.status_notice().unwrap().starts_with("could not open x"));
    // The inbox handle still works after the lab was replaced.
    assert_eq!(app.device_names().len(), 4);
    app.inbox().push(Incoming::Failed("again".into()));
    frame(&ctx, &mut app);
    assert_eq!(app.status_notice(), Some("again"));
}

#[test]
fn the_toolbar_with_file_controls_renders() {
    let ctx = Context::default();
    let mut app = App::new();
    for _ in 0..3 {
        frame(&ctx, &mut app);
    }
    assert_eq!(app.device_names().len(), 4);
}
