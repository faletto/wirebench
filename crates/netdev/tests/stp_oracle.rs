//! Our spanning tree against mstpd's.
//!
//! A script (the format is in `testkit/mstpd-oracle/harness.c`) describes bridges, links and what
//! happens to them. It is run here through our [`Stp`] and, when `MSTPD_ORACLE` names the binary
//! `testkit/mstpd-oracle/build.sh` builds, through mstpd's protocol core. Both wire bridges together
//! the same way (BPDUs delivered at once, in order; one second at a time, bridge by bridge) and
//! print the same report: each bridge's root, cost and root port, and each port's role and state.
//! The scripts only ask for a report once the network has had time to settle, so the timing of
//! the two may differ; where it ends up may not.
//!
//! Without the oracle, the scripts in `testkit/mstpd-oracle/cases` are still checked against the
//! answers mstpd gave when they were written (`.expected`, refreshed with `MSTPD_BLESS=1`).

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use netdev::{Stp, StpAction, StpMode, StpRole, StpState};
use proptest::prelude::*;
use wire::{BridgeId, MacAddr};

struct Net {
    bridges: Vec<Stp>,
    /// `(bridge, port index)` pairs, each cabled to the other.
    links: Vec<[(usize, usize); 2]>,
    up: Vec<bool>,
    seconds: u32,
    out: String,
}

impl Net {
    fn peer(&self, b: usize, p: usize) -> Option<(usize, usize)> {
        self.links.iter().zip(&self.up).find_map(|(e, up)| {
            if !up {
                None
            } else if e[0] == (b, p) {
                Some(e[1])
            } else if e[1] == (b, p) {
                Some(e[0])
            } else {
                None
            }
        })
    }

    fn deliver(&mut self, from: usize, actions: Vec<StpAction>) {
        let mut queue: std::collections::VecDeque<(usize, StpAction)> =
            actions.into_iter().map(|a| (from, a)).collect();
        let mut steps = 0;
        while let Some((b, a)) = queue.pop_front() {
            let StpAction::Send { port, bpdu } = a else {
                continue;
            };
            let Some((pb, pp)) = self.peer(b, port) else {
                continue;
            };
            steps += 1;
            assert!(steps < 1_000_000, "BPDUs never stop");
            let reply = self.bridges[pb].receive(pp, &bpdu);
            queue.extend(reply.into_iter().map(|a| (pb, a)));
        }
    }

    fn dump(&mut self) {
        let mut o = format!("t={}\n", self.seconds);
        for (i, br) in self.bridges.iter().enumerate() {
            let root = br.root();
            let rp = br
                .root_port()
                .map_or("-".to_string(), |p| (p + 1).to_string());
            let m = root.root.mac.0;
            writeln!(
                o,
                "b{i} root {}.{:02x}{:02x}{:02x}{:02x}{:02x}{:02x} cost {} rootport {rp}",
                root.root.priority, m[0], m[1], m[2], m[3], m[4], m[5], root.cost
            )
            .unwrap();
            for p in 0..br.n_ports() {
                let v = br.port(p);
                let role = match v.role {
                    StpRole::Disabled => "disabled",
                    StpRole::Root => "root",
                    StpRole::Designated => "designated",
                    StpRole::Alternate => "alternate",
                    StpRole::Backup => "backup",
                };
                let state = match v.state {
                    StpState::Discarding | StpState::Listening => "discarding",
                    StpState::Learning => "learning",
                    StpState::Forwarding => "forwarding",
                };
                writeln!(o, "b{i}p{} {role} {state}", p + 1).unwrap();
            }
        }
        self.out += &o;
    }
}

