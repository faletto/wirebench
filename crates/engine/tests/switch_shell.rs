use engine::lab::load_and_build;
use engine::shell::{switch_config, Shell};
use kernel::Trace;
use netdev::L2Switch;

const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "pc1"
template = "host-pc"
pos = [0, 0]
[device.host]
ipv4 = "10.0.0.1/24"

[[device]]
name = "pc2"
template = "host-pc"
pos = [200, 0]
[device.host]
ipv4 = "10.0.0.2/24"

[[device]]
name = "sw"
template = "switch-l2-8"
pos = [100, 0]

[[link]]
a = "pc1:eth0"
b = "sw:Fa0/1"

[[link]]
a = "pc2:eth0"
b = "sw:Fa0/2"
"#;

fn run(sh: &mut Shell, w: &mut engine::World, lines: &[&str]) -> String {
    let mut out = String::new();
    for l in lines {
        out = sh.exec(w, l).unwrap();
    }
    out
}

#[test]
fn mac_table_shows_learned_hosts_with_one_based_ports() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let (pc1, sw) = (lab.devices[0].id, lab.devices[2].id);
    let mut host = Shell::host(pc1);
    let ping = run(&mut host, &mut lab.world, &["ping 10.0.0.2"]);
    assert!(ping.contains("received 4"), "{ping}");
    let mut s = Shell::switch(sw);
    let table = run(
        &mut s,
        &mut lab.world,
        &["enable", "show mac address-table"],
    );
    assert!(table.starts_with("Vlan  Mac Address"), "{table}");
    assert!(table.contains("\n1     "), "learned in VLAN 1: {table}");
    assert!(
        table.contains("Fa0/1") && table.contains("Fa0/2"),
        "{table}"
    );
    assert!(
        !table.contains("Fa0/0"),
        "ports are numbered from 1: {table}"
    );
}

#[test]
fn empty_table_says_so_and_unknown_commands_error() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut s = Shell::switch(lab.devices[2].id);
    let t = run(&mut s, &mut lab.world, &["enable", "sh mac add"]);
    assert!(t.contains("(no addresses learned)"), "{t}");
    let e = run(&mut s, &mut lab.world, &["spanning-tree"]);
    assert!(e.contains('^'), "caret error expected: {e}");
}

#[test]
fn hostname_changes_the_prompt_and_is_modal() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut s = Shell::switch(lab.devices[2].id);
    assert_eq!(s.prompt(&lab.world), "sw>");
    run(
        &mut s,
        &mut lab.world,
        &["enable", "conf t", "hostname access1"],
    );
    assert_eq!(s.prompt(&lab.world), "access1(config)#");
    // hostname only exists in config mode; from privileged mode it must fail.
    run(&mut s, &mut lab.world, &["end"]);
    let e = run(&mut s, &mut lab.world, &["hostname other"]);
    assert!(e.contains('^'), "{e}");
    assert_eq!(s.prompt(&lab.world), "access1#");
}

#[test]
fn saved_hostname_survives_a_lab_round_trip() {
    let with_cfg = LAB.replace(
        "template = \"switch-l2-8\"\npos = [100, 0]",
        "template = \"switch-l2-8\"\npos = [100, 0]\nstartup_config = \"\"\"\nhostname core\n!\nend\n\"\"\"",
    );
    let lab = load_and_build(&with_cfg, Trace::new()).unwrap();
    assert!(lab.warnings.is_empty(), "{:?}", lab.warnings);
    let sw = lab.devices[2].id;
    let name = lab.world.device::<L2Switch>(sw).name.clone();
    assert_eq!(name, "core");
    assert_eq!(
        switch_config(lab.world.device::<L2Switch>(sw)),
        "hostname core\n!\nend"
    );
}

// ---- VLANs ----

fn conf(lab: &mut engine::lab::BuiltLab, lines: &[&str]) -> String {
    let sw = lab.devices[2].id;
    let mut s = Shell::switch(sw);
    let mut all = String::new();
    for l in ["enable", "configure terminal"].iter().chain(lines) {
        let out = s.exec(&mut lab.world, l).unwrap();
        if !out.is_empty() {
            all += &out;
            all += "\n";
        }
    }
    all
}

fn show(lab: &mut engine::lab::BuiltLab, cmd: &str) -> String {
    let mut s = Shell::switch(lab.devices[2].id);
    run(&mut s, &mut lab.world, &["enable", cmd])
}

#[test]
fn vlan_commands_build_the_database_and_assign_ports() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let out = conf(
        &mut lab,
        &[
            "vlan 10",
            "name sales",
            "exit",
            "interface fa0/1",
            "switchport access vlan 10",
            "end",
        ],
    );
    assert_eq!(out, "", "no complaints: {out}");
    let brief = show(&mut lab, "show vlan brief");
    assert!(brief.starts_with("VLAN Name"), "{brief}");
    assert!(brief.contains("1    default"), "{brief}");
    assert!(
        brief.contains("10   sales") && brief.contains("active    Fa0/1"),
        "{brief}"
    );
    assert!(
        brief
            .lines()
            .any(|l| l.starts_with("1 ") && l.contains("Fa0/2")),
        "unmoved ports stay in VLAN 1: {brief}"
    );
}

