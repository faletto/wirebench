//! CDP and LLDP through devices: who hears whom, what a switch does with the frames, and how
//! entries age.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::{SimTime, Trace};
use netdev::{L2Switch, Router};

/// r1 and r2 on two ports of one switch.
const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "r1"
template = "router-2g"
pos = [0, 0]

[[device]]
name = "sw"
template = "switch-l2-8"
pos = [100, 0]

[[device]]
name = "r2"
template = "router-2g"
pos = [200, 0]

[[link]]
a = "r1:Gi0/0"
b = "sw:Fa0/1"

[[link]]
a = "r2:Gi0/0"
b = "sw:Fa0/2"
"#;

const R1: usize = 0;
const SW: usize = 1;
const R2: usize = 2;

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let mut sh = Shell::router(lab.devices[dev].id);
    if dev == SW {
        sh = Shell::switch(lab.devices[dev].id);
    }
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

#[test]
fn a_switch_not_running_cdp_passes_it_on_and_one_running_it_does_not() {
    let mut lab = lab();
    console(&mut lab, R1, &["configure terminal", "cdp run", "end"]);
    console(&mut lab, R2, &["configure terminal", "cdp run", "end"]);
    wait(&mut lab, 5);
    let out = console(&mut lab, R1, &["show cdp neighbors"]);
    assert!(out.contains("r2") || out.contains("Router"), "{out}");
    assert!(
        lab.world
            .device::<Router>(lab.devices[R1].id)
            .disc
            .cdp_neighbours()
            .len()
            == 1,
        "the switch is invisible: r1 hears r2 directly"
    );
    assert_eq!(
        lab.world
            .device::<Router>(lab.devices[R1].id)
            .disc
            .cdp_neighbours()[0]
            .device_id,
        "r2"
    );
    // Now the switch listens too: the routers stop hearing each other (after the entries age out).
    console(&mut lab, SW, &["configure terminal", "cdp run", "end"]);
    console(&mut lab, R1, &["clear cdp table"]);
    wait(&mut lab, 70);
    let r1 = lab.world.device::<Router>(lab.devices[R1].id);
    assert_eq!(r1.disc.cdp_neighbours().len(), 1);
    assert_eq!(
        r1.disc.cdp_neighbours()[0].platform,
        "wirebench L2 switch",
        "only the switch is heard now"
    );
}

#[test]
fn lldp_is_never_bridged() {
    let mut lab = lab();
    for d in [R1, R2] {
        console(&mut lab, d, &["configure terminal", "lldp run", "end"]);
    }
    wait(&mut lab, 40);
    for d in [R1, R2] {
        let r = lab.world.device::<Router>(lab.devices[d].id);
        assert!(
            r.disc.lldp_neighbours().is_empty(),
            "heard through a switch"
        );
    }
    console(&mut lab, SW, &["configure terminal", "lldp run", "end"]);
    wait(&mut lab, 40);
    let sw = lab.world.device::<L2Switch>(lab.devices[SW].id);
    assert_eq!(sw.disc.lldp_neighbours().len(), 2);
}

#[test]
fn entries_age_out_after_the_holdtime_when_a_neighbour_goes_quiet() {
    let mut lab = lab();
    console(&mut lab, R1, &["configure terminal", "cdp run", "end"]);
    console(
        &mut lab,
        R2,
        &[
            "configure terminal",
            "cdp run",
            "cdp holdtime 20",
            "cdp timer 5",
            "end",
        ],
    );
    wait(&mut lab, 12);
    let n = lab
        .world
        .device::<Router>(lab.devices[R1].id)
        .disc
        .cdp_neighbours()
        .len();
    assert_eq!(n, 1);
    // r2 stops talking.
    console(&mut lab, R2, &["configure terminal", "no cdp run", "end"]);
    wait(&mut lab, 10);
    let n = lab
        .world
        .device::<Router>(lab.devices[R1].id)
        .disc
        .cdp_neighbours()
        .len();
    assert_eq!(n, 1, "still held");
    wait(&mut lab, 15);
    let n = lab
        .world
        .device::<Router>(lab.devices[R1].id)
        .disc
        .cdp_neighbours()
        .len();
    assert_eq!(n, 0, "20 s after the last advertisement");
}

#[test]
fn settings_survive_save_and_reload_and_the_reloaded_router_announces_by_itself() {
    let mut lab = lab();
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "cdp run",
            "cdp timer 20",
            "lldp run",
            "interface g0/0",
            "no lldp transmit",
            "end",
            "copy running-config startup-config",
        ],
    );
    let before = console(&mut lab, R1, &["show running-config"]);
    assert!(
        before.contains("cdp run") && before.contains("cdp timer 20"),
        "{before}"
    );
    assert!(before.contains("no lldp transmit"), "{before}");
    let mut sh = Shell::router(lab.devices[R1].id);
    sh.exec(&mut lab.world, "enable").unwrap();
    sh.exec(&mut lab.world, "reload").unwrap();
    sh.exec(&mut lab.world, "").unwrap();
    let after = console(&mut lab, R1, &["show running-config"]);
    assert_eq!(before, after);
    // The reloaded router announces by itself, with no command typed after the reload.
    console(&mut lab, R2, &["configure terminal", "cdp run", "end"]);
    wait(&mut lab, 30);
    let r2 = lab.world.device::<Router>(lab.devices[R2].id);
    assert_eq!(r2.disc.cdp_neighbours().len(), 1);
}

#[test]
fn a_shut_port_loses_its_neighbour_at_once() {
    let mut lab = lab();
    console(&mut lab, R1, &["configure terminal", "cdp run", "end"]);
    console(&mut lab, SW, &["configure terminal", "cdp run", "end"]);
    wait(&mut lab, 5);
    assert_eq!(
        lab.world
            .device::<L2Switch>(lab.devices[SW].id)
            .disc
            .cdp_neighbours()
            .len(),
        1
    );
    console(
        &mut lab,
        SW,
        &["configure terminal", "interface fa0/1", "shutdown", "end"],
    );
    wait(&mut lab, 2);
    assert!(lab
        .world
        .device::<L2Switch>(lab.devices[SW].id)
        .disc
        .cdp_neighbours()
        .is_empty());
}
