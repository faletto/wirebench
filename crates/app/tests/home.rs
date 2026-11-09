//! The home screen, the example labs, Continue, and the component bar, headless.

use app::autosave::MemoryStore;
use app::widget::TermAction;
use app::{App, Choice, Pending, Resume, Tool};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};
use engine::lab::LabKind;

fn frame(ctx: &Context, app: &mut App, events: Vec<Event>) {
    let raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    };
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
    for i in 1..=6 {
        let p = from + (to - from) * (i as f32 / 6.0);
        frame(ctx, app, vec![Event::PointerMoved(p)]);
    }
    frame(ctx, app, vec![button(to, false)]);
    frame(ctx, app, vec![]);
}

/// Run `line` at `device`'s console and return everything its console shows.
fn run(app: &mut App, device: &str, line: &str) -> String {
    let tab = app
        .device_names()
        .iter()
        .position(|n| *n == device)
        .unwrap_or_else(|| panic!("no {device}"));
    app.select(tab);
    app.apply(tab, TermAction::Submit(line.into()));
    app.active_scrollback().join("\n")
}

#[test]
fn the_home_screen_draws_and_a_new_lab_is_empty_and_named() {
    let mut app = App::new();
    app.show_home();
    let ctx = ready(&mut app);
    assert!(app.on_home());
    app.new_lab("  My first lab ");
    frame(&ctx, &mut app, vec![]);
    assert!(!app.on_home(), "the new lab is shown");
    assert_eq!(app.device_count(), 0);
    assert_eq!(app.title_text(), "My first lab — wirebench");
    assert!(!app.is_dirty(), "nothing to save in an empty lab");
    // An empty lab can still be built on.
    app.add_device(LabKind::Router, [0, 0]);
    assert_eq!(app.device_names(), ["r1"]);
}

#[test]
fn a_new_lab_without_a_name_gets_one() {
    let mut app = App::new();
    app.new_lab("");
    assert_eq!(app.title_text(), "Untitled lab — wirebench");
}

#[test]
fn every_example_opens_cleanly_and_draws() {
    let titles = App::example_titles();
    assert!(titles.len() >= 4);
    for (i, title) in titles.iter().enumerate() {
        let mut app = App::new();
        app.show_home();
        app.open_example(i);
        assert!(!app.on_home(), "{title}");
        let notice = app.status_notice().unwrap_or_default().to_string();
        assert!(
            notice.starts_with("opened") && !notice.contains('('),
            "{title}: {notice}"
        );
        assert!(app.device_count() > 0, "{title}");
        assert!(!app.is_dirty(), "{title}: an example opens unchanged");
        let ctx = ready(&mut app);
        for name in app.device_names() {
            let c = app.node_centre(name).unwrap();
            assert!(
                c.x > 0.0 && c.x < 1280.0 && c.y > 0.0 && c.y < 800.0,
                "{title}: {name}"
            );
        }
        drop(ctx);
    }
}

fn example(title: &str) -> App {
    let i = App::example_titles()
        .iter()
        .position(|t| *t == title)
        .unwrap_or_else(|| panic!("no example {title}"));
    let mut app = App::new();
    app.open_example(i);
    app
}

#[test]
fn the_vlan_example_keeps_the_vlans_apart() {
    let mut app = example("Two VLANs across a trunk");
    let out = run(&mut app, "pc1", "ping 10.0.10.2");
    assert!(out.contains("received 4"), "{out}");
    let out = run(&mut app, "pc1", "ping 10.0.20.4");
    assert!(out.contains("received 0"), "{out}");
    let out = run(&mut app, "pc3", "ping 10.0.20.4");
    assert!(out.contains("received 4"), "{out}");
}

#[test]
fn the_router_on_a_stick_example_routes_between_vlans() {
    let mut app = example("Router on a stick");
    let out = run(&mut app, "pc1", "ping 10.0.20.2");
    assert!(out.contains("received 4"), "{out}");
}

#[test]
fn the_static_routes_example_pings_end_to_end() {
    let mut app = example("Two routers, static routes");
    let out = run(&mut app, "pc1", "ping 10.0.2.2");
    assert!(out.contains("received 4"), "{out}");
}

