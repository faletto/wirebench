//! Golden transcripts of the M2 labs, configured only through console commands.
//! Re-bless deliberately: `BLESS=1 cargo test -p engine --test vlan_scenario`.

use engine::lab::{load_and_build, LabKind};
use engine::shell::{Shell, PASSWORD_PROMPT};
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

const MGMT: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_mgmt.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_mgmt.script"),
    golden: "../../testkit/scenarios/m2_mgmt.out",
};

const DHCP: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_dhcp.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_dhcp.script"),
    golden: "../../testkit/scenarios/m2_dhcp.out",
};

const TRACE: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_traceroute.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_traceroute.script"),
    golden: "../../testkit/scenarios/m2_traceroute.out",
};

const STP: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_stp.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_stp.script"),
    golden: "../../testkit/scenarios/m2_stp.out",
};

const DTP: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_dtp.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_dtp.script"),
    golden: "../../testkit/scenarios/m2_dtp.out",
};

const SVI: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m2_svi.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m2_svi.script"),
    golden: "../../testkit/scenarios/m2_svi.out",
};

const ACL: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3_acl.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3_acl.script"),
    golden: "../../testkit/scenarios/m3_acl.out",
};

const OSPF: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3_ospf.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3_ospf.script"),
    golden: "../../testkit/scenarios/m3_ospf.out",
};

const OSPF_AREAS: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3_ospf_areas.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3_ospf_areas.script"),
    golden: "../../testkit/scenarios/m3_ospf_areas.out",
};

const EXIT_DEMO: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3_exit_demo.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3_exit_demo.script"),
    golden: "../../testkit/scenarios/m3_exit_demo.out",
};

const PSEC: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3b_psec.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3b_psec.script"),
    golden: "../../testkit/scenarios/m3b_psec.out",
};

const ETHERCHANNEL: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3b_etherchannel.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3b_etherchannel.script"),
    golden: "../../testkit/scenarios/m3b_etherchannel.out",
};

const DISCOVERY: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3b_discovery.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3b_discovery.script"),
    golden: "../../testkit/scenarios/m3b_discovery.out",
};

const NTP: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3b_ntp.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3b_ntp.script"),
    golden: "../../testkit/scenarios/m3b_ntp.out",
};

