//! OSPF from the console: errors, saving and replaying, and a layer 3 switch speaking it.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{running_config, switch_config, Shell};
use kernel::{SimTime, Trace};
use netdev::{L2Switch, Router};

/// r1 - sw1 (layer 3) on sw1's Fa0/1; pc1 behind r1; pc2 on sw1's Fa0/2.
const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "r1"
template = "router-2g"
pos = [0, 0]

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [100, 0]

[[device]]
name = "pc1"
template = "host-pc"
pos = [-100, 0]

[[device]]
name = "pc2"
template = "host-pc"
pos = [200, 0]

[[link]]
a = "r1:Gi0/0"
b = "sw1:Fa0/1"

[[link]]
a = "r1:Gi0/1"
b = "pc1:eth0"

[[link]]
a = "pc2:eth0"
b = "sw1:Fa0/2"
"#;

fn run(sh: &mut Shell, lab: &mut BuiltLab, lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .map(|l| sh.exec(&mut lab.world, l).unwrap_or_default())
        .collect()
}

fn wait(lab: &mut BuiltLab, secs: u64) {
    let to = lab.world.kernel.now() + SimTime::from_secs(secs);
    lab.world.run_until(to);
}

fn ping(lab: &mut BuiltLab, pc: usize, dst: &str) -> String {
    let mut sh = Shell::host(lab.devices[pc].id);
    run(&mut sh, lab, &[&format!("ping {dst}")]).remove(0)
}

fn up() -> (BuiltLab, Shell, Shell) {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut r1 = Shell::router(lab.devices[0].id);
    let mut sw = Shell::switch(lab.devices[1].id);
    let outs = run(
        &mut r1,
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "router ospf 1",
            "network 10.0.0.0 0.255.255.255 area 0",
            "exit",
            "interface g0/0",
            "ip address 10.0.0.1 255.255.255.0",
            "no shutdown",
            "exit",
            "interface g0/1",
            "ip address 10.0.1.1 255.255.255.0",
            "no shutdown",
            "end",
        ],
    );
    assert!(outs.iter().all(String::is_empty), "{outs:?}");
    let outs = run(
        &mut sw,
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "ip routing",
            "vlan 20",
            "exit",
            "interface fa0/2",
            "switchport mode access",
            "switchport access vlan 20",
            "exit",
            "interface vlan 1",
            "ip address 10.0.0.2 255.255.255.0",
            "no shutdown",
            "exit",
            "interface vlan 20",
            "ip address 10.0.20.1 255.255.255.0",
            "no shutdown",
            "exit",
            "router ospf 1",
            "network 10.0.0.0 0.255.255.255 area 0",
            "end",
        ],
    );
    assert!(outs.iter().all(String::is_empty), "{outs:?}");
    let mut pc1 = Shell::host(lab.devices[2].id);
    run(
        &mut pc1,
        &mut lab,
        &[
            "ipconfig set 10.0.1.10 255.255.255.0",
            "ipconfig gateway 10.0.1.1",
        ],
    );
    let mut pc2 = Shell::host(lab.devices[3].id);
    run(
        &mut pc2,
        &mut lab,
        &[
            "ipconfig set 10.0.20.10 255.255.255.0",
            "ipconfig gateway 10.0.20.1",
        ],
    );
    (lab, r1, sw)
}

#[test]
fn a_layer_3_switch_and_a_router_become_neighbours_and_share_routes() {
    let (mut lab, mut r1, mut sw) = up();
    wait(&mut lab, 70);
    let nbr = run(&mut sw, &mut lab, &["show ip ospf neighbor"]).remove(0);
    assert!(nbr.contains("FULL/"), "{nbr}");
    let routes = run(&mut sw, &mut lab, &["show ip route"]).remove(0);
    assert!(
        routes.contains("O  10.0.1.0/24  [110/2] via 10.0.0.1, Vlan1"),
        "{routes}"
    );
    let routes = run(&mut r1, &mut lab, &["show ip route"]).remove(0);
    assert!(
        routes.contains("O  10.0.20.0/24  [110/2] via 10.0.0.2"),
        "{routes}"
    );
    assert!(ping(&mut lab, 3, "10.0.1.10").contains("received 4"));
    assert!(ping(&mut lab, 2, "10.0.20.10").contains("received 4"));
}

