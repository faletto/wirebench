//! `tracert`, `traceroute` and a router's `ping` from the console.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::{SimTime, Trace};

const CHAIN: &str = include_str!("../../../testkit/scenarios/m2_traceroute.lab.toml");

const SETUP: &[(&str, &[&str])] = &[
    (
        "r1",
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.1.1 255.255.255.0",
            "no shutdown",
            "interface g0/1",
            "ip address 10.0.12.1 255.255.255.252",
            "no shutdown",
            "exit",
            "ip route 10.0.2.0 255.255.255.0 10.0.12.2",
            "ip route 10.77.0.0 255.255.0.0 10.0.12.2",
            "ip route 10.0.23.0 255.255.255.252 10.0.12.2",
            "end",
        ],
    ),
    (
        "r2",
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.12.2 255.255.255.252",
            "no shutdown",
            "interface g0/1",
            "ip address 10.0.23.1 255.255.255.252",
            "no shutdown",
            "exit",
            "ip route 10.0.1.0 255.255.255.0 10.0.12.1",
            "ip route 10.0.2.0 255.255.255.0 10.0.23.2",
            "end",
        ],
    ),
    (
        "r3",
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.23.2 255.255.255.252",
            "no shutdown",
            "interface g0/1",
            "ip address 10.0.2.1 255.255.255.0",
            "no shutdown",
            "exit",
            "ip route 10.0.1.0 255.255.255.0 10.0.23.1",
            "ip route 10.0.12.0 255.255.255.252 10.0.23.1",
            "end",
        ],
    ),
    (
        "pc1",
        &[
            "ipconfig set 10.0.1.2 255.255.255.0",
            "ipconfig gateway 10.0.1.1",
        ],
    ),
    (
        "pc2",
        &[
            "ipconfig set 10.0.2.2 255.255.255.0",
            "ipconfig gateway 10.0.2.1",
        ],
    ),
];

fn chain() -> (BuiltLab, impl FnMut(&mut BuiltLab, &str, &str) -> String) {
    let mut lab = load_and_build(CHAIN, Trace::new()).unwrap();
    let mut shells: Vec<(String, Shell)> = lab
        .devices
        .iter()
        .map(|d| {
            let sh = match d.kind {
                engine::lab::LabKind::Host => Shell::host(d.id),
                _ => Shell::router(d.id),
            };
            (d.name.clone(), sh)
        })
        .collect();
    for (who, lines) in SETUP {
        let sh = &mut shells.iter_mut().find(|(n, _)| n == who).unwrap().1;
        for l in *lines {
            let out = sh.exec(&mut lab.world, l).unwrap();
            assert!(out.is_empty(), "{who}: `{l}` printed {out:?}");
        }
    }
    let run = move |lab: &mut BuiltLab, who: &str, line: &str| {
        let sh = &mut shells.iter_mut().find(|(n, _)| n == who).unwrap().1;
        sh.exec(&mut lab.world, line).unwrap()
    };
    (lab, run)
}

#[test]
fn tracert_lists_every_hop_then_the_destination() {
    let (mut lab, mut run) = chain();
    let out = run(&mut lab, "pc1", "tracert 10.0.2.2");
    let hops: Vec<&str> = out.lines().filter(|l| l.starts_with("  ")).collect();
    assert_eq!(hops.len(), 4, "{out}");
    for (line, want) in hops
        .iter()
        .zip(["10.0.1.1", "10.0.12.2", "10.0.23.2", "10.0.2.2"])
    {
        assert!(line.ends_with(want), "{line} should end with {want}\n{out}");
        assert_eq!(
            line.matches(" us").count(),
            3,
            "three probes per hop: {line}"
        );
    }
    assert!(
        out.starts_with("Tracing route to 10.0.2.2 over a maximum of 30 hops\n\n"),
        "{out}"
    );
    assert!(out.ends_with("\nTrace complete."), "{out}");
}

