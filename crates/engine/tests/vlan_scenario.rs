//! Golden transcript of the two-VLAN trunk lab, configured only through console commands.
//! Re-bless deliberately: `BLESS=1 cargo test -p engine --test vlan_scenario`.

use engine::lab::{load_and_build, LabKind};
use engine::shell::Shell;
use kernel::Trace;
use std::path::Path;

const LAB: &str = include_str!("../../../testkit/scenarios/m2_vlans.lab.toml");
const SCRIPT: &str = include_str!("../../../testkit/scenarios/m2_vlans.script");
const GOLDEN: &str = "../../testkit/scenarios/m2_vlans.out";

fn transcript() -> String {
    let mut lab = load_and_build(LAB, Trace::new()).expect("lab builds");
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
    for line in SCRIPT.lines() {
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

#[test]
fn the_two_vlan_lab_matches_its_golden_transcript() {
    let got = transcript();
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(Path::new(GOLDEN), &got).unwrap();
    }
    let want = std::fs::read_to_string(GOLDEN)
        .expect("golden file; bless with BLESS=1")
        .replace("\r\n", "\n");
    assert_eq!(got, want, "transcript changed; re-bless if intended");
}

#[test]
fn the_transcript_shows_isolation_and_the_trunk_cut() {
    let t = transcript();
    // pc1 to pc2 (VLAN 10) answers four times, twice (before and after cutting VLAN 20).
    assert_eq!(t.matches("Sent 4, received 4").count(), 3, "{t}");
    // pc1 to pc4 crosses VLANs and times out; pc3 to pc4 fails once the trunk drops VLAN 20.
    assert_eq!(t.matches("Sent 4, received 0").count(), 2, "{t}");
}
