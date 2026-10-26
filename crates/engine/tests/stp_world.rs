//! Spanning tree on real switches: BPDUs in frames over links, ports that block, hosts that
//! still reach each other, and recovery when a cable goes dead.

use engine::World;
use kernel::{DeviceId, LinkConfig, LinkId, PortId, SimTime, Trace};
use netdev::{Host, L2Switch, PortMode, StpMode, StpRole, StpState};
use wire::{Ipv4Addr, MacAddr};

fn ip(n: u8) -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, n)
}

fn host(n: u8) -> Host {
    Host::new(&format!("pc{n}"), MacAddr([2, 0, 0, 0, 0, n]), ip(n), 24)
}

struct Tri {
    w: World,
    sw: [DeviceId; 3],
    pc: [DeviceId; 2],
    /// 0-1, 0-2, 1-2.
    links: [LinkId; 3],
}

/// Three switches in a triangle, one PC on switch 1 and one on switch 2. Ports: 0 and 1 are the
/// triangle, 2 is for a PC.
fn triangle(mode: StpMode, portfast_pcs: bool) -> Tri {
    let mut w = World::new(Trace::new());
    let sw = [0, 1, 2].map(|i| {
        let mut s = L2Switch::new(4);
        s.set_unit(i + 1);
        w.add_device(s)
    });
    let pc = [w.add_device(host(1)), w.add_device(host(2))];
    let l = LinkConfig::FAST_ETHERNET;
    let links = [
        w.connect((sw[0], 0), (sw[1], 0), l),
        w.connect((sw[0], 1), (sw[2], 0), l),
        w.connect((sw[1], 1), (sw[2], 1), l),
    ];
    w.connect((pc[0], 0), (sw[1], 2), l);
    w.connect((pc[1], 0), (sw[2], 2), l);
    for (i, s) in sw.iter().enumerate() {
        w.with_device::<L2Switch, _>(*s, |s, ctx| {
            s.configure_spanning(|c| {
                c.mode = Some(mode);
                if i == 0 {
                    c.priority.insert(1, 4096);
                }
            });
            if portfast_pcs {
                s.configure_port(PortId(2), |c| c.portfast = true);
            }
            s.refresh(ctx);
        });
    }
    Tri { w, sw, pc, links }
}

fn run(w: &mut World, secs: u64) {
    let to = w.kernel.now() + SimTime::from_secs(secs);
    w.run_until(to);
}

fn ping(w: &mut World, from: DeviceId, to: Ipv4Addr) -> bool {
    let before = w.device::<Host>(from).ping_results.len();
    w.with_device::<Host, _>(from, |h, c| h.ping(c, to));
    run(w, 2);
    w.device::<Host>(from).ping_results.len() > before
}

fn role(t: &Tri, sw: usize, port: u16) -> (StpRole, StpState) {
    let v =
        t.w.device::<L2Switch>(t.sw[sw])
            .tree(1)
            .unwrap()
            .port(port as usize);
    (v.role, v.state)
}

#[test]
fn rapid_spanning_tree_blocks_one_port_of_the_triangle_and_hosts_still_talk() {
    let mut t = triangle(StpMode::Rapid, true);
    run(&mut t.w, 10);
    assert!(t.w.device::<L2Switch>(t.sw[0]).tree(1).unwrap().is_root());
    // The 1-2 link is the one that loses: switch 2 has the higher address.
    assert_eq!(role(&t, 2, 1), (StpRole::Alternate, StpState::Discarding));
    assert_eq!(role(&t, 1, 1), (StpRole::Designated, StpState::Forwarding));
    assert_eq!(role(&t, 1, 0).0, StpRole::Root);
    assert_eq!(role(&t, 2, 0).0, StpRole::Root);
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    assert!(ping(&mut t.w, t.pc[1], ip(1)));
}

#[test]
fn classic_spanning_tree_holds_ports_back_for_about_thirty_seconds() {
    let mut t = triangle(StpMode::Stp, true);
    run(&mut t.w, 10);
    assert!(!ping(&mut t.w, t.pc[0], ip(2)), "still listening");
    run(&mut t.w, 40);
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    assert_eq!(role(&t, 2, 1), (StpRole::Alternate, StpState::Discarding));
}

#[test]
fn a_broadcast_does_not_storm_around_a_triangle_once_a_port_is_blocked() {
    let mut t = triangle(StpMode::Rapid, true);
    run(&mut t.w, 10);
    let before = t.w.kernel.delivered();
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    let used = t.w.kernel.delivered() - before;
    // ARP out, ARP back, echo and reply, plus the BPDUs of two seconds; a storm would be unbounded.
    assert!(used < 300, "{used} events for one ping");
}

#[test]
fn the_network_recovers_when_the_cable_to_the_root_is_cut() {
    let mut t = triangle(StpMode::Rapid, true);
    run(&mut t.w, 10);
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    // Cut 0-1. Switch 1's only way to the root is now through switch 2.
    t.w.kernel.set_link_up(t.links[0], false);
    run(&mut t.w, 12);
    assert_eq!(role(&t, 1, 1), (StpRole::Root, StpState::Forwarding));
    assert_eq!(role(&t, 2, 1), (StpRole::Designated, StpState::Forwarding));
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    // And back again.
    t.w.kernel.set_link_up(t.links[0], true);
    run(&mut t.w, 12);
    assert_eq!(role(&t, 1, 0).0, StpRole::Root);
    assert!(ping(&mut t.w, t.pc[1], ip(1)));
}