#[test]
fn tracert_h_limits_the_hops_and_a_router_that_cannot_deliver_ends_it() {
    let (mut lab, mut run) = chain();
    let out = run(&mut lab, "pc1", "tracert -h 2 10.0.2.2");
    assert_eq!(
        out.lines().filter(|l| l.starts_with("  ")).count(),
        2,
        "{out}"
    );
    assert!(out.contains("maximum of 2 hops"));
    let t0 = lab.world.kernel.now();
    let out = run(&mut lab, "pc1", "tracert 10.0.9.9");
    assert!(
        out.contains("reports: Destination net unreachable."),
        "{out}"
    );
    assert!(out.contains("10.0.1.1  reports"), "{out}");
    // It stops at the first unreachable instead of waiting out the other probes of that hop.
    assert!(
        lab.world.kernel.now().0 - t0.0 < 1_000_000_000,
        "no timeouts were waited out"
    );
}

#[test]
fn tracert_to_a_silent_address_shows_timeouts() {
    let (mut lab, mut run) = chain();
    let out = run(&mut lab, "pc1", "tracert -h 5 10.0.2.99");
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("  ")).collect();
    assert_eq!(lines.len(), 5, "{out}");
    assert!(
        lines[2].ends_with("10.0.23.2"),
        "r3 still answers at hop 3: {out}"
    );
    assert!(lines[3].ends_with("Request timed out."), "{out}");
    assert!(lines[3].matches('*').count() == 3, "{out}");
    // Each of the six silent probes waited out the three-second timeout.
    assert!(lab.world.kernel.now() >= SimTime::from_secs(18));
}

#[test]
fn a_router_pings_with_marks_and_a_summary() {
    let (mut lab, mut run) = chain();
    let ok = run(&mut lab, "r1", "ping 10.0.2.2");
    assert!(
        ok.contains("Sending 5, 100-byte ICMP Echos to 10.0.2.2, timeout is 2 seconds:\n!!!!!\n"),
        "{ok}"
    );
    assert!(
        ok.contains("Success rate is 100 percent (5/5), round-trip min/avg/max = "),
        "{ok}"
    );
    let bad = run(&mut lab, "r1", "ping 10.0.2.99");
    assert!(
        bad.contains("\n.....\n") && bad.contains("0 percent (0/5)"),
        "{bad}"
    );
    assert!(!bad.contains("round-trip"), "{bad}");
    // A router that cannot deliver is a `U`, not a timeout.
    let u = run(&mut lab, "r1", "ping 10.77.0.1");
    assert!(u.contains("\nU"), "{u}");
}

#[test]
fn a_routers_own_address_and_a_missing_route_are_handled() {
    let (mut lab, mut run) = chain();
    let own = run(&mut lab, "r1", "ping 10.0.1.1");
    assert!(own.contains("!!!!!") && own.contains("(5/5)"), "{own}");
    assert!(run(&mut lab, "r1", "traceroute 9.9.9.9").starts_with("% No route to 9.9.9.9"));
    assert!(!run(&mut lab, "r1", "ping 9.9.9.9").contains('!'));
    let own_trace = run(&mut lab, "r1", "traceroute 10.0.1.1");
    assert!(own_trace.contains("  1 10.0.1.1"), "{own_trace}");
}

#[test]
fn traceroute_marks_where_a_router_could_not_deliver() {
    let (mut lab, mut run) = chain();
    let t0 = lab.world.kernel.now();
    let out = run(&mut lab, "r1", "traceroute 10.77.0.1");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[lines.len() - 1], "  2 10.0.12.2 !N", "{out}");
    assert!(
        lab.world.kernel.now().0 - t0.0 < 1_000_000_000,
        "stopped at the first !N"
    );
}

