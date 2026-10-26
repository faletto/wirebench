//! DHCP from the console: server and relay configuration, ipconfig, saved labs.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::{running_config, switch_config, Shell};
use kernel::Trace;
use netdev::{DhcpState, Host, L2Switch, Router};

/// pc1 and pc2 on sw1; r1 on sw1's last port.
const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "pc1"
template = "host-pc"
pos = [0, 0]

[[device]]
name = "pc2"
template = "host-pc"
pos = [0, 100]

[[device]]
name = "sw1"
template = "switch-l2-8"
pos = [100, 0]

[[device]]
name = "r1"
template = "router-2g"
pos = [200, 0]

[[link]]
a = "pc1:eth0"
b = "sw1:Fa0/1"

[[link]]
a = "pc2:eth0"
b = "sw1:Fa0/2"

[[link]]
a = "sw1:Fa0/8"
b = "r1:Gi0/0"
"#;

fn lab() -> BuiltLab {
    load_and_build(LAB, Trace::new()).unwrap()
}

fn run(sh: &mut Shell, lab: &mut BuiltLab, lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .map(|l| sh.exec(&mut lab.world, l).unwrap())
        .collect()
}

fn router_shell(lab: &BuiltLab) -> Shell {
    Shell::router(lab.devices[3].id)
}

const SERVER: &[&str] = &[
    "enable",
    "configure terminal",
    "interface g0/0",
    "ip address 10.0.0.1 255.255.255.0",
    "no shutdown",
    "exit",
    "ip dhcp excluded-address 10.0.0.1 10.0.0.9",
    "ip dhcp pool lan",
    "network 10.0.0.0 255.255.255.0",
    "default-router 10.0.0.1",
    "dns-server 8.8.8.8 8.8.4.4",
    "lease 0 8",
    "end",
];

fn server_up(lab: &mut BuiltLab) -> Shell {
    let mut sh = router_shell(lab);
    let outs = run(&mut sh, lab, SERVER);
    assert!(outs.iter().all(String::is_empty), "{outs:?}");
    sh
}

#[test]
fn a_pc_asks_for_an_address_with_ipconfig_dhcp() {
    let mut lab = lab();
    server_up(&mut lab);
    let mut pc = Shell::host(lab.devices[0].id);
    let out = run(&mut pc, &mut lab, &["ipconfig dhcp"]).remove(0);
    assert!(out.contains("Address : 10.0.0.10"), "{out}");
    assert!(out.contains("Mask    : 255.255.255.0"), "{out}");
    assert!(out.contains("Gateway : 10.0.0.1"), "{out}");
    assert!(out.contains("DNS     : 8.8.8.8 8.8.4.4"), "{out}");
    assert!(
        out.contains("DHCP    : bound, server 10.0.0.1, lease 08:00:00 left"),
        "{out}"
    );
    let again = run(&mut pc, &mut lab, &["ipconfig"]).remove(0);
    assert!(again.contains("Address : 10.0.0.10"), "{again}");
    let ping = run(&mut pc, &mut lab, &["ping 10.0.0.1"]).remove(0);
    assert!(ping.contains("received 4"), "{ping}");
}

#[test]
fn without_a_server_the_pc_says_so_and_shows_it_is_still_looking() {
    let mut lab = lab();
    let mut pc = Shell::host(lab.devices[0].id);
    let out = run(&mut pc, &mut lab, &["ipconfig dhcp"]).remove(0);
    assert!(out.starts_with("No DHCP server answered"), "{out}");
    let state = run(&mut pc, &mut lab, &["ipconfig"]).remove(0);
    assert!(state.contains("Address : 0.0.0.0"), "{state}");
    assert!(state.contains("DHCP    : looking for a server"), "{state}");
}

#[test]
fn release_renew_and_a_static_address() {
    let mut lab = lab();
    let mut sh = server_up(&mut lab);
    let mut pc = Shell::host(lab.devices[0].id);
    run(&mut pc, &mut lab, &["ipconfig dhcp"]);
    assert_eq!(
        run(&mut pc, &mut lab, &["ipconfig release"])[0],
        "Address released."
    );
    let shown = run(&mut pc, &mut lab, &["ipconfig"]).remove(0);
    assert!(
        shown.contains("Address : 0.0.0.0") && shown.contains("DHCP    : released"),
        "{shown}"
    );
    let bindings = run(&mut sh, &mut lab, &["show ip dhcp binding"]).remove(0);
    assert!(bindings.contains("(no bindings)"), "{bindings}");
    let renewed = run(&mut pc, &mut lab, &["ipconfig renew"]).remove(0);
    assert!(renewed.contains("Address : 10.0.0.10"), "{renewed}");
    // Typing an address turns DHCP off.
    run(&mut pc, &mut lab, &["ipconfig set 10.0.0.77 255.255.255.0"]);
    let shown = run(&mut pc, &mut lab, &["ipconfig"]).remove(0);
    assert!(
        shown.contains("Address : 10.0.0.77") && !shown.contains("DHCP"),
        "{shown}"
    );
    assert!(!lab.world.device::<Host>(lab.devices[0].id).dhcp.enabled);
}