#[test]
fn a_learned_address_behind_a_failed_link_is_forgotten_at_once() {
    let mut t = triangle(StpMode::Rapid, true);
    run(&mut t.w, 10);
    assert!(ping(&mut t.w, t.pc[0], ip(2)));
    let seen = |t: &Tri| {
        t.w.device::<L2Switch>(t.sw[0])
            .mac_entries(t.w.kernel.now())
            .len()
    };
    assert!(seen(&t) > 0, "the root learned the PCs");
    t.w.kernel.set_link_up(t.links[1], false);
    run(&mut t.w, 4);
    // Whatever the root learned on the cut link was flushed by the topology change.
    let on_dead_port =
        t.w.device::<L2Switch>(t.sw[0])
            .mac_entries(t.w.kernel.now())
            .into_iter()
            .filter(|e| e.port == PortId(1))
            .count();
    assert_eq!(on_dead_port, 0);
}

#[test]
fn each_vlan_has_its_own_tree_so_different_links_can_block() {
    // Triangle of trunks; VLAN 10 is rooted at switch 0, VLAN 20 at switch 1.
    let mut w = World::new(Trace::new());
    let sw = [0, 1, 2].map(|i| {
        let mut s = L2Switch::new(3);
        s.set_unit(i + 1);
        w.add_device(s)
    });
    let l = LinkConfig::FAST_ETHERNET;
    w.connect((sw[0], 0), (sw[1], 0), l);
    w.connect((sw[0], 1), (sw[2], 0), l);
    w.connect((sw[1], 1), (sw[2], 1), l);
    for (i, s) in sw.iter().enumerate() {
        w.with_device::<L2Switch, _>(*s, |s, ctx| {
            for v in [10, 20] {
                s.add_vlan(v);
            }
            for p in 0..2 {
                s.configure_port(PortId(p), |c| c.mode = PortMode::Trunk);
            }
            s.configure_spanning(|c| {
                c.mode = Some(StpMode::Rapid);
                c.priority.insert(10, if i == 0 { 4096 } else { 32768 });
                c.priority.insert(20, if i == 1 { 4096 } else { 32768 });
            });
            s.refresh(ctx);
        });
    }
    run(&mut w, 15);
    let blocked = |w: &World, vlan: u16| -> Vec<(usize, usize)> {
        let mut b = vec![];
        for (i, s) in sw.iter().enumerate() {
            let tree = w.device::<L2Switch>(*s).tree(vlan).unwrap();
            for p in 0..2 {
                if tree.port(p).state == StpState::Discarding {
                    b.push((i, p));
                }
            }
        }
        b
    };
    let (b10, b20) = (blocked(&w, 10), blocked(&w, 20));
    assert_eq!(b10.len(), 1, "{b10:?}");
    assert_eq!(b20.len(), 1, "{b20:?}");
    assert_ne!(b10, b20, "the two VLANs block different links");
    assert!(w.device::<L2Switch>(sw[0]).tree(10).unwrap().is_root());
    assert!(w.device::<L2Switch>(sw[1]).tree(20).unwrap().is_root());
}

#[test]
fn a_switch_with_spanning_tree_off_floods_bpdus_like_any_multicast() {
    // Two switches with STP and a dumb one between them: the BPDUs cross it.
    let mut w = World::new(Trace::new());
    let a = w.add_device({
        let mut s = L2Switch::new(2);
        s.set_unit(1);
        s
    });
    let mid = w.add_device({
        let mut s = L2Switch::new(2);
        s.set_unit(2);
        s
    });
    let b = w.add_device({
        let mut s = L2Switch::new(2);
        s.set_unit(3);
        s
    });
    let l = LinkConfig::FAST_ETHERNET;
    w.connect((a, 0), (mid, 0), l);
    w.connect((mid, 1), (b, 0), l);
    for s in [a, b] {
        w.with_device::<L2Switch, _>(s, |s, ctx| {
            s.configure_spanning(|c| c.mode = Some(StpMode::Rapid));
            s.refresh(ctx);
        });
    }
    run(&mut w, 10);
    let ra = w.device::<L2Switch>(a).tree(1).unwrap().root();
    let rb = w.device::<L2Switch>(b).tree(1).unwrap().root();
    assert_eq!(
        ra.root, rb.root,
        "they agree on one root through the middle"
    );
}

#[test]
fn a_loop_without_spanning_tree_storms_but_the_simulation_stays_in_control() {
    let mut w = World::new(Trace::new());
    let sw = [0, 1, 2].map(|i| {
        let mut s = L2Switch::new(3);
        s.set_unit(i + 1);
        w.add_device(s)
    });
    let pc = w.add_device(host(1));
    let l = LinkConfig::FAST_ETHERNET;
    w.connect((sw[0], 0), (sw[1], 0), l);
    w.connect((sw[1], 1), (sw[2], 0), l);
    w.connect((sw[2], 1), (sw[0], 1), l);
    w.connect((pc, 0), (sw[0], 2), l);
    // An ARP request is a broadcast and goes round for ever.
    w.with_device::<Host, _>(pc, |h, c| h.ping(c, ip(9)));
    w.run_to_idle();
    assert!(w.storming, "the broadcast never died down");
    let at = w.kernel.now();
    assert!(
        at < SimTime::from_secs(2),
        "{at:?}: but it did give control back"
    );
}

#[test]
fn a_quiet_network_reports_no_storm() {
    let mut t = triangle(StpMode::Rapid, true);
    run(&mut t.w, 10);
    t.w.with_device::<Host, _>(t.pc[0], |h, c| h.ping(c, ip(2)));
    t.w.run_to_idle();
    assert!(!t.w.storming);
}