#[test]
fn the_dhcp_example_hands_out_addresses_through_the_relay() {
    let mut app = example("DHCP with a relay");
    // The first may be lost while addresses are resolved.
    let out = run(&mut app, "pc3", "ping 10.0.1.1");
    assert!(out.contains("Reply from 10.0.1.1"), "{out}");
    let out = run(&mut app, "pc1", "ipconfig");
    assert!(
        out.contains("10.0.1.1"),
        "pc1 has the gateway from the pool: {out}"
    );
}

#[test]
fn the_spanning_tree_example_converges_and_forwards() {
    let mut app = example("Three switches and spanning tree");
    let out = run(&mut app, "pc1", "ping 10.0.0.2");
    assert!(out.contains("Reply from 10.0.0.2"), "{out}");
    let out = run(&mut app, "sw2", "show spanning-tree");
    assert!(out.contains("VLAN0001"), "{out}");
}

#[test]
fn continue_reopens_the_last_session_even_after_it_was_saved() {
    let store = MemoryStore::default();
    let mut a = App::new();
    a.set_store(Box::new(store.clone()));
    assert_eq!(a.resume(), None, "nothing to continue the first time");
    a.open_example(1);
    let opened = a.export_lab();
    // Next time the app starts, the home screen offers it back.
    let mut b = App::new();
    b.set_store(Box::new(store.clone()));
    b.show_home();
    match b.resume() {
        Some(Resume::Last { title, .. }) => assert_eq!(title, "Two VLANs across a trunk"),
        other => panic!("{other:?}"),
    }
    b.continue_session();
    assert!(!b.on_home());
    assert_eq!(b.export_lab(), opened);
    assert!(!b.is_dirty());
}

#[test]
fn continue_prefers_unsaved_work() {
    let store = MemoryStore::default();
    let mut a = App::new();
    a.set_store(Box::new(store.clone()));
    a.open_example(0);
    // Unsaved work left behind by a session that ended without saving.
    *store.0.borrow_mut() = Some(app::autosave::Autosave {
        text: a
            .export_lab()
            .replace("Two routers, static routes", "Half done"),
        unix_secs: app::autosave::now_unix_secs(),
    });
    let mut b = App::new();
    b.set_store(Box::new(store.clone()));
    match b.resume() {
        Some(Resume::Recovered { title, .. }) => assert_eq!(title, "Half done"),
        other => panic!("{other:?}"),
    }
    b.continue_session();
    assert_eq!(
        b.title_text(),
        "● Half done — wirebench",
        "recovered work is unsaved"
    );
}

#[test]
fn replacing_a_changed_lab_from_the_home_screen_asks_first() {
    let mut app = App::new();
    app.add_device(LabKind::Host, [0, 300]);
    assert!(app.is_dirty());
    app.show_home();
    app.open_example(1);
    assert_eq!(app.pending_confirm(), Some(Pending::Replace));
    app.confirm(Choice::Cancel);
    assert_eq!(app.device_count(), 5, "nothing replaced");
    assert!(app.on_home());
    app.open_example(1);
    app.confirm(Choice::Discard);
    assert!(!app.on_home());
    assert_eq!(app.title_text(), "Two VLANs across a trunk — wirebench");
}

#[test]
fn home_and_back_keeps_the_lab() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.add_device(LabKind::Host, [0, 300]);
    app.show_home();
    frame(&ctx, &mut app, vec![]);
    assert!(app.on_home());
    app.show_workspace();
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.device_count(), 5);
}

#[test]
fn clicking_a_device_opens_its_window_and_escape_closes_it() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    assert_eq!(app.device_window(), None, "nothing pops up by itself");
    let at = app.node_centre("r2").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.device_window(), Some("r2"));
    frame(
        &ctx,
        &mut app,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.device_window(), None);
}

#[test]
fn the_settings_tab_draws_for_every_kind_of_device() {
    let mut app = example("Router on a stick");
    let ctx = ready(&mut app);
    app.set_device_tab(app::DeviceTab::Settings);
    for i in 0..app.device_count() {
        app.select(i);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert!(app.device_window().is_some());
    }
}

