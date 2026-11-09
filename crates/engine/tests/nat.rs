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
fn the_nat_example_lab_works_as_its_notes_say() {
    let mut lab = load_and_build(
        include_str!("../../../labs/examples/nat-pat.lab.toml"),
        Trace::new(),
    )
    .unwrap();
    let dev = |lab: &BuiltLab, name: &str| lab.devices.iter().position(|d| d.name == name).unwrap();
    let (pc1, pc2, r1) = (dev(&lab, "pc1"), dev(&lab, "pc2"), dev(&lab, "r1"));
    // Let the switch's ports come up.
    let t = lab.world.kernel.now() + SimTime::from_secs(40);
    lab.world.run_until(t);
    for pc in [pc1, pc2] {
        let out = run(&mut lab, pc, &["ping 198.51.100.10"]);
        assert!(out.contains("received 4"), "{out}");
    }
    let out = run(&mut lab, r1, &["enable", "show ip nat translations"]);
    assert_eq!(out.matches("203.0.113.1:").count(), 2, "{out}");
}

#[test]
fn the_router_answers_arp_for_global_addresses_so_the_neighbour_needs_no_route() {
    let mut lab = lab();
    // Widen the outside link so a public address lives on it, and take the static route away.
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "interface g0/1",
            "ip address 203.0.113.1 255.255.255.248",
            "exit",
            "ip nat inside source static 10.0.1.10 203.0.113.5",
            "end",
        ],
    );
    run(
        &mut lab,
        R2,
        &[
            "enable",
            "configure terminal",
            "no ip route 192.0.2.0 255.255.255.248 203.0.113.1",
            "interface g0/0",
            "ip address 203.0.113.2 255.255.255.248",
            "end",
        ],
    );
    let out = run(&mut lab, R2, &["enable", "ping 203.0.113.5"]);
    assert!(out.contains("Success rate is 100 percent"), "{out}");
    // An address nothing translates is not answered for.
    let out = run(&mut lab, R2, &["enable", "ping 203.0.113.6"]);
    assert!(out.contains("Success rate is 0 percent"), "{out}");
}

#[test]
fn translation_timeouts_can_be_set_and_are_saved() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat translation icmp-timeout 5",
            "ip nat translation tcp-timeout 600",
            "end",
        ],
    );
    run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(translations(&mut lab).contains("icmp"));
    let later = lab.world.kernel.now() + SimTime::from_secs(6);
    lab.world.run_until(later);
    assert!(
        !translations(&mut lab).contains("icmp"),
        "gone after 5 s, not 60"
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(cfg.contains("ip nat translation icmp-timeout 5\n"), "{cfg}");
    assert!(
        cfg.contains("ip nat translation tcp-timeout 600\n"),
        "{cfg}"
    );
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "no ip nat translation icmp-timeout",
            "end",
        ],
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(
        !cfg.contains("icmp-timeout"),
        "back to the default, so not printed:\n{cfg}"
    );
    assert!(cfg.contains("tcp-timeout 600"), "{cfg}");
}

#[test]
fn an_inside_hosts_icmp_error_leaves_from_the_global_address() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat inside source static udp 10.0.1.10 5000 203.0.113.1 6000",
            "end",
        ],
    );
    // The server sends a datagram to the forwarded port; nothing listens on pc1, which answers
    // with port unreachable. The server must see it coming from 203.0.113.1, not 10.0.1.10.
    let srv = lab.devices[SRV].id;
    lab.world.with_device::<Host, _>(srv, |h, c| {
        h.udp_send(c, Ipv4Addr::new(203, 0, 113, 1), 4000, 6000, b"hello")
    });
    lab.world.run_to_idle();
    let h = lab.world.device::<Host>(srv);
    assert_eq!(h.icmp_errors.len(), 1);
    assert_eq!(h.probes.len(), 1);
    assert_eq!(h.probes[0].from, Ipv4Addr::new(203, 0, 113, 1));
}

