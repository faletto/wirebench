//! Spanning tree from the console: configuring it, reading it back, keeping it across a reload.

use engine::lab::{load_and_build, BuiltLab, LabKind};
use engine::shell::Shell;
use kernel::{SimTime, Trace};

const TRIANGLE: &str = include_str!("../../../testkit/scenarios/m2_stp.lab.toml");

struct Rig {
    lab: BuiltLab,
    shells: Vec<(String, Shell)>,
}

impl Rig {
    fn new() -> Rig {
        let lab = load_and_build(TRIANGLE, Trace::new()).unwrap();
        let shells = lab
            .devices
            .iter()
            .map(|d| {
                let sh = match d.kind {
                    LabKind::Host => Shell::host(d.id),
                    LabKind::Router => Shell::router(d.id),
                    LabKind::Switch => Shell::switch(d.id),
                };
                (d.name.clone(), sh)
            })
            .collect();
        Rig { lab, shells }
    }

    fn run(&mut self, who: &str, line: &str) -> String {
        let sh = &mut self.shells.iter_mut().find(|(n, _)| n == who).unwrap().1;
        sh.exec(&mut self.lab.world, line).unwrap_or_default()
    }

    /// Lines typed in order, each of which must print nothing.
    fn quietly(&mut self, who: &str, lines: &[&str]) {
        for l in lines {
            let out = self.run(who, l);
            assert!(out.is_empty(), "{who}: `{l}` printed {out:?}");
        }
    }

    /// Get `who` to the privileged prompt, whatever it was in.
    fn privileged(&mut self, who: &str) {
        let sh = &mut self.shells.iter_mut().find(|(n, _)| n == who).unwrap().1;
        if sh.prompt(&self.lab.world).ends_with('>') {
            sh.exec(&mut self.lab.world, "enable");
        }
        if !sh.prompt(&self.lab.world).ends_with("#") || sh.prompt(&self.lab.world).contains('(') {
            sh.exec(&mut self.lab.world, "end");
        }
    }

    fn wait(&mut self, secs: u64) {
        let to = self.lab.world.kernel.now() + SimTime::from_secs(secs);
        self.lab.world.run_until(to);
    }

    /// Every switch runs rapid spanning tree and sw1 is the root.
    fn rapid(&mut self) {
        for sw in ["sw1", "sw2", "sw3"] {
            self.privileged(sw);
            self.quietly(sw, &["configure terminal", "spanning-tree mode rapid-pvst"]);
            if sw == "sw1" {
                self.quietly(sw, &["spanning-tree vlan 1 priority 4096"]);
            }
            self.quietly(sw, &["end"]);
        }
        self.wait(15);
    }
}

#[test]
fn with_spanning_tree_off_the_show_commands_say_so() {
    let mut r = Rig::new();
    r.quietly("sw1", &["enable"]);
    assert_eq!(
        r.run("sw1", "show spanning-tree"),
        "No spanning tree instance exists."
    );
    assert_eq!(
        r.run("sw1", "show spanning-tree vlan 1"),
        "Spanning tree instance(s) for vlan 1 does not exist."
    );
    // Nothing about it in the running config either.
    assert!(!r
        .run("sw1", "show running-config")
        .contains("spanning-tree"));
}

#[test]
fn bridge_priority_must_be_a_multiple_of_4096() {
    let mut r = Rig::new();
    r.quietly("sw1", &["enable", "configure terminal"]);
    let out = r.run("sw1", "spanning-tree vlan 1 priority 5000");
    assert!(
        out.starts_with("% Bridge Priority must be in increments of 4096."),
        "{out}"
    );
    assert!(out.contains("61440"), "{out}");
    r.quietly(
        "sw1",
        &[
            "spanning-tree mode rapid-pvst",
            "spanning-tree vlan 1 priority 8192",
            "end",
        ],
    );
    let out = r.run("sw1", "show spanning-tree");
    assert!(
        out.contains("Priority    8193  (priority 8192 sys-id-ext 1)"),
        "{out}"
    );
    let out = r.run("sw1", "show running-config");
    assert!(out.contains("spanning-tree vlan 1 priority 8192"), "{out}");
}