#[test]
fn dragging_a_model_out_of_the_component_bar_adds_it_where_it_is_dropped() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.show_components("switches");
    frame(&ctx, &mut app, vec![]);
    let from = app.component_centre("switch-l2-24").expect("card drawn");
    let r1 = app.node_centre("r1").unwrap();
    let to = Pos2::new(r1.x, r1.y + 120.0);
    drag(&ctx, &mut app, from, to);
    assert_eq!(app.device_names(), ["r1", "r2", "pc1", "pc2", "sw1"]);
    assert!(app.export_lab().contains("switch-l2-24"));
    frame(&ctx, &mut app, vec![]);
    let c = app.node_centre("sw1").unwrap();
    assert!((c - to).length() < 3.0, "dropped at {to:?}, drawn at {c:?}");
    assert_eq!(app.device_window(), None, "adding does not pop a window up");
    assert_eq!(app.tool(), Tool::Select, "a drag leaves the tool alone");
}

#[test]
fn clicking_a_model_then_the_canvas_places_it() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.show_components("routers");
    frame(&ctx, &mut app, vec![]);
    let card = app.component_centre("router-4g").unwrap();
    click(&ctx, &mut app, card);
    assert_eq!(app.tool(), Tool::Place(LabKind::Router));
    let r1 = app.node_centre("r1").unwrap();
    click(&ctx, &mut app, Pos2::new(r1.x, r1.y + 120.0));
    assert_eq!(app.tool(), Tool::Select, "one click, one device");
    assert_eq!(app.device_names().last(), Some(&"r3"));
    assert!(app.export_lab().contains("router-4g"));
}

#[test]
fn a_cable_dropped_on_a_device_starts_there_and_a_click_ends_it() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    app.add_device(LabKind::Switch, [350, 260]);
    app.add_device(LabKind::Host, [150, 260]);
    app.show_components("cables");
    frame(&ctx, &mut app, vec![]);
    frame(&ctx, &mut app, vec![]);
    let from = app.component_centre("cable").unwrap();
    let sw = app.node_centre("sw1").unwrap();
    drag(&ctx, &mut app, from, sw);
    assert_eq!(app.tool(), Tool::Link);
    let row = app.popup_row_centre("Fa0/1").expect("which port?");
    click(&ctx, &mut app, row);
    let pc = app.node_centre("pc3").unwrap();
    click(&ctx, &mut app, pc);
    let row = app.popup_row_centre("eth0").expect("which port?");
    click(&ctx, &mut app, row);
    assert!(app.cables().contains(&("sw1", "pc3")), "{:?}", app.cables());
}

fn wheel(ctx: &Context, app: &mut App, at: Pos2) {
    frame(ctx, app, vec![Event::PointerMoved(at)]);
    frame(
        ctx,
        app,
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        }],
    );
    for _ in 0..8 {
        frame(ctx, app, vec![]);
    }
}

#[test]
fn scrolling_over_a_device_window_does_not_zoom_the_topology() {
    let mut app = App::new();
    let ctx = ready(&mut app);
    let at = app.node_centre("r2").unwrap();
    click(&ctx, &mut app, at);
    assert_eq!(app.device_window(), Some("r2"));
    frame(&ctx, &mut app, vec![]);
    // Find a point covered by the window and one on the bare canvas.
    let mut inside = None;
    let mut bare = None;
    let canvas = app.canvas_rect();
    for y in (canvas.top() as i32..canvas.bottom() as i32).step_by(10) {
        for x in (canvas.left() as i32..canvas.right() as i32).step_by(10) {
            let p = Pos2::new(x as f32, y as f32);
            match ctx.layer_id_at(p).map(|l| l.order) {
                Some(egui::Order::Middle) if inside.is_none() => inside = Some(p),
                Some(egui::Order::Background) if bare.is_none() => bare = Some(p),
                _ => {}
            }
        }
    }
    let (inside, bare) = (
        inside.expect("window covers some point"),
        bare.expect("canvas visible"),
    );
    let z0 = app.canvas_zoom();
    wheel(&ctx, &mut app, inside);
    assert_eq!(app.canvas_zoom(), z0, "the window keeps the wheel");
    wheel(&ctx, &mut app, bare);
    assert_ne!(app.canvas_zoom(), z0, "the canvas itself still zooms");
}