#[test]
fn assigning_a_missing_vlan_creates_it_and_says_so() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let out = conf(&mut lab, &["interface fa0/2", "switchport access vlan 30"]);
    assert!(out.contains("Creating vlan 30"), "{out}");
    assert!(show(&mut lab, "show vlan brief").contains("VLAN0030"));
}

#[test]
fn trunk_commands_show_up_in_show_interfaces_trunk() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    assert_eq!(show(&mut lab, "show interfaces trunk"), "", "no trunks yet");
    let out = conf(
        &mut lab,
        &[
            "vlan 10",
            "vlan 20",
            "interface fa0/8",
            "switchport mode trunk",
            "switchport trunk native vlan 10",
            "switchport trunk allowed vlan 10,20",
            "switchport trunk allowed vlan add 30-31",
            "switchport trunk allowed vlan remove 31",
        ],
    );
    assert_eq!(out, "", "{out}");
    let t = show(&mut lab, "show interfaces trunk");
    assert!(
        t.contains("Fa0/8") && t.contains("802.1q") && t.contains("trunking"),
        "{t}"
    );
    assert!(
        t.contains("10,20,30"),
        "allowed list after add and remove: {t}"
    );
    assert!(
        t.lines()
            .any(|l| l.starts_with("Fa0/8") && l.trim_end().ends_with("10")),
        "native vlan column: {t}"
    );
    // 30 is allowed but was never created, so it is not active.
    let active = t.split("management domain").nth(1).unwrap();
    assert!(active.contains("10,20") && !active.contains("30"), "{t}");
    let brief = show(&mut lab, "show vlan brief");
    assert!(
        !brief.contains("Fa0/8"),
        "trunk ports are not listed under a VLAN: {brief}"
    );
}

#[test]
fn bad_vlan_input_is_refused_without_changing_anything() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let before = show(&mut lab, "show running-config");
    let out = conf(
        &mut lab,
        &[
            "no vlan 1",
            "no vlan 99",
            "vlan 5000",
            "interface fa0/9",
            "interface fa0/1",
            "switchport trunk allowed vlan 5-3",
            "switchport trunk allowed vlan add banana",
        ],
    );
    assert!(out.contains("Default VLAN 1 may not be deleted"), "{out}");
    assert!(out.contains("VLAN does not exist"), "{out}");
    assert!(out.contains("No such interface: fa0/9"), "{out}");
    assert!(out.matches("Invalid VLAN list").count() == 2, "{out}");
    assert!(
        out.contains("^"),
        "vlan 5000 is a parse error with a caret: {out}"
    );
    assert_eq!(show(&mut lab, "show running-config"), before);
}

#[test]
fn deleting_a_vlan_leaves_its_ports_assigned_but_inactive() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    conf(
        &mut lab,
        &[
            "interface fa0/1",
            "switchport access vlan 10",
            "exit",
            "no vlan 10",
        ],
    );
    let brief = show(&mut lab, "show vlan brief");
    assert!(
        !brief.contains("VLAN0010") && !brief.contains("Fa0/1"),
        "{brief}"
    );
    // Still configured on the port: bringing the VLAN back reconnects it.
    assert!(show(&mut lab, "show running-config").contains("switchport access vlan 10"));
    conf(&mut lab, &["vlan 10"]);
    assert!(show(&mut lab, "show vlan brief").contains("Fa0/1"));
}

#[test]
fn vlan_config_survives_a_save_and_reload() {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    conf(
        &mut lab,
        &[
            "vlan 10",
            "name sales",
            "vlan 20",
            "interface fa0/1",
            "switchport access vlan 10",
            "interface fa0/8",
            "switchport mode trunk",
            "switchport trunk native vlan 20",
            "switchport trunk allowed vlan 10,20",
        ],
    );
    let cfg = show(&mut lab, "show running-config");
    assert!(cfg.contains("vlan 10\n name sales"), "{cfg}");
    assert!(
        cfg.contains("interface FastEthernet0/8\n switchport mode trunk"),
        "{cfg}"
    );
    // Same lab, config stored as the startup config of the switch, loaded from scratch.
    let with_cfg = LAB.replace(
        "template = \"switch-l2-8\"\npos = [100, 0]",
        &format!("template = \"switch-l2-8\"\npos = [100, 0]\nstartup_config = '''\n{cfg}\n'''"),
    );
    let mut again = load_and_build(&with_cfg, Trace::new()).unwrap();
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    assert_eq!(show(&mut again, "show running-config"), cfg);
    assert_eq!(
        show(&mut again, "show interfaces trunk"),
        show(&mut lab, "show interfaces trunk")
    );
}

#[test]
fn help_and_abbreviations_work_for_switchport() {
    let lab = load_and_build(LAB, Trace::new()).unwrap();
    let mut lab = lab;
    let sw = lab.devices[2].id;
    let mut s = Shell::switch(sw);
    run(&mut s, &mut lab.world, &["enable", "conf t", "int fa0/1"]);
    let help = s.help("switchport trunk allowed vlan ");
    for word in ["add", "remove", "all", "none"] {
        assert!(help.contains(word), "{help}");
    }
    // Keywords win over the list parameter, and unambiguous prefixes expand.
    let out = run(
        &mut s,
        &mut lab.world,
        &["sw mo tr", "sw tr al vl none", "sw tr al vl all"],
    );
    assert_eq!(out, "");
    assert!(show(&mut lab, "show interfaces trunk").contains("1-4094"));
}