#[test]
fn show_and_clear_commands_report_the_server_state() {
    let mut lab = lab();
    let mut sh = server_up(&mut lab);
    let mut pc1 = Shell::host(lab.devices[0].id);
    let mut pc2 = Shell::host(lab.devices[1].id);
    run(&mut pc1, &mut lab, &["ipconfig dhcp"]);
    run(&mut pc2, &mut lab, &["ipconfig dhcp"]);
    let b = run(&mut sh, &mut lab, &["enable", "show ip dhcp binding"]).remove(1);
    assert!(b.contains("10.0.0.10") && b.contains("10.0.0.11"), "{b}");
    assert!(b.contains("0102.0000.0000.01"), "client ids: {b}");
    assert!(b.contains("Automatic") && b.contains("07:5"), "{b}");
    let p = run(&mut sh, &mut lab, &["show ip dhcp pool"]).remove(0);
    assert!(p.contains("Pool lan :") && p.contains("10.0.0.0/24"), "{p}");
    assert!(
        p.contains("Addresses in use : 2") && p.contains("0 days 8 hours 0 minutes"),
        "{p}"
    );
    assert!(p.contains("DNS servers      : 8.8.8.8 8.8.4.4"), "{p}");
    run(&mut sh, &mut lab, &["clear ip dhcp binding 10.0.0.10"]);
    let b = run(&mut sh, &mut lab, &["show ip dhcp binding"]).remove(0);
    assert!(!b.contains("10.0.0.10") && b.contains("10.0.0.11"), "{b}");
    run(&mut sh, &mut lab, &["clear ip dhcp binding *"]);
    assert!(run(&mut sh, &mut lab, &["show ip dhcp binding"])[0].contains("(no bindings)"));
    assert_eq!(
        run(&mut sh, &mut lab, &["show ip dhcp conflict"])[0],
        "No conflicts."
    );
}

#[test]
fn service_dhcp_and_the_pool_can_be_turned_off() {
    let mut lab = lab();
    let mut sh = server_up(&mut lab);
    run(
        &mut sh,
        &mut lab,
        &["configure terminal", "no service dhcp", "end"],
    );
    let mut pc = Shell::host(lab.devices[0].id);
    assert!(run(&mut pc, &mut lab, &["ipconfig dhcp"])[0].starts_with("No DHCP server"));
    let text = run(&mut sh, &mut lab, &["show running-config"]).remove(0);
    assert!(text.contains("no service dhcp"), "{text}");
    run(
        &mut sh,
        &mut lab,
        &[
            "configure terminal",
            "service dhcp",
            "no ip dhcp pool lan",
            "end",
        ],
    );
    assert!(!run(&mut sh, &mut lab, &["show running-config"])[0].contains("ip dhcp pool"));
}

#[test]
fn bad_dhcp_input_is_refused_and_changes_nothing() {
    let mut lab = lab();
    let mut sh = router_shell(&lab);
    run(&mut sh, &mut lab, &["enable", "configure terminal"]);
    let outs = run(
        &mut sh,
        &mut lab,
        &[
            "ip dhcp excluded-address 10.0.0.9 10.0.0.1",
            "no ip dhcp excluded-address 10.0.0.1",
            "no ip dhcp pool nope",
            "interface g0/0",
            "no ip helper-address 1.2.3.4",
            "ip dhcp pool p",
            "network 10.0.0.0 255.255.255.0 extra",
            "lease 999",
            "dns-server 1.2.3.4 5.6.7.8 9.9.9.9 8.8.8.8",
        ],
    );
    assert!(outs[0].contains("below the first"), "{outs:?}");
    assert!(outs[1].contains("not excluded"), "{outs:?}");
    assert!(outs[2].contains("No such pool"), "{outs:?}");
    assert!(outs[4].contains("not configured"), "{outs:?}");
    for i in [6, 7, 8] {
        assert!(
            outs[i].contains('^'),
            "line {i} should be a parse error: {outs:?}"
        );
    }
}