#[test]
fn outside_source_lets_the_inside_reach_an_outside_host_by_another_address() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip route 10.99.0.10 255.255.255.255 203.0.113.2",
            "ip nat outside source static 198.51.100.10 10.99.0.10",
            "end",
        ],
    );
    // pc1 pings the outside local address; the server answers and pc1 sees the same address.
    let out = run(&mut lab, PC1, &["ping 10.99.0.10"]);
    assert!(out.contains("received 4"), "{out}");
    assert!(out.contains("Reply from 10.99.0.10"), "{out}");
    let t = translations(&mut lab);
    assert!(
        t.contains("10.99.0.10") && t.contains("198.51.100.10"),
        "{t}"
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(
        cfg.contains("ip nat outside source static 198.51.100.10 10.99.0.10"),
        "{cfg}"
    );
}

#[test]
fn static_entries_sharing_an_address_need_extendable_and_the_port_forward_wins_for_its_port() {
    let mut lab = lab();
    let mut r1 = sh(&lab, R1);
    for l in ["enable", "configure terminal"] {
        r1.exec(&mut lab.world, l);
    }
    let mut say = |lab: &mut BuiltLab, l: &str| r1.exec(&mut lab.world, l).unwrap();
    say(&mut lab, "ip nat inside source static 10.0.1.10 192.0.2.6");
    let refused = say(
        &mut lab,
        "ip nat inside source static tcp 10.0.1.11 80 192.0.2.6 8080",
    );
    assert!(refused.contains("extendable"), "{refused}");
    // The same thing twice is not a conflict.
    assert_eq!(
        say(&mut lab, "ip nat inside source static 10.0.1.10 192.0.2.6"),
        ""
    );
    assert_eq!(
        say(
            &mut lab,
            "ip nat inside source static tcp 10.0.1.11 80 192.0.2.6 8080 extendable"
        ),
        ""
    );
    // Port 8080 of the shared address goes to pc2, everything else to pc1.
    let (pc2, srv) = (lab.devices[PC2].id, lab.devices[SRV].id);
    lab.world
        .with_device::<Host, _>(pc2, |h, _| h.tcp.set_listeners(&[80]));
    let conn = lab
        .world
        .with_device::<Host, _>(srv, |h, c| h.tcp_open(c, Ipv4Addr::new(192, 0, 2, 6), 8080))
        .unwrap();
    lab.world.run_to_idle();
    assert_eq!(
        lab.world.device::<Host>(srv).tcp.state_of(conn),
        Some(TcpState::Established)
    );
    let out = run(&mut lab, R2, &["enable", "ping 192.0.2.6"]);
    assert!(out.contains("Success rate is 100 percent"), "{out}");
    let t = translations(&mut lab);
    assert!(
        t.contains("192.0.2.6:8080") && t.contains("10.0.1.11:80"),
        "{t}"
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(
        cfg.contains("static tcp 10.0.1.11 80 192.0.2.6 8080 extendable"),
        "{cfg}"
    );
}

#[test]
fn a_syn_nobody_answers_is_forgotten_after_the_syn_timeout() {
    let tcp_entries_after = |syn_timeout: Option<&str>| {
        let mut lab = lab();
        if let Some(t) = syn_timeout {
            run(
                &mut lab,
                R1,
                &[
                    "enable",
                    "configure terminal",
                    &format!("ip nat translation syn-timeout {t}"),
                    "end",
                ],
            );
        }
        let pc = lab.devices[PC1].id;
        // Nothing answers at this address: the SYN is sent and sent again.
        lab.world.with_device::<Host, _>(pc, |h, c| {
            h.tcp_open(c, Ipv4Addr::new(198, 51, 100, 99), 23)
        });
        lab.world.run_until(SimTime::from_secs(2));
        assert!(translations(&mut lab).contains("tcp"), "made by the SYN");
        let later = lab.world.kernel.now() + SimTime::from_secs(58);
        lab.world.run_until(later);
        translations(&mut lab).matches("tcp").count()
    };
    // By default a minute after the last SYN: still there at 60 s. With 3 s: long gone.
    assert_eq!(tcp_entries_after(None), 1);
    assert_eq!(tcp_entries_after(Some("3")), 0);
}

#[test]
fn max_entries_stops_new_translations_but_not_existing_ones() {
    let mut lab = lab();
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat translation max-entries 1",
            "end",
        ],
    );
    assert!(run(&mut lab, PC1, &["ping 198.51.100.10"]).contains("received 4"));
    assert!(run(&mut lab, PC2, &["ping 198.51.100.10"]).contains("received 0"));
    // pc1's entry still works.
    assert!(run(&mut lab, PC1, &["ping 198.51.100.10"]).contains("received 4"));
    let stats = run(&mut lab, R1, &["show ip nat statistics"]);
    assert!(stats.contains("Maximum translations: 1"), "{stats}");
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(cfg.contains("ip nat translation max-entries 1"), "{cfg}");
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "no ip nat translation max-entries",
            "end",
        ],
    );
    assert!(run(&mut lab, PC2, &["ping 198.51.100.10"]).contains("received 4"));
}