#[test]
fn a_passive_interface_advertises_its_network_but_forms_no_neighbour() {
    let (mut lab, mut r1, mut sw) = up();
    // The switch's side of the LAN is passive: it still advertises 10.0.0.0/24 but says no hello.
    run(
        &mut sw,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 1",
            "passive-interface vlan1",
            "end",
        ],
    );
    wait(&mut lab, 120);
    let nbr = run(&mut r1, &mut lab, &["show ip ospf neighbor"]).remove(0);
    assert!(!nbr.contains("FULL"), "{nbr}");
    let routes = run(&mut r1, &mut lab, &["show ip route"]).remove(0);
    assert!(!routes.contains("10.0.20.0"), "{routes}");
    let ifs = run(&mut sw, &mut lab, &["show ip ospf interface"]).remove(0);
    assert!(ifs.contains("No Hellos (Passive interface)"), "{ifs}");
}

#[test]
fn mistakes_are_refused() {
    let (mut lab, mut r1, _) = up();
    let outs = run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 2",
            "router ospf 1",
            "network 10.9.0.0 0.0.255.255 area 1",
            "network 10.9.0.0 0.0.255.255 area x.y",
            "no network 10.8.0.0 0.0.255.255 area 0",
            "passive-interface g9/9",
            "no router ospf 7",
        ],
    );
    assert!(outs[1].contains("Only one OSPF process"), "{}", outs[1]);
    assert!(outs[3].is_empty(), "a second area is fine: {}", outs[3]);
    assert!(outs[4].contains("Invalid area"), "{}", outs[4]);
    assert!(outs[5].contains("not configured"), "{}", outs[5]);
    assert!(outs[6].contains("No such interface"), "{}", outs[6]);
    assert!(outs[7].contains("not configured"), "{}", outs[7]);
}

const SETUP: &[&str] = &[
    "configure terminal",
    "router ospf 1",
    "router-id 9.9.9.9",
    "auto-cost reference-bandwidth 1000",
    "network 10.0.1.0 0.0.0.255 area 0",
    "passive-interface g0/1",
    "exit",
    "interface g0/0",
    "ip ospf cost 7",
    "ip ospf priority 0",
    "ip ospf hello-interval 5",
    "ip ospf dead-interval 25",
    "ip ospf network point-to-point",
    "ip mtu 1400",
    "ip ospf mtu-ignore",
    "end",
];

