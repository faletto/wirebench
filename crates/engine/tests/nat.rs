//! NAT and PAT end to end: inside PCs behind a translating router, an "internet" router beyond.
//!
//! pc1, pc2 - sw1 - r1:g0/0 (inside)   r1:g0/1 (outside) - r2:g0/0   r2:g0/1 - srv

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{running_config, Shell};
use kernel::{SimTime, Trace};
use netdev::{Host, Router, TcpState};
use wire::Ipv4Addr;

const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "pc1"
template = "host-pc"
pos = [0, 0]
[device.host]
ipv4 = "10.0.1.10/24"
gateway = "10.0.1.1"

[[device]]
name = "pc2"
template = "host-pc"
pos = [0, 100]
[device.host]
ipv4 = "10.0.1.11/24"
gateway = "10.0.1.1"

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [100, 50]

[[device]]
name = "r1"
template = "router-2g"
pos = [200, 50]

[[device]]
name = "r2"
template = "router-2g"
pos = [300, 50]

[[device]]
name = "srv"
template = "host-pc"
pos = [400, 50]
[device.host]
ipv4 = "198.51.100.10/24"
gateway = "198.51.100.1"

[[link]]
a = "pc1:eth0"
b = "sw1:Fa0/1"
[[link]]
a = "pc2:eth0"
b = "sw1:Fa0/2"
[[link]]
a = "sw1:Fa0/3"
b = "r1:Gi0/0"
[[link]]
a = "r1:Gi0/1"
b = "r2:Gi0/0"
[[link]]
a = "r2:Gi0/1"
b = "srv:eth0"
"#;

const PC1: usize = 0;
const PC2: usize = 1;
const R1: usize = 3;
const R2: usize = 4;
const SRV: usize = 5;

fn sh(lab: &BuiltLab, dev: usize) -> Shell {
    let id = lab.devices[dev].id;
    match dev {
        PC1 | PC2 | SRV => Shell::host(id),
        _ => Shell::router(id),
    }
}

fn run(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let mut s = sh(lab, dev);
    let mut out = String::new();
    for l in lines {
        out = s.exec(&mut lab.world, l).unwrap_or_default();
    }
    out
}

/// Everything addressed; r1 translates with PAT through its outside address.
fn lab() -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.1.1 255.255.255.0",
            "ip nat inside",
            "no shutdown",
            "exit",
            "interface g0/1",
            "ip address 203.0.113.1 255.255.255.252",
            "ip nat outside",
            "no shutdown",
            "exit",
            "ip route 0.0.0.0 0.0.0.0 203.0.113.2",
            "access-list 1 permit 10.0.1.0 0.0.0.255",
            "ip nat inside source list 1 interface g0/1 overload",
            "end",
        ],
    );
    run(
        &mut lab,
        R2,
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 203.0.113.2 255.255.255.252",
            "no shutdown",
            "exit",
            "interface g0/1",
            "ip address 198.51.100.1 255.255.255.0",
            "no shutdown",
            "exit",
            "ip route 192.0.2.0 255.255.255.248 203.0.113.1",
            "line vty 0 4",
            "no login",
            "end",
        ],
    );
    lab
}

fn translations(lab: &mut BuiltLab) -> String {
    run(lab, R1, &["show ip nat translations"])
}

#[test]
fn without_nat_private_addresses_get_no_answer_from_the_internet() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "no ip nat inside source list 1 interface g0/1 overload",
            "end",
        ],
    );
    let out = run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(out.contains("received 0"), "{out}");
}

#[test]
fn pat_lets_every_inside_host_reach_the_server_through_one_address() {
    let mut lab = lab();
    for pc in [PC1, PC2] {
        let out = run(&mut lab, pc, &["ping 198.51.100.10"]);
        assert!(out.contains("received 4"), "pc{pc}: {out}");
    }
    let t = translations(&mut lab);
    assert_eq!(t.matches("icmp").count(), 2, "{t}");
    // Both are 203.0.113.1, with different identifiers; the inside locals differ.
    assert!(t.contains("10.0.1.10") && t.contains("10.0.1.11"), "{t}");
    let globals: Vec<&str> = t
        .lines()
        .filter(|l| l.starts_with("icmp"))
        .map(|l| l.split_whitespace().nth(1).unwrap())
        .collect();
    assert!(globals.iter().all(|g| g.starts_with("203.0.113.1:")), "{t}");
    assert_ne!(globals[0], globals[1], "{t}");
}

