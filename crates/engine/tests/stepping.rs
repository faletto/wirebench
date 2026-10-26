use engine::lab::{load_and_build, LabKind};
use engine::protocol::{decode, encode, Engine, EngineCmd, EngineEvent, ShellKind};
use kernel::Trace;

const LAB: &str = include_str!("../../../labs/m1_static_routes.lab.toml");

/// Engine for the bundled lab plus the device ids by name.
fn engine() -> (Engine, impl Fn(&str) -> u32) {
    let built = load_and_build(LAB, Trace::new()).unwrap();
    let ids: Vec<(String, u32)> = built
        .devices
        .iter()
        .map(|d| (d.name.clone(), d.id.0))
        .collect();
    let mut e = Engine::new(built.world);
    for d in &built.devices {
        let kind = match d.kind {
            LabKind::Router => ShellKind::Router,
            LabKind::Host => ShellKind::Host,
            LabKind::Switch => ShellKind::Switch,
        };
        e = e.with_kind(d.id, kind);
    }
    let id = move |n: &str| ids.iter().find(|(m, _)| m == n).unwrap().1;
    (e, id)
}

fn step_all(e: &mut Engine) -> Vec<EngineEvent> {
    let mut out = Vec::new();
    loop {
        let ev = e.handle(EngineCmd::Step).remove(0);
        let done = matches!(ev, EngineEvent::Idle { .. });
        out.push(ev);
        if done {
            return out;
        }
    }
}

