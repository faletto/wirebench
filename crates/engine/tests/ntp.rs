//! NTP and the device clock, end to end through routers and a switch.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::{SimTime, Trace};
use netdev::{L2Switch, Router};

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_ntp.lab.toml");
const R1: usize = 0;
const SW1: usize = 1;
const R2: usize = 2;

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let mut sh = if dev == SW1 {
        Shell::switch(lab.devices[dev].id)
    } else {
        Shell::router(lab.devices[dev].id)
    };
    let mut out = String::new();
    for l in std::iter::once(&"enable").chain(lines) {
        out = sh.exec(&mut lab.world, l).unwrap();
    }
    out
}

fn wait(lab: &mut BuiltLab, secs: u64) {
    let to = lab.world.kernel.now() + SimTime::from_secs(secs);
    lab.world.run_until(to);
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
    lab
}

fn clock_of(lab: &BuiltLab, dev: usize) -> i64 {
    let now = lab.world.kernel.now();
    let r = if dev == SW1 {
        lab.world.device::<L2Switch>(lab.devices[dev].id).l3()
    } else {
        lab.world.device::<Router>(lab.devices[dev].id)
    };
    r.clock.now_ns(now)
}

#[test]
fn clocks_agree_after_sync_and_not_before() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "clock set 12:00:00 8 Nov 2026",
            "configure terminal",
            "ntp master 3",
            "end",
        ],
    );
    console(
        &mut lab,
        R2,
        &["configure terminal", "ntp server 10.0.0.1", "end"],
    );
    wait(&mut lab, 100);
    let gap = (clock_of(&lab, R1) - clock_of(&lab, R2)).abs();
    assert!(
        gap > 1_000_000_000_000,
        "unsynchronised: years apart, got {gap} ns"
    );
    wait(&mut lab, 60);
    let gap = (clock_of(&lab, R1) - clock_of(&lab, R2)).abs();
    assert!(
        gap < 5_000_000,
        "within a few ms once synchronised, got {gap} ns"
    );
    let out = console(&mut lab, R2, &["show ntp status"]);
    assert!(
        out.starts_with("Clock is synchronized, stratum 4, reference is 10.0.0.1"),
        "{out}"
    );
}

#[test]
fn a_switch_follows_through_its_vlan_interface_and_the_chain_adds_a_stratum() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "clock set 12:00:00 8 Nov 2026",
            "configure terminal",
            "ntp master 2",
            "end",
        ],
    );
    console(
        &mut lab,
        R2,
        &["configure terminal", "ntp server 10.0.0.1", "end"],
    );
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "exit",
            "ntp server 10.0.0.2",
            "end",
        ],
    );
    wait(&mut lab, 400);
    let out = console(&mut lab, SW1, &["show ntp status"]);
    assert!(out.contains("stratum 4, reference is 10.0.0.2"), "{out}");
    assert!((clock_of(&lab, R1) - clock_of(&lab, SW1)).abs() < 10_000_000);
}

#[test]
fn an_unsynchronised_server_is_not_a_source() {
    let mut lab = lab();
    // r1 has a different time but is not a master: r2 must not believe it.
    console(&mut lab, R1, &["clock set 12:00:00 8 Nov 2026"]);
    console(
        &mut lab,
        R2,
        &["configure terminal", "ntp server 10.0.0.1", "end"],
    );
    wait(&mut lab, 400);
    let out = console(&mut lab, R2, &["show ntp status"]);
    assert!(out.starts_with("Clock is unsynchronized"), "{out}");
    assert!(clock_of(&lab, R2) < clock_of(&lab, R1) - 1_000_000_000_000);
}

#[test]
fn the_lower_stratum_server_wins() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "clock set 12:00:00 8 Nov 2026",
            "configure terminal",
            "ntp master 2",
            "end",
        ],
    );
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "exit",
            "ntp master 9",
            "end",
        ],
    );
    console(
        &mut lab,
        R2,
        &[
            "configure terminal",
            "ntp server 10.0.0.3",
            "ntp server 10.0.0.1",
            "end",
        ],
    );
    wait(&mut lab, 300);
    let out = console(&mut lab, R2, &["show ntp status"]);
    assert!(out.contains("stratum 3, reference is 10.0.0.1"), "{out}");
    let out = console(&mut lab, R2, &["show ntp associations"]);
    assert!(out.lines().any(|l| l.starts_with("*~10.0.0.1")), "{out}");
    assert!(out.lines().any(|l| l.starts_with(" ~10.0.0.3")), "{out}");
}

#[test]
fn the_clock_survives_a_reload_but_ntp_settings_come_back_from_the_saved_config() {
    let mut lab = lab();
    console(
        &mut lab,
        R2,
        &[
            "clock set 08:15:00 3 Feb 2026",
            "configure terminal",
            "clock timezone CET 1",
            "ntp server 10.0.0.1",
            "end",
            "copy running-config startup-config",
        ],
    );
    let before = console(&mut lab, R2, &["show running-config"]);
    assert!(
        before.contains("clock timezone CET 1") && before.contains("ntp server 10.0.0.1"),
        "{before}"
    );
    let mut sh = Shell::router(lab.devices[R2].id);
    sh.exec(&mut lab.world, "enable").unwrap();
    sh.exec(&mut lab.world, "reload").unwrap();
    sh.exec(&mut lab.world, "").unwrap();
    let after = console(&mut lab, R2, &["show running-config"]);
    assert_eq!(before, after);
    let out = console(&mut lab, R2, &["show clock"]);
    assert!(out.contains("CET Tue Feb 3 2026"), "{out}");
}

#[test]
fn clock_set_checks_its_arguments() {
    let mut lab = lab();
    for (cmd, want) in [
        ("clock set 25:00:00 1 Jan 2026", "hh:mm:ss"),
        ("clock set 10:61:00 1 Jan 2026", "hh:mm:ss"),
        ("clock set 10:00 1 Jan 2026", "hh:mm:ss"),
        ("clock set 10:00:00 1 Foo 2026", "month"),
        ("clock set 10:00:00 30 Feb 2026", "no such day"),
        ("clock set 10:00:00 29 Feb 2028", ""),
    ] {
        let out = console(&mut lab, R1, &[cmd]);
        assert!(out.contains(want), "{cmd}: {out}");
        assert_eq!(out.is_empty(), want.is_empty(), "{cmd}: {out}");
    }
    let out = console(&mut lab, R1, &["show clock"]);
    assert!(out.contains("Tue Feb 29 2028"), "{out}");
}

#[test]
fn a_zone_shifts_what_is_shown_and_what_is_typed() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "clock timezone EST -5",
            "end",
            "clock set 07:00:00 8 Nov 2026",
        ],
    );
    let out = console(&mut lab, R1, &["show clock"]);
    assert!(
        out.contains("07:00:00") && out.contains("EST Sun Nov 8 2026"),
        "{out}"
    );
    // Underneath it is 12:00 UTC.
    let utc = netdev::unix_from_civil(2026, 11, 8, 12, 0, 0) * 1_000_000_000;
    let now = clock_of(&lab, R1);
    assert!((now - utc).abs() < 10_000_000_000, "{now} vs {utc}");
}
