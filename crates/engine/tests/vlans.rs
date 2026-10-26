//! VLAN behaviour of the switch: access and trunk ports, tagging, native VLAN, allowed lists.

use engine::World;
use kernel::{DeviceId, LinkConfig, PortId, Trace};
use netdev::{Ctx, Device, DeviceEvent, Host, L2Switch, PortMode};
use std::any::Any;
use wire::{EtherType, EthernetFrame, Ipv4Addr, MacAddr, VlanTag};

/// Records every frame it receives and sends raw frames on demand.
#[derive(Default)]
struct Tap {
    seen: Vec<Vec<u8>>,
}

impl Device for Tap {
    fn handle(&mut self, _: &mut Ctx, ev: DeviceEvent) {
        if let DeviceEvent::FrameRx { frame, .. } = ev {
            self.seen.push(frame);
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn host(n: u8) -> Host {
    Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, 0, n]),
        Ipv4Addr::new(10, 0, 0, n),
        24,
    )
}

fn access(w: &mut World, sw: DeviceId, port: u16, vlan: u16) {
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.add_vlan(vlan);
        s.configure_port(PortId(port), |c| c.access_vlan = vlan);
    });
}

fn trunk(w: &mut World, sw: DeviceId, port: u16) {
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.configure_port(PortId(port), |c| c.mode = PortMode::Trunk)
    });
}

fn ping(w: &mut World, from: DeviceId, to: u8) -> bool {
    let before = w.device::<Host>(from).ping_results.len();
    w.with_device::<Host, _>(from, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, to)));
    w.run_to_idle();
    w.device::<Host>(from).ping_results.len() > before
}

/// One switch, four hosts: pc1 and pc2 in VLAN 10, pc3 and pc4 in VLAN 20.
fn one_switch() -> (World, [DeviceId; 4], DeviceId) {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(4));
    let hs = [1u8, 2, 3, 4].map(|n| w.add_device(host(n)));
    for (i, h) in hs.iter().enumerate() {
        w.connect((*h, 0), (sw, i as u16), LinkConfig::FAST_ETHERNET);
        access(&mut w, sw, i as u16, if i < 2 { 10 } else { 20 });
    }
    (w, hs, sw)
}

#[test]
fn hosts_in_different_vlans_cannot_reach_each_other_but_same_vlan_can() {
    let (mut w, hs, _) = one_switch();
    assert!(ping(&mut w, hs[0], 2), "same VLAN");
    assert!(ping(&mut w, hs[2], 4), "same VLAN, other id");
    assert!(!ping(&mut w, hs[0], 3), "10 to 20 is isolated");
}

#[test]
fn mac_table_is_kept_per_vlan() {
    let (mut w, hs, sw) = one_switch();
    ping(&mut w, hs[0], 2);
    ping(&mut w, hs[2], 4);
    let t = w.kernel.now();
    let mut seen: Vec<(u16, u16)> = w
        .device::<L2Switch>(sw)
        .mac_entries(t)
        .iter()
        .map(|e| (e.vlan, e.port.0))
        .collect();
    seen.sort();
    assert_eq!(seen, [(10, 0), (10, 1), (20, 2), (20, 3)]);
}

#[test]
fn a_port_in_a_vlan_that_does_not_exist_carries_nothing() {
    let (mut w, hs, sw) = one_switch();
    assert!(w.with_device::<L2Switch, _>(sw, |s, _| s.remove_vlan(10)));
    assert!(!ping(&mut w, hs[0], 2));
    // Recreating the VLAN brings the ports back without touching them.
    w.with_device::<L2Switch, _>(sw, |s, _| s.add_vlan(10));
    assert!(ping(&mut w, hs[0], 2));
}

#[test]
fn vlan_one_cannot_be_deleted() {
    let mut s = L2Switch::new(2);
    assert!(!s.remove_vlan(1));
    assert!(s.vlan_exists(1));
}

/// pc1 (10) and pc3 (20) on sw1; pc2 (10) and pc4 (20) on sw2; port 2 of each is the trunk.
fn two_switches() -> (World, [DeviceId; 4], [DeviceId; 2]) {
    let mut w = World::new(Trace::new());
    let sws = [
        w.add_device(L2Switch::new(3)),
        w.add_device(L2Switch::new(3)),
    ];
    let hs = [1u8, 2, 3, 4].map(|n| w.add_device(host(n)));
    // pc1 and pc3 on sw1, pc2 and pc4 on sw2.
    for (h, sw, port, vlan) in [(0, 0, 0, 10), (2, 0, 1, 20), (1, 1, 0, 10), (3, 1, 1, 20)] {
        w.connect((hs[h], 0), (sws[sw], port), LinkConfig::FAST_ETHERNET);
        access(&mut w, sws[sw], port, vlan);
    }
    w.connect((sws[0], 2), (sws[1], 2), LinkConfig::FAST_ETHERNET);
    for sw in sws {
        trunk(&mut w, sw, 2);
        w.with_device::<L2Switch, _>(sw, |s, _| s.add_vlan(20));
    }
    (w, hs, sws)
}

