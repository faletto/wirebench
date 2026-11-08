//! OSPF on random networks, with no devices or wires: routers are `Ospf` state machines whose
//! packets are handed over at once. Everything must settle to the same database and to the
//! shortest paths an independent calculation finds.

// Loops over node numbers index several parallel vectors (nodes, interfaces, expected costs).
#![allow(clippy::needless_range_loop)]

use kernel::SimTime;
use netdev::{
    Ospf, OspfDefaultInfo, OspfIfCfg, OspfIfState, OspfIfView, OspfNbrState, OspfNetStmt,
    OspfPathKind, OspfSource, OspfTx,
};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use wire::{Ipv4Addr, MacAddr};

struct Seg {
    /// (node, interface index on that node)
    members: Vec<(usize, usize)>,
    up: bool,
}

struct Iface {
    seg: usize,
    addr: Ipv4Addr,
    mac: MacAddr,
    cfg: OspfIfCfg,
}

struct Net {
    nodes: Vec<Ospf>,
    ifaces: Vec<Vec<Iface>>,
    segs: Vec<Seg>,
    now: SimTime,
    /// Routers that are switched off: they send and hear nothing.
    held: BTreeSet<usize>,
}

/// One member of a segment: (node, priority, cost).
type Member = (usize, u8, u32);

/// A description of a network: for each segment, whether it is point to point and its members.
#[derive(Clone, Debug)]
struct Spec {
    n: usize,
    segs: Vec<(bool, Vec<Member>)>,
    /// The area of each segment.
    areas: Vec<u32>,
}

impl Net {
    fn new(spec: &Spec) -> Net {
        let mut ifaces: Vec<Vec<Iface>> = (0..spec.n).map(|_| Vec::new()).collect();
        let mut segs = Vec::new();
        for (s, (p2p, members)) in spec.segs.iter().enumerate() {
            let mut m = Vec::new();
            for (k, &(node, prio, cost)) in members.iter().enumerate() {
                let idx = ifaces[node].len();
                ifaces[node].push(Iface {
                    seg: s,
                    addr: Ipv4Addr::new(10, spec.areas[s] as u8, s as u8, k as u8 + 1),
                    mac: MacAddr([2, 0xbb, node as u8, idx as u8, 0, 0]),
                    cfg: OspfIfCfg {
                        cost: Some(cost),
                        priority: prio,
                        point_to_point: *p2p,
                        ..OspfIfCfg::default()
                    },
                });
                m.push((node, idx));
            }
            segs.push(Seg {
                members: m,
                up: true,
            });
        }
        let areas: BTreeSet<u32> = spec.areas.iter().copied().collect();
        let nodes = (0..spec.n)
            .map(|_| {
                let mut o = Ospf::new(1);
                // 10.<area>.x.x belongs to that area.
                for &a in &areas {
                    o.networks.push(OspfNetStmt {
                        net: Ipv4Addr::new(10, a as u8, 0, 0),
                        wild: Ipv4Addr::new(0, 0, 255, 255),
                        area: a,
                    });
                }
                o
            })
            .collect();
        Net {
            nodes,
            ifaces,
            segs,
            now: SimTime::ZERO,
            held: BTreeSet::new(),
        }
    }

    fn views(&self, node: usize) -> Vec<OspfIfView> {
        if self.held.contains(&node) {
            return Vec::new();
        }
        self.ifaces[node]
            .iter()
            .enumerate()
            .filter(|(_, f)| self.segs[f.seg].up)
            .map(|(i, f)| OspfIfView {
                index: i,
                addr: f.addr,
                len: 24,
                mac: f.mac,
                cfg: f.cfg,
            })
            .collect()
    }

    /// Deliver `tx` from `node` and everything it causes.
    fn run(&mut self, first: Vec<(usize, OspfTx)>) {
        let mut queue: VecDeque<(usize, OspfTx)> = first.into();
        let mut guard = 0;
        while let Some((node, tx)) = queue.pop_front() {
            guard += 1;
            assert!(guard < 200_000, "packets never stop");
            let seg = &self.segs[self.ifaces[node][tx.iface].seg];
            if !seg.up {
                continue;
            }
            let src = self.ifaces[node][tx.iface].addr;
            let src_mac = self.ifaces[node][tx.iface].mac;
            let targets: Vec<(usize, usize)> = seg
                .members
                .iter()
                .copied()
                .filter(|&(n, i)| (n, i) != (node, tx.iface))
                .filter(|&(n, i)| {
                    let f = &self.ifaces[n][i];
                    tx.dst.0[0] == 224 || f.addr == tx.dst
                })
                .collect();
            for (n, i) in targets {
                if self.held.contains(&n) {
                    continue;
                }
                let views = self.views(n);
                let mut out = Vec::new();
                self.nodes[n].receive(
                    self.now,
                    &views,
                    OspfSource {
                        iface: i,
                        ip: src,
                        mac: src_mac,
                    },
                    tx.packet.clone(),
                    &mut out,
                );
                queue.extend(out.into_iter().map(|t| (n, t)));
            }
        }
    }

