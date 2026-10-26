//! Dynamic trunking: which pairs of port modes end up trunking, and what a neighbour that stops
//! talking, or never talked, leaves behind.

use engine::World;
use kernel::{DeviceId, LinkConfig, LinkId, PortId, SimTime, Trace};
use netdev::{Host, L2Switch, PortMode};
use wire::{Ipv4Addr, MacAddr};

const MODES: [PortMode; 4] = [
    PortMode::Access,
    PortMode::Trunk,
    PortMode::DynamicAuto,
    PortMode::DynamicDesirable,
];

fn run(w: &mut World, secs: u64) {
    let to = w.kernel.now() + SimTime::from_secs(secs);
    w.run_until(to);
}

fn set(w: &mut World, sw: DeviceId, mode: PortMode) {
    w.with_device::<L2Switch, _>(sw, |s, ctx| {
        s.configure_port(PortId(0), |c| c.mode = mode);
        s.refresh(ctx);
    });
}

fn pair(a: PortMode, b: PortMode) -> (World, [DeviceId; 2], LinkId) {
    let mut w = World::new(Trace::new());
    let sw = [1, 2].map(|i| {
        let mut s = L2Switch::new(2);
        s.set_unit(i);
        w.add_device(s)
    });
    let l = w.connect((sw[0], 0), (sw[1], 0), LinkConfig::FAST_ETHERNET);
    set(&mut w, sw[0], a);
    set(&mut w, sw[1], b);
    run(&mut w, 5);
    (w, sw, l)
}

fn trunks(w: &World, sw: DeviceId) -> bool {
    w.device::<L2Switch>(sw).is_trunk(PortId(0))
}

/// The textbook table: a trunk forms when one side is trunk or desirable and the other is anything
/// that listens (not access), or when desirable meets auto.
fn expected(a: PortMode, b: PortMode) -> bool {
    use PortMode::*;
    !matches!(
        (a, b),
        (Access, _) | (_, Access) | (DynamicAuto, DynamicAuto)
    )
}

#[test]
fn every_pair_of_modes_trunks_or_not_as_the_table_says() {
    for a in MODES {
        for b in MODES {
            let (w, sw, _) = pair(a, b);
            let (ta, tb) = (trunks(&w, sw[0]), trunks(&w, sw[1]));
            // A static trunk is a trunk whatever the other end does.
            let want_a = a == PortMode::Trunk || expected(a, b);
            let want_b = b == PortMode::Trunk || expected(a, b);
            assert_eq!((ta, tb), (want_a, want_b), "{a:?} against {b:?}");
        }
    }
}

#[test]
fn the_order_the_ends_are_configured_in_does_not_matter() {
    for (a, b) in [
        (PortMode::DynamicDesirable, PortMode::DynamicAuto),
        (PortMode::Trunk, PortMode::DynamicAuto),
    ] {
        let mut w = World::new(Trace::new());
        let sw = [1, 2].map(|i| {
            let mut s = L2Switch::new(2);
            s.set_unit(i);
            w.add_device(s)
        });
        w.connect((sw[0], 0), (sw[1], 0), LinkConfig::FAST_ETHERNET);
        // Configured first on the second switch, then the first, with time between.
        set(&mut w, sw[1], b);
        run(&mut w, 3);
        set(&mut w, sw[0], a);
        run(&mut w, 3);
        assert!(trunks(&w, sw[0]) && trunks(&w, sw[1]), "{a:?} / {b:?}");
    }
}

#[test]
fn a_switch_nobody_configured_still_answers_and_becomes_a_trunk() {
    let mut w = World::new(Trace::new());
    let a = w.add_device(L2Switch::new(2));
    let b = w.add_device(L2Switch::new(2));
    w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
    set(&mut w, a, PortMode::DynamicDesirable);
    run(&mut w, 3);
    // b never had a command typed at it (the default is dynamic auto).
    assert!(trunks(&w, a) && trunks(&w, b));
}

#[test]
fn nonegotiate_neither_sends_nor_listens() {
    // A static trunk with nonegotiate against a dynamic desirable port: nothing is said, so the
    // desirable side stays access and the trunk is one-sided.
    let (mut w, sw, _) = pair(PortMode::Trunk, PortMode::DynamicDesirable);
    assert!(trunks(&w, sw[1]));
    w.with_device::<L2Switch, _>(sw[0], |s, ctx| {
        s.configure_port(PortId(0), |c| c.nonegotiate = true);
        s.refresh(ctx);
    });
    // The desirable side hears nothing new, but its record of the neighbour ages out.
    run(&mut w, 120);
    assert!(trunks(&w, sw[0]), "static trunks stay trunks");
    assert!(
        !trunks(&w, sw[1]),
        "the dynamic side gave up after 90 s of silence"
    );
    assert!(w
        .device::<L2Switch>(sw[1])
        .dtp_neighbour(PortId(0))
        .is_none());
}