#[test]
fn a_trunk_carries_both_vlans_and_keeps_them_apart() {
    let (mut w, hs, _) = two_switches();
    assert!(
        ping(&mut w, hs[0], 2),
        "pc1 to pc2 across the trunk, VLAN 10"
    );
    assert!(
        ping(&mut w, hs[2], 4),
        "pc3 to pc4 across the trunk, VLAN 20"
    );
    assert!(!ping(&mut w, hs[0], 4), "VLAN 10 to VLAN 20 stays isolated");
}

#[test]
fn removing_a_vlan_from_the_allowed_list_cuts_only_that_vlan() {
    let (mut w, hs, sws) = two_switches();
    w.with_device::<L2Switch, _>(sws[0], |s, _| {
        s.configure_port(PortId(2), |c| {
            c.allowed.remove(&20);
        })
    });
    assert!(ping(&mut w, hs[0], 2), "VLAN 10 still crosses");
    assert!(!ping(&mut w, hs[2], 4), "VLAN 20 no longer does");
}

#[test]
fn trunk_frames_are_tagged_except_the_native_vlan() {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(3));
    let tap = w.add_device(Tap::default());
    let (a, b) = (w.add_device(host(1)), w.add_device(host(2)));
    w.connect((a, 0), (sw, 0), LinkConfig::FAST_ETHERNET);
    w.connect((b, 0), (sw, 1), LinkConfig::FAST_ETHERNET);
    w.connect((tap, 0), (sw, 2), LinkConfig::FAST_ETHERNET);
    access(&mut w, sw, 0, 10);
    access(&mut w, sw, 1, 1);
    trunk(&mut w, sw, 2);
    w.with_device::<L2Switch, _>(sw, |s, _| s.add_vlan(10));
    // ARP broadcasts flood: VLAN 10 from pc1, the native VLAN 1 from pc2.
    w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 9)));
    w.with_device::<Host, _>(b, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 9)));
    w.run_to_idle();
    let frames: Vec<EthernetFrame> = w
        .device::<Tap>(tap)
        .seen
        .iter()
        .map(|f| EthernetFrame::parse(f).unwrap())
        .collect();
    let from = |mac: u8| {
        frames
            .iter()
            .find(|f| f.src == MacAddr([2, 0, 0, 0, 0, mac]))
    };
    assert_eq!(from(1).unwrap().vlan.map(|t| t.vid), Some(10));
    assert_eq!(from(2).unwrap().vlan, None, "native VLAN goes untagged");
    assert_eq!(
        from(1).unwrap().ethertype,
        EtherType::ARP,
        "inner type is kept"
    );
}

#[test]
fn a_tagged_frame_on_a_trunk_lands_in_its_vlan_and_a_disallowed_one_is_dropped() {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(3));
    let tap = w.add_device(Tap::default());
    let (a, b) = (w.add_device(Tap::default()), w.add_device(Tap::default()));
    w.connect((tap, 0), (sw, 0), LinkConfig::FAST_ETHERNET);
    w.connect((a, 0), (sw, 1), LinkConfig::FAST_ETHERNET);
    w.connect((b, 0), (sw, 2), LinkConfig::FAST_ETHERNET);
    trunk(&mut w, sw, 0);
    access(&mut w, sw, 1, 10);
    access(&mut w, sw, 2, 20);
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.configure_port(PortId(0), |c| {
            c.allowed.remove(&30);
        })
    });
    let send = |w: &mut World, vid: u16| {
        let f = EthernetFrame {
            dst: MacAddr::BROADCAST,
            src: MacAddr([2, 0, 0, 0, 0, 0x77]),
            vlan: Some(VlanTag {
                pcp: 5,
                dei: false,
                vid,
            }),
            ethertype: EtherType(0x88b5),
            payload: vec![0; 46],
        };
        w.with_device::<Tap, _>(tap, |_, ctx| ctx.transmit(PortId(0), f.emit()));
        w.run_to_idle();
    };
    send(&mut w, 10);
    send(&mut w, 30);
    assert_eq!(
        w.device::<Tap>(a).seen.len(),
        1,
        "VLAN 10 reaches its access port"
    );
    assert_eq!(w.device::<Tap>(b).seen.len(), 0, "VLAN 20 is not involved");
    // The access port sends it untagged.
    assert_eq!(
        EthernetFrame::parse(&w.device::<Tap>(a).seen[0])
            .unwrap()
            .vlan,
        None
    );
}

