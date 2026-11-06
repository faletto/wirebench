//! Access lists from the console: forwarding decisions, counters, and saving and replaying.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{running_config, Shell};
use kernel::Trace;
use netdev::Router;

/// pc1 - r1:g0/0, r1:g0/1 - pc2.
const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "pc1"
template = "host-pc"
pos = [0, 0]

[[device]]
name = "r1"
template = "router-2g"
pos = [100, 0]

[[device]]
name = "pc2"
template = "host-pc"
pos = [200, 0]

[[link]]
a = "pc1:eth0"
b = "r1:Gi0/0"

[[link]]
a = "r1:Gi0/1"
b = "pc2:eth0"
"#;

fn run(sh: &mut Shell, lab: &mut BuiltLab, lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .map(|l| sh.exec(&mut lab.world, l).unwrap_or_default())
        .collect()
}

/// A lab with addresses on everything; the router's shell is returned in privileged mode.
fn lab_up() -> (BuiltLab, Shell) {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut r1 = Shell::router(lab.devices[1].id);
    run(
        &mut r1,
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "interface g0/0",
            "ip address 10.0.1.1 255.255.255.0",
            "no shutdown",
            "exit",
            "interface g0/1",
            "ip address 10.0.2.1 255.255.255.0",
            "no shutdown",
            "end",
        ],
    );
    let mut pc1 = Shell::host(lab.devices[0].id);
    run(
        &mut pc1,
        &mut lab,
        &[
            "ipconfig set 10.0.1.2 255.255.255.0",
            "ipconfig gateway 10.0.1.1",
        ],
    );
    let mut pc2 = Shell::host(lab.devices[2].id);
    run(
        &mut pc2,
        &mut lab,
        &[
            "ipconfig set 10.0.2.2 255.255.255.0",
            "ipconfig gateway 10.0.2.1",
        ],
    );
    (lab, r1)
}

fn ping(lab: &mut BuiltLab, pc: usize, dst: &str) -> String {
    let mut sh = Shell::host(lab.devices[pc].id);
    run(&mut sh, lab, &[&format!("ping {dst}")]).remove(0)
}

fn config(lab: &BuiltLab) -> String {
    running_config(lab.world.device::<Router>(lab.devices[1].id))
}

#[test]
fn an_inbound_list_also_judges_traffic_to_the_router_itself() {
    let (mut lab, mut r1) = lab_up();
    assert!(ping(&mut lab, 0, "10.0.1.1").contains("received 4"));
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "access-list 100 deny icmp any host 10.0.1.1",
            "access-list 100 permit ip any any",
            "interface g0/0",
            "ip access-group 100 in",
            "end",
        ],
    );
    assert!(ping(&mut lab, 0, "10.0.1.1").contains("received 0"));
    // Other destinations still pass.
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("received 4"));
}

#[test]
fn an_outbound_list_leaves_the_routers_own_packets_alone() {
    let (mut lab, mut r1) = lab_up();
    // Nothing is permitted outward, yet the router reaches pc2 and pc1 cannot.
    let outs = run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "access-list 20 deny any",
            "interface g0/1",
            "ip access-group 20 out",
            "end",
            "ping 10.0.2.2",
        ],
    );
    assert!(
        outs[5].contains("Success rate is 100 percent"),
        "{}",
        outs[5]
    );
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("received 0"));
    // The refusal reached pc1 as an ICMP error, so it knows at once.
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("Destination unreachable"));
}

#[test]
fn the_list_is_looked_up_by_name_when_a_packet_arrives() {
    let (mut lab, mut r1) = lab_up();
    // Applied before it exists: permits everything. Created later: takes effect at once.
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "interface g0/0",
            "ip access-group LATE in",
            "end",
        ],
    );
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("received 4"));
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "ip access-list extended LATE",
            "deny ip any any",
            "end",
        ],
    );
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("received 0"));
    // Deleting it lets everything through again.
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "no ip access-list extended LATE",
            "end",
        ],
    );
    assert!(ping(&mut lab, 0, "10.0.2.2").contains("received 4"));
}

#[test]
fn counters_count_and_clear() {
    let (mut lab, mut r1) = lab_up();
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "access-list 101 permit icmp any any echo",
            "access-list 101 deny ip any any",
            "interface g0/0",
            "ip access-group 101 in",
            "end",
        ],
    );
    ping(&mut lab, 0, "10.0.2.2");
    let shown = run(&mut r1, &mut lab, &["show access-lists 101"]).remove(0);
    assert!(
        shown.contains("10 permit icmp any any echo (4 matches)"),
        "{shown}"
    );
    assert!(shown.ends_with("20 deny ip any any"), "{shown}");
    run(&mut r1, &mut lab, &["clear access-list counters 101"]);
    let shown = run(&mut r1, &mut lab, &["show access-lists"]).remove(0);
    assert!(!shown.contains("match"), "{shown}");
}

