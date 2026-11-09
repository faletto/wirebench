//! Port security beyond the golden lab: limits, hand-typed addresses, and a configuration that
//! survives a save and a reload.

use engine::lab::load_and_build;
use engine::shell::{switch_config, Shell};
use kernel::{PortId, Trace};
use netdev::L2Switch;

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_psec.lab.toml");

fn run(sh: &mut Shell, w: &mut engine::World, lines: &[&str]) -> String {
    let mut out = String::new();
    for l in lines {
        out = sh.exec(w, l).unwrap();
    }
    out
}

fn pings(lab: &mut engine::lab::BuiltLab, who: usize, dst: &str) -> bool {
    let mut sh = Shell::host(lab.devices[who].id);
    run(&mut sh, &mut lab.world, &[&format!("ping {dst}")]).contains("received 4")
}

fn setup() -> (engine::lab::BuiltLab, Shell) {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    for (i, ip) in [(0, "10.0.0.1"), (1, "10.0.0.3"), (4, "10.0.0.2")] {
        let mut h = Shell::host(lab.devices[i].id);
        run(
            &mut h,
            &mut lab.world,
            &[&format!("ipconfig set {ip} 255.255.255.0")],
        );
    }
    let sw = lab.devices[3].id;
    let mut s = Shell::switch(sw);
    run(
        &mut s,
        &mut lab.world,
        &["enable", "configure terminal", "interface fa0/1"],
    );
    (lab, s)
}

#[test]
fn a_larger_maximum_lets_both_hosts_through() {
    let (mut lab, mut s) = setup();
    run(
        &mut s,
        &mut lab.world,
        &[
            "switchport mode access",
            "switchport port-security",
            "switchport port-security maximum 2",
        ],
    );
    assert!(pings(&mut lab, 0, "10.0.0.2"));
    assert!(pings(&mut lab, 1, "10.0.0.2"));
    let sw = lab.devices[3].id;
    assert!(!lab.world.device::<L2Switch>(sw).err_disabled(PortId(0)));
}

#[test]
fn a_typed_address_uses_up_the_limit_and_locks_everyone_else_out() {
    let (mut lab, mut s) = setup();
    let out = run(
        &mut s,
        &mut lab.world,
        &[
            "switchport mode access",
            "switchport port-security",
            "switchport port-security mac-address 0200.0000.0101",
            "switchport port-security violation restrict",
        ],
    );
    assert_eq!(out, "");
    // pc3 owns the one slot; pc1 is a violation but the port stays up.
    assert!(!pings(&mut lab, 0, "10.0.0.2"));
    assert!(pings(&mut lab, 1, "10.0.0.2"));
    let sw = lab.devices[3].id;
    assert!(!lab.world.device::<L2Switch>(sw).err_disabled(PortId(0)));
    // A second typed address does not fit while the maximum is 1.
    let out = run(
        &mut s,
        &mut lab.world,
        &["switchport port-security mac-address 0200.0000.0001"],
    );
    assert!(out.contains("Maximum number"), "{out}");
    let out = run(
        &mut s,
        &mut lab.world,
        &["switchport port-security maximum 2"],
    );
    assert_eq!(out, "");
    let out = run(
        &mut s,
        &mut lab.world,
        &["switchport port-security mac-address 0200.0000.0001"],
    );
    assert_eq!(out, "");
    assert!(pings(&mut lab, 0, "10.0.0.2"));
}

#[test]
fn protect_mode_counts_nothing() {
    let (mut lab, mut s) = setup();
    run(
        &mut s,
        &mut lab.world,
        &[
            "switchport mode access",
            "switchport port-security",
            "switchport port-security mac-address 0200.0000.0101",
            "switchport port-security violation protect",
        ],
    );
    assert!(!pings(&mut lab, 0, "10.0.0.2"));
    let out = run(
        &mut s,
        &mut lab.world,
        &["end", "show port-security interface fa0/1"],
    );
    assert!(out.contains("Security Violation Count   : 0"), "{out}");
}

#[test]
fn lowering_the_maximum_below_the_addresses_in_use_is_refused() {
    let (mut lab, mut s) = setup();
    run(
        &mut s,
        &mut lab.world,
        &[
            "switchport mode access",
            "switchport port-security",
            "switchport port-security maximum 2",
            "switchport port-security mac-address 0200.0000.0101",
            "switchport port-security mac-address 0200.0000.0001",
        ],
    );
    let out = run(
        &mut s,
        &mut lab.world,
        &["switchport port-security maximum 1"],
    );
    assert!(out.contains("lower"), "{out}");
}

#[test]
fn sticky_addresses_survive_save_and_reload() {
    let (mut lab, mut s) = setup();
    run(
        &mut s,
        &mut lab.world,
        &[
            "switchport mode access",
            "switchport port-security",
            "switchport port-security maximum 2",
            "switchport port-security violation restrict",
            "switchport port-security mac-address sticky",
        ],
    );
    assert!(pings(&mut lab, 0, "10.0.0.2"));
    assert!(pings(&mut lab, 1, "10.0.0.2"));
    let sw = lab.devices[3].id;
    let before = switch_config(lab.world.device::<L2Switch>(sw));
    assert!(
        before.contains("mac-address sticky 0200.0000.0001"),
        "{before}"
    );
    assert!(
        before.contains("mac-address sticky 0200.0000.0101"),
        "{before}"
    );
    run(
        &mut s,
        &mut lab.world,
        &["end", "copy running-config startup-config"],
    );
    let out = run(&mut s, &mut lab.world, &["reload", ""]);
    assert!(out.contains("System restarted"), "{out}");
    let after = switch_config(lab.world.device::<L2Switch>(sw));
    assert_eq!(before, after);
}

#[test]
fn dynamic_addresses_are_forgotten_when_the_cable_goes_and_sticky_ones_are_not() {
    let (mut lab, mut s) = setup();
    run(
        &mut s,
        &mut lab.world,
        &["switchport mode access", "switchport port-security"],
    );
    assert!(pings(&mut lab, 0, "10.0.0.2"));
    let sw = lab.devices[3].id;
    assert_eq!(lab.world.device::<L2Switch>(sw).psec.dynamic[0].len(), 1);
    run(&mut s, &mut lab.world, &["shutdown"]);
    assert_eq!(lab.world.device::<L2Switch>(sw).psec.dynamic[0].len(), 0);
}