#[test]
fn vlan_lists_are_checked_and_applied_to_each_vlan() {
    let mut r = Rig::new();
    r.quietly(
        "sw1",
        &["enable", "configure terminal", "vlan 10", "vlan 30"],
    );
    r.quietly("sw1", &["exit"]);
    let out = r.run("sw1", "spanning-tree vlan 10,30 priority 4096");
    assert!(out.is_empty(), "{out}");
    let out = r.run("sw1", "spanning-tree vlan banana priority 4096");
    assert!(out.starts_with("% Invalid VLAN list:"), "{out}");
    r.quietly("sw1", &["spanning-tree mode rapid-pvst", "end"]);
    let cfg = r.run("sw1", "show running-config");
    assert!(cfg.contains("spanning-tree vlan 10 priority 4096"), "{cfg}");
    assert!(cfg.contains("spanning-tree vlan 30 priority 4096"), "{cfg}");
    // One tree per VLAN that exists, and VLAN 20 was never made.
    let out = r.run("sw1", "show spanning-tree");
    assert_eq!(out.matches("VLAN0010").count(), 1, "{out}");
    assert!(
        out.contains("VLAN0001") && out.contains("VLAN0030"),
        "{out}"
    );
    assert!(!out.contains("VLAN0020"));
}

#[test]
fn root_primary_and_secondary_pick_priorities_the_way_the_macro_does() {
    let mut r = Rig::new();
    r.quietly(
        "sw2",
        &[
            "enable",
            "configure terminal",
            "spanning-tree mode rapid-pvst",
        ],
    );
    // Nobody else is root yet, so primary is 24576.
    r.quietly("sw2", &["spanning-tree vlan 1 root primary"]);
    assert!(r
        .run("sw2", "do show running-config")
        .contains("spanning-tree vlan 1 priority 24576"));
    r.quietly("sw2", &["spanning-tree vlan 1 root secondary"]);
    assert!(r
        .run("sw2", "do show running-config")
        .contains("spanning-tree vlan 1 priority 28672"));
    // With a root whose priority is already low, primary goes one step below it.
    r.quietly(
        "sw1",
        &[
            "enable",
            "configure terminal",
            "spanning-tree mode rapid-pvst",
            "spanning-tree vlan 1 priority 8192",
            "end",
        ],
    );
    r.quietly("sw2", &["end"]);
    r.wait(15);
    r.quietly(
        "sw2",
        &["configure terminal", "spanning-tree vlan 1 root primary"],
    );
    assert!(r
        .run("sw2", "do show running-config")
        .contains("spanning-tree vlan 1 priority 4096"));
}

#[test]
fn the_triangle_settles_and_show_spanning_tree_reads_like_the_real_thing() {
    let mut r = Rig::new();
    r.rapid();
    let out = r.run("sw1", "show spanning-tree");
    assert!(out.contains("This bridge is the root"), "{out}");
    let out = r.run("sw3", "show spanning-tree");
    assert!(out.contains("Port        1 (FastEthernet0/1)"), "{out}");
    assert!(
        out.contains("Fa0/1               Root FWD 19        128.1    P2p"),
        "{out}"
    );
    assert!(out.contains("Fa0/2               Altn BLK 19"), "{out}");
}

#[test]
fn a_port_cost_changes_which_way_traffic_goes() {
    let mut r = Rig::new();
    r.quietly(
        "sw3",
        &[
            "enable",
            "configure terminal",
            "interface fa0/1",
            "spanning-tree cost 100",
            "end",
        ],
    );
    r.rapid();
    // sw3's direct link to the root now costs 100; through sw2 it is 38.
    let out = r.run("sw3", "show spanning-tree");
    assert!(out.contains("Cost        38"), "{out}");
    assert!(out.contains("Port        2 (FastEthernet0/2)"), "{out}");
    // Put the cost back and it prefers the direct link again.
    r.quietly(
        "sw3",
        &[
            "configure terminal",
            "interface fa0/1",
            "no spanning-tree cost",
            "end",
        ],
    );
    r.wait(10);
    let out = r.run("sw3", "show spanning-tree");
    assert!(out.contains("Cost        19"), "{out}");
}