/// Runs a script through our spanning tree and returns the report.
fn ours(script: &str) -> String {
    let mut mode = StpMode::Rapid;
    let mut n = Net {
        bridges: vec![],
        links: vec![],
        up: vec![],
        seconds: 0,
        out: String::new(),
    };
    for (no, line) in script.lines().enumerate() {
        let line = line.split('#').next().unwrap();
        let w: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| -> usize {
            w.get(i)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| panic!("line {}: {line}", no + 1))
        };
        let Some(cmd) = w.first() else { continue };
        match *cmd {
            "mode" => {
                mode = match w.get(1) {
                    Some(&"stp") => StpMode::Stp,
                    Some(&"rstp") => StpMode::Rapid,
                    _ => panic!("line {}: {line}", no + 1),
                }
            }
            "bridge" => {
                let hex = w[2];
                let mut mac = [0u8; 6];
                for (k, b) in mac.iter_mut().enumerate() {
                    *b = u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).unwrap();
                }
                let id = BridgeId {
                    priority: num(1) as u16,
                    mac: MacAddr(mac),
                };
                n.bridges.push(Stp::new(mode, id, num(3)));
            }
            "cost" => {
                let (b, p) = (num(1), num(2) - 1);
                let a = n.bridges[b].set_cost(p, num(3) as u32);
                n.deliver(b, a);
            }
            "portprio" => {
                let (b, p) = (num(1), num(2) - 1);
                let a = n.bridges[b].set_port_priority(p, num(3) as u8);
                n.deliver(b, a);
            }
            "prio" => {
                let b = num(1);
                let id = BridgeId {
                    priority: num(2) as u16,
                    mac: n.bridges[b].bridge().mac,
                };
                let a = n.bridges[b].set_bridge(id);
                n.deliver(b, a);
            }
            "link" => {
                n.links.push([(num(1), num(2) - 1), (num(3), num(4) - 1)]);
                n.up.push(false);
            }
            "up" | "down" => {
                let l = num(1);
                let up = *cmd == "up";
                n.up[l] = up;
                for (b, p) in n.links[l] {
                    let a = n.bridges[b].set_enabled(p, up);
                    n.deliver(b, a);
                }
            }
            "run" => {
                for _ in 0..num(1) {
                    n.seconds += 1;
                    for b in 0..n.bridges.len() {
                        let a = n.bridges[b].tick();
                        n.deliver(b, a);
                    }
                }
            }
            "dump" => n.dump(),
            _ => panic!("line {}: unknown command {cmd}", no + 1),
        }
    }
    n.out
}

fn oracle_path() -> Option<PathBuf> {
    std::env::var_os("MSTPD_ORACLE").map(PathBuf::from)
}

/// Runs a script through mstpd.
fn mstpd(bin: &Path, script: &str) -> String {
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("running {}: {e}", bin.display()));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "mstpd oracle failed: {}\nscript:\n{script}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The first report line where two reports differ, with the reports in full.
fn differences(script: &str, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs {
        return None;
    }
    let first = ours
        .lines()
        .zip(theirs.lines())
        .find(|(a, b)| a != b)
        .map_or("(lengths differ)".to_string(), |(a, b)| {
            format!("ours  : {a}\nmstpd : {b}")
        });
    Some(format!(
        "{first}\n--- script\n{script}--- ours\n{ours}--- mstpd\n{theirs}"
    ))
}

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testkit/mstpd-oracle/cases")
}

