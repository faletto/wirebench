//! EtherChannel: which mode pairs form a bundle, how a bundle behaves when a cable goes, how
//! settings reach the members, and that a saved configuration comes back the same.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{switch_config, Shell};
use kernel::{PortId, SimTime, Trace};
use netdev::L2Switch;

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_etherchannel.lab.toml");
const SW1: usize = 2;
const SW2: usize = 3;

fn run(sh: &mut Shell, w: &mut engine::World, lines: &[&str]) -> String {
    let mut out = String::new();
    for l in lines {
        out = sh.exec(w, l).unwrap();
    }
    out
}

fn wait(lab: &mut BuiltLab, secs: u64) {
    let to = lab.world.kernel.now() + SimTime::from_secs(secs);
    lab.world.run_until(to);
}

/// Both switches get the same two-port group; `a` and `b` are the channel modes.
fn pair(a: &str, b: &str, stp: bool) -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    for (i, ip) in [(0, "10.0.0.1"), (4, "10.0.0.2")] {
        let mut h = Shell::host(lab.devices[i].id);
        run(
            &mut h,
            &mut lab.world,
            &[&format!("ipconfig set {ip} 255.255.255.0")],
        );
    }
    for (sw, mode) in [(SW1, a), (SW2, b)] {
        let mut s = Shell::switch(lab.devices[sw].id);
        run(&mut s, &mut lab.world, &["enable", "configure terminal"]);
        if stp {
            run(&mut s, &mut lab.world, &["spanning-tree mode rapid-pvst"]);
        }
        run(
            &mut s,
            &mut lab.world,
            &[
                "interface range fa0/1 - 2",
                "switchport mode trunk",
                &format!("channel-group 1 mode {mode}"),
            ],
        );
    }
    wait(&mut lab, 5);
    lab
}

/// A console on `sw`, already in global configuration mode.
fn conf(lab: &mut BuiltLab, sw: usize) -> Shell {
    let mut s = Shell::switch(lab.devices[sw].id);
    run(&mut s, &mut lab.world, &["enable", "configure terminal"]);
    s
}

fn bundled(lab: &BuiltLab, sw: usize) -> usize {
    lab.world
        .device::<L2Switch>(lab.devices[sw].id)
        .bundled(1)
        .len()
}

fn pings(lab: &mut BuiltLab) -> bool {
    let mut h = Shell::host(lab.devices[0].id);
    run(&mut h, &mut lab.world, &["ping 10.0.0.2"]).contains("received 4")
}

#[test]
fn which_mode_pairs_form_a_bundle() {
    // (this end, other end, ports bundled at each end afterwards). A static end bundles by
    // itself, so a mismatch leaves it bundled and its LACP neighbour on its own.
    let table = [
        ("on", "on", (2, 2)),
        ("active", "active", (2, 2)),
        ("active", "passive", (2, 2)),
        ("passive", "active", (2, 2)),
        ("passive", "passive", (0, 0)),
        ("on", "active", (2, 0)),
        ("active", "on", (0, 2)),
        ("on", "passive", (2, 0)),
        ("passive", "on", (0, 2)),
    ];
    for (a, b, want) in table {
        let mut lab = pair(a, b, false);
        // Long enough for LACP to give up on a neighbour that is not speaking it.
        wait(&mut lab, 100);
        assert_eq!(
            (bundled(&lab, SW1), bundled(&lab, SW2)),
            want,
            "{a} against {b}"
        );
    }
}

#[test]
fn passive_ends_never_start_the_conversation() {
    let lab = pair("passive", "passive", false);
    let s = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    assert!(s.lacp_neighbour(PortId(0)).is_none());
}

#[test]
fn a_bundle_survives_losing_one_cable_without_spanning_tree_noticing() {
    for mode in ["on", "active"] {
        let mut lab = pair(mode, if mode == "on" { "on" } else { "passive" }, true);
        wait(&mut lab, 40);
        assert!(pings(&mut lab), "{mode}");
        let before = format!(
            "{:?}",
            lab.world
                .device::<L2Switch>(lab.devices[SW2].id)
                .trees()
                .map(|(v, t)| (v, t.root().root, t.root_port()))
                .collect::<Vec<_>>()
        );
        // Take Fa0/1 down at sw1 the way a shutdown would.
        let mut s = conf(&mut lab, SW1);
        run(
            &mut s,
            &mut lab.world,
            &["interface fa0/1", "shutdown", "end"],
        );
        wait(&mut lab, 5);
        assert_eq!((bundled(&lab, SW1), bundled(&lab, SW2)), (1, 1), "{mode}");
        assert!(pings(&mut lab), "{mode}: still works on one cable");
        let after = format!(
            "{:?}",
            lab.world
                .device::<L2Switch>(lab.devices[SW2].id)
                .trees()
                .map(|(v, t)| (v, t.root().root, t.root_port()))
                .collect::<Vec<_>>()
        );
        assert_eq!(before, after, "{mode}: the root did not move");
        run(
            &mut s,
            &mut lab.world,
            &[
                "configure terminal",
                "interface fa0/1",
                "no shutdown",
                "end",
            ],
        );
        wait(&mut lab, 40);
        assert_eq!((bundled(&lab, SW1), bundled(&lab, SW2)), (2, 2), "{mode}");
        assert!(pings(&mut lab), "{mode}");
    }
}