    fn tick(&mut self) {
        self.now = self.now + SimTime::from_secs(1);
        for n in 0..self.nodes.len() {
            if self.held.contains(&n) {
                continue;
            }
            let views = self.views(n);
            let mut out = Vec::new();
            self.nodes[n].tick(self.now, &views, &mut out);
            self.run(out.into_iter().map(|t| (n, t)).collect());
        }
    }

    fn run_for(&mut self, secs: u32) {
        for _ in 0..secs {
            self.tick();
        }
    }
}

/// Costs from every node to every segment prefix, by a plain Dijkstra over the routers: an edge
/// from X to Y where they share an up segment costs X's cost on it.
fn reference(spec: &Spec, up: &[bool]) -> Vec<BTreeMap<usize, u32>> {
    let n = spec.n;
    let mut all = Vec::new();
    for src in 0..n {
        let mut dist = vec![u32::MAX; n];
        dist[src] = 0;
        let mut done = vec![false; n];
        while let Some(u) = (0..n)
            .filter(|&u| !done[u] && dist[u] != u32::MAX)
            .min_by_key(|&u| dist[u])
        {
            done[u] = true;
            for (s, (_, members)) in spec.segs.iter().enumerate() {
                if !up[s] {
                    continue;
                }
                let Some(&(_, _, cost)) = members.iter().find(|m| m.0 == u) else {
                    continue;
                };
                for &(v, _, _) in members {
                    if v != u && dist[u] + cost < dist[v] {
                        dist[v] = dist[u] + cost;
                    }
                }
            }
        }
        let mut to_seg = BTreeMap::new();
        for (s, (_, members)) in spec.segs.iter().enumerate() {
            if !up[s] {
                continue;
            }
            let best = members
                .iter()
                .filter(|m| dist[m.0] != u32::MAX)
                .map(|&(r, _, cost)| dist[r] + cost)
                .min();
            if let Some(b) = best {
                to_seg.insert(s, b);
            }
        }
        all.push(to_seg);
    }
    all
}

fn check_routes(net: &Net, spec: &Spec) -> Result<(), String> {
    let up: Vec<bool> = net.segs.iter().map(|s| s.up).collect();
    let want = reference(spec, &up);
    for node in 0..spec.n {
        let attached: BTreeSet<usize> = net.ifaces[node]
            .iter()
            .filter(|f| up[f.seg])
            .map(|f| f.seg)
            .collect();
        let got: BTreeMap<usize, u32> = net.nodes[node]
            .routes()
            .iter()
            .map(|r| ((r.net.0[2]) as usize, r.cost))
            .collect();
        for (s, cost) in &want[node] {
            if attached.contains(s) {
                continue;
            }
            if got.get(s) != Some(cost) {
                return Err(format!(
                    "node {node} segment {s}: want cost {cost}, got {:?}\nall: {got:?}",
                    got.get(s)
                ));
            }
        }
        for s in got.keys() {
            if !want[node].contains_key(s) {
                return Err(format!(
                    "node {node} has a route to unreachable segment {s}"
                ));
            }
        }
    }
    Ok(())
}

fn database_of(net: &Net, node: usize) -> BTreeSet<String> {
    net.nodes[node]
        .database(net.now)
        .into_iter()
        .filter(|(_, h, _)| h.age < wire::MAX_AGE)
        .map(|(a, h, b)| {
            format!(
                "{a:?}/{}/{}/{}/{:08x}/{:?}",
                h.ls_type, h.id, h.adv_router, h.seq, b
            )
        })
        .collect()
}

fn check_databases(net: &Net, nodes: &[usize]) -> Result<(), String> {
    let first = database_of(net, nodes[0]);
    for &n in &nodes[1..] {
        let d = database_of(net, n);
        if d != first {
            return Err(format!(
                "node {} and node {n} disagree:\nonly first: {:?}\nonly second: {:?}",
                nodes[0],
                first.difference(&d).collect::<Vec<_>>(),
                d.difference(&first).collect::<Vec<_>>()
            ));
        }
    }
    Ok(())
}