#[test]
fn an_access_port_drops_frames_tagged_for_another_vlan() {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(2));
    let (tx, rx) = (w.add_device(Tap::default()), w.add_device(Tap::default()));
    w.connect((tx, 0), (sw, 0), LinkConfig::FAST_ETHERNET);
    w.connect((rx, 0), (sw, 1), LinkConfig::FAST_ETHERNET);
    access(&mut w, sw, 0, 10);
    access(&mut w, sw, 1, 10);
    w.with_device::<L2Switch, _>(sw, |s, _| s.add_vlan(20));
    for vid in [20u16, 10] {
        let f = EthernetFrame {
            dst: MacAddr::BROADCAST,
            src: MacAddr([2, 0, 0, 0, 0, 0x77]),
            vlan: Some(VlanTag {
                pcp: 0,
                dei: false,
                vid,
            }),
            ethertype: EtherType(0x88b5),
            payload: vec![0; 46],
        };
        w.with_device::<Tap, _>(tx, |_, ctx| ctx.transmit(PortId(0), f.emit()));
        w.run_to_idle();
    }
    assert_eq!(
        w.device::<Tap>(rx).seen.len(),
        1,
        "only the VLAN 10 tag is accepted"
    );
}

#[test]
fn a_native_vlan_mismatch_leaks_untagged_traffic_between_vlans() {
    // pc1 sits in VLAN 1, which is sw1's native VLAN and travels untagged. sw2 thinks the native
    // VLAN is 10, so it files the frames under 10 and pc2 (VLAN 10) hears them.
    let (mut w, hs, sws) = two_switches();
    access(&mut w, sws[0], 0, 1);
    w.with_device::<L2Switch, _>(sws[1], |s, _| {
        s.configure_port(PortId(2), |c| c.native_vlan = 10)
    });
    assert!(
        ping(&mut w, hs[0], 2),
        "traffic crosses VLANs through the mismatch"
    );
}

#[test]
fn changing_a_ports_vlan_forgets_what_was_learned_on_it() {
    let (mut w, hs, sw) = one_switch();
    ping(&mut w, hs[0], 2);
    let t = w.kernel.now();
    assert_eq!(w.device::<L2Switch>(sw).mac_entries(t).len(), 2);
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.configure_port(PortId(0), |c| c.access_vlan = 20)
    });
    let left: Vec<u16> = w
        .device::<L2Switch>(sw)
        .mac_entries(t)
        .iter()
        .map(|e| e.port.0)
        .collect();
    assert_eq!(left, [1], "pc1's entry on port 0 is gone");
}

mod never_panics {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Whatever arrives, on any kind of port, the switch neither panics nor sends a frame
        /// back out of the port it came in on.
        #[test]
        fn arbitrary_frames_on_any_port_are_survivable(
            frame in proptest::collection::vec(any::<u8>(), 0..80),
            tagged in any::<bool>(),
            vid in 0u16..4096,
            port in 0u16..4,
            trunk_port in any::<bool>(),
        ) {
            let mut w = World::new(Trace::new());
            let sw = w.add_device(L2Switch::new(3));
            let taps: Vec<DeviceId> = (0..3).map(|_| w.add_device(Tap::default())).collect();
            for (i, t) in taps.iter().enumerate() {
                w.connect((*t, 0), (sw, i as u16), LinkConfig::FAST_ETHERNET);
            }
            access(&mut w, sw, 1, 10);
            if trunk_port {
                trunk(&mut w, sw, 0);
            }
            let mut f = frame;
            if tagged && f.len() >= 18 {
                f[12..14].copy_from_slice(&[0x81, 0x00]);
                f[14] = (f[14] & 0xf0) | (vid >> 8) as u8;
                f[15] = vid as u8;
            }
            let src = taps[(port % 3) as usize];
            w.with_device::<Tap, _>(src, |_, ctx| ctx.transmit(PortId(0), f));
            w.run_to_idle();
            prop_assert!(w.device::<Tap>(src).seen.is_empty(), "echoed to the sender");
        }
    }
}
