//! Syslog, end to end: devices log, a PC collects.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::{SimTime, Trace};

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_syslog.lab.toml");
const PC1: usize = 0;
const R1: usize = 1;
const SW1: usize = 2;
const R2: usize = 3;

fn shell(lab: &BuiltLab, dev: usize) -> Shell {
    let id = lab.devices[dev].id;
    match dev {
        PC1 => Shell::host(id),
        SW1 => Shell::switch(id),
        _ => Shell::router(id),
    }
}

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let mut sh = shell(lab, dev);
    let mut out = String::new();
    let enable = if dev == PC1 { None } else { Some("enable") };
    for l in enable.iter().chain(lines) {
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
    console(&mut lab, PC1, &["syslogd start"]);
    lab
}

fn collected(lab: &mut BuiltLab) -> String {
    console(lab, PC1, &["syslogd show"])
}

#[test]
fn link_changes_reach_the_buffer_and_when_the_link_is_back_the_collector() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "logging host 10.0.0.10",
            "logging buffered",
            "end",
        ],
    );
    wait(&mut lab, 3);
    // The switch drops r1's port: r1 notes it, but cannot tell anybody.
    console(
        &mut lab,
        SW1,
        &["configure terminal", "interface fa0/2", "shutdown", "end"],
    );
    wait(&mut lab, 3);
    let log = console(&mut lab, R1, &["show logging"]);
    assert!(
        log.contains("%LINK-3-UPDOWN: Interface GigabitEthernet0/0, changed state to down")
            && log.contains("%LINEPROTO-5-UPDOWN: Line protocol on Interface GigabitEthernet0/0, changed state to down"),
        "{log}"
    );
    assert!(
        !collected(&mut lab).contains("changed state to down"),
        "{}",
        collected(&mut lab)
    );
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface fa0/2",
            "no shutdown",
            "end",
        ],
    );
    wait(&mut lab, 8);
    let got = collected(&mut lab);
    assert!(
        got.contains("local7.errors")
            && got.contains("%LINK-3-UPDOWN: Interface GigabitEthernet0/0, changed state to up")
            && got.contains("%LINEPROTO-5-UPDOWN: Line protocol on Interface GigabitEthernet0/0, changed state to up"),
        "{got}"
    );
    // Messages are stamped with the clock, flagged `*` as it was never set.
    assert!(got.contains("*Mar  1 00:00:"), "{got}");
}

#[test]
fn administrative_shutdown_reads_that_way_and_the_trap_level_filters_what_hosts_get() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "logging host 10.0.0.10",
            "logging trap errors",
            "logging buffered 8192 informational",
            "end",
        ],
    );
    wait(&mut lab, 3);
    console(
        &mut lab,
        R1,
        &["configure terminal", "interface g0/0", "shutdown", "end"],
    );
    wait(&mut lab, 3);
    console(
        &mut lab,
        R1,
        &["configure terminal", "interface g0/0", "no shutdown", "end"],
    );
    wait(&mut lab, 8);
    let log = console(&mut lab, R1, &["show logging"]);
    assert!(
        log.contains("changed state to administratively down"),
        "{log}"
    );
    assert!(log.contains("Trap logging: level errors"), "{log}");
    assert!(
        log.contains("    Buffer logging:  level informational, "),
        "{log}"
    );
    // Only the severity-3 line went out: not the CONFIG_I and LINEPROTO notifications.
    let got = collected(&mut lab);
    assert!(got.contains("1 message received."), "{got}");
    assert!(got.contains("%LINK-3-UPDOWN"), "{got}");
}

#[test]
fn a_switch_logs_its_own_ports_and_config_changes() {
    let mut lab = lab();
    console(
        &mut lab,
        SW1,
        &["configure terminal", "logging buffered", "end"],
    );
    wait(&mut lab, 3);
    console(
        &mut lab,
        SW1,
        &["configure terminal", "interface fa0/1", "shutdown", "end"],
    );
    wait(&mut lab, 3);
    let log = console(&mut lab, SW1, &["show logging"]);
    assert!(
        log.contains(
            "%LINK-5-CHANGED: Interface FastEthernet0/1, changed state to administratively down"
        ),
        "{log}"
    );
    assert!(
        log.contains("%SYS-5-CONFIG_I: Configured from console by console"),
        "{log}"
    );
    assert!(log.contains("%LINEPROTO-5-UPDOWN: Line protocol on Interface FastEthernet0/1, changed state to down"), "{log}");
}

#[test]
fn a_stopped_collector_keeps_nothing_and_clear_empties_the_buffer() {
    let mut lab = lab();
    console(&mut lab, PC1, &["syslogd stop"]);
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "logging host 10.0.0.10",
            "logging buffered",
            "end",
        ],
    );
    wait(&mut lab, 3);
    console(&mut lab, R1, &["configure terminal", "end"]);
    wait(&mut lab, 2);
    assert!(collected(&mut lab).contains("stopped. 0 messages received."));
    assert!(console(&mut lab, R1, &["show logging"]).contains("%SYS-5-CONFIG_I"));
    console(&mut lab, R1, &["clear logging"]);
    assert!(!console(&mut lab, R1, &["show logging"]).contains("%SYS-5-CONFIG_I"));
    console(&mut lab, PC1, &["syslogd start"]);
    console(&mut lab, R1, &["configure terminal", "end"]);
    wait(&mut lab, 2);
    assert!(collected(&mut lab).contains("1 message received."));
    console(&mut lab, PC1, &["syslogd clear"]);
    assert!(collected(&mut lab).contains("0 messages received."));
}

#[test]
fn logging_settings_are_checked_and_come_back_from_the_saved_config() {
    let mut lab = lab();
    let out = console(&mut lab, R1, &["configure terminal", "logging trap loud"]);
    assert!(out.contains("the level is a name"), "{out}");
    let out = console(
        &mut lab,
        R1,
        &["configure terminal", "no logging host 10.9.9.9"],
    );
    assert!(out.contains("not configured"), "{out}");
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "logging host 10.0.0.10",
            "logging host 10.0.0.11",
            "no logging host 10.0.0.11",
            "logging trap 4",
            "logging buffered 16384 notifications",
            "no service timestamps log datetime msec",
            "end",
            "copy running-config startup-config",
        ],
    );
    let before = console(&mut lab, R1, &["show running-config"]);
    for want in [
        "logging buffered 16384 notifications\n",
        "logging trap warnings\n",
        "logging host 10.0.0.10\n",
        "no service timestamps log datetime msec\n",
    ] {
        assert!(before.contains(want), "{want}: {before}");
    }
    assert!(!before.contains("10.0.0.11"), "{before}");
    let mut sh = Shell::router(lab.devices[R1].id);
    sh.exec(&mut lab.world, "enable").unwrap();
    sh.exec(&mut lab.world, "reload").unwrap();
    sh.exec(&mut lab.world, "").unwrap();
    assert_eq!(before, console(&mut lab, R1, &["show running-config"]));
    // Without timestamps a line starts at its tag.
    console(&mut lab, R1, &["configure terminal", "end"]);
    let log = console(&mut lab, R1, &["show logging"]);
    assert!(
        log.contains("\n%SYS-5-CONFIG_I: Configured from console by console"),
        "{log}"
    );
}