#[test]
fn a_list_cannot_change_kind_and_a_number_must_be_in_range() {
    let (mut lab, mut r1) = lab_up();
    let outs = run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "access-list 150 permit ip any any",
            "ip access-list standard 150",
            "access-list 150 permit 10.0.0.1",
            "access-list 500 permit ip any any",
            "no ip access-list extended 99",
            "ip access-list extended E",
            "10 permit ip any any",
            "10 deny ip any any",
            "no 99",
            "no permit tcp any any",
        ],
    );
    assert!(outs[2].contains("other kind"), "{}", outs[2]);
    assert!(outs[3].contains("Invalid input"), "{}", outs[3]);
    assert!(
        outs[4].contains("no such access list number"),
        "{}",
        outs[4]
    );
    assert!(outs[5].contains("No such access list"), "{}", outs[5]);
    assert!(outs[8].contains("Duplicate sequence"), "{}", outs[8]);
    assert!(outs[9].contains("sequence number"), "{}", outs[9]);
    assert!(outs[10].contains("No such entry"), "{}", outs[10]);
}

const SETUP: &[&str] = &[
    "configure terminal",
    "access-list 1 permit 10.0.1.0 0.0.0.255",
    "access-list 101 remark web in, pings out",
    "access-list 101 permit tcp 10.0.1.0 0.0.0.255 any eq www",
    "access-list 101 permit icmp any any echo-reply",
    "ip access-list extended EDGE",
    "remark keep this",
    "5 deny udp host 10.0.1.9 range 1000 2000 any neq 53",
    "permit ip any any",
    "interface g0/0",
    "ip access-group 101 in",
    "ip access-group EDGE out",
    "exit",
    "line vty 0 4",
    "access-class 1 in",
    "end",
];

#[test]
fn lists_print_as_commands_and_replay_to_the_same_config() {
    let (mut lab, mut r1) = lab_up();
    run(&mut r1, &mut lab, SETUP);
    let text = config(&lab);
    for want in [
        "access-list 1 permit 10.0.1.0 0.0.0.255\n",
        "access-list 101 remark web in, pings out\n",
        "access-list 101 permit tcp 10.0.1.0 0.0.0.255 any eq www\n",
        "ip access-list extended EDGE\n 5 deny udp host 10.0.1.9 range 1000 2000 any neq domain\n 10 remark keep this\n permit ip any any\n",
        " ip access-group 101 in\n ip access-group EDGE out\n",
        "line vty 0 4\n access-class 1 in\n",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    // Replay onto a router that has been reset to nothing.
    let (mut other, _) = lab_up();
    other
        .world
        .with_device::<Router, _>(other.devices[1].id, |r, _| r.factory_reset());
    let mut sh2 = Shell::router(other.devices[1].id);
    let problems = sh2.replay(&mut other.world, &text);
    assert!(problems.is_empty(), "{problems:?}\n{text}");
    assert_eq!(config(&other), text);
}

#[test]
fn a_saved_config_and_a_reload_keep_the_lists() {
    let (mut lab, mut r1) = lab_up();
    run(&mut r1, &mut lab, SETUP);
    let before = config(&lab);
    run(&mut r1, &mut lab, &["write"]);
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "no access-list 1",
            "no ip access-list extended EDGE",
            "end",
        ],
    );
    assert_ne!(config(&lab), before);
    // Unsaved changes: decline to save, then confirm the reload.
    let outs = run(&mut r1, &mut lab, &["reload", "no", ""]);
    assert!(outs[2].contains("System restarted."), "{outs:?}");
    assert_eq!(config(&lab), before);
}

#[test]
fn a_numbered_list_edited_by_sequence_prints_as_a_block_and_replays() {
    let (mut lab, mut r1) = lab_up();
    run(
        &mut r1,
        &mut lab,
        &[
            "configure terminal",
            "access-list 110 permit ip host 10.0.1.2 any",
            "access-list 110 deny ip any any",
            "ip access-list extended 110",
            "15 permit icmp any any",
            "end",
        ],
    );
    let text = config(&lab);
    assert!(
        text.contains("ip access-list extended 110\n permit ip host 10.0.1.2 any\n 15 permit icmp any any\n 20 deny ip any any\n"),
        "{text}"
    );
    let (mut other, _) = lab_up();
    other
        .world
        .with_device::<Router, _>(other.devices[1].id, |r, _| r.factory_reset());
    let mut sh2 = Shell::router(other.devices[1].id);
    let problems = sh2.replay(&mut other.world, &text);
    assert!(problems.is_empty(), "{problems:?}\n{text}");
    assert_eq!(config(&other), text);
}