#[test]
fn a_neighbour_going_to_access_ends_the_trunk_at_once() {
    let (mut w, sw, _) = pair(PortMode::DynamicDesirable, PortMode::DynamicAuto);
    assert!(trunks(&w, sw[0]) && trunks(&w, sw[1]));
    set(&mut w, sw[0], PortMode::Access);
    run(&mut w, 2);
    assert!(!trunks(&w, sw[1]), "it was told");
}

#[test]
fn a_cable_that_goes_away_takes_the_negotiated_trunk_with_it() {
    let (mut w, sw, l) = pair(PortMode::DynamicDesirable, PortMode::DynamicAuto);
    assert!(trunks(&w, sw[1]));
    w.kernel.set_link_up(l, false);
    run(&mut w, 3);
    assert!(!trunks(&w, sw[1]));
    // Nothing ticks on a quiet switch, so the next thing typed at either one is what notices.
    w.kernel.set_link_up(l, true);
    for d in sw {
        w.with_device::<L2Switch, _>(d, |s, ctx| s.refresh(ctx));
    }
    run(&mut w, 5);
    assert!(trunks(&w, sw[1]), "and it comes back with the cable");
}

#[test]
fn a_dynamic_port_facing_a_host_stays_access_and_carries_its_vlan() {
    let mut w = World::new(Trace::new());
    let s = w.add_device(L2Switch::new(2));
    let h1 = w.add_device(Host::new(
        "pc1",
        MacAddr([2, 0, 0, 0, 0, 1]),
        Ipv4Addr::new(10, 0, 0, 1),
        24,
    ));
    let h2 = w.add_device(Host::new(
        "pc2",
        MacAddr([2, 0, 0, 0, 0, 2]),
        Ipv4Addr::new(10, 0, 0, 2),
        24,
    ));
    w.connect((h1, 0), (s, 0), LinkConfig::FAST_ETHERNET);
    w.connect((h2, 0), (s, 1), LinkConfig::FAST_ETHERNET);
    w.with_device::<L2Switch, _>(s, |sw, ctx| {
        sw.configure_port(PortId(0), |c| c.mode = PortMode::DynamicDesirable);
        sw.refresh(ctx);
    });
    run(&mut w, 40);
    assert!(!w.device::<L2Switch>(s).is_trunk(PortId(0)));
    w.with_device::<Host, _>(h1, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    assert_eq!(w.device::<Host>(h1).ping_results.len(), 1);
}

#[test]
fn vlans_flow_over_a_negotiated_trunk_but_not_over_a_failed_negotiation() {
    for (a, b, flows) in [
        (PortMode::DynamicDesirable, PortMode::DynamicAuto, true),
        (PortMode::DynamicAuto, PortMode::DynamicAuto, false),
    ] {
        let mut w = World::new(Trace::new());
        let sw = [1, 2].map(|i| {
            let mut s = L2Switch::new(3);
            s.set_unit(i);
            s.add_vlan(10);
            s.configure_port(PortId(1), |c| c.mode = PortMode::Access);
            s.configure_port(PortId(1), |c| c.access_vlan = 10);
            w.add_device(s)
        });
        let hs = [1u8, 2].map(|n| {
            w.add_device(Host::new(
                &format!("pc{n}"),
                MacAddr([2, 0, 0, 0, 0, n]),
                Ipv4Addr::new(10, 0, 0, n),
                24,
            ))
        });
        w.connect((sw[0], 0), (sw[1], 0), LinkConfig::FAST_ETHERNET);
        w.connect((hs[0], 0), (sw[0], 1), LinkConfig::FAST_ETHERNET);
        w.connect((hs[1], 0), (sw[1], 1), LinkConfig::FAST_ETHERNET);
        set(&mut w, sw[0], a);
        set(&mut w, sw[1], b);
        run(&mut w, 5);
        w.with_device::<Host, _>(hs[0], |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
        run(&mut w, 5);
        assert_eq!(
            !w.device::<Host>(hs[0]).ping_results.is_empty(),
            flows,
            "{a:?} / {b:?}"
        );
    }
}