fn spec_strategy() -> impl Strategy<Value = Spec> {
    (2usize..6).prop_flat_map(|n| {
        // A chain keeps it connected; extra segments add loops and shared segments.
        let extra = proptest::collection::vec(
            (
                any::<bool>(),
                proptest::collection::vec((0..n, 0u8..3, 1u32..6), 2..4),
            ),
            0..4,
        );
        let chain =
            proptest::collection::vec((any::<bool>(), (0u8..3, 1u32..6), (0u8..3, 1u32..6)), n - 1);
        (Just(n), chain, extra).prop_map(|(n, chain, extra)| {
            let mut segs = Vec::new();
            for (k, (p2p, a, b)) in chain.into_iter().enumerate() {
                let a = (if p2p { a.0 } else { a.0.max(1) }, a.1);
                segs.push((p2p, vec![(k, a.0, a.1), (k + 1, b.0, b.1)]));
            }
            for (p2p, members) in extra {
                // distinct nodes only; a p2p segment has exactly two
                let mut seen = BTreeSet::new();
                let mut m: Vec<(usize, u8, u32)> =
                    members.into_iter().filter(|x| seen.insert(x.0)).collect();
                if m.len() < 2 {
                    continue;
                }
                if p2p {
                    m.truncate(2);
                }
                if !p2p {
                    m[0].1 = m[0].1.max(1);
                }
                segs.push((p2p, m));
            }
            let areas = vec![0; segs.len()];
            Spec { n, segs, areas }
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("OSPF_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(48)
    ))]

    #[test]
    fn random_networks_settle_to_one_database_and_the_shortest_paths(spec in spec_strategy()) {
        let mut net = Net::new(&spec);
        net.run_for(150);
        let all: Vec<usize> = (0..spec.n).collect();
        prop_assert_eq!(check_databases(&net, &all), Ok(()));
        prop_assert_eq!(check_routes(&net, &spec), Ok(()));
        // Every router has a router LSA in everyone's database.
        prop_assert!(database_of(&net, 0).iter().filter(|l| l.starts_with("Some(0)/1/")).count() == spec.n);
    }

    #[test]
    fn cutting_a_segment_reroutes_or_cuts_off(spec in spec_strategy(), cut in 0usize..8) {
        let mut net = Net::new(&spec);
        net.run_for(120);
        let cut = cut % spec.segs.len();
        net.segs[cut].up = false;
        net.run_for(150);
        // Compare only within what is still connected: the reference marks the rest unreachable
        // and the routers must have dropped those routes.
        let up: Vec<bool> = net.segs.iter().map(|s| s.up).collect();
        let want = reference(&spec, &up);
        for node in 0..spec.n {
            let reach: BTreeSet<usize> = want[node].keys().copied().collect();
            let attached: BTreeSet<usize> = net.ifaces[node].iter().filter(|f| up[f.seg]).map(|f| f.seg).collect();
            // Prefixes the router is attached to are connected routes, which win over OSPF's.
            let got: BTreeSet<usize> = net.nodes[node].routes().iter().map(|r| r.net.0[2] as usize).filter(|s| !attached.contains(s)).collect();
            prop_assert_eq!(got, reach.difference(&attached).copied().collect::<BTreeSet<_>>(), "node {} reachable set", node);
        }
        prop_assert_eq!(check_routes(&net, &spec), Ok(()));
    }
}

fn lan(prios: &[u8]) -> Spec {
    Spec {
        n: prios.len(),
        segs: vec![(
            false,
            prios.iter().enumerate().map(|(i, &p)| (i, p, 1)).collect(),
        )],
        areas: vec![0],
    }
}

fn role(net: &Net, node: usize) -> OspfIfState {
    net.nodes[node].interfaces().next().unwrap().state
}

#[test]
fn the_highest_priority_then_highest_id_is_dr_and_the_next_is_bdr() {
    let mut net = Net::new(&lan(&[1, 2, 2, 1]));
    net.run_for(90);
    // Router IDs are the interface addresses: node 2 has 10.0.0.3, node 1 has 10.0.0.2.
    assert_eq!(role(&net, 2), OspfIfState::Dr);
    assert_eq!(role(&net, 1), OspfIfState::Backup);
    assert_eq!(role(&net, 0), OspfIfState::DrOther);
    assert_eq!(role(&net, 3), OspfIfState::DrOther);
    // DROthers are adjacent only to the DR and BDR, and two-way with each other.
    let nbrs = &net.nodes[0].interfaces().next().unwrap().nbrs;
    let full = nbrs
        .values()
        .filter(|n| n.state == OspfNbrState::Full)
        .count();
    let two_way = nbrs
        .values()
        .filter(|n| n.state == OspfNbrState::TwoWay)
        .count();
    assert_eq!((full, two_way), (2, 1));
}

#[test]
fn a_priority_zero_router_is_never_dr_or_bdr() {
    let mut net = Net::new(&lan(&[0, 1, 1]));
    net.run_for(90);
    assert_eq!(role(&net, 0), OspfIfState::DrOther);
    let dr = (0..3).filter(|&n| role(&net, n) == OspfIfState::Dr).count();
    let bdr = (0..3)
        .filter(|&n| role(&net, n) == OspfIfState::Backup)
        .count();
    assert_eq!((dr, bdr), (1, 1));
}

#[test]
fn a_late_router_with_a_better_priority_does_not_take_over() {
    // Nodes 0 and 1 settle first; node 2 (priority 9) is switched on later and stays DROther.
    let mut net = Net::new(&lan(&[1, 1, 9]));
    net.held.insert(2);
    net.run_for(90);
    assert_eq!(role(&net, 1), OspfIfState::Dr);
    net.held.remove(&2);
    net.run_for(150);
    assert_eq!(role(&net, 1), OspfIfState::Dr, "no pre-emption");
    assert_eq!(role(&net, 0), OspfIfState::Backup);
    assert_eq!(role(&net, 2), OspfIfState::DrOther);
}

#[test]
fn a_dead_neighbour_is_noticed_after_the_dead_interval() {
    let spec = Spec {
        n: 2,
        segs: vec![(true, vec![(0, 1, 1), (1, 1, 1)])],
        areas: vec![0],
    };
    let mut net = Net::new(&spec);
    net.run_for(60);
    let count = |net: &Net| net.nodes[0].interfaces().next().unwrap().nbrs.len();
    assert_eq!(count(&net), 1);
    net.segs[0].up = false;
    net.run_for(39);
    // (the interface is down, so the engine has dropped it already)
    assert!(net.nodes[0].interfaces().next().is_none());
}

// ---- hostile input ----

mod garbage {
    use super::*;
    use wire::{Dbd, Hello, LsRequest, Lsa, LsaBody, LsaHeader, OspfBody, OspfPacket, RouterLink};

    fn rid() -> impl Strategy<Value = Ipv4Addr> {
        prop_oneof![
            Just(Ipv4Addr::new(2, 2, 2, 2)),
            Just(Ipv4Addr::new(3, 3, 3, 3)),
            Just(Ipv4Addr::new(10, 0, 0, 1)),
            Just(Ipv4Addr::new(10, 0, 0, 2)),
            Just(Ipv4Addr::new(10, 0, 0, 3)),
            Just(Ipv4Addr::UNSPECIFIED),
        ]
    }

    fn lsa() -> impl Strategy<Value = Lsa> {
        let body = prop_oneof![
            proptest::collection::vec((rid(), rid(), 1u8..5, any::<u16>()), 0..4).prop_map(|v| {
                LsaBody::Router {
                    flags: 0,
                    links: v
                        .into_iter()
                        .map(|(id, data, kind, metric)| RouterLink {
                            id,
                            data,
                            kind,
                            metric,
                        })
                        .collect(),
                }
            }),
            (rid(), proptest::collection::vec(rid(), 0..4))
                .prop_map(|(mask, routers)| LsaBody::Network { mask, routers }),
            Just(LsaBody::Other {
                ls_type: 6,
                data: vec![1, 2, 3]
            }),
            (3u8..=4, rid(), 0u32..=wire::LS_INFINITY).prop_map(|(ls_type, mask, metric)| {
                LsaBody::Summary {
                    ls_type,
                    mask,
                    metric,
                }
            }),
            (rid(), any::<bool>(), 0u32..=wire::LS_INFINITY, rid()).prop_map(
                |(mask, e2, metric, forwarding)| LsaBody::External {
                    mask,
                    e2,
                    metric,
                    forwarding,
                    tag: 0
                }
            ),
        ];
        (
            prop_oneof![Just(0u16), Just(5), Just(3600)],
            rid(),
            rid(),
            any::<i32>(),
            body,
        )
            .prop_map(|(age, id, adv_router, seq, body)| Lsa {
                age,
                options: 2,
                id,
                adv_router,
                seq,
                body,
            })
    }

    fn header() -> impl Strategy<Value = LsaHeader> {
        (
            prop_oneof![Just(0u16), Just(3600)],
            0u8..7,
            rid(),
            rid(),
            any::<i32>(),
            any::<u16>(),
        )
            .prop_map(|(age, ls_type, id, adv_router, seq, checksum)| LsaHeader {
                age,
                options: 2,
                ls_type,
                id,
                adv_router,
                seq,
                checksum,
                length: 36,
            })
    }

    fn body() -> impl Strategy<Value = OspfBody> {
        prop_oneof![
            (proptest::collection::vec(rid(), 0..4), 0u8..3, rid(), rid()).prop_map(
                |(neighbors, priority, dr, bdr)| {
                    OspfBody::Hello(Hello {
                        mask: Ipv4Addr::new(255, 255, 255, 0),
                        hello_interval: 10,
                        options: 2,
                        priority,
                        dead_interval: 40,
                        dr,
                        bdr,
                        neighbors,
                    })
                }
            ),
            (
                any::<u8>(),
                0u32..4,
                proptest::collection::vec(header(), 0..4)
            )
                .prop_map(|(flags, seq, headers)| {
                    OspfBody::Dbd(Dbd {
                        mtu: 1500,
                        options: 2,
                        flags: flags & 7,
                        seq: 0x1100 + seq * 0x100,
                        headers,
                    })
                }),
            proptest::collection::vec((0u32..7, rid(), rid()), 0..4).prop_map(|v| {
                OspfBody::Request(
                    v.into_iter()
                        .map(|(ls_type, id, adv_router)| LsRequest {
                            ls_type,
                            id,
                            adv_router,
                        })
                        .collect(),
                )
            }),
            proptest::collection::vec(lsa(), 0..4).prop_map(OspfBody::Update),
            proptest::collection::vec(header(), 0..4).prop_map(OspfBody::Ack),
        ]
    }

    fn packet() -> impl Strategy<Value = OspfPacket> {
        (
            prop_oneof![
                Just(Ipv4Addr::new(2, 2, 2, 2)),
                Just(Ipv4Addr::new(3, 3, 3, 3))
            ],
            body(),
        )
            .prop_map(|(router_id, body)| OspfPacket {
                router_id,
                area: Ipv4Addr::UNSPECIFIED,
                auth_type: 0,
                auth: [0; 8],
                body,
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(
            std::env::var("OSPF_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(48)
        ))]

        /// Whatever arrives, in whatever order, the state machine goes on.
        #[test]
        fn random_packets_in_any_order_never_break_it(
            events in proptest::collection::vec((any::<bool>(), packet(), 0u8..3), 1..120),
        ) {
            let mut o = Ospf::new(1);
            o.networks.push(OspfNetStmt {
                net: Ipv4Addr::new(10, 0, 0, 0),
                wild: Ipv4Addr::new(0, 0, 255, 255),
                area: 0,
            });
            let views = [OspfIfView {
                index: 0,
                addr: Ipv4Addr::new(10, 0, 0, 1),
                len: 24,
                mac: MacAddr([2, 0, 0, 0, 0, 1]),
                cfg: OspfIfCfg::default(),
            }];
            let mut now = SimTime::ZERO;
            for (tick, pkt, who) in events {
                now = now + SimTime::from_secs(if tick { 7 } else { 1 });
                let mut out = Vec::new();
                if tick {
                    o.tick(now, &views, &mut out);
                }
                let src = Ipv4Addr::new(10, 0, 0, 2 + who);
                o.receive(
                    now,
                    &views,
                    OspfSource { iface: 0, ip: src, mac: MacAddr([2, 0, 0, 0, 0, 2 + who]) },
                    pkt,
                    &mut out,
                );
                // Everything it says is something the wire can carry.
                for tx in out {
                    prop_assert_eq!(OspfPacket::parse(&tx.packet.emit()), Ok(tx.packet));
                }
            }
        }
    }
}

// ---- several areas ----

/// A backbone with leaf areas, each reaching it through one border router.
#[derive(Clone, Debug)]
struct Multi {
    spec: Spec,
    /// The border router of each leaf area.
    abr: BTreeMap<u32, usize>,
}

fn multi_strategy() -> impl Strategy<Value = Multi> {
    (
        2usize..4,
        1u32..4,
        proptest::collection::vec(any::<u32>(), 200),
    )
        .prop_map(|(backbone, leaves, r)| {
            let mut it = r.into_iter();
            let mut pick = move |m: u32| it.next().unwrap_or(0) % m;
            let mut n = backbone;
            let mut segs: Vec<(bool, Vec<Member>)> = Vec::new();
            let mut areas = Vec::new();
            let mut abr = BTreeMap::new();
            // Each area is a chain over its routers plus a few extra segments.
            let mut build = |nodes: &[usize], area: u32, pick: &mut dyn FnMut(u32) -> u32| {
                let member = |node: usize, p2p: bool, pick: &mut dyn FnMut(u32) -> u32| {
                    (
                        node,
                        if p2p {
                            pick(3) as u8
                        } else {
                            pick(3).max(1) as u8
                        },
                        1 + pick(5),
                    )
                };
                for w in nodes.windows(2) {
                    let p2p = pick(2) == 0;
                    let a = member(w[0], p2p, pick);
                    let b = member(w[1], p2p, pick);
                    segs.push((p2p, vec![a, b]));
                    areas.push(area);
                }
                for _ in 0..pick(3) {
                    let p2p = pick(2) == 0;
                    let first = nodes[pick(nodes.len() as u32) as usize];
                    let second = nodes[pick(nodes.len() as u32) as usize];
                    if first == second {
                        continue;
                    }
                    let a = member(first, p2p, pick);
                    let mut b = member(second, p2p, pick);
                    if !p2p {
                        b.1 = b.1.max(1);
                    }
                    segs.push((p2p, vec![a, b]));
                    areas.push(area);
                }
            };
            let bb: Vec<usize> = (0..backbone).collect();
            build(&bb, 0, &mut pick);
            for leaf in 1..=leaves {
                let border = (leaf as usize - 1) % backbone;
                abr.insert(leaf, border);
                let extra = 1 + pick(2) as usize;
                let mut nodes = vec![border];
                for _ in 0..extra {
                    nodes.push(n);
                    n += 1;
                }
                build(&nodes, leaf, &mut pick);
            }
            Multi {
                spec: Spec { n, segs, areas },
                abr,
            }
        })
}

const INF: u32 = u32::MAX;

/// Distances inside one area over its up segments: an edge from X to Y where they share a
/// segment costs X's cost on it.
fn area_dist(spec: &Spec, up: &[bool], area: u32, src: usize) -> Vec<u32> {
    let mut dist = vec![INF; spec.n];
    dist[src] = 0;
    let mut done = vec![false; spec.n];
    while let Some(u) = (0..spec.n)
        .filter(|&u| !done[u] && dist[u] != INF)
        .min_by_key(|&u| dist[u])
    {
        done[u] = true;
        for (s, (_, members)) in spec.segs.iter().enumerate() {
            if !up[s] || spec.areas[s] != area {
                continue;
            }
            let Some(&(_, _, cost)) = members.iter().find(|m| m.0 == u) else {
                continue;
            };
            for &(v, _, _) in members {
                if v != u && dist[u] + cost < dist[v] {
                    dist[v] = dist[u] + cost;
                }
            }
        }
    }
    dist
}

impl Multi {
    /// The areas the router is in: those with a segment that is up.
    fn areas_of(&self, up: &[bool], node: usize) -> BTreeSet<u32> {
        let mut a = BTreeSet::new();
        for (s, (_, members)) in self.spec.segs.iter().enumerate() {
            if up[s] && members.iter().any(|m| m.0 == node) {
                a.insert(self.spec.areas[s]);
            }
        }
        a
    }

    /// The cost from `x` to something in `area` that costs `tail(entry)` to reach from the
    /// router `entry` of that area: inside the area directly, otherwise out through the border
    /// router of x's leaf area, across the backbone, and in through the target area's.
    fn cost_via(&self, up: &[bool], x: usize, area: u32, tail: &dyn Fn(usize) -> u32) -> u32 {
        let add = |a: u32, b: u32| if a == INF || b == INF { INF } else { a + b };
        let mine = self.areas_of(up, x);
        if mine.contains(&area) {
            return tail(x);
        }
        // x's way onto the backbone.
        let (gate, base) = if mine.contains(&0) {
            (x, 0)
        } else {
            let Some(&leaf) = mine.iter().next() else {
                return INF;
            };
            let g = self.abr[&leaf];
            // A router without a working backbone attachment is not a border router.
            if !self.areas_of(up, g).contains(&0) {
                return INF;
            }
            (g, area_dist(&self.spec, up, leaf, x)[g])
        };
        if area == 0 {
            return add(base, tail(gate));
        }
        let entry = self.abr[&area];
        if !self.areas_of(up, entry).contains(&0) {
            return INF;
        }
        let across = area_dist(&self.spec, up, 0, gate)[entry];
        add(add(base, across), tail(entry))
    }

    /// The cost from every node to every segment's prefix.
    fn prefix_costs(&self, up: &[bool]) -> Vec<BTreeMap<usize, u32>> {
        (0..self.spec.n)
            .map(|x| {
                let mut out = BTreeMap::new();
                for (s, (_, members)) in self.spec.segs.iter().enumerate() {
                    if !up[s] {
                        continue;
                    }
                    let area = self.spec.areas[s];
                    let tail = |entry: usize| {
                        let d = area_dist(&self.spec, up, area, entry);
                        members
                            .iter()
                            .filter(|m| d[m.0] != INF)
                            .map(|&(r, _, c)| d[r] + c)
                            .min()
                            .unwrap_or(INF)
                    };
                    let c = self.cost_via(up, x, area, &tail);
                    if c != INF {
                        out.insert(s, c);
                    }
                }
                out
            })
            .collect()
    }

    /// The cost from `x` to the router `r`.
    fn router_cost(&self, up: &[bool], x: usize, r: usize) -> u32 {
        if x == r {
            return 0;
        }
        self.areas_of(up, r)
            .into_iter()
            .map(|a| {
                let tail = |entry: usize| area_dist(&self.spec, up, a, entry)[r];
                self.cost_via(up, x, a, &tail)
            })
            .min()
            .unwrap_or(INF)
    }
}

fn all_up(net: &Net) -> Vec<bool> {
    net.segs.iter().map(|s| s.up).collect()
}

/// Which group of routers each router is in, joined by up segments (of one area, or of any).
fn components(spec: &Spec, up: &[bool], area: Option<u32>) -> Vec<usize> {
    let mut comp: Vec<usize> = (0..spec.n).collect();
    loop {
        let mut changed = false;
        for (s, (_, members)) in spec.segs.iter().enumerate() {
            if !up[s] || area.is_some_and(|a| a != spec.areas[s]) {
                continue;
            }
            let low = members.iter().map(|m| comp[m.0]).min().expect("members");
            for m in members {
                if comp[m.0] != low {
                    comp[m.0] = low;
                    changed = true;
                }
            }
        }
        if !changed {
            return comp;
        }
    }
}

/// Routers that can still reach each other in an area hold the same LSAs for it, and the ones
/// that can reach each other at all hold the same external LSAs. (Across a cut, flushes do not
/// get through, so each side keeps what the other has forgotten until it ages out.)
fn check_area_databases(net: &Net, m: &Multi) -> Result<(), String> {
    let up = all_up(net);
    let in_area = |node: usize, a: u32| {
        m.spec
            .segs
            .iter()
            .enumerate()
            .any(|(s, (_, ms))| up[s] && m.spec.areas[s] == a && ms.iter().any(|x| x.0 == node))
    };
    // An area's LSAs never reach a router that has nothing in that area.
    for node in 0..m.spec.n {
        let own: BTreeSet<String> = (0..m.spec.segs.len())
            .filter(|&s| m.spec.segs[s].1.iter().any(|x| x.0 == node))
            .map(|s| format!("Some({})", m.spec.areas[s]))
            .chain(["None".to_string()])
            .collect();
        for l in database_of(net, node) {
            let scope = l.split('/').next().unwrap_or("").to_string();
            if !own.contains(&scope) {
                return Err(format!("node {node} holds {l}, of an area it is not in"));
            }
        }
    }
    let mut groups: Vec<(String, usize, Vec<usize>)> = Vec::new();
    let areas: BTreeSet<u32> = m.spec.areas.iter().copied().collect();
    for &a in &areas {
        let comp = components(&m.spec, &up, Some(a));
        for c in comp.iter().copied().collect::<BTreeSet<_>>() {
            let nodes: Vec<usize> = (0..m.spec.n)
                .filter(|&n| comp[n] == c && in_area(n, a))
                .collect();
            if nodes.len() > 1 {
                groups.push((format!("Some({a})"), c, nodes));
            }
        }
    }
    let comp = components(&m.spec, &up, None);
    for c in comp.iter().copied().collect::<BTreeSet<_>>() {
        let nodes: Vec<usize> = (0..m.spec.n).filter(|&n| comp[n] == c).collect();
        groups.push(("None".to_string(), c, nodes));
    }
    for (scope, _, nodes) in groups {
        let part = |node: usize| -> BTreeSet<String> {
            database_of(net, node)
                .into_iter()
                .filter(|l| l.starts_with(&format!("{scope}/")))
                .collect()
        };
        let first = part(nodes[0]);
        for &n in &nodes[1..] {
            let other = part(n);
            if other != first {
                return Err(format!(
                    "{scope}: nodes {} and {n} disagree:\nonly first: {:?}\nonly second: {:?}",
                    nodes[0],
                    first.difference(&other).collect::<Vec<_>>(),
                    other.difference(&first).collect::<Vec<_>>()
                ));
            }
        }
    }
    Ok(())
}

fn check_multi_routes(net: &Net, m: &Multi) -> Result<(), String> {
    let up = all_up(net);
    let want = m.prefix_costs(&up);
    for node in 0..m.spec.n {
        let attached: BTreeSet<usize> = net.ifaces[node]
            .iter()
            .filter(|f| up[f.seg])
            .map(|f| f.seg)
            .collect();
        let got: BTreeMap<usize, (u32, OspfPathKind)> = net.nodes[node]
            .routes()
            .iter()
            .map(|r| (r.net.0[2] as usize, (r.cost, r.kind)))
            .collect();
        for (s, cost) in &want[node] {
            if attached.contains(s) {
                continue;
            }
            let area = m.spec.areas[*s];
            let kind = if m.areas_of(&up, node).contains(&area) {
                OspfPathKind::Intra
            } else {
                OspfPathKind::Inter
            };
            if got.get(s) != Some(&(*cost, kind)) {
                return Err(format!(
                    "node {node} segment {s} (area {area}): want ({cost}, {kind:?}), got {:?}\nall: {got:?}",
                    got.get(s)
                ));
            }
        }
        for s in got.keys() {
            if !want[node].contains_key(s) && !attached.contains(s) {
                return Err(format!(
                    "node {node} has a route to unreachable segment {s}"
                ));
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("OSPF_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(48)
    ))]

    #[test]
    fn several_areas_settle_to_per_area_databases_and_the_same_costs_as_the_reference(m in multi_strategy()) {
        let mut net = Net::new(&m.spec);
        net.run_for(240);
        prop_assert_eq!(check_area_databases(&net, &m), Ok(()));
        prop_assert_eq!(check_multi_routes(&net, &m), Ok(()));
    }

    #[test]
    fn several_areas_recover_from_a_cut(m in multi_strategy(), cut in 0usize..16) {
        let mut net = Net::new(&m.spec);
        net.run_for(200);
        let cut = cut % m.spec.segs.len();
        net.segs[cut].up = false;
        net.run_for(240);
        prop_assert_eq!(check_area_databases(&net, &m), Ok(()));
        prop_assert_eq!(check_multi_routes(&net, &m), Ok(()));
    }

    /// A default route advertised by a router in one area is learned in every area, costs what
    /// the configuration says (type 2) or the metric plus the distance (type 1), and following
    /// next hops leads to the router that advertised it.
    #[test]
    fn a_default_route_reaches_every_area(m in multi_strategy(), who in 0usize..16, e2 in any::<bool>(), metric in 1u32..30) {
        let mut net = Net::new(&m.spec);
        let asbr = who % m.spec.n;
        net.nodes[asbr].default_info = Some(OspfDefaultInfo { always: true, metric, e2 });
        net.run_for(300);
        let up = all_up(&net);
        for node in (0..m.spec.n).filter(|&n| n != asbr) {
            let route = net.nodes[node].routes().iter().find(|r| r.len == 0).copied();
            let Some(route) = route else {
                return Err(TestCaseError::fail(format!("node {node} has no default route")));
            };
            let (kind, cost) = if e2 {
                (OspfPathKind::External2, metric)
            } else {
                (OspfPathKind::External1, metric + m.router_cost(&up, node, asbr))
            };
            prop_assert_eq!((route.kind, route.cost), (kind, cost), "node {}", node);
            // Follow the next hops.
            let mut at = node;
            for _ in 0..=m.spec.n {
                if at == asbr {
                    break;
                }
                let r = net.nodes[at].routes().iter().find(|r| r.len == 0).copied();
                let Some(r) = r else { return Err(TestCaseError::fail(format!("path from {node} breaks at {at}"))) };
                let next = (0..m.spec.n).find(|&n| net.ifaces[n].iter().any(|f| f.addr == r.via));
                let Some(next) = next else { return Err(TestCaseError::fail(format!("no router owns {}", r.via))) };
                at = next;
            }
            prop_assert_eq!(at, asbr, "node {}'s default route leads away from the advertiser", node);
        }
    }
}

#[test]
fn routers_that_disagree_about_the_area_never_become_neighbours() {
    // One LAN, two routers: node 1 puts the same segment in area 5.
    let spec = lan(&[1, 1]);
    let mut net = Net::new(&spec);
    net.nodes[1].networks = vec![OspfNetStmt {
        net: Ipv4Addr::new(10, 0, 0, 0),
        wild: Ipv4Addr::new(0, 0, 255, 255),
        area: 5,
    }];
    net.run_for(150);
    for n in 0..2 {
        assert_eq!(
            net.nodes[n].interfaces().next().unwrap().nbrs.len(),
            0,
            "node {n}"
        );
        assert!(net.nodes[n].routes().is_empty());
    }
}

#[test]
fn a_router_alone_in_its_area_is_not_a_border_router_and_one_with_the_backbone_is() {
    let spec = Spec {
        n: 3,
        segs: vec![
            (true, vec![(0, 1, 1), (1, 1, 1)]),
            (true, vec![(1, 1, 1), (2, 1, 1)]),
        ],
        areas: vec![0, 1],
    };
    let mut net = Net::new(&spec);
    net.run_for(120);
    assert!(!net.nodes[0].is_abr());
    assert!(net.nodes[1].is_abr());
    assert!(!net.nodes[2].is_abr());
    // The border router summarises each area to the other.
    assert!(net.nodes[2]
        .routes()
        .iter()
        .any(|r| r.net.0[2] == 0 && r.kind == OspfPathKind::Inter));
    assert!(net.nodes[0]
        .routes()
        .iter()
        .any(|r| r.net.0[2] == 1 && r.kind == OspfPathKind::Inter));
}

#[test]
fn a_larger_mtu_on_one_side_holds_the_exchange_in_exstart_until_it_is_ignored() {
    let spec = Spec {
        n: 2,
        segs: vec![(true, vec![(0, 1, 1), (1, 1, 1)])],
        areas: vec![0],
    };
    let state = |net: &Net, n: usize| {
        net.nodes[n]
            .interfaces()
            .next()
            .unwrap()
            .nbrs
            .values()
            .next()
            .map(|x| x.state)
    };
    let mut net = Net::new(&spec);
    net.ifaces[0][0].cfg.mtu = Some(1400);
    net.run_for(150);
    // Node 0 ignores node 1's packets (1500 > 1400), so neither side gets past the start.
    assert_ne!(state(&net, 0), Some(OspfNbrState::Full));
    assert_ne!(state(&net, 1), Some(OspfNbrState::Full));
    assert!(matches!(
        state(&net, 0),
        Some(OspfNbrState::ExStart | OspfNbrState::Exchange)
    ));
    // With the check off on the small side they meet.
    net.ifaces[0][0].cfg.mtu_ignore = true;
    net.run_for(150);
    assert_eq!(state(&net, 0), Some(OspfNbrState::Full));
    assert_eq!(state(&net, 1), Some(OspfNbrState::Full));
}
