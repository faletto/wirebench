//! Building and editing a lab through the message protocol alone, as a UI in another thread
//! (a Web Worker) has to: every command and answer goes through postcard bytes.

use engine::protocol::{decode, encode, Engine, EngineCmd, EngineEvent, LabView, ShellKind};
use kernel::Trace;

const LAB: &str = include_str!("../../../labs/m1_static_routes.lab.toml");

/// An engine that has never seen a lab, and a way to talk to it over bytes.
struct Rig {
    e: Engine,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            e: Engine::new(engine::World::new(Trace::new())),
        }
    }

    fn send(&mut self, cmd: EngineCmd) -> Vec<EngineEvent> {
        let out = self.e.handle_bytes(&encode(&cmd));
        decode(&out).expect("answers decode")
    }

    fn view(&mut self, cmd: EngineCmd) -> LabView {
        match self.send(cmd).remove(0) {
            EngineEvent::Lab { view } => view,
            other => panic!("expected a lab view, got {other:?}"),
        }
    }

    fn error(&mut self, cmd: EngineCmd) -> String {
        match self.send(cmd).remove(0) {
            EngineEvent::Error { message } => message,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    fn text(&mut self) -> String {
        match self.send(EngineCmd::Snapshot).remove(0) {
            EngineEvent::LabText { text } => text,
            other => panic!("expected text, got {other:?}"),
        }
    }
}

#[test]
fn before_a_lab_is_loaded_edits_and_snapshots_are_refused() {
    let mut r = Rig::new();
    assert_eq!(r.error(EngineCmd::Snapshot), "no lab loaded");
    assert_eq!(
        r.error(EngineCmd::AddDevice {
            kind: ShellKind::Host,
            pos: [0, 0]
        }),
        "no lab loaded"
    );
}

#[test]
fn loading_describes_the_lab_and_lets_every_device_open_a_console() {
    let mut r = Rig::new();
    let v = r.view(EngineCmd::Load { lab: LAB.into() });
    assert!(!v.devices.is_empty() && !v.links.is_empty());
    assert!(v.devices.iter().any(|d| d.kind == ShellKind::Router));
    for d in &v.devices {
        let ev = r
            .send(EngineCmd::OpenTerminal { device: d.device })
            .remove(0);
        assert!(
            matches!(ev, EngineEvent::TerminalOpened { .. }),
            "{}: {ev:?}",
            d.name
        );
    }
    let bad = r.error(EngineCmd::Load {
        lab: "not a lab".into(),
    });
    assert!(!bad.is_empty());
    // A refused load leaves the old lab in place.
    assert!(r.text().contains("pc1"));
}

#[test]
fn adding_devices_and_cables_updates_the_view_and_the_snapshot() {
    let mut r = Rig::new();
    r.view(EngineCmd::Load { lab: LAB.into() });
    let before = r.text();
    let v = r.view(EngineCmd::AddDevice {
        kind: ShellKind::Switch,
        pos: [40, 50],
    });
    let sw = v.devices.last().unwrap().clone();
    assert_eq!(
        (sw.name.as_str(), sw.pos, sw.kind),
        ("sw1", [40, 50], ShellKind::Switch)
    );
    // The new device has a console straight away.
    let ev = r
        .send(EngineCmd::OpenTerminal { device: sw.device })
        .remove(0);
    assert!(matches!(ev, EngineEvent::TerminalOpened { .. }));
    let n = v.devices.len();
    r.view(EngineCmd::AddDevice {
        kind: ShellKind::Switch,
        pos: [90, 50],
    });
    let v = r.view(EngineCmd::AddLink { a: n - 1, b: n });
    assert_eq!(v.links.last().map(|l| (l.a, l.b)), Some((n - 1, n)));
    let v = r.view(EngineCmd::MoveDevice {
        device: n - 1,
        pos: [1, 2],
    });
    assert_eq!(v.devices[n - 1].pos, [1, 2]);
    let after = r.text();
    assert_ne!(after, before);
    assert!(after.contains("sw1"));
    // Loading the snapshot elsewhere gives the same lab and the same text.
    let mut r2 = Rig::new();
    let v2 = r2.view(EngineCmd::Load { lab: after.clone() });
    assert_eq!(v2.devices.len(), n + 1);
    assert_eq!(r2.text(), after);
}

#[test]
fn a_cable_with_nowhere_to_go_is_refused_with_a_reason() {
    let mut r = Rig::new();
    r.view(EngineCmd::Load { lab: LAB.into() });
    assert!(!r.error(EngineCmd::AddLink { a: 0, b: 0 }).is_empty());
    assert!(!r.error(EngineCmd::AddLink { a: 0, b: 99 }).is_empty());
}

#[test]
fn removing_a_cable_or_device_happens_in_place_and_closes_only_that_devices_console() {
    let mut r = Rig::new();
    let v = r.view(EngineCmd::Load { lab: LAB.into() });
    let open = |r: &mut Rig, i: usize| match r
        .send(EngineCmd::OpenTerminal {
            device: v.devices[i].device,
        })
        .remove(0)
    {
        EngineEvent::TerminalOpened { terminal, .. } => terminal,
        other => panic!("{other:?}"),
    };
    let (t0, t1) = (open(&mut r, 0), open(&mut r, 1));
    let links = v.links.len();
    let v2 = r.view(EngineCmd::RemoveLink { link: 0 });
    assert_eq!(v2.links.len(), links - 1);
    assert_eq!(v2.devices.len(), v.devices.len());
    // Consoles are untouched by a cable going.
    let exec = |r: &mut Rig, t: u32| {
        r.send(EngineCmd::Exec {
            terminal: t,
            line: "".into(),
        })
        .remove(0)
    };
    assert!(matches!(exec(&mut r, t0), EngineEvent::Output { .. }));
    let first = v2.devices[0].name.clone();
    let v3 = r.view(EngineCmd::RemoveDevice { device: 0 });
    assert_eq!(v3.devices.len(), v.devices.len() - 1);
    assert!(v3.devices.iter().all(|d| d.name != first));
    assert!(v3
        .links
        .iter()
        .all(|l| l.a < v3.devices.len() && l.b < v3.devices.len()));
    // The removed device's console is gone; the other device's is the same session.
    assert!(matches!(exec(&mut r, t0), EngineEvent::Error { .. }));
    assert!(matches!(exec(&mut r, t1), EngineEvent::Output { .. }));
    assert!(!r.error(EngineCmd::RemoveDevice { device: 99 }).is_empty());
}

#[test]
fn configuration_typed_at_a_console_is_in_the_snapshot() {
    let mut r = Rig::new();
    let v = r.view(EngineCmd::Load { lab: LAB.into() });
    let router = v
        .devices
        .iter()
        .find(|d| d.kind == ShellKind::Router)
        .unwrap()
        .clone();
    let t = match r
        .send(EngineCmd::OpenTerminal {
            device: router.device,
        })
        .remove(0)
    {
        EngineEvent::TerminalOpened { terminal, .. } => terminal,
        other => panic!("{other:?}"),
    };
    for line in ["enable", "configure terminal", "hostname typed-here", "end"] {
        r.send(EngineCmd::Exec {
            terminal: t,
            line: line.into(),
        });
    }
    assert!(r.text().contains("typed-here"));
}

#[test]
fn now_reports_the_clock_and_whether_upkeep_is_pending() {
    let mut r = Rig::new();
    r.view(EngineCmd::Load { lab: LAB.into() });
    match r.send(EngineCmd::Now).remove(0) {
        EngineEvent::Time { ns, background } => {
            assert_eq!(ns, 0);
            assert!(!background);
        }
        other => panic!("{other:?}"),
    }
}

fn texts(evs: &[EngineEvent]) -> Vec<&String> {
    evs.iter()
        .filter_map(|e| match e {
            EngineEvent::LabText { text } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn the_engine_pushes_the_lab_text_after_what_can_change_it_and_only_when_it_did() {
    let mut r = Rig::new();
    let evs = r.send(EngineCmd::Load { lab: LAB.into() });
    assert!(matches!(evs[0], EngineEvent::Lab { .. }));
    assert_eq!(texts(&evs).len(), 1, "a load is followed by the text");
    let loaded = texts(&evs)[0].clone();
    assert_eq!(loaded, r.text());
    let evs = r.send(EngineCmd::AddDevice {
        kind: ShellKind::Switch,
        pos: [1, 2],
    });
    assert_eq!(texts(&evs).len(), 1);
    assert_ne!(*texts(&evs)[0], loaded);
    let EngineEvent::Lab { view } = &evs[0] else {
        panic!()
    };
    let (n, dev) = (
        view.devices.len() - 1,
        view.devices[view.devices.len() - 1].clone(),
    );
    // A move to where the device already is changes nothing, so nothing is pushed.
    let evs = r.send(EngineCmd::MoveDevice {
        device: n,
        pos: dev.pos,
    });
    assert!(texts(&evs).is_empty(), "{evs:?}");
    let evs = r.send(EngineCmd::MoveDevice {
        device: n,
        pos: [7, 8],
    });
    assert_eq!(texts(&evs).len(), 1);
    // A console line that changes the config is followed by the text; one that does not, is not.
    let term = match r
        .send(EngineCmd::OpenTerminal { device: dev.device })
        .remove(0)
    {
        EngineEvent::TerminalOpened { terminal, .. } => terminal,
        other => panic!("{other:?}"),
    };
    let say = |r: &mut Rig, line: &str| {
        r.send(EngineCmd::Exec {
            terminal: term,
            line: line.into(),
        })
    };
    assert!(texts(&say(&mut r, "enable")).is_empty());
    assert!(texts(&say(&mut r, "show vlan brief")).is_empty());
    say(&mut r, "configure terminal");
    let evs = say(&mut r, "hostname pushed");
    assert_eq!(texts(&evs).len(), 1);
    assert!(texts(&evs)[0].contains("pushed"));
    // The output still comes first.
    assert!(matches!(evs[0], EngineEvent::Output { .. }));
}
