//! TFTP, end to end: routers and a switch back up their configuration to a PC and restore it.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::Trace;

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_syslog.lab.toml");
const PC1: usize = 0;
const R1: usize = 1;
const SW1: usize = 2;
const R2: usize = 3;

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let id = lab.devices[dev].id;
    let mut sh = match dev {
        PC1 => Shell::host(id),
        SW1 => Shell::switch(id),
        _ => Shell::router(id),
    };
    let mut out = String::new();
    let enable = if dev == PC1 { None } else { Some("enable") };
    for l in enable.iter().chain(lines) {
        out = sh.exec(&mut lab.world, l).unwrap();
    }
    out
}

fn lab() -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    for (d, ip) in [(R1, "10.0.0.1"), (R2, "10.0.0.2")] {
        console(
            &mut lab,
            d,
            &[
                "configure terminal",
                "interface g0/0",
                &format!("ip address {ip} 255.255.255.0"),
                "no shutdown",
                "end",
            ],
        );
    }
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "end",
        ],
    );
    console(&mut lab, PC1, &["tftpd start"]);
    lab
}

#[test]
fn a_router_backs_up_to_the_pc_and_restores_from_it() {
    let mut lab = lab();
    let out = console(
        &mut lab,
        R1,
        &["copy running-config tftp://10.0.0.10/r1-confg"],
    );
    assert!(
        out.starts_with("Writing r1-confg !") && out.contains("bytes copied in"),
        "{out}"
    );
    let list = console(&mut lab, PC1, &["tftpd list"]);
    assert!(
        list.contains("1 file.") && list.contains("r1-confg"),
        "{list}"
    );
    let text = console(&mut lab, PC1, &["tftpd show r1-confg"]);
    assert!(
        text.contains("hostname r1") && text.contains("ip address 10.0.0.1 255.255.255.0"),
        "{text}"
    );
    // Change the router, then bring the saved configuration back.
    console(
        &mut lab,
        R1,
        &["configure terminal", "hostname other", "end"],
    );
    let out = console(
        &mut lab,
        R1,
        &["copy tftp://10.0.0.10/r1-confg running-config"],
    );
    assert!(
        out.contains("[OK - ") && out.contains("bytes copied in"),
        "{out}"
    );
    let run = console(&mut lab, R1, &["show running-config"]);
    assert!(run.starts_with("hostname r1\n"), "{run}");
}

#[test]
fn a_file_of_many_blocks_arrives_whole() {
    let mut lab = lab();
    let mut lines = vec!["configure terminal".to_string()];
    for n in 1..=60 {
        lines.push(format!("ip route 192.168.{n}.0 255.255.255.0 10.0.0.9"));
    }
    lines.push("end".to_string());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    console(&mut lab, R1, &refs);
    let run = console(&mut lab, R1, &["show running-config"]);
    assert!(run.len() > 1500, "{}", run.len());
    let out = console(&mut lab, R1, &["copy running-config tftp://10.0.0.10/big"]);
    assert!(
        out.contains(&format!("{} bytes copied", run.len() + 1)),
        "{out}"
    );
    let held = console(&mut lab, PC1, &["tftpd show big"]);
    assert_eq!(held, run.trim_end());
    // And back through a second router, which has none of those routes.
    let out = console(&mut lab, R2, &["copy tftp://10.0.0.10/big startup-config"]);
    assert!(
        out.contains(&format!("{} bytes copied", run.len() + 1)),
        "{out}"
    );
    let start = console(&mut lab, R2, &["show startup-config"]);
    assert!(
        start.contains("ip route 192.168.60.0 255.255.255.0 10.0.0.9"),
        "{start}"
    );
}

#[test]
fn errors_name_what_went_wrong() {
    let mut lab = lab();
    let out = console(
        &mut lab,
        R1,
        &["copy tftp://10.0.0.10/nothing running-config"],
    );
    assert_eq!(
        out,
        "%Error opening tftp://10.0.0.10/nothing (File not found)"
    );
    // Nobody at that address: the client gives up after its retries.
    let t0 = lab.world.kernel.now();
    let out = console(&mut lab, R1, &["copy running-config tftp://10.0.0.77/x"]);
    assert_eq!(out, "%Error opening tftp://10.0.0.77/x (Timed out)");
    assert!((lab.world.kernel.now().0 - t0.0) / 1_000_000_000 >= 10);
    // A stopped server says nothing either.
    console(&mut lab, PC1, &["tftpd stop"]);
    let out = console(&mut lab, R1, &["copy running-config tftp://10.0.0.10/x"]);
    assert!(out.contains("(Timed out)"), "{out}");
    for bad in [
        "10.0.0.10/x",
        "tftp://10.0.0.10",
        "tftp://10.0.0/x",
        "tftp://host/x",
        "tftp://10.0.0.10/",
    ] {
        let out = console(&mut lab, R1, &[&format!("copy running-config {bad}")]);
        assert!(out.contains("tftp://<ip address>/<file>"), "{bad}: {out}");
    }
    let out = console(&mut lab, R1, &["copy startup-config tftp://10.0.0.10/x"]);
    assert!(out.contains("startup-config is not present"), "{out}");
}

#[test]
fn saving_the_configuration_still_works_beside_the_tftp_forms() {
    let mut lab = lab();
    let out = console(&mut lab, R1, &["copy running-config startup-config"]);
    assert_eq!(out, "Configuration saved.");
    assert!(console(&mut lab, R1, &["show startup-config"]).contains("hostname r1"));
    let out = console(
        &mut lab,
        R1,
        &["copy startup-config tftp://10.0.0.10/saved"],
    );
    assert!(out.contains("bytes copied"), "{out}");
}

#[test]
fn a_switch_is_a_client_through_its_vlan_interface() {
    let mut lab = lab();
    console(
        &mut lab,
        SW1,
        &["configure terminal", "hostname lab-switch", "end"],
    );
    let out = console(
        &mut lab,
        SW1,
        &["copy running-config tftp://10.0.0.10/sw1-confg"],
    );
    assert!(out.contains("bytes copied"), "{out}");
    assert!(console(&mut lab, PC1, &["tftpd show sw1-confg"]).contains("hostname lab-switch"));
    console(&mut lab, SW1, &["configure terminal", "hostname x", "end"]);
    console(
        &mut lab,
        SW1,
        &["copy tftp://10.0.0.10/sw1-confg running-config"],
    );
    assert!(console(&mut lab, SW1, &["show running-config"]).starts_with("hostname lab-switch\n"));
}

#[test]
fn the_pc_deletes_files_and_logs_what_it_did() {
    let mut lab = lab();
    console(&mut lab, R1, &["copy running-config tftp://10.0.0.10/a"]);
    console(
        &mut lab,
        R2,
        &["copy tftp://10.0.0.10/missing running-config"],
    );
    let log = console(&mut lab, PC1, &["tftpd log"]);
    assert!(
        log.contains("received a (") && log.contains("asked for missing: not found"),
        "{log}"
    );
    assert_eq!(console(&mut lab, PC1, &["tftpd delete a"]), "");
    assert!(console(&mut lab, PC1, &["tftpd delete a"]).contains("does not exist"));
    assert!(console(&mut lab, PC1, &["tftpd list"]).contains("0 files."));
}