#[test]
fn recorded_cases_match_mstpd() {
    let oracle = oracle_path();
    let bless = std::env::var_os("MSTPD_BLESS").is_some();
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(cases_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "script"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty());
    let mut failures = vec![];
    for path in scripts {
        let script = std::fs::read_to_string(&path).unwrap();
        let expected_path = path.with_extension("expected");
        let expected = match &oracle {
            Some(bin) => {
                let answer = mstpd(bin, &script);
                if bless {
                    std::fs::write(&expected_path, &answer).unwrap();
                } else {
                    let recorded = std::fs::read_to_string(&expected_path).unwrap_or_default();
                    assert_eq!(
                        recorded,
                        answer,
                        "{} no longer matches mstpd; MSTPD_BLESS=1 rewrites it",
                        expected_path.display()
                    );
                }
                answer
            }
            None => std::fs::read_to_string(&expected_path)
                .unwrap_or_else(|e| panic!("{}: {e}", expected_path.display())),
        };
        if let Some(d) = differences(&script, &ours(&script), &expected) {
            failures.push(format!("{}\n{d}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

// ---- random networks ----

#[derive(Clone, Debug)]
struct Bridge {
    priority: u16,
    mac_low: u8,
    ports: usize,
}

/// A random network and a random history, as a script.
fn network() -> impl Strategy<Value = String> {
    let mode = prop_oneof![Just("stp"), Just("rstp")];
    let priorities = prop_oneof![
        4 => Just(32768u16),
        1 => Just(4096u16),
        1 => Just(28672u16),
        1 => Just(61440u16),
    ];
    let bridges = (2usize..=6).prop_flat_map(move |n| {
        (
            proptest::collection::vec((priorities.clone(), 1usize..=4), n),
            Just((1..=n as u8).collect::<Vec<u8>>()).prop_shuffle(),
        )
            .prop_map(|(v, macs)| {
                v.into_iter()
                    .zip(macs)
                    .map(|((priority, ports), mac_low)| Bridge {
                        priority,
                        mac_low,
                        ports,
                    })
                    .collect::<Vec<_>>()
            })
    });
    (mode, bridges).prop_flat_map(|(mode, bridges)| {
        let n = bridges.len();
        let max_ports: usize = 4;
        let cables = proptest::collection::vec((0..n, 0..max_ports, 0..n, 0..max_ports), 1..=n * 2);
        let costs = proptest::collection::vec(
            (
                0..n,
                0..max_ports,
                prop_oneof![Just(4u32), Just(19), Just(100), 1u32..200],
            ),
            0..=3,
        );
        let port_prios =
            proptest::collection::vec((0..n, 0..max_ports, (0u32..16).prop_map(|p| p * 16)), 0..=2);
        // What happens once it has settled: links pulled or restored, with a few seconds between.
        let flaps = proptest::collection::vec((0..64usize, 0u32..5), 0..=6);
        let new_prio = proptest::option::of((0..n, (0u16..16).prop_map(|p| p * 4096)));
        (
            Just(mode),
            Just(bridges),
            cables,
            costs,
            port_prios,
            flaps,
            new_prio,
        )
            .prop_map(
                |(mode, bridges, cables, costs, port_prios, flaps, new_prio)| {
                    script(
                        mode,
                        &bridges,
                        &cables,
                        &costs,
                        &port_prios,
                        &flaps,
                        new_prio,
                    )
                },
            )
    })
}

fn script(
    mode: &str,
    bridges: &[Bridge],
    cables: &[(usize, usize, usize, usize)],
    costs: &[(usize, usize, u32)],
    port_prios: &[(usize, usize, u32)],
    flaps: &[(usize, u32)],
    new_prio: Option<(usize, u16)>,
) -> String {
    let mut s = format!("mode {mode}\n");
    for b in bridges {
        writeln!(
            s,
            "bridge {} 0200000000{:02x} {}",
            b.priority, b.mac_low, b.ports
        )
        .unwrap();
    }
    for &(b, p, c) in costs {
        if p < bridges[b].ports {
            writeln!(s, "cost {b} {} {c}", p + 1).unwrap();
        }
    }
    for &(b, p, pr) in port_prios {
        if p < bridges[b].ports {
            writeln!(s, "portprio {b} {} {pr}", p + 1).unwrap();
        }
    }
    // Each port takes at most one cable; a cable may join two ports of one bridge.
    let mut used = std::collections::HashSet::new();
    let mut links = 0;
    for &(a, pa, b, pb) in cables {
        if pa >= bridges[a].ports || pb >= bridges[b].ports || (a, pa) == (b, pb) {
            continue;
        }
        if used.contains(&(a, pa)) || used.contains(&(b, pb)) {
            continue;
        }
        used.insert((a, pa));
        used.insert((b, pb));
        writeln!(s, "link {a} {} {b} {}", pa + 1, pb + 1).unwrap();
        links += 1;
    }
    for l in 0..links {
        writeln!(s, "up {l}").unwrap();
    }
    s += "run 90\ndump\n";
    if links > 0 && (!flaps.is_empty() || new_prio.is_some()) {
        let mut up = vec![true; links];
        for &(l, wait) in flaps {
            let l = l % links;
            up[l] = !up[l];
            writeln!(s, "{} {l}", if up[l] { "up" } else { "down" }).unwrap();
            if wait > 0 {
                writeln!(s, "run {wait}").unwrap();
            }
        }
        if let Some((b, p)) = new_prio {
            writeln!(s, "prio {b} {p}").unwrap();
        }
        s += "run 90\ndump\n";
    }
    s
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("MSTPD_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(256),
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_networks_settle_as_mstpd_does(script in network()) {
        let Some(bin) = oracle_path() else {
            return Ok(());
        };
        let theirs = mstpd(&bin, &script);
        if let Some(d) = differences(&script, &ours(&script), &theirs) {
            prop_assert!(false, "{}", d);
        }
    }
}
