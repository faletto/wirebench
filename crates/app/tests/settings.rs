//! The device settings panel: what it reads, what it changes, and that it draws.

use app::App;
use egui::{Context, Pos2, RawInput};
use engine::panel::DeviceState;

const SWITCHES: &str = include_str!("../../../testkit/scenarios/m2_stp.lab.toml");

fn frame(ctx: &Context, app: &mut App, t: f64) {
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

fn open(app: &mut App, name: &str) {
    let tab = app
        .device_names()
        .iter()
        .position(|n| *n == name)
        .expect("device in the lab");
    app.select(tab);
    assert_eq!(app.active_device(), name);
}

#[test]
fn the_panel_reads_the_open_devices_state() {
    let mut app = App::new();
    open(&mut app, "pc1");
    assert!(matches!(app.active_state(), Some(DeviceState::Host(_))));
    open(&mut app, "r1");
    assert!(matches!(app.active_state(), Some(DeviceState::Router(_))));
}

#[test]
fn a_change_from_the_panel_is_a_change_to_the_lab() {
    let mut app = App::new();
    open(&mut app, "pc1");
    let before = app.export_lab();
    assert!(!app.is_dirty());
    let said = app.configure_active(&["ipconfig set 10.0.1.77 255.255.255.0"]);
    assert!(said.is_empty(), "{said:?}");
    assert!(app.is_dirty());
    assert_ne!(app.export_lab(), before);
    assert!(app.export_lab().contains("10.0.1.77/24"));
    let Some(DeviceState::Host(h)) = app.active_state() else {
        panic!()
    };
    assert_eq!(h.ip, "10.0.1.77");
}

#[test]
fn what_the_cli_refuses_comes_back_against_its_line() {
    let mut app = App::new();
    open(&mut app, "r1");
    let said = app.configure_active(&["interface g0/0", "ip address 999.1.1.1 255.0.0.0"]);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("ip address 999.1.1.1 255.0.0.0: "));
}

#[test]
fn a_console_command_is_seen_by_the_panel_without_reopening_the_device() {
    use app::widget::TermAction;
    let mut app = App::from_lab_text(SWITCHES).unwrap();
    open(&mut app, "sw1");
    let Some(DeviceState::Switch(s)) = app.active_state() else {
        panic!()
    };
    assert!(s.vlans.iter().all(|(v, _)| *v != 30));
    let tab = app.device_names().iter().position(|n| *n == "sw1").unwrap();
    for line in ["enable", "configure terminal", "vlan 30", "exit"] {
        app.apply(tab, TermAction::Submit(line.into()));
    }
    let Some(DeviceState::Switch(s)) = app.active_state() else {
        panic!()
    };
    assert!(s.vlans.iter().any(|(v, _)| *v == 30));
}

#[test]
fn switch_settings_cover_vlans_ports_and_spanning_tree() {
    let mut app = App::from_lab_text(SWITCHES).unwrap();
    open(&mut app, "sw2");
    let said = app.configure_active(&[
        "vlan 10",
        "name sales",
        "exit",
        "interface fa0/3",
        "switchport mode access",
        "switchport access vlan 10",
        "exit",
        "spanning-tree mode rapid-pvst",
    ]);
    assert!(said.is_empty(), "{said:?}");
    let Some(DeviceState::Switch(s)) = app.active_state() else {
        panic!()
    };
    assert_eq!(s.ports[2].access_vlan, 10);
    assert_eq!(s.spanning_tree.as_deref(), Some("rapid-pvst"));
    // Another device's panel is unaffected.
    open(&mut app, "sw3");
    let Some(DeviceState::Switch(s3)) = app.active_state() else {
        panic!()
    };
    assert!(s3.spanning_tree.is_none());
}

#[test]
fn the_panel_draws_for_every_kind_of_device_without_panicking() {
    let mut app = App::from_lab_text(SWITCHES).unwrap();
    let ctx = Context::default();
    let mut t = 0.0;
    for name in ["sw1", "pc1", "sw2"] {
        open(&mut app, name);
        for _ in 0..3 {
            t += 0.1;
            frame(&ctx, &mut app, t);
        }
    }
    let mut routers = App::new();
    for name in ["r1", "pc1"] {
        open(&mut routers, name);
        for _ in 0..3 {
            t += 0.1;
            frame(&ctx, &mut routers, t);
        }
    }
}
