use engine::lab::{build, load_and_build, BuildError, LabKind};
use kernel::Trace;
use netdev::{Host, Router};
use std::path::PathBuf;
use wire::Ipv4Addr;

fn lab_text() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../labs/m1_static_routes.lab.toml"),
    )
    .unwrap()
}

fn parse(text: &str) -> labfile::LabFile {
    labfile::load(text).unwrap().lab
}

#[test]
fn shipped_lab_builds_with_no_warnings_and_pings_end_to_end() {
    let mut b = load_and_build(&lab_text(), Trace::keeping()).unwrap();
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    assert_eq!(b.title, "Two routers, static routes");
    assert_eq!(
        b.devices
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["r1", "r2", "pc1", "pc2"]
    );
    assert_eq!(b.links.len(), 3);
    assert_eq!(b.annotations.len(), 1);
    let pc1 = b.devices[2].id;
    b.world
        .with_device::<Host, _>(pc1, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 2, 2)));
    b.world.run_to_idle();
    assert_eq!(b.world.device::<Host>(pc1).ping_results.len(), 1);
}

#[test]
fn startup_config_is_applied_through_the_cli() {
    let b = load_and_build(&lab_text(), Trace::new()).unwrap();
    let r1 = b.world.device::<Router>(b.devices[0].id);
    assert_eq!(r1.name, "r1");
    assert_eq!(r1.statics().len(), 1);
    assert!(r1
        .interfaces()
        .iter()
        .all(|i| !i.shutdown && i.addr.is_some()));
    assert_eq!(r1.fib().routes().count(), 3, "two connected + one static");
}

#[test]
fn building_twice_gives_the_same_trace() {
    let run = || {
        let mut b = load_and_build(&lab_text(), Trace::keeping()).unwrap();
        let pc1 = b.devices[2].id;
        b.world
            .with_device::<Host, _>(pc1, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 2, 2)));
        b.world.run_to_idle();
        b.world.trace_hash()
    };
    assert_eq!(run(), run());
}

#[test]
fn rejected_config_lines_become_warnings_not_errors() {
    let text = lab_text().replace(
        " ip address 10.0.1.1 255.255.255.0",
        " ip adress 10.0.1.1 255.255.255.0",
    );
    let b = load_and_build(&text, Trace::new()).unwrap();
    assert_eq!(b.warnings.len(), 1, "{:?}", b.warnings);
    assert!(
        b.warnings[0].starts_with("r1 config line"),
        "{}",
        b.warnings[0]
    );
    assert!(b.warnings[0].contains("ip adress"));
}

fn err_of(edit: impl Fn(&mut labfile::LabFile)) -> BuildError {
    let mut lab = parse(&lab_text());
    edit(&mut lab);
    build(&lab, Trace::new()).unwrap_err()
}

#[test]
fn bad_labs_are_rejected_with_specific_errors() {
    assert!(matches!(
        err_of(|l| l.devices[0].template = "router-cisco-2911".into()),
        BuildError::UnknownTemplate { .. }
    ));
    assert_eq!(
        err_of(|l| l.devices[1].name = "r1".into()),
        BuildError::DuplicateDevice("r1".into())
    );
    assert_eq!(
        err_of(|l| l.links[0].a = "pc1".into()),
        BuildError::BadEndpoint("pc1".into())
    );
    assert!(matches!(
        err_of(|l| l.links[0].a = "nope:eth0".into()),
        BuildError::UnknownDevice { .. }
    ));
    assert!(matches!(
        err_of(|l| l.links[0].b = "r1:Gi0/9".into()),
        BuildError::UnknownPort { .. }
    ));
    assert!(matches!(
        err_of(|l| l.links[0].a = "pc1:eth1".into()),
        BuildError::UnknownPort { .. }
    ));
    // r1:Gi0/1 is already cabled to r2.
    assert!(matches!(
        err_of(|l| l.links[2].a = "r1:Gi0/1".into()),
        BuildError::PortInUse { .. }
    ));
    assert!(matches!(
        err_of(|l| l.devices[2].host.as_mut().unwrap().ipv4 = "10.0.1.2".into()),
        BuildError::BadHostAddress { .. }
    ));
    assert!(matches!(
        err_of(|l| l.devices[2].host.as_mut().unwrap().ipv4 = "10.0.1.2/40".into()),
        BuildError::BadHostAddress { .. }
    ));
}

#[test]
fn switch_template_cables_and_forwards() {
    let text = r#"format = "labsim-lab"
version = 1
[[device]]
name = "sw"
template = "switch-l2-8"
pos = [100, 100]
[[device]]
name = "a"
template = "host-pc"
pos = [0, 0]
[device.host]
ipv4 = "10.0.0.1/24"
[[device]]
name = "b"
template = "host-pc"
pos = [200, 0]
[device.host]
ipv4 = "10.0.0.2/24"
[[link]]
a = "a:eth0"
b = "sw:Fa0/1"
[[link]]
a = "b:eth0"
b = "sw:FastEthernet0/8"
"#;
    let mut b = load_and_build(text, Trace::new()).unwrap();
    assert_eq!(b.devices[0].kind, LabKind::Switch);
    let a = b.devices[1].id;
    b.world
        .with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    b.world.run_to_idle();
    assert_eq!(b.world.device::<Host>(a).ping_results.len(), 1);
    // Out-of-range switch ports are rejected (Fa0/9 on an 8-port switch, and Fa0/0).
    for bad in ["sw:Fa0/9", "sw:Fa0/0"] {
        let mut lab = parse(text);
        lab.links[0].b = bad.into();
        assert!(
            matches!(
                build(&lab, Trace::new()),
                Err(BuildError::UnknownPort { .. })
            ),
            "{bad}"
        );
    }
}

#[test]
fn empty_lab_builds() {
    let b = load_and_build("format = \"labsim-lab\"\nversion = 1\n", Trace::new()).unwrap();
    assert!(b.devices.is_empty());
}