#[test]
fn port_priority_is_a_multiple_of_sixteen_and_shows_in_the_table() {
    let mut r = Rig::new();
    r.quietly(
        "sw1",
        &[
            "enable",
            "configure terminal",
            "spanning-tree mode rapid-pvst",
            "interface fa0/1",
        ],
    );
    assert_eq!(
        r.run("sw1", "spanning-tree port-priority 100"),
        "% Port priority must be a multiple of 16."
    );
    r.quietly("sw1", &["spanning-tree port-priority 64", "end"]);
    assert!(r.run("sw1", "show spanning-tree").contains("64.1"));
    assert!(r
        .run("sw1", "show running-config")
        .contains(" spanning-tree port-priority 64"));
}

#[test]
fn shutting_a_port_takes_it_out_of_the_tree_and_no_shutdown_brings_it_back() {
    let mut r = Rig::new();
    r.rapid();
    r.quietly(
        "sw2",
        &["configure terminal", "interface fa0/1", "shutdown", "end"],
    );
    r.wait(6);
    let out = r.run("sw2", "show spanning-tree");
    assert!(!out.contains("Fa0/1 "), "a down port is not listed:\n{out}");
    assert!(out.contains("Port        2"), "{out}");
    assert!(r.run("sw2", "show running-config").contains(" shutdown"));
    r.quietly(
        "sw2",
        &[
            "configure terminal",
            "interface fa0/1",
            "no shutdown",
            "end",
        ],
    );
    r.wait(6);
    assert!(r
        .run("sw2", "show spanning-tree")
        .contains("Fa0/1               Root FWD"));
}

#[test]
fn turning_spanning_tree_off_for_a_vlan_or_the_switch_removes_its_trees() {
    let mut r = Rig::new();
    r.rapid();
    r.quietly("sw2", &["configure terminal", "no spanning-tree vlan 1"]);
    assert!(r
        .run("sw2", "do show spanning-tree vlan 1")
        .contains("does not exist"));
    assert!(r
        .run("sw2", "do show running-config")
        .contains("no spanning-tree vlan 1"));
    r.quietly("sw2", &["spanning-tree vlan 1"]);
    assert!(r.run("sw2", "do show spanning-tree").contains("VLAN0001"));
    r.quietly("sw2", &["no spanning-tree mode"]);
    assert_eq!(
        r.run("sw2", "do show spanning-tree"),
        "No spanning tree instance exists."
    );
}

#[test]
fn the_saved_configuration_brings_spanning_tree_back_after_a_reload() {
    let mut r = Rig::new();
    r.rapid();
    r.privileged("sw2");
    assert_eq!(r.run("sw2", "write"), "Configuration saved.");
    r.run("sw2", "reload");
    r.run("sw2", "");
    // Gone until it has booted and had time; then the same tree.
    r.wait(15);
    let out = r.run("sw2", "show spanning-tree");
    assert!(out.contains("protocol rstp"), "{out}");
    assert!(out.contains("Fa0/1               Root FWD"), "{out}");
}

#[test]
fn the_running_config_replays_into_the_same_spanning_tree_settings() {
    let mut r = Rig::new();
    r.rapid();
    r.quietly(
        "sw2",
        &[
            "configure terminal",
            "interface fa0/3",
            "spanning-tree portfast",
            "spanning-tree cost 7",
            "end",
        ],
    );
    let cfg = r.run("sw2", "show running-config");
    let mut fresh = Rig::new();
    let mut boot = Shell::switch(
        fresh
            .lab
            .devices
            .iter()
            .find(|d| d.name == "sw2")
            .unwrap()
            .id,
    );
    let problems = boot.replay(&mut fresh.lab.world, &cfg);
    assert!(problems.is_empty(), "{problems:?}");
    let sh = boot
        .exec(&mut fresh.lab.world, "do show running-config")
        .unwrap();
    assert_eq!(sh, cfg);
}
