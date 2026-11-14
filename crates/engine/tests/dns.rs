//! DNS, end to end: names for ping, a PC's name server, a router as name server.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::Trace;

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_syslog.lab.toml");
const PC1: usize = 0;
const R1: usize = 1;
const SW1: usize = 2;
const R2: usize = 3;

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let id = lab.devices[dev].id;
    let mut sh = match dev {
        PC1 => Shell::host(id),
        SW1 => Shell::switch(id),
        _ => Shell::router(id),
    };
    let mut out = String::new();
    let enable = if dev == PC1 { None } else { Some("enable") };
    for l in enable.iter().chain(lines) {
        out = sh.exec(&mut lab.world, l).unwrap();
    }
    out
}

fn lab() -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    for (d, ip) in [(R1, "10.0.0.1"), (R2, "10.0.0.2")] {
        console(
            &mut lab,
            d,
            &[
                "configure terminal",
                "interface g0/0",
                &format!("ip address {ip} 255.255.255.0"),
                "no shutdown",
                "end",
            ],
        );
    }
    console(
        &mut lab,
        PC1,
        &[
            "dnsd start",
            "dnsd add web.example.com 10.0.0.2",
            "dnsd add PC.example.com 10.0.0.10",
        ],
    );
    lab
}

#[test]
fn a_router_resolves_a_name_through_its_name_server_and_the_default_domain() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "ip name-server 10.0.0.10",
            "ip domain-name example.com",
            "end",
        ],
    );
    let out = console(&mut lab, R1, &["ping web"]);
    assert!(
        out.starts_with("Translating \"web\"...domain server (10.0.0.10) [OK]\n"),
        "{out}"
    );
    assert!(out.contains("!!!") || out.contains("Success"), "{out}");
    // A name with a dot is looked up as typed.
    let out = console(&mut lab, R1, &["ping web.example.com"]);
    assert!(out.starts_with("Translating"), "{out}");
    let out = console(&mut lab, R1, &["ping nosuch"]);
    assert_eq!(out, "Translating \"nosuch\"...domain server (10.0.0.10)\n% Unrecognized host or address, or protocol not running.");
    // web.example.com twice, then nosuch.example.com and the bare name nosuch.
    let list = console(&mut lab, PC1, &["dnsd list"]);
    assert!(list.contains("4 queries answered"), "{list}");
}

#[test]
fn static_hosts_win_and_lookup_can_be_switched_off() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "ip name-server 10.0.0.10",
            "ip host Lab 10.0.0.2",
            "end",
        ],
    );
    let out = console(&mut lab, R1, &["ping lab"]);
    assert!(!out.contains("Translating"), "{out}");
    assert!(out.contains("!!!") || out.contains("Success"), "{out}");
    assert_eq!(
        console(&mut lab, PC1, &["dnsd list"])
            .matches("0 queries")
            .count(),
        1
    );
    console(
        &mut lab,
        R1,
        &["configure terminal", "no ip domain-lookup", "end"],
    );
    assert_eq!(
        console(&mut lab, R1, &["ping web.example.com"]),
        "% Unrecognized host or address, or protocol not running."
    );
    let hosts = console(&mut lab, R1, &["show hosts"]);
    assert!(
        hosts.contains("lookup is disabled") && hosts.contains("lab") && hosts.contains("10.0.0.2"),
        "{hosts}"
    );
    let run = console(&mut lab, R1, &["show running-config"]);
    assert!(
        run.contains("no ip domain-lookup\nip name-server 10.0.0.10\nip host lab 10.0.0.2\n"),
        "{run}"
    );
}

#[test]
fn a_silent_name_server_ends_in_unrecognized_after_the_waits() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &["configure terminal", "ip name-server 10.0.0.77", "end"],
    );
    let t0 = lab.world.kernel.now();
    let out = console(&mut lab, R1, &["ping web.example.com"]);
    assert!(
        out.ends_with("% Unrecognized host or address, or protocol not running."),
        "{out}"
    );
    assert!((lab.world.kernel.now().0 - t0.0) / 1_000_000_000 >= 4);
    // A second server that does answer is tried next.
    console(
        &mut lab,
        R1,
        &["configure terminal", "ip name-server 10.0.0.10", "end"],
    );
    let out = console(&mut lab, R1, &["ping web.example.com"]);
    assert!(out.contains("domain server (10.0.0.10) [OK]"), "{out}");
}

