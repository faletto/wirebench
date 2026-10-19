use engine::chain::blank_chain;
use engine::script::run_script;
use engine::shell::{parse_ifname, running_config, Shell};
use netdev::Router;
use std::path::PathBuf;

fn testkit(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testkit")
        .join(rel)
}

/// Golden transcript for the m1 scenario. `BLESS=1` rewrites it; review the diff by eye.
#[test]
fn m1_static_routes_transcript_is_golden() {
    let script = std::fs::read_to_string(testkit("scenarios/m1_static_routes.script")).unwrap();
    let actual = run_script(&script).unwrap();
    let golden = testkit("scenarios/m1_static_routes.out");
    if std::env::var("BLESS").is_ok() {
        std::fs::write(&golden, &actual).unwrap();
        return;
    }
    assert_eq!(actual, std::fs::read_to_string(golden).unwrap());
}

#[test]
fn script_is_deterministic() {
    let script = std::fs::read_to_string(testkit("scenarios/m1_static_routes.script")).unwrap();
    assert_eq!(run_script(&script).unwrap(), run_script(&script).unwrap());
}

/// Feed `show running-config` back through the CLI on a fresh router: same config out.
#[test]
fn running_config_roundtrip() {
    let mut a = blank_chain();
    let mut sh = Shell::router(a.r1);
    for line in [
        "enable",
        "configure terminal",
        "hostname edge",
        "interface g0/0",
        "ip address 192.168.5.1 255.255.255.128",
        "no shutdown",
        "exit",
        "interface g0/1",
        "ip address 10.0.12.1 255.255.255.252",
        "no shutdown",
        "end",
        "configure terminal",
        "ip route 172.16.0.0 255.255.0.0 10.0.12.2",
        "ip route 0.0.0.0 0.0.0.0 10.0.12.2",
        "end",
    ] {
        sh.exec(&mut a.world, line).unwrap();
    }
    let cfg = running_config(a.world.device::<Router>(a.r1));

    let mut b = blank_chain();
    let mut sh = Shell::router(b.r1);
    sh.exec(&mut b.world, "enable").unwrap();
    sh.exec(&mut b.world, "configure terminal").unwrap();
    for line in cfg.lines() {
        let line = line.trim();
        if line == "!" || line == "end" {
            continue;
        }
        let out = sh.exec(&mut b.world, line).unwrap();
        assert_eq!(out, "", "replaying `{line}` printed: {out}");
    }
    assert_eq!(running_config(b.world.device::<Router>(b.r1)), cfg);
}

#[test]
fn copy_run_start_saves_the_running_config() {
    let mut c = blank_chain();
    let mut sh = Shell::router(c.r1);
    assert!(sh.startup_config().is_none());
    for line in ["enable", "copy running-config startup-config"] {
        sh.exec(&mut c.world, line).unwrap();
    }
    assert!(sh.startup_config().unwrap().starts_with("hostname r1"));
}

#[test]
fn interface_names_accept_prefixes_and_reject_nonsense() {
    for ok in ["g0/1", "gi0/1", "Gig0/1", "GigabitEthernet0/1"] {
        assert_eq!(parse_ifname(ok, 2), Some(1), "{ok}");
    }
    for bad in [
        "0/1",
        "g0/2",
        "e0/1",
        "g1/0",
        "g0/x",
        "gigabitethernetx0/1",
        "g",
    ] {
        assert_eq!(parse_ifname(bad, 2), None, "{bad}");
    }
}

#[test]
fn bad_interface_stays_in_config_mode_with_message() {
    let mut c = blank_chain();
    let mut sh = Shell::router(c.r1);
    for line in ["enable", "configure terminal"] {
        sh.exec(&mut c.world, line).unwrap();
    }
    let out = sh.exec(&mut c.world, "interface g9/9").unwrap();
    assert!(out.starts_with("% No such interface"), "{out}");
    assert_eq!(sh.prompt(&c.world), "r1(config)#");
}