#[test]
fn the_server_sees_the_translated_address_on_a_tcp_session() {
    let mut lab = lab();
    // pc1 telnets to the internet router; the router sees r1's outside address.
    let mut pc = sh(&lab, PC1);
    let said = pc.exec(&mut lab.world, "telnet 203.0.113.2");
    assert_eq!(
        pc.prompt(&lab.world),
        "r2>",
        "session opened through NAT: {said:?}"
    );
    let at_r2 = run(&mut lab, R2, &["show tcp brief"]);
    assert!(
        at_r2.contains("203.0.113.2.23") && at_r2.contains("203.0.113.1."),
        "{at_r2}"
    );
    assert!(!at_r2.contains("10.0.1.10"), "{at_r2}");
    let t = translations(&mut lab);
    assert!(t.contains("tcp") && t.contains("10.0.1.10"), "{t}");
    // A normal exit sends FINs: the entry's timer drops to a minute and it is then gone.
    pc.exec(&mut lab.world, "exit");
    let later = lab.world.kernel.now() + SimTime::from_secs(90);
    lab.world.run_until(later);
    let t = translations(&mut lab);
    assert!(!t.contains("tcp"), "{t}");
}

#[test]
fn a_static_one_to_one_translation_works_in_both_directions() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat inside source static 10.0.1.10 192.0.2.5",
            "end",
        ],
    );
    // The internet can reach pc1 at its global address.
    let out = run(&mut lab, R2, &["enable", "ping 192.0.2.5"]);
    assert!(out.contains("Success rate is 100 percent"), "{out}");
    let t = translations(&mut lab);
    assert!(t.contains("192.0.2.5") && t.contains("10.0.1.10"), "{t}");
    // pc2 is not covered by the static entry and still uses PAT.
    let out = run(&mut lab, PC2, &["ping 198.51.100.10"]);
    assert!(out.contains("received 4"), "{out}");
}

#[test]
fn a_port_forward_reaches_a_server_inside() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat inside source static tcp 10.0.1.10 80 203.0.113.1 8080",
            "end",
        ],
    );
    // The internet side needs a route to the outside address's network: it is connected.
    let (pc, srv) = (lab.devices[PC1].id, lab.devices[SRV].id);
    lab.world
        .with_device::<Host, _>(pc, |h, _| h.tcp.set_listeners(&[80]));
    // srv reaches 203.0.113.1 through r2.
    run(&mut lab, R2, &["enable", "configure terminal", "end"]);
    let conn = lab
        .world
        .with_device::<Host, _>(srv, |h, c| {
            h.tcp_open(c, Ipv4Addr::new(203, 0, 113, 1), 8080)
        })
        .unwrap();
    lab.world.run_to_idle();
    assert_eq!(
        lab.world.device::<Host>(srv).tcp.state_of(conn),
        Some(TcpState::Established)
    );
    let accepted = lab
        .world
        .with_device::<Host, _>(pc, |h, _| h.tcp.accept())
        .unwrap();
    lab.world
        .with_device::<Host, _>(srv, |h, c| h.tcp_write(c, conn, b"GET /"));
    lab.world.run_to_idle();
    let got = lab
        .world
        .with_device::<Host, _>(pc, |h, c| h.tcp_take(c, accepted));
    assert_eq!(got, b"GET /");
    // Another port on the same address is the router's own: nothing listens there.
    let other = lab
        .world
        .with_device::<Host, _>(srv, |h, c| {
            h.tcp_open(c, Ipv4Addr::new(203, 0, 113, 1), 8081)
        })
        .unwrap();
    lab.world.run_to_idle();
    assert_eq!(
        lab.world
            .device::<Host>(srv)
            .tcp
            .conn(other)
            .unwrap()
            .failure,
        Some(netdev::TcpFailure::Refused)
    );
}