#[test]
fn a_router_answers_dns_from_its_host_table_for_a_pc() {
    let mut lab = lab();
    console(
        &mut lab,
        R2,
        &[
            "configure terminal",
            "ip host printer.lab 10.0.0.1",
            "ip dns server",
            "end",
        ],
    );
    let out = console(&mut lab, PC1, &["nslookup printer.lab 10.0.0.2"]);
    assert!(
        out.contains("Name:    printer.lab") && out.contains("Address:  10.0.0.1"),
        "{out}"
    );
    let out = console(&mut lab, PC1, &["nslookup nothere.lab 10.0.0.2"]);
    assert!(
        out.contains("can't find nothere.lab: Non-existent domain"),
        "{out}"
    );
    let out = console(&mut lab, PC1, &["nslookup printer.lab"]);
    assert_eq!(out, "*** Default servers are not available");
    console(
        &mut lab,
        PC1,
        &["netsh interface ip set dns eth0 static 10.0.0.2"],
    );
    let out = console(&mut lab, PC1, &["ping printer.lab"]);
    assert!(out.starts_with("Pinging printer.lab [10.0.0.1]:"), "{out}");
    assert!(out.contains("Reply from 10.0.0.1"), "{out}");
    let out = console(&mut lab, PC1, &["ping nothere.lab"]);
    assert_eq!(
        out,
        "Ping request could not find host nothere.lab. Please check the name and try again."
    );
    let out = console(&mut lab, PC1, &["tracert printer.lab"]);
    assert!(
        out.starts_with("Tracing route to printer.lab [10.0.0.1]"),
        "{out}"
    );
    // Without `ip dns server` the router says nothing and the PC times out.
    console(
        &mut lab,
        R2,
        &["configure terminal", "no ip dns server", "end"],
    );
    let out = console(&mut lab, PC1, &["nslookup printer.lab 10.0.0.2"]);
    assert!(out.starts_with("DNS request timed out."), "{out}");
}

#[test]
fn settings_are_checked_and_survive_a_reload_and_a_switch_can_resolve_too() {
    let mut lab = lab();
    let out = console(
        &mut lab,
        R1,
        &["configure terminal", "ip host bad_name! 10.0.0.1"],
    );
    assert!(out.contains("host name is"), "{out}");
    let out = console(&mut lab, R1, &["configure terminal", "no ip host nothing"]);
    assert!(out.contains("not in the table"), "{out}");
    let out = console(
        &mut lab,
        R1,
        &["configure terminal", "no ip name-server 10.9.9.9"],
    );
    assert!(out.contains("not configured"), "{out}");
    let mut cmds = vec!["configure terminal"];
    let servers: Vec<String> = (1..=7)
        .map(|n| format!("ip name-server 10.0.0.{n}"))
        .collect();
    cmds.extend(servers.iter().map(String::as_str));
    let out = console(&mut lab, R1, &cmds);
    assert!(out.contains("At most six"), "{out}");
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "ip host lab 10.0.0.2",
            "ip dns server",
            "end",
            "copy running-config startup-config",
        ],
    );
    let before = console(&mut lab, R1, &["show running-config"]);
    let mut sh = Shell::router(lab.devices[R1].id);
    sh.exec(&mut lab.world, "enable").unwrap();
    sh.exec(&mut lab.world, "reload").unwrap();
    sh.exec(&mut lab.world, "").unwrap();
    assert_eq!(before, console(&mut lab, R1, &["show running-config"]));
    // The switch through its VLAN interface.
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "exit",
            "ip name-server 10.0.0.10",
            "end",
        ],
    );
    let out = console(&mut lab, SW1, &["ping web.example.com"]);
    assert!(out.contains("domain server (10.0.0.10) [OK]"), "{out}");
}
