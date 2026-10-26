//! Spanning tree machines wired together directly: no frames, no clock, just BPDUs passed along
//! point to point links at zero delay and a one second tick.

use netdev::{Stp, StpAction, StpMode, StpRole, StpState};
use proptest::prelude::*;
use wire::{BridgeId, MacAddr};

struct Net {
    bridges: Vec<Stp>,
    /// `(bridge, port)` pairs, each cabled to the other.
    links: Vec<[(usize, usize); 2]>,
    down: Vec<bool>,
    flushes: Vec<usize>,
    seconds: u32,
}

fn id(priority: u16, n: usize) -> BridgeId {
    BridgeId {
        priority,
        mac: MacAddr([2, 0, 0, 0, 0, n as u8 + 1]),
    }
}

impl Net {
    /// `ports[i]` ports on bridge `i`; `prio[i]` its priority.
    fn new(mode: StpMode, prio: &[u16], ports: usize, links: &[[(usize, usize); 2]]) -> Net {
        let mut n = Net {
            bridges: prio
                .iter()
                .enumerate()
                .map(|(i, p)| Stp::new(mode, id(*p, i), ports))
                .collect(),
            links: links.to_vec(),
            down: vec![false; links.len()],
            flushes: vec![0; prio.len()],
            seconds: 0,
        };
        for l in 0..n.links.len() {
            n.set_link(l, true);
        }
        n
    }