#[test]
fn traceroute_ttl_range_and_the_far_address_of_a_router() {
    let (mut lab, mut run) = chain();
    let out = run(&mut lab, "r1", "traceroute 10.0.2.2 ttl 2 3");
    let hops: Vec<&str> = out.lines().filter(|l| l.starts_with("  ")).collect();
    assert_eq!(hops.len(), 2, "{out}");
    assert!(
        hops[0].starts_with("  2 10.0.23.2") && hops[1].starts_with("  3 10.0.2.2"),
        "{out}"
    );
    // A router's other interface answers from the address that was asked for.
    let far = run(&mut lab, "r1", "traceroute 10.0.2.1");
    assert!(
        far.lines().last().unwrap().starts_with("  2 10.0.2.1 "),
        "{far}"
    );
    // A range whose end is below its start still sends the first probe.
    let one = run(&mut lab, "r1", "traceroute 10.0.2.2 ttl 3 1");
    assert_eq!(
        one.lines().filter(|l| l.starts_with("  ")).count(),
        1,
        "{one}"
    );
}

#[test]
fn bad_arguments_are_parse_errors() {
    let (mut lab, mut run) = chain();
    for (who, line) in [
        ("pc1", "tracert"),
        ("pc1", "tracert -h 0 10.0.2.2"),
        ("pc1", "tracert -h 300 10.0.2.2"),
        ("r1", "traceroute"),
        ("r1", "traceroute 10.0.2.2 ttl 0 5"),
        ("r1", "traceroute 10.0.2.2 ttl 1"),
        ("r1", "ping"),
        ("r1", "ping 10.0.2"),
    ] {
        let out = run(&mut lab, who, line);
        assert!(
            out.contains('^') || out.contains("incomplete"),
            "{who}: {line} -> {out}"
        );
    }
}

const SWITCH: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "pc1"
template = "host-pc"
pos = [0, 0]
[device.host]
ipv4 = "10.0.10.2/24"
gateway = "10.0.10.1"

[[device]]
name = "pc2"
template = "host-pc"
pos = [0, 100]
[device.host]
ipv4 = "10.0.20.2/24"
gateway = "10.0.20.1"

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [100, 0]

[[link]]
a = "pc1:eth0"
b = "sw1:Fa0/1"

[[link]]
a = "pc2:eth0"
b = "sw1:Fa0/2"
"#;

#[test]
fn a_layer_3_switch_pings_and_traces_through_its_svis() {
    let mut lab = load_and_build(SWITCH, Trace::new()).unwrap();
    let mut sw = Shell::switch(lab.devices[2].id);
    for l in [
        "enable",
        "configure terminal",
        "interface fa0/1",
        "switchport access vlan 10",
        "interface fa0/2",
        "switchport access vlan 20",
        "interface vlan 10",
        "ip address 10.0.10.1 255.255.255.0",
        "interface vlan 20",
        "ip address 10.0.20.1 255.255.255.0",
        "exit",
        "ip routing",
        "end",
    ] {
        sw.exec(&mut lab.world, l);
    }
    let ping = sw.exec(&mut lab.world, "ping 10.0.20.2").unwrap();
    assert!(ping.contains("!!!!!") && ping.contains("(5/5)"), "{ping}");
    // The switch is the first hop for its own probes: the PC answers straight away.
    let t = sw.exec(&mut lab.world, "traceroute 10.0.10.2").unwrap();
    assert!(
        t.lines().last().unwrap().starts_with("  1 10.0.10.2 "),
        "{t}"
    );
    // A PC behind it reaches the other VLAN through the switch, and sees it as hop 1.
    let mut pc = Shell::host(lab.devices[0].id);
    let t = pc.exec(&mut lab.world, "tracert 10.0.20.2").unwrap();
    let hops: Vec<&str> = t.lines().filter(|l| l.starts_with("  ")).collect();
    assert_eq!(hops.len(), 2, "{t}");
    assert!(
        hops[0].ends_with("10.0.10.1") && hops[1].ends_with("10.0.20.2"),
        "{t}"
    );
    // With routing off the switch does not forward, so the trace dies after the first hop.
    sw.exec(&mut lab.world, "configure terminal");
    sw.exec(&mut lab.world, "no ip routing");
    sw.exec(&mut lab.world, "end");
    let t = pc.exec(&mut lab.world, "tracert -h 3 10.0.20.2").unwrap();
    assert!(
        t.lines()
            .filter(|l| l.contains("Request timed out."))
            .count()
            >= 1,
        "{t}"
    );
}