const FHRP: Scenario = Scenario {
    lab: include_str!("../../../testkit/scenarios/m3b_fhrp.lab.toml"),
    script: include_str!("../../../testkit/scenarios/m3b_fhrp.script"),
    golden: "../../testkit/scenarios/m3b_fhrp.out",
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
    for raw in sc.script.lines() {
        // `# ` starts a comment (a lone `#` is text: a banner delimiter). An empty line is skipped;
        // `<enter>` sends an empty line, as pressing Return at a [confirm] prompt does.
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with("# ") {
            continue;
        }
        // `%wait 30` lets thirty seconds of simulated time pass (spanning tree needs it).
        if let Some(secs) = line.strip_prefix("%wait ") {
            let to = lab.world.kernel.now() + kernel::SimTime::from_secs(secs.parse().unwrap());
            lab.world.run_until(to);
            out += &format!("\n-- {secs} s pass\n");
            continue;
        }
        let line = if line == "<enter>" { "" } else { line };
        if let Some(name) = raw.trim_end().strip_prefix('@') {
            let (name, fresh) = match name.strip_suffix('!') {
                Some(n) => (n, true),
                None => (name, false),
            };
            cur = shells
                .iter()
                .position(|(n, _)| n == name)
                .expect("known device");
            out += &format!("\n== {name}{}\n", if fresh { " (new session)" } else { "" });
            if fresh {
                let d = &lab.devices[cur];
                let mut sh = match d.kind {
                    LabKind::Host => Shell::host(d.id),
                    LabKind::Router => Shell::router(d.id),
                    LabKind::Switch => Shell::switch(d.id),
                };
                let greeting = sh.greeting(&lab.world);
                if !greeting.is_empty() {
                    out += &greeting;
                    out += "\n";
                }
                shells[cur].1 = sh;
            }
            continue;
        }
        let sh = &mut shells[cur].1;
        let prompt = sh.prompt(&lab.world);
        // Typed passwords are not echoed, as in the app.
        if prompt == PASSWORD_PROMPT {
            out += &format!("{prompt}\n");
        } else {
            out += &format!("{prompt}{line}\n");
        }
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
fn the_management_lab_matches_its_golden_transcript() {
    check_golden(&MGMT);
}

#[test]
fn the_dhcp_lab_matches_its_golden_transcript() {
    check_golden(&DHCP);
}

#[test]
fn the_traceroute_lab_matches_its_golden_transcript() {
    check_golden(&TRACE);
}

#[test]
fn the_spanning_tree_lab_matches_its_golden_transcript() {
    check_golden(&STP);
}

#[test]
fn the_dtp_lab_matches_its_golden_transcript() {
    check_golden(&DTP);
}

#[test]
fn the_svi_lab_matches_its_golden_transcript() {
    check_golden(&SVI);
}

#[test]
fn the_acl_lab_matches_its_golden_transcript() {
    check_golden(&ACL);
}

#[test]
fn the_ospf_lab_matches_its_golden_transcript() {
    check_golden(&OSPF);
}

#[test]
fn the_ospf_areas_lab_matches_its_golden_transcript() {
    check_golden(&OSPF_AREAS);
}

#[test]
fn the_port_security_lab_matches_its_golden_transcript() {
    check_golden(&PSEC);
}

#[test]
fn the_etherchannel_lab_matches_its_golden_transcript() {
    check_golden(&ETHERCHANNEL);
}

#[test]
fn the_discovery_lab_matches_its_golden_transcript() {
    check_golden(&DISCOVERY);
}

#[test]
fn the_ntp_lab_matches_its_golden_transcript() {
    check_golden(&NTP);
}

#[test]
fn the_fhrp_lab_matches_its_golden_transcript() {
    check_golden(&FHRP);
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

#[test]
fn the_m3a_exit_demo_matches_its_golden_transcript() {
    check_golden(&EXIT_DEMO);
}

/// What the exit demo is supposed to show, said in assertions as well as in the golden file.
#[test]
fn the_m3a_exit_demo_shows_what_it_claims() {
    let t = transcript(&EXIT_DEMO);
    let after =
        |marker: &str| -> &str { &t[t.find(marker).unwrap_or_else(|| panic!("{marker}"))..] };
    // OSPF carried r2's default route to the other routers.
    assert!(t.contains("O E2  0.0.0.0/0  [110/1] via 10.0.0.2"), "{t}");
    // All three PCs reach the server, two of them through one shared address.
    assert_eq!(t.matches("Sent 4, received 4").count(), 3, "{t}");
    let nat = after("r2#show ip nat translations");
    let shared = nat
        .lines()
        .take(5)
        .filter(|l| l.starts_with("icmp 203.0.113.1:"))
        .count();
    assert_eq!(shared, 2, "{nat}");
    assert!(
        nat.lines().nth(2).unwrap().contains("203.0.113.5")
            && nat.lines().nth(2).unwrap().contains("10.0.1.10"),
        "{nat}"
    );
    // The internet reaches pc1's public address and nothing else inside.
    let isp = after("isp#ping 203.0.113.5");
    assert!(isp.contains("Success rate is 100 percent (5/5)"), "{isp}");
    assert!(isp.contains("Success rate is 0 percent (0/5)"), "{isp}");
    // The VTY access list lets the corporate network in and keeps the internet out.
    assert!(after("C:\\>telnet 10.0.0.2").contains("r2>"), "{t}");
    assert!(
        after("isp#telnet 203.0.113.1")
            .starts_with("isp#telnet 203.0.113.1\n% Connection refused by remote host"),
        "{t}"
    );
    // A session through the NAT: the far router sees r2's outside address, not r1's.
    let brief = after("isp>show tcp brief");
    let line = brief.lines().nth(2).unwrap();
    assert!(
        line.contains("203.0.113.2.23")
            && line.contains("203.0.113.1.")
            && !line.contains("10.0.0.1"),
        "{line}"
    );
}