#[test]
fn server_config_and_helper_addresses_print_and_replay() {
    let mut lab = lab();
    let mut sh = server_up(&mut lab);
    run(
        &mut sh,
        &mut lab,
        &[
            "configure terminal",
            "interface g0/1",
            "ip address 10.9.0.1 255.255.255.252",
            "ip helper-address 10.9.0.2",
            "ip helper-address 10.9.0.3",
            "exit",
            "ip dhcp excluded-address 10.0.0.200",
            "ip dhcp pool second",
            "network 172.16.0.0 255.255.0.0",
            "lease 3 4 5",
            "end",
        ],
    );
    let text = run(&mut sh, &mut lab, &["show running-config"]).remove(0);
    for want in [
        "ip dhcp excluded-address 10.0.0.1 10.0.0.9\n",
        "ip dhcp excluded-address 10.0.0.200\n",
        "ip dhcp pool lan\n network 10.0.0.0 255.255.255.0\n default-router 10.0.0.1\n dns-server 8.8.8.8 8.8.4.4\n lease 0 8\n",
        "ip dhcp pool second\n network 172.16.0.0 255.255.0.0\n lease 3 4 5\n",
        " ip address 10.9.0.1 255.255.255.252\n ip helper-address 10.9.0.2\n ip helper-address 10.9.0.3\n",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    // Replaying the printed text on a fresh router gives the same config back.
    let mut other = lab_fresh();
    let mut sh2 = router_shell(&other);
    let problems = sh2.replay(&mut other.world, &text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        running_config(other.world.device::<Router>(other.devices[3].id)),
        text
    );
}

fn lab_fresh() -> BuiltLab {
    lab()
}

#[test]
fn a_saved_config_and_a_reload_keep_the_pools() {
    let mut lab = lab();
    let mut sh = server_up(&mut lab);
    run(&mut sh, &mut lab, &["write"]);
    let outs = run(&mut sh, &mut lab, &["reload", ""]);
    assert!(outs[1].contains("System restarted."), "{outs:?}");
    let sh = &mut router_shell(&lab);
    run(sh, &mut lab, &["enable"]);
    let pools = run(sh, &mut lab, &["show ip dhcp pool"]).remove(0);
    assert!(
        pools.contains("Pool lan"),
        "the saved config came back: {pools}"
    );
}

#[test]
fn a_layer_3_switch_serves_dhcp_on_an_svi() {
    let mut lab = lab();
    let mut sw = Shell::switch(lab.devices[2].id);
    let outs = run(
        &mut sw,
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.1 255.255.255.0",
            "exit",
            "ip dhcp excluded-address 10.0.0.1 10.0.0.9",
            "ip dhcp pool lan",
            "network 10.0.0.0 255.255.255.0",
            "default-router 10.0.0.1",
            "end",
        ],
    );
    assert!(outs.iter().all(String::is_empty), "{outs:?}");
    let mut pc = Shell::host(lab.devices[1].id);
    let out = run(&mut pc, &mut lab, &["ipconfig dhcp"]).remove(0);
    assert!(
        out.contains("Address : 10.0.0.10") && out.contains("server 10.0.0.1"),
        "{out}"
    );
    // The switch prints its server config and keeps it across a reload from the saved config.
    let text = switch_config(lab.world.device::<L2Switch>(lab.devices[2].id));
    assert!(
        text.contains("ip dhcp pool lan") && text.contains("interface Vlan1"),
        "{text}"
    );
    let mut other = lab_fresh();
    let mut sw2 = Shell::switch(other.devices[2].id);
    assert_eq!(sw2.replay(&mut other.world, &text), Vec::<String>::new());
    assert_eq!(
        switch_config(other.world.device::<L2Switch>(other.devices[2].id)),
        text
    );
}

#[test]
fn a_helper_address_works_on_an_svi_too() {
    let mut lab = lab();
    let mut sw = Shell::switch(lab.devices[2].id);
    let outs = run(
        &mut sw,
        &mut lab,
        &[
            "enable",
            "configure terminal",
            "interface vlan 10",
            "ip address 10.0.10.1 255.255.255.0",
            "ip helper-address 10.0.99.2",
            "no ip helper-address 10.0.99.3",
            "end",
        ],
    );
    assert!(outs[5].contains("not configured"), "{outs:?}");
    let text = run(&mut sw, &mut lab, &["show running-config"]).remove(0);
    assert!(
        text.contains(
            "interface Vlan10\n ip address 10.0.10.1 255.255.255.0\n ip helper-address 10.0.99.2\n"
        ),
        "{text}"
    );
}

#[test]
fn a_lab_file_can_say_a_pc_uses_dhcp_and_it_asks_when_time_runs() {
    let text = LAB.replace(
        "name = \"pc1\"\ntemplate = \"host-pc\"\npos = [0, 0]",
        "name = \"pc1\"\ntemplate = \"host-pc\"\npos = [0, 0]\n[device.host]\nipv4 = \"dhcp\"",
    );
    let mut built = load_and_build(&text, Trace::new()).unwrap();
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    let pc = built.devices[0].id;
    assert_eq!(
        built.world.device::<Host>(pc).dhcp.state,
        DhcpState::Selecting
    );
    assert_eq!(built.world.device::<Host>(pc).ip.to_string(), "0.0.0.0");
    // Configure the server afterwards, then let the clock run: the client's retries find it.
    let mut sh = router_shell(&built);
    run(&mut sh, &mut built, SERVER);
    built.world.run_until(kernel::SimTime::from_secs(10));
    assert_eq!(built.world.device::<Host>(pc).ip.to_string(), "10.0.0.10");
    // Saving writes "dhcp", never the lease.
    let (w, topo, _) = built.into_parts();
    let saved = labfile::save(&topo.snapshot(&w));
    assert!(saved.contains("ipv4 = \"dhcp\""), "{saved}");
    assert!(!saved.contains("10.0.0.10"), "{saved}");
}