#[test]
fn inside_destination_nat_spreads_connections_over_a_rotary_pool() {
    let mut lab = lab();
    // A public address on the outside link that is really two inside web servers.
    run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "interface g0/1",
            "ip address 203.0.113.1 255.255.255.248",
            "exit",
            "ip nat pool WEB 10.0.1.10 10.0.1.11 prefix-length 24",
            "ip nat pool WEB 10.0.1.10 10.0.1.11 prefix-length 24 type rotary",
            "access-list 5 permit 203.0.113.5",
            "ip nat inside destination list 5 pool WEB",
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
            "ip address 203.0.113.2 255.255.255.248",
            "end",
        ],
    );
    let (pc1, pc2, srv) = (
        lab.devices[PC1].id,
        lab.devices[PC2].id,
        lab.devices[SRV].id,
    );
    for pc in [pc1, pc2] {
        lab.world
            .with_device::<Host, _>(pc, |h, _| h.tcp.set_listeners(&[80]));
    }
    let mut conns = vec![];
    for _ in 0..4 {
        let c = lab
            .world
            .with_device::<Host, _>(srv, |h, c| h.tcp_open(c, Ipv4Addr::new(203, 0, 113, 5), 80))
            .unwrap();
        lab.world.run_to_idle();
        conns.push(c);
    }
    for c in &conns {
        assert_eq!(
            lab.world.device::<Host>(srv).tcp.state_of(*c),
            Some(TcpState::Established)
        );
    }
    let accepted = |lab: &mut BuiltLab, pc| {
        let mut n = 0;
        while lab
            .world
            .with_device::<Host, _>(pc, |h, _| h.tcp.accept())
            .is_some()
        {
            n += 1;
        }
        n
    };
    assert_eq!((accepted(&mut lab, pc1), accepted(&mut lab, pc2)), (2, 2));
    // Data goes both ways on one of them: the reply leaves from the virtual address.
    let first = conns[0];
    lab.world
        .with_device::<Host, _>(srv, |h, c| h.tcp_write(c, first, b"GET /"));
    lab.world.run_to_idle();
    let t = translations(&mut lab);
    assert_eq!(t.matches("203.0.113.5:80").count(), 4, "{t}");
    let stats = run(&mut lab, R1, &["show ip nat statistics"]);
    assert!(
        stats.contains("-- Inside Destination") && stats.contains("type rotary"),
        "{stats}"
    );
    let cfg = running_config(lab.world.device::<Router>(lab.devices[R1].id));
    assert!(
        cfg.contains("ip nat pool WEB 10.0.1.10 10.0.1.11 netmask 255.255.255.0 type rotary"),
        "{cfg}"
    );
    assert!(
        cfg.contains("ip nat inside destination list 5 pool WEB"),
        "{cfg}"
    );
    // A pool that is not rotary cannot be used.
    let say = run(
        &mut lab,
        R1,
        &[
            "enable",
            "configure terminal",
            "ip nat pool PLAIN 192.0.2.1 192.0.2.2 netmask 255.255.255.248",
            "ip nat inside destination list 5 pool PLAIN",
        ],
    );
    assert!(say.contains("not of type rotary"), "{say}");
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
            "ip nat inside source static tcp 10.0.1.11 22 192.0.2.5 2222 extendable",
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
        "ip nat inside source static tcp 10.0.1.11 22 192.0.2.5 2222 extendable",
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