#[test]
fn ospf_config_prints_and_replays_and_survives_a_reload() {
    let (mut lab, mut r1, _) = up();
    run(&mut r1, &mut lab, SETUP);
    let text = running_config(lab.world.device::<Router>(lab.devices[0].id));
    for want in [
        "router ospf 1\n router-id 9.9.9.9\n auto-cost reference-bandwidth 1000\n network 10.0.0.0 0.255.255.255 area 0\n network 10.0.1.0 0.0.0.255 area 0\n passive-interface GigabitEthernet0/1\n!\n",
        " ip ospf cost 7\n ip ospf priority 0\n ip ospf hello-interval 5\n ip ospf dead-interval 25\n ip ospf network point-to-point\n ip mtu 1400\n ip ospf mtu-ignore\n",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    let (mut other, _, _) = up();
    other
        .world
        .with_device::<Router, _>(other.devices[0].id, |r, _| r.factory_reset());
    let mut sh2 = Shell::router(other.devices[0].id);
    let problems = sh2.replay(&mut other.world, &text);
    assert!(problems.is_empty(), "{problems:?}\n{text}");
    assert_eq!(
        running_config(other.world.device::<Router>(other.devices[0].id)),
        text
    );
    // Saved and reloaded.
    run(&mut r1, &mut lab, &["write"]);
    let outs = run(&mut r1, &mut lab, &["reload", ""]);
    assert!(outs[1].contains("System restarted."), "{outs:?}");
    assert_eq!(
        running_config(lab.world.device::<Router>(lab.devices[0].id)),
        text
    );
}

#[test]
fn a_switch_prints_its_ospf_config_too() {
    let (mut lab, _, mut sw) = up();
    run(
        &mut sw,
        &mut lab,
        &[
            "configure terminal",
            "interface vlan 20",
            "ip ospf cost 3",
            "end",
        ],
    );
    let text = switch_config(lab.world.device::<L2Switch>(lab.devices[1].id));
    assert!(
        text.contains("interface Vlan20\n ip address 10.0.20.1 255.255.255.0\n ip ospf cost 3\n"),
        "{text}"
    );
    assert!(
        text.contains("router ospf 1\n network 10.0.0.0 0.255.255.255 area 0\n!\n"),
        "{text}"
    );
}

#[test]
fn removing_ospf_removes_what_it_taught() {
    let (mut lab, mut r1, _) = up();
    wait(&mut lab, 70);
    let routes = run(&mut r1, &mut lab, &["show ip route"]).remove(0);
    assert!(routes.contains("O  10.0.20.0/24"), "{routes}");
    run(
        &mut r1,
        &mut lab,
        &["configure terminal", "no router ospf 1", "end"],
    );
    let routes = run(&mut r1, &mut lab, &["show ip route"]).remove(0);
    assert!(!routes.contains("O  "), "{routes}");
    assert!(!routes.contains("OSPF"), "{routes}");
}

#[test]
fn default_information_prints_replays_and_is_refused_without_a_process() {
    let (mut lab, mut r1, _) = up();
    let outs = run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 1",
            "default-information originate always metric 5 metric-type 1",
            "default-information originate metric 99999999",
            "default-information originate sometimes",
            "end",
        ],
    );
    assert!(outs[2].is_empty(), "{}", outs[2]);
    assert!(outs[3].contains("Invalid option"), "{}", outs[3]);
    assert!(outs[4].contains("Invalid option"), "{}", outs[4]);
    let text = running_config(lab.world.device::<Router>(lab.devices[0].id));
    assert!(
        text.contains(" default-information originate always metric 5 metric-type 1\n"),
        "{text}"
    );
    let (mut other, _, _) = up();
    other
        .world
        .with_device::<Router, _>(other.devices[0].id, |r, _| r.factory_reset());
    let mut sh2 = Shell::router(other.devices[0].id);
    assert!(sh2.replay(&mut other.world, &text).is_empty());
    assert_eq!(
        running_config(other.world.device::<Router>(other.devices[0].id)),
        text
    );
    // Plain `default-information originate` is the default form, and `no` removes it.
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 1",
            "default-information originate",
            "end",
        ],
    );
    let text = running_config(lab.world.device::<Router>(lab.devices[0].id));
    assert!(text.contains(" default-information originate\n"), "{text}");
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 1",
            "no default-information originate",
            "end",
        ],
    );
    let text = running_config(lab.world.device::<Router>(lab.devices[0].id));
    assert!(!text.contains("default-information"), "{text}");
    run(
        &mut r1,
        &mut lab,
        &["configure terminal", "no router ospf 1"],
    );
    let out = run(
        &mut r1,
        &mut lab,
        &["router ospf 1", "default-information originate"],
    );
    assert!(out[1].is_empty());
}

#[test]
fn show_ip_protocols_reports_what_is_configured() {
    let (mut lab, mut r1, _) = up();
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "router ospf 1",
            "passive-interface g0/1",
            "end",
        ],
    );
    let out = run(&mut r1, &mut lab, &["show ip protocols"]).remove(0);
    assert!(out.starts_with("Routing Protocol is \"ospf 1\""), "{out}");
    assert!(out.contains("10.0.0.0 0.255.255.255 area 0"), "{out}");
    assert!(
        out.contains("Passive Interface(s):\n    GigabitEthernet0/1"),
        "{out}"
    );
    assert!(out.ends_with("Distance: (default is 110)"), "{out}");
}