#[test]
fn a_pool_without_overload_gives_one_address_per_host_and_runs_out() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "no ip nat inside source list 1 interface g0/1 overload",
            "ip nat pool P 192.0.2.1 192.0.2.1 netmask 255.255.255.248",
            "ip nat inside source list 1 pool P",
            "end",
        ],
    );
    let out = run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(out.contains("received 4"), "{out}");
    // The one address is taken: pc2's packets are dropped.
    let out = run(&mut lab, PC2, &["ping 198.51.100.10"]);
    assert!(out.contains("received 0"), "{out}");
    let stats = run(&mut lab, R1, &["show ip nat statistics"]);
    assert!(
        stats.contains("pool P") && stats.contains("allocated 1 (100%)"),
        "{stats}"
    );
    assert!(
        stats.contains("Total active translations: 1 (0 static, 1 dynamic; 0 extended)"),
        "{stats}"
    );
}

#[test]
fn traceroute_through_pat_shows_the_hops_beyond() {
    let mut lab = lab();
    let out = run(&mut lab, PC1, &["tracert 198.51.100.10"]);
    // Hop 1 is the NAT router's inside address, hop 2 the internet router, then the server.
    assert!(out.contains("10.0.1.1"), "{out}");
    assert!(out.contains("203.0.113.2"), "{out}");
    assert!(out.contains("198.51.100.10"), "{out}");
}

#[test]
fn entries_time_out_and_clear_removes_them() {
    let mut lab = lab();
    run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(translations(&mut lab).contains("icmp"));
    let later = lab.world.kernel.now() + SimTime::from_secs(61);
    lab.world.run_until(later);
    assert!(!translations(&mut lab).contains("icmp"));
    run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(translations(&mut lab).contains("icmp"));
    run(&mut lab, R1, &["enable", "clear ip nat translation *"]);
    assert!(!translations(&mut lab).contains("icmp"));
    let stats = run(&mut lab, R1, &["show ip nat statistics"]);
    assert!(stats.contains("Expired translations: 1"), "{stats}");
    assert!(
        stats.contains("Outside interfaces:\nGigabitEthernet0/1"),
        "{stats}"
    );
    assert!(
        stats.contains("Inside interfaces:\nGigabitEthernet0/0"),
        "{stats}"
    );
}

#[test]
fn nat_configuration_is_printed_saved_and_comes_back_after_a_reload() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat pool P 192.0.2.2 192.0.2.4 prefix-length 29",
            "ip nat inside source static 10.0.1.10 192.0.2.5",
            "ip nat inside source static tcp 10.0.1.11 22 192.0.2.5 2222",
            "end",
        ],
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    for line in [
        " ip nat inside",
        " ip nat outside",
        "ip nat pool P 192.0.2.2 192.0.2.4 netmask 255.255.255.248",
        "ip nat inside source list 1 interface GigabitEthernet0/1 overload",
        "ip nat inside source static 10.0.1.10 192.0.2.5",
        "ip nat inside source static tcp 10.0.1.11 22 192.0.2.5 2222",
    ] {
        assert!(cfg.contains(line), "missing {line:?} in\n{cfg}");
    }
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "copy running-config startup-config",
            "reload",
            "no",
            "yes",
        ],
    );
    let after = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert_eq!(after, cfg, "the saved config replays to the same config");
}

#[test]
fn bad_nat_commands_are_refused_with_a_reason() {
    let mut lab = lab();
    let mut r1 = sh(&lab, R1);
    for l in ["enable", "configure terminal"] {
        r1.exec(&mut lab.world, l);
    }
    let mut say = |l: &str| r1.exec(&mut lab.world, l).unwrap();
    assert!(say("ip nat pool X 10.0.0.9 10.0.0.1 netmask 255.255.255.0").contains("below"));
    assert!(say("ip nat pool X 10.0.0.1 10.0.1.1 netmask 255.255.255.0").contains("one network"));
    assert!(say("ip nat inside source list 1 pool NOPE").contains("No such NAT pool"));
    assert!(
        say("ip nat inside source list 1 interface g9/9 overload").contains("No such interface")
    );
    say("ip nat pool Y 192.0.2.1 192.0.2.2 netmask 255.255.255.248");
    say("ip nat inside source list 1 pool Y");
    assert!(say("no ip nat pool Y").contains("used by a translation rule"));
    assert!(say("no ip nat inside source static 1.1.1.1 2.2.2.2").contains("not configured"));
}
