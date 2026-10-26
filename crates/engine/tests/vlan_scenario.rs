//! Golden transcripts of the M2 labs, configured only through console commands.
//! Re-bless deliberately: `BLESS=1 cargo test -p engine --test vlan_scenario`.

use engine::lab::{load_and_build, LabKind};
use engine::shell::Shell;
use kernel::Trace;
use std::path::Path;

struct Scenario {
    lab: &'static str,
    script: &'static str,
    golden: &'static str,
}

const VLANS: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_vlans.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_vlans.script"),
    golden: "../../testkit/scenarios/m2_vlans.out",
};

const INTER_VLAN: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_inter_vlan.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_inter_vlan.script"),
    golden: "../../testkit/scenarios/m2_inter_vlan.out",
};

const SVI: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_svi.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_svi.script"),
    golden: "../../testkit/scenarios/m2_svi.out",
};

fn transcript(sc: &Scenario) -> String {
    let mut lab = load_and_build(sc.lab, Trace::new()).expect("lab builds");
    assert!(lab.warnings.is_empty(), "{:?}", lab.warnings);
    let mut shells: Vec<(String, Shell)> = lab
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
    let mut out = String::new();
    let mut cur = 0;
    for line in sc.script.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('@') {
            cur = shells
                .iter()
                .position(|(n, _)| n == name)
                .expect("known device");
            out += &format!("\n== {name}\n");
            continue;
        }
        let sh = &mut shells[cur].1;
        out += &format!("{}{line}\n", sh.prompt(&lab.world));
        if let Some(text) = sh.exec(&mut lab.world, line) {
            if !text.is_empty() {
                out += &text;
                out += "\n";
            }
        }
    }
    out
}

fn check_golden(sc: &Scenario) {
    let got = transcript(sc);
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(Path::new(sc.golden), &got).unwrap();
    }
    let want = std::fs::read_to_string(sc.golden)
        .expect("golden file; bless with BLESS=1")
        .replace("\r\n", "\n");
    assert_eq!(got, want, "transcript changed; re-bless if intended");
}

#[test]
fn the_two_vlan_lab_matches_its_golden_transcript() {
    check_golden(&VLANS);
}

#[test]
fn the_router_on_a_stick_lab_matches_its_golden_transcript() {
    check_golden(&INTER_VLAN);
}

#[test]
fn the_svi_lab_matches_its_golden_transcript() {
    check_golden(&SVI);
}

#[test]
fn the_transcript_shows_isolation_and_the_trunk_cut() {
    let t = transcript(&VLANS);
    // pc1 to pc2 (VLAN 10) answers four times, twice (before and after cutting VLAN 20).
    assert_eq!(t.matches("Sent 4, received 4").count(), 3, "{t}");
    // pc1 to pc4 crosses VLANs and times out; pc3 to pc4 fails once the trunk drops VLAN 20.
    assert_eq!(t.matches("Sent 4, received 0").count(), 2, "{t}");
}

#[test]
fn the_router_is_the_only_way_between_vlans() {
    let t = transcript(&INTER_VLAN);
    // Routed ping, gateway ping, then (after the port is brought back) the routed ping again.
    assert_eq!(t.matches("Sent 4, received 4").count(), 3, "{t}");
    // Subinterface 20 shut, then the parent shut: both fail.
    assert_eq!(t.matches("Sent 4, received 0").count(), 2, "{t}");
}