fn hops(evs: &[EngineEvent]) -> Vec<(u32, String)> {
    evs.iter()
        .filter_map(|e| match e {
            EngineEvent::Hop {
                device, summary, ..
            } => Some((*device, summary.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn stepping_a_ping_shows_each_hop() {
    let (mut e, id) = engine();
    let (pc1, r1) = (id("pc1"), id("r1"));
    e.handle(EngineCmd::PingOnce {
        device: pc1,
        dst: [10, 0, 1, 1],
    });
    let evs = step_all(&mut e);
    assert_eq!(
        hops(&evs),
        vec![
            (
                r1,
                "ARP request: who has 10.0.1.1? tell 10.0.1.2".to_string()
            ),
            (pc1, "ARP reply: 10.0.1.1 is at 02aa.0000.0000".to_string()),
            (
                r1,
                "ICMP echo request seq=1: 10.0.1.2 > 10.0.1.1 ttl=64".to_string()
            ),
            (
                pc1,
                "ICMP echo reply seq=1: 10.0.1.1 > 10.0.1.2 ttl=255".to_string()
            ),
        ]
    );
    // The ARP timeout timer is not a hop but is still stepped, and time never goes backwards.
    assert!(evs
        .iter()
        .any(|e| matches!(e, EngineEvent::Tick { device, .. } if *device == pc1)));
    let times: Vec<u64> = evs
        .iter()
        .map(|e| match e {
            EngineEvent::Hop { ns, .. }
            | EngineEvent::Tick { ns, .. }
            | EngineEvent::Idle { ns } => *ns,
            _ => unreachable!(),
        })
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    // Idle stays idle.
    assert!(matches!(
        e.handle(EngineCmd::Step)[0],
        EngineEvent::Idle { .. }
    ));
}

#[test]
fn stepping_is_deterministic_and_matches_running_straight_through() {
    let run = |stepped: bool| {
        let (mut e, id) = engine();
        e.handle(EngineCmd::PingOnce {
            device: id("pc1"),
            dst: [10, 0, 2, 2],
        });
        if stepped {
            step_all(&mut e);
        } else {
            e.handle(EngineCmd::RunToIdle);
        }
        e.world.trace_hash()
    };
    assert_eq!(run(true), run(true));
    assert_eq!(
        run(true),
        run(false),
        "Step and RunToIdle must dispatch the same events"
    );
}

#[test]
fn only_pcs_can_ping_and_new_messages_survive_the_byte_protocol() {
    let (mut e, id) = engine();
    assert!(matches!(
        e.handle(EngineCmd::PingOnce {
            device: id("r1"),
            dst: [10, 0, 1, 2]
        })[0],
        EngineEvent::Error { .. }
    ));
    let cmd = EngineCmd::PingOnce {
        device: id("pc1"),
        dst: [10, 0, 1, 1],
    };
    assert_eq!(decode::<EngineCmd>(&encode(&cmd)).unwrap(), cmd);
    let out: Vec<EngineEvent> = decode(&e.handle_bytes(&encode(&EngineCmd::Step))).unwrap();
    assert!(matches!(out[0], EngineEvent::Idle { .. }));
    let hop = EngineEvent::Hop {
        ns: 5,
        device: 1,
        port: 2,
        summary: "x".into(),
        bytes: 60,
        frame: vec![1, 2, 3],
        from: 3,
        sent_ns: 2,
    };
    assert_eq!(decode::<EngineEvent>(&encode(&hop)).unwrap(), hop);
}

/// Each hop says who sent the frame and when it left: what the UI needs to animate it.
#[test]
fn hops_carry_sender_and_departure_time() {
    let (mut e, id) = engine();
    let (pc1, r1) = (id("pc1"), id("r1"));
    e.handle(EngineCmd::PingOnce {
        device: pc1,
        dst: [10, 0, 1, 1],
    });
    let evs = step_all(&mut e);
    let hops: Vec<(u32, u32, u64, u64)> = evs
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Hop {
                device,
                from,
                ns,
                sent_ns,
                ..
            } => Some((*from, *device, *sent_ns, *ns)),
            _ => None,
        })
        .collect();
    // ARP request pc1 -> r1, ARP reply r1 -> pc1, echo request pc1 -> r1, echo reply r1 -> pc1.
    let ends: Vec<(u32, u32)> = hops.iter().map(|h| (h.0, h.1)).collect();
    assert_eq!(ends, [(pc1, r1), (r1, pc1), (pc1, r1), (r1, pc1)]);
    for (_, _, sent, arrived) in &hops {
        assert!(sent < arrived, "a frame takes time on the wire: {hops:?}");
    }
    // The first frame leaves at t=0 (serialization starts at once).
    assert_eq!(hops[0].2, 0);
}

#[test]
fn stepping_back_restores_the_world_exactly() {
    let (mut e, id) = engine();
    e.handle(EngineCmd::PingOnce {
        device: id("pc1"),
        dst: [10, 0, 1, 1],
    });
    let mut seen = vec![e.world.trace_hash()];
    for _ in 0..5 {
        e.handle(EngineCmd::Step);
        seen.push(e.world.trace_hash());
    }
    // Back through every step lands on each earlier state, and the ping itself can be undone.
    for want in seen.iter().rev().skip(1) {
        let ev = e.handle(EngineCmd::StepBack).remove(0);
        assert!(matches!(ev, EngineEvent::Rewound { .. }), "{ev:?}");
        assert_eq!(e.world.trace_hash(), *want);
    }
    let ev = e.handle(EngineCmd::StepBack).remove(0);
    assert!(matches!(ev, EngineEvent::Rewound { left: 0, .. }));
    assert!(matches!(
        e.handle(EngineCmd::StepBack).remove(0),
        EngineEvent::Error { .. }
    ));
    // And forward again gives the same history.
    e.handle(EngineCmd::PingOnce {
        device: id("pc1"),
        dst: [10, 0, 1, 1],
    });
    for want in seen.iter().skip(1) {
        e.handle(EngineCmd::Step);
        assert_eq!(e.world.trace_hash(), *want);
    }
}

#[test]
fn a_console_command_or_a_run_forgets_what_could_be_undone() {
    let (mut e, id) = engine();
    e.handle(EngineCmd::PingOnce {
        device: id("pc1"),
        dst: [10, 0, 1, 1],
    });
    e.handle(EngineCmd::Step);
    e.handle(EngineCmd::ForgetHistory);
    assert!(matches!(
        e.handle(EngineCmd::StepBack).remove(0),
        EngineEvent::Error { .. }
    ));
    e.handle(EngineCmd::Step);
    e.handle(EngineCmd::RunToIdle);
    assert!(matches!(
        e.handle(EngineCmd::StepBack).remove(0),
        EngineEvent::Error { .. }
    ));
}

#[test]
fn a_step_that_finds_nothing_leaves_nothing_to_undo() {
    let (mut e, _) = engine();
    assert!(matches!(
        e.handle(EngineCmd::Step).remove(0),
        EngineEvent::Idle { .. }
    ));
    assert!(matches!(
        e.handle(EngineCmd::StepBack).remove(0),
        EngineEvent::Error { .. }
    ));
}

#[test]
fn hops_carry_the_frame_itself() {
    let (mut e, id) = engine();
    e.handle(EngineCmd::PingOnce {
        device: id("pc1"),
        dst: [10, 0, 1, 1],
    });
    let evs = step_all(&mut e);
    let frames: Vec<&Vec<u8>> = evs
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Hop { frame, bytes, .. } => {
                assert_eq!(frame.len() as u32, *bytes);
                Some(frame)
            }
            _ => None,
        })
        .collect();
    assert!(!frames.is_empty());
    assert!(frames.iter().all(|f| wire::EthernetFrame::parse(f).is_ok()));
}
