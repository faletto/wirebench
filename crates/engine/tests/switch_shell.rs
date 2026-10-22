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
    assert!(table.starts_with("Mac Address"), "{table}");
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
