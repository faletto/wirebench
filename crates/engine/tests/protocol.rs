use engine::chain::blank_chain;
use engine::protocol::{decode, encode, Engine, EngineCmd, EngineEvent, ShellKind};

#[test]
fn protocol_messages_roundtrip() {
    let cmds = vec![
        EngineCmd::OpenTerminal { device: 3 },
        EngineCmd::Exec {
            terminal: 1,
            line: "conf t".into(),
        },
        EngineCmd::Help {
            terminal: 0,
            line: "show ".into(),
        },
        EngineCmd::Tab {
            terminal: 2,
            line: "sh".into(),
        },
        EngineCmd::Advance { ns: u64::MAX },
    ];
    for c in cmds {
        assert_eq!(decode::<EngineCmd>(&encode(&c)).unwrap(), c);
    }
    let evs = vec![
        EngineEvent::TerminalOpened {
            terminal: 0,
            prompt: "r1>".into(),
            greeting: String::new(),
        },
        EngineEvent::Output {
            terminal: 0,
            text: "x\ny".into(),
            prompt: "r1#".into(),
        },
        EngineEvent::Completion {
            terminal: 0,
            line: "show ".into(),
        },
        EngineEvent::Closed { terminal: 7 },
        EngineEvent::Time {
            ns: 123,
            background: false,
        },
        EngineEvent::Error {
            message: "no".into(),
        },
    ];
    assert_eq!(decode::<Vec<EngineEvent>>(&encode(&evs)).unwrap(), evs);
}

#[test]
fn garbage_bytes_yield_an_error_event_not_a_panic() {
    let c = blank_chain();
    let mut e = Engine::new(c.world);
    let out: Vec<EngineEvent> = decode(&e.handle_bytes(&[0xff, 0xff, 0xff])).unwrap();
    assert!(matches!(out[0], EngineEvent::Error { .. }));
}

#[test]
fn terminal_session_over_the_byte_protocol() {
    let c = blank_chain();
    let (r1, pc1) = (c.r1, c.pc1);
    let mut e = Engine::new(c.world)
        .with_kind(r1, ShellKind::Router)
        .with_kind(pc1, ShellKind::Host);
    let mut call =
        |cmd: EngineCmd| -> Vec<EngineEvent> { decode(&e.handle_bytes(&encode(&cmd))).unwrap() };

    let EngineEvent::TerminalOpened {
        terminal, prompt, ..
    } = call(EngineCmd::OpenTerminal { device: r1.0 }).remove(0)
    else {
        panic!("expected TerminalOpened")
    };
    assert_eq!(prompt, "r1>");
    let out = call(EngineCmd::Exec {
        terminal,
        line: "enable".into(),
    });
    assert_eq!(
        out,
        vec![EngineEvent::Output {
            terminal,
            text: String::new(),
            prompt: "r1#".into()
        }]
    );
    let out = call(EngineCmd::Tab {
        terminal,
        line: "conf".into(),
    });
    assert_eq!(
        out,
        vec![EngineEvent::Completion {
            terminal,
            line: "configure ".into()
        }]
    );
    let out = call(EngineCmd::Help {
        terminal,
        line: "show ip ".into(),
    });
    let EngineEvent::Output { text, .. } = &out[0] else {
        panic!()
    };
    assert!(text.contains("route"));
    assert_eq!(
        call(EngineCmd::Exec {
            terminal,
            line: "exit".into()
        }),
        vec![EngineEvent::Closed { terminal }]
    );
    // The closed terminal is gone.
    assert!(matches!(
        call(EngineCmd::Exec {
            terminal,
            line: "x".into()
        })[0],
        EngineEvent::Error { .. }
    ));
    assert_eq!(
        call(EngineCmd::Advance { ns: 5_000 }),
        vec![EngineEvent::Time {
            ns: 5_000,
            background: false,
        }]
    );
}

#[test]
fn requests_and_responses_carry_the_number_that_pairs_them() {
    use engine::protocol::{Request, Response};
    let mut e = Engine::new(engine::World::new(kernel::Trace::new()));
    let out = e.serve(&encode(&Request {
        id: 41,
        cmd: EngineCmd::Now,
    }));
    let r: Response = decode(&out).unwrap();
    assert_eq!(r.id, 41);
    assert!(matches!(r.events[0], EngineEvent::Time { .. }));
    // Garbage is answered, with a number that matches nothing.
    let r: Response = decode(&e.serve(&[0xff, 0xff, 0xff])).unwrap();
    assert_eq!(r.id, u64::MAX);
    assert!(matches!(r.events[0], EngineEvent::Error { .. }));
}