    fn peer(&self, b: usize, p: usize) -> Option<(usize, usize)> {
        self.links.iter().enumerate().find_map(|(l, e)| {
            if self.down[l] {
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
        let mut queue: Vec<(usize, Vec<StpAction>)> = vec![(from, actions)];
        let mut steps = 0;
        while let Some((b, acts)) = queue.pop() {
            for a in acts {
                match a {
                    StpAction::Flush { .. } => self.flushes[b] += 1,
                    StpAction::Send { port, bpdu } => {
                        if let Some((pb, pp)) = self.peer(b, port) {
                            steps += 1;
                            assert!(steps < 100_000, "BPDUs never stop");
                            let reply = self.bridges[pb].receive(pp, &bpdu);
                            queue.insert(0, (pb, reply));
                        }
                    }
                }
            }
        }
        let e = self.live_edges();
        assert!(
            !has_cycle(self.bridges.len(), &e),
            "loop once the BPDUs settled at t={}: {e:?}\n{}",
            self.seconds,
            self.dump()
        );
    }

    fn set_link(&mut self, l: usize, up: bool) {
        self.down[l] = !up;
        for (b, p) in self.links[l] {
            let a = self.bridges[b].set_enabled(p, up);
            self.deliver(b, a);
        }
    }

    fn run(&mut self, secs: u32) {
        for _ in 0..secs {
            self.seconds += 1;
            for b in 0..self.bridges.len() {
                let a = self.bridges[b].tick();
                self.deliver(b, a);
            }
        }
    }

    fn dump(&self) -> String {
        let mut o = String::new();
        for (b, br) in self.bridges.iter().enumerate() {
            for p in 0..br.n_ports() {
                let v = br.port(p);
                o += &format!(
                    "  b{b}p{p} {:?} {:?} peer {:?}\n",
                    v.role,
                    v.state,
                    self.peer(b, p)
                );
            }
        }
        o
    }

    fn forwarding(&self, b: usize, p: usize) -> bool {
        self.bridges[b].forwards(p)
    }

    /// Links whose both ends pass traffic: the edges frames can really cross.
    fn live_edges(&self) -> Vec<(usize, usize)> {
        self.links
            .iter()
            .enumerate()
            .filter(|(l, e)| !self.down[*l] && e.iter().all(|(b, p)| self.forwarding(*b, *p)))
            .map(|(_, e)| (e[0].0, e[1].0))
            .collect()
    }

    /// Components of the cabled graph.
    fn components(&self) -> Vec<usize> {
        let n = self.bridges.len();
        let mut c: Vec<usize> = (0..n).collect();
        loop {
            let mut changed = false;
            for (l, e) in self.links.iter().enumerate() {
                if self.down[l] {
                    continue;
                }
                let m = c[e[0].0].min(c[e[1].0]);
                for x in [e[0].0, e[1].0] {
                    if c[x] != m {
                        c[x] = m;
                        changed = true;
                    }
                }
            }
            if !changed {
                return c;
            }
        }
    }

    /// The invariants of a settled network, per connected component: one root (the best bridge id),
    /// every other bridge has a root port, the live edges form a spanning tree, and each bridge's
    /// path cost to the root is the cheapest there is.
    fn assert_settled(&self, cost: u32) {
        let comp = self.components();
        let n = self.bridges.len();
        for c in 0..n {
            let members: Vec<usize> = (0..n).filter(|b| comp[*b] == c).collect();
            if members.is_empty() {
                continue;
            }
            let root = *members
                .iter()
                .min_by_key(|b| self.bridges[**b].bridge())
                .unwrap();
            for &b in &members {
                for p in 0..self.bridges[b].n_ports() {
                    let v = self.bridges[b].port(p);
                    let want = match v.role {
                        StpRole::Root | StpRole::Designated => StpState::Forwarding,
                        _ => StpState::Discarding,
                    };
                    assert_eq!(v.state, want, "bridge {b} port {p} is {:?}", v.role);
                }
                assert_eq!(
                    self.bridges[b].root().root,
                    self.bridges[root].bridge(),
                    "bridge {b} agrees on the root"
                );
                assert_eq!(self.bridges[b].is_root(), b == root, "only {root} is root");
            }
            // Spanning tree: n-1 live edges, all connected, no cycle.
            let edges: Vec<(usize, usize)> = self
                .live_edges()
                .into_iter()
                .filter(|(a, _)| comp[*a] == c)
                .collect();
            assert_eq!(
                edges.len(),
                members.len() - 1,
                "live edges {edges:?} of {members:?} are a tree"
            );
            let mut seen = vec![root];
            let mut grew = true;
            while grew {
                grew = false;
                for (a, b) in &edges {
                    for (x, y) in [(a, b), (b, a)] {
                        if seen.contains(x) && !seen.contains(y) {
                            seen.push(*y);
                            grew = true;
                        }
                    }
                }
            }
            assert_eq!(seen.len(), members.len(), "the tree reaches every bridge");
            // Costs: shortest path with a fixed cost per link.
            let mut dist = vec![u32::MAX; n];
            dist[root] = 0;
            for _ in 0..n {
                for (l, e) in self.links.iter().enumerate() {
                    if self.down[l] {
                        continue;
                    }
                    let (a, b) = (e[0].0, e[1].0);
                    if dist[a] != u32::MAX && dist[a] + cost < dist[b] {
                        dist[b] = dist[a] + cost;
                    }
                    if dist[b] != u32::MAX && dist[b] + cost < dist[a] {
                        dist[a] = dist[b] + cost;
                    }
                }
            }
            for &b in &members {
                assert_eq!(self.bridges[b].root().cost, dist[b], "cost at bridge {b}");
            }
        }
    }
}

/// Whether the edges contain a cycle (parallel edges count).
fn has_cycle(n: usize, edges: &[(usize, usize)]) -> bool {
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], x: usize) -> usize {
        if r[x] != x {
            let up = find(r, r[x]);
            r[x] = up;
        }
        r[x]
    }
    for (a, b) in edges {
        let (ra, rb) = (find(&mut root, *a), find(&mut root, *b));
        if ra == rb {
            return true;
        }
        root[ra] = rb;
    }
    false
}

const ALL: [StpMode; 2] = [StpMode::Stp, StpMode::Rapid];

fn triangle(mode: StpMode) -> Net {
    // 0 is the root (lowest priority), 1 and 2 each reach it directly and each other.
    Net::new(
        mode,
        &[4096, 32768, 32768],
        3,
        &[[(0, 0), (1, 0)], [(0, 1), (2, 0)], [(1, 1), (2, 1)]],
    )
}

#[test]
fn a_triangle_settles_into_a_tree_in_either_mode() {
    for mode in ALL {
        let mut n = triangle(mode);
        n.run(100);
        n.assert_settled(19);
        // Bridge 2 has the higher MAC so it keeps the blocked port on the 1-2 link.
        assert_eq!(n.bridges[2].port(1).role, StpRole::Alternate, "{mode:?}");
        assert_eq!(n.bridges[2].port(1).state, StpState::Discarding);
        assert_eq!(n.bridges[1].port(1).role, StpRole::Designated);
    }
}

#[test]
fn rapid_mode_converges_in_seconds_and_classic_waits_out_its_timers() {
    let mut rapid = triangle(StpMode::Rapid);
    rapid.run(6);
    rapid.assert_settled(19);
    let mut classic = triangle(StpMode::Stp);
    classic.run(6);
    assert!(
        classic.live_edges().is_empty(),
        "classic ports are still listening"
    );
    classic.run(50);
    classic.assert_settled(19);
}

#[test]
fn nothing_forwards_in_a_loop_while_the_network_is_settling() {
    // At no second may the forwarding edges contain a cycle, whatever mode.
    for mode in ALL {
        let mut n = triangle(mode);
        for _ in 0..100 {
            n.run(1);
            let e = n.live_edges();
            assert!(!has_cycle(3, &e), "{mode:?} t={} edges {e:?}", n.seconds);
        }
    }
}

#[test]
fn losing_the_link_to_the_root_moves_traffic_to_the_other_path() {
    for mode in ALL {
        let mut n = triangle(mode);
        n.run(100);
        // Cut 0-1: bridge 1 must reach the root through bridge 2.
        n.set_link(0, false);
        n.run(100);
        n.assert_settled(19);
        assert_eq!(n.bridges[1].root().cost, 38, "{mode:?}");
        assert_eq!(n.bridges[2].port(1).role, StpRole::Designated);
        assert_eq!(n.bridges[1].port(1).role, StpRole::Root);
    }
}

#[test]
fn rapid_mode_fails_over_to_an_alternate_port_in_a_few_seconds() {
    let mut n = triangle(StpMode::Rapid);
    n.run(30);
    // Bridge 2's root port is on the 0-2 link; its alternate is towards bridge 1.
    n.set_link(1, false);
    n.run(8);
    n.assert_settled(19);
}

#[test]
fn a_better_bridge_joining_takes_over_as_root() {
    for mode in ALL {
        let mut n = Net::new(
            mode,
            &[32768, 32768, 4096],
            2,
            &[[(0, 0), (1, 0)], [(1, 1), (2, 0)]],
        );
        // Start with the new bridge unplugged.
        n.set_link(1, false);
        n.run(60);
        assert!(n.bridges[0].is_root() || n.bridges[1].is_root());
        n.set_link(1, true);
        n.run(100);
        n.assert_settled(19);
        assert!(n.bridges[2].is_root());
    }
}

#[test]
fn an_edge_port_forwards_at_once_and_is_not_part_of_the_tree() {
    for mode in ALL {
        let mut n = triangle(mode);
        // Port 2 on bridge 1 faces a host.
        let a = n.bridges[1].set_edge(2, true);
        n.deliver(1, a);
        let a = n.bridges[1].set_enabled(2, true);
        n.deliver(1, a);
        assert!(n.bridges[1].forwards(2), "{mode:?}");
        n.run(100);
        assert!(n.bridges[1].forwards(2));
        n.assert_settled(19);
    }
}

#[test]
fn a_rapid_bridge_falls_back_to_classic_rules_towards_a_classic_one() {
    let mut n = Net::new(StpMode::Rapid, &[4096, 32768], 1, &[[(0, 0), (1, 0)]]);
    // Swap the second bridge for a classic one and cable them again.
    n.set_link(0, false);
    n.bridges[1] = Stp::new(StpMode::Stp, id(32768, 1), 1);
    n.set_link(0, true);
    n.run(10);
    assert!(n.bridges[0].port(0).compat);
    assert!(!n.forwarding(0, 0), "no handshake with a classic neighbour");
    n.run(50);
    n.assert_settled(19);
}

#[test]
fn a_topology_change_flushes_learned_addresses() {
    for mode in ALL {
        let mut n = triangle(mode);
        n.run(100);
        let before: Vec<usize> = n.flushes.clone();
        n.set_link(1, false);
        n.run(60);
        assert!(
            n.flushes.iter().zip(&before).any(|(a, b)| a > b),
            "{mode:?}: somebody flushed"
        );
    }
}

#[test]
fn a_bridge_alone_is_its_own_root_and_a_loop_back_cable_blocks_one_end() {
    for mode in ALL {
        let mut n = Net::new(mode, &[32768], 3, &[[(0, 0), (0, 1)]]);
        n.run(60);
        assert!(n.bridges[0].is_root());
        let roles = [n.bridges[0].port(0).role, n.bridges[0].port(1).role];
        assert!(
            roles.contains(&StpRole::Backup) && roles.contains(&StpRole::Designated),
            "{mode:?}: {roles:?}"
        );
        // Exactly one of the two ends passes traffic: no loop.
        assert_eq!(
            [n.forwarding(0, 0), n.forwarding(0, 1)]
                .iter()
                .filter(|f| **f)
                .count(),
            1
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any connected cabling of up to six bridges, any priorities, either mode: after enough time
    /// the network is a spanning tree, with the best bridge as root, at every second after
    /// settling the same, and never a forwarding loop on the way there.
    #[test]
    fn random_networks_settle_into_a_tree_and_never_loop(
        n in 2usize..6,
        extra in proptest::collection::vec((0usize..6, 0usize..6), 0..6),
        prios in proptest::collection::vec(0u16..8, 6),
        rapid in any::<bool>(),
    ) {
        let mode = if rapid { StpMode::Rapid } else { StpMode::Stp };
        // A path guarantees connection; the extras make loops (and parallel links).
        let mut pairs: Vec<(usize, usize)> = (1..n).map(|i| (i - 1, i)).collect();
        pairs.extend(extra.into_iter().map(|(a, b)| (a % n, b % n)).filter(|(a, b)| a != b));
        let mut used = vec![0usize; n];
        let mut links = vec![];
        for (a, b) in pairs {
            links.push([(a, used[a]), (b, used[b])]);
            used[a] += 1;
            used[b] += 1;
        }
        let prio: Vec<u16> = prios[..n].iter().map(|p| p * 4096).collect();
        let ports = *used.iter().max().unwrap();
        let mut net = Net::new(mode, &prio, ports, &links);
        for _ in 0..120 {
            net.run(1);
            // A cycle among edges that both ends forward would be a loop.
            let e = net.live_edges();
            prop_assert!(!has_cycle(n, &e), "t={} {mode:?} edges {e:?} on {links:?}", net.seconds);
        }
        net.assert_settled(19);
        net.run(30);
        net.assert_settled(19);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Cables are pulled and plugged back at random times. Between events nothing may loop; once
    /// the cabling stops changing the network settles into a tree again.
    #[test]
    fn links_flapping_never_cause_a_loop_and_the_network_recovers(
        n in 3usize..6,
        extra in proptest::collection::vec((0usize..6, 0usize..6), 1..5),
        prios in proptest::collection::vec(0u16..8, 6),
        flaps in proptest::collection::vec((0usize..12, 0u32..12), 1..8),
        rapid in any::<bool>(),
    ) {
        let mode = if rapid { StpMode::Rapid } else { StpMode::Stp };
        let mut pairs: Vec<(usize, usize)> = (1..n).map(|i| (i - 1, i)).collect();
        pairs.extend(extra.into_iter().map(|(a, b)| (a % n, b % n)).filter(|(a, b)| a != b));
        let mut used = vec![0usize; n];
        let mut links = vec![];
        for (a, b) in pairs {
            links.push([(a, used[a]), (b, used[b])]);
            used[a] += 1;
            used[b] += 1;
        }
        let prio: Vec<u16> = prios[..n].iter().map(|p| p * 4096).collect();
        let mut net = Net::new(mode, &prio, *used.iter().max().unwrap(), &links);
        net.run(60);
        for (l, wait) in flaps {
            let l = l % links.len();
            let up = net.down[l];
            net.set_link(l, up);
            for _ in 0..wait {
                net.run(1);
                let e = net.live_edges();
                prop_assert!(!has_cycle(n, &e), "t={} {mode:?} edges {e:?} on {links:?}", net.seconds);
            }
        }
        net.run(150);
        net.assert_settled(19);
    }
}

#[test]
fn parallel_links_after_a_flap_settle() {
    let links = [
        [(0, 0), (1, 0)],
        [(1, 1), (2, 0)],
        [(2, 1), (3, 0)],
        [(2, 2), (1, 2)],
    ];
    let mut n = Net::new(StpMode::Rapid, &[0, 0, 0, 0], 3, &links);
    n.run(60);
    for _ in 0..3 {
        let up = n.down[0];
        n.set_link(0, up);
    }
    n.run(150);
    n.assert_settled(19);
}

/// Found by the flapping property: a root cut off from part of a loop leaves stale information
/// circulating, and topology change notifications passed on at once chased each other round it
/// forever. They now wait for the next hello.
#[test]
fn stale_root_information_in_a_loop_does_not_make_notifications_chase_each_other() {
    // Five bridges: a path plus extra links 3-2, 4-2 and 0-4; bridge 1 is the best root.
    let pairs = [(0, 1), (1, 2), (2, 3), (3, 4), (3, 2), (4, 2), (0, 4)];
    let mut used = [0usize; 5];
    let mut links = vec![];
    for (a, b) in pairs {
        links.push([(a, used[a]), (b, used[b])]);
        used[a] += 1;
        used[b] += 1;
    }
    let mut net = Net::new(StpMode::Stp, &[16384, 0, 0, 0, 0], 4, &links);
    net.run(60);
    net.set_link(1, false);
    net.run(1);
    net.set_link(6, false);
    net.run(150);
    net.assert_settled(19);
}
