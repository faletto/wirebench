//! NAT on a layer 3 switch: translation between VLAN interfaces.
//!
//! pc1 - sw1 (Fa0/1 in VLAN 10) ... sw1 (Fa0/2 in VLAN 20) - r2:g0/0, r2:g0/1 - srv

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{switch_config, Shell};
use kernel::{SimTime, Trace};
use netdev::L2Switch;

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
name = "sw1"
template = "switch-l2-8"
pos = [100, 0]

[[device]]
name = "r2"
template = "router-2g"
pos = [200, 0]

[[device]]
name = "srv"
template = "host-pc"
pos = [300, 0]
[device.host]
ipv4 = "198.51.100.10/24"
gateway = "198.51.100.1"

[[link]]
a = "pc1:eth0"
b = "sw1:Fa0/1"
[[link]]
a = "sw1:Fa0/2"
b = "r2:Gi0/0"
[[link]]
a = "r2:Gi0/1"
b = "srv:eth0"
"#;

const PC1: usize = 0;
const SW1: usize = 1;
const R2: usize = 2;

fn sh(lab: &BuiltLab, dev: usize) -> Shell {
    let id = lab.devices[dev].id;
    match dev {
        PC1 => Shell::host(id),
        SW1 => Shell::switch(id),
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

fn lab() -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    run(
        &mut lab,
        SW1,
        &[
            "enable",
            "configure terminal",
            "vlan 10",
            "exit",
            "vlan 20",
            "exit",
            "interface fa0/1",
            "switchport mode access",
            "switchport access vlan 10",
            "exit",
            "interface fa0/2",
            "switchport mode access",
            "switchport access vlan 20",
            "exit",
            "ip routing",
            "interface vlan 10",
            "ip address 10.0.1.1 255.255.255.0",
            "ip nat inside",
            "no shutdown",
            "exit",
            "interface vlan 20",
            "ip address 203.0.113.1 255.255.255.252",
            "ip nat outside",
            "no shutdown",
            "exit",
            "ip route 0.0.0.0 0.0.0.0 203.0.113.2",
            "access-list 1 permit 10.0.1.0 0.0.0.255",
            "ip nat inside source list 1 interface vlan20 overload",
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
            "end",
        ],
    );
    // The switch's ports come up through spanning tree.
    let t = lab.world.kernel.now() + SimTime::from_secs(40);
    lab.world.run_until(t);
    lab
}

#[test]
fn a_layer_3_switch_translates_between_its_vlan_interfaces() {
    let mut lab = lab();
    let out = run(&mut lab, PC1, &["ping 198.51.100.10"]);
    assert!(out.contains("received 4"), "{out}");
    let t = run(&mut lab, SW1, &["enable", "show ip nat translations"]);
    assert!(t.contains("203.0.113.1:") && t.contains("10.0.1.10"), "{t}");
    let stats = run(&mut lab, SW1, &["show ip nat statistics"]);
    assert!(stats.contains("Outside interfaces:\nVlan20"), "{stats}");
    assert!(stats.contains("Inside interfaces:\nVlan10"), "{stats}");
}

#[test]
fn the_switchs_nat_configuration_is_in_its_running_config() {
    let mut lab = lab();
    let cfg = switch_config(lab.world.device::<L2Switch>(lab.devices[SW1].id));
    for line in [
        " ip nat inside",
        " ip nat outside",
        "ip nat inside source list 1 interface Vlan20 overload",
    ] {
        assert!(cfg.contains(line), "missing {line:?} in\n{cfg}");
    }
    // The saved configuration replays to the same thing.
    run(
        &mut lab,
        SW1,
        &[
            "enable",
            "copy running-config startup-config",
            "reload",
            "no",
            "yes",
        ],
    );
    let after = switch_config(lab.world.device::<L2Switch>(lab.devices[SW1].id));
    assert_eq!(after, cfg);
}

#[test]
fn nat_is_refused_on_a_switch_port_that_is_not_routed() {
    let mut lab = lab();
    let out = run(
        &mut lab,
        SW1,
        &[
            "enable",
            "configure terminal",
            "interface fa0/3",
            "ip nat inside",
        ],
    );
    assert!(out.contains("routed interfaces"), "{out}");
}