#[test]
fn a_bundle_does_not_loop_broadcasts_even_without_spanning_tree() {
    let mut lab = pair("active", "active", false);
    wait(&mut lab, 5);
    let before = lab.world.kernel.delivered();
    assert!(pings(&mut lab));
    let used = lab.world.kernel.delivered() - before;
    // 4 echoes and replies cross 3 hops, plus ARP and the odd control frame: far below a storm.
    assert!(used < 200, "{used} frames delivered");
}

#[test]
fn port_channel_settings_reach_the_members_and_back() {
    let mut lab = pair("on", "on", false);
    let mut s = conf(&mut lab, SW1);
    run(
        &mut s,
        &mut lab.world,
        &[
            "interface port-channel 1",
            "switchport trunk allowed vlan 10,20",
            "switchport trunk native vlan 7",
        ],
    );
    for p in [0, 1] {
        let c = lab
            .world
            .device::<L2Switch>(lab.devices[SW1].id)
            .port(PortId(p))
            .unwrap()
            .clone();
        assert_eq!(c.native_vlan, 7);
        assert_eq!(c.allowed.len(), 2);
    }
    // And the other way: a member typed on its own changes the group and its sibling.
    run(
        &mut s,
        &mut lab.world,
        &["interface fa0/2", "switchport trunk native vlan 9"],
    );
    let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    assert_eq!(sw.group(1).unwrap().native_vlan, 9);
    assert_eq!(sw.port(PortId(0)).unwrap().native_vlan, 9);
}

#[test]
fn shutting_one_member_leaves_the_others_up_but_shutting_the_group_does_not() {
    let mut lab = pair("on", "on", false);
    let mut s = conf(&mut lab, SW1);
    run(&mut s, &mut lab.world, &["interface fa0/1", "shutdown"]);
    let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    assert!(sw.port(PortId(0)).unwrap().shutdown);
    assert!(!sw.port(PortId(1)).unwrap().shutdown);
    run(
        &mut s,
        &mut lab.world,
        &["no shutdown", "interface port-channel 1", "shutdown"],
    );
    let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    assert!(sw.port(PortId(0)).unwrap().shutdown && sw.port(PortId(1)).unwrap().shutdown);
    wait(&mut lab, 5);
    assert!(!pings(&mut lab));
}

#[test]
fn group_numbers_protocol_mix_and_member_limit_are_checked() {
    let mut lab = pair("active", "active", false);
    let mut s = conf(&mut lab, SW1);
    let out = run(
        &mut s,
        &mut lab.world,
        &["interface fa0/3", "channel-group 1 mode on"],
    );
    assert!(out.contains("mismatch"), "{out}");
    let out = run(&mut s, &mut lab.world, &["channel-group 7 mode on"]);
    assert!(out.contains("Invalid") || out.contains('%'), "{out}");
    let out = run(
        &mut s,
        &mut lab.world,
        &["interface port-channel 1", "channel-group 1 mode on"],
    );
    assert!(
        out.starts_with('%'),
        "a port-channel is not a member: {out}"
    );
    let out = run(
        &mut s,
        &mut lab.world,
        &["exit", "no interface port-channel 2"],
    );
    assert!(out.contains("No such"), "{out}");
}

#[test]
fn running_config_survives_save_and_reload() {
    let mut lab = pair("active", "passive", false);
    let mut s = conf(&mut lab, SW1);
    run(
        &mut s,
        &mut lab.world,
        &[
            "interface port-channel 1",
            "switchport trunk allowed vlan 1,20",
            "exit",
            "port-channel load-balance dst-mac",
        ],
    );
    let before = switch_config(lab.world.device::<L2Switch>(lab.devices[SW1].id));
    assert!(before.contains("interface Port-channel1"), "{before}");
    assert!(before.contains("channel-group 1 mode active"), "{before}");
    run(
        &mut s,
        &mut lab.world,
        &["end", "copy running-config startup-config"],
    );
    run(&mut s, &mut lab.world, &["reload", ""]);
    let after = switch_config(lab.world.device::<L2Switch>(lab.devices[SW1].id));
    assert_eq!(before, after);
}

#[test]
fn interface_range_runs_each_command_on_each_port() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut s = conf(&mut lab, SW1);
    for spec in [
        "interface range fa0/1 - 3",
        "interface range fa0/1-3",
        "interface range fa0/1,fa0/2,fa0/3",
    ] {
        run(
            &mut s,
            &mut lab.world,
            &[spec, "switchport access vlan 5", "exit"],
        );
        let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
        for p in 0..3 {
            assert_eq!(sw.port(PortId(p)).unwrap().access_vlan, 5, "{spec}");
        }
        assert_eq!(sw.port(PortId(3)).unwrap().access_vlan, 1);
        run(
            &mut s,
            &mut lab.world,
            &[spec, "no switchport access vlan", "exit"],
        );
    }
    let out = run(&mut s, &mut lab.world, &["interface range fa0/1 - 20"]);
    assert!(out.contains("Invalid"), "{out}");
}
