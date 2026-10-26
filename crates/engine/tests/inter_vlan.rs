//! Router subinterfaces (router on a stick): what goes on the wire, and what the router accepts.

use engine::lab::load_and_build;
use engine::shell::{running_config, Shell};
use engine::World;
use kernel::{DeviceId, LinkConfig, PortId, Trace};
use netdev::{Ctx, Device, DeviceEvent, EncapError, Router};
use std::any::Any;
use wire::{ArpOp, ArpPacket, EtherType, EthernetFrame, Ipv4Addr, MacAddr, VlanTag};

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

const ROUTER_MAC: MacAddr = MacAddr([2, 0xaa, 0, 0, 1, 0]);
const TAP_MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 0x55]);

/// A tap wired to port 0 of a router that has subinterfaces .10 (VLAN 10, 10.0.10.1/24) and
/// .20 (VLAN 20, 10.0.20.1/24) and an unaddressed, enabled parent port.
fn rig() -> (World, DeviceId, DeviceId) {
    let mut w = World::new(Trace::new());
    let r = w.add_device(Router::new("r1", 1, 2));
    let tap = w.add_device(Tap::default());
    w.connect((tap, 0), (r, 0), LinkConfig::FAST_ETHERNET);
    w.with_device::<Router, _>(r, |r, _| {
        r.set_shutdown(PortId(0), false);
        for (num, vlan, ip) in [(10u16, 10u16, [10, 0, 10, 1]), (20, 20, [10, 0, 20, 1])] {
            let i = r.add_subinterface(PortId(0), num).unwrap();
            r.set_encapsulation(i, Some(vlan), false).unwrap();
            r.set_address(
                PortId(i as u16),
                Some((Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), 24)),
            );
        }
    });
    (w, r, tap)
}

fn arp_request(vid: Option<u16>, target: [u8; 4]) -> Vec<u8> {
    let req = ArpPacket {
        op: ArpOp::Request,
        sender_mac: TAP_MAC,
        sender_ip: Ipv4Addr::new(target[0], target[1], target[2], 99),
        target_mac: MacAddr::default(),
        target_ip: Ipv4Addr::new(target[0], target[1], target[2], target[3]),
    };
    EthernetFrame {
        dst: MacAddr::BROADCAST,
        src: TAP_MAC,
        vlan: vid.map(|vid| VlanTag {
            pcp: 0,
            dei: false,
            vid,
        }),
        ethertype: EtherType::ARP,
        payload: req.emit(),
    }
    .emit()
}

fn ask(w: &mut World, tap: DeviceId, frame: Vec<u8>) -> Vec<EthernetFrame> {
    w.with_device::<Tap, _>(tap, |t, ctx| {
        t.seen.clear();
        ctx.transmit(PortId(0), frame)
    });
    w.run_to_idle();
    w.device::<Tap>(tap)
        .seen
        .iter()
        .map(|f| EthernetFrame::parse(f).unwrap())
        .collect()
}

#[test]
fn a_subinterface_answers_in_its_vlan_with_the_ports_mac() {
    let (mut w, _, tap) = rig();
    for (vid, target) in [(10, [10, 0, 10, 1]), (20, [10, 0, 20, 1])] {
        let replies = ask(&mut w, tap, arp_request(Some(vid), target));
        assert_eq!(replies.len(), 1, "VLAN {vid}");
        assert_eq!(
            replies[0].vlan.map(|t| t.vid),
            Some(vid),
            "tagged for its VLAN"
        );
        assert_eq!(
            replies[0].src, ROUTER_MAC,
            "subinterfaces share the port's MAC"
        );
        assert_eq!(replies[0].dst, TAP_MAC);
    }
}

#[test]
fn frames_for_the_wrong_vlan_or_no_subinterface_are_ignored() {
    let (mut w, _, tap) = rig();
    // Asking for the VLAN 20 address inside a VLAN 10 tag, a VLAN with no subinterface, and
    // untagged (the parent has no address).
    assert!(ask(&mut w, tap, arp_request(Some(10), [10, 0, 20, 1])).is_empty());
    assert!(ask(&mut w, tap, arp_request(Some(30), [10, 0, 10, 1])).is_empty());
    assert!(ask(&mut w, tap, arp_request(None, [10, 0, 10, 1])).is_empty());
}

#[test]
fn the_native_subinterface_takes_untagged_frames_and_replies_untagged() {
    let (mut w, r, tap) = rig();
    w.with_device::<Router, _>(r, |r, _| {
        let i = r.add_subinterface(PortId(0), 1).unwrap();
        r.set_encapsulation(i, Some(1), true).unwrap();
        r.set_address(PortId(i as u16), Some((Ipv4Addr::new(10, 0, 1, 1), 24)));
    });
    let replies = ask(&mut w, tap, arp_request(None, [10, 0, 1, 1]));
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].vlan, None, "native traffic is untagged");
    // A tagged frame with the native VLAN's id does not belong to the native subinterface.
    assert!(ask(&mut w, tap, arp_request(Some(1), [10, 0, 1, 1])).is_empty());
}

#[test]
fn shutting_the_port_or_the_subinterface_silences_it() {
    let (mut w, r, tap) = rig();
    w.with_device::<Router, _>(r, |r, _| r.set_shutdown(PortId(2), true));
    assert!(ask(&mut w, tap, arp_request(Some(10), [10, 0, 10, 1])).is_empty());
    assert_eq!(
        ask(&mut w, tap, arp_request(Some(20), [10, 0, 20, 1])).len(),
        1
    );
    w.with_device::<Router, _>(r, |r, _| r.set_shutdown(PortId(2), false));
    assert_eq!(
        ask(&mut w, tap, arp_request(Some(10), [10, 0, 10, 1])).len(),
        1
    );
    w.with_device::<Router, _>(r, |r, _| r.set_shutdown(PortId(0), true));
    assert!(ask(&mut w, tap, arp_request(Some(10), [10, 0, 10, 1])).is_empty());
    assert!(
        !w.device::<Router>(r).is_up(2),
        "a subinterface is down with its port"
    );
}

#[test]
fn encapsulation_rules() {
    let (mut w, r, _) = rig();
    w.with_device::<Router, _>(r, |r, _| {
        assert_eq!(
            r.set_encapsulation(0, Some(5), false),
            Err(EncapError::NotSubinterface)
        );
        let third = r.add_subinterface(PortId(0), 30).unwrap();
        assert_eq!(
            r.set_encapsulation(third, Some(10), false),
            Err(EncapError::InUse)
        );
        // The same VLAN on a different physical port is fine.
        let other = r.add_subinterface(PortId(1), 10).unwrap();
        assert_eq!(r.set_encapsulation(other, Some(10), false), Ok(()));
        // Adding the same subinterface again finds it instead of duplicating it.
        assert_eq!(r.add_subinterface(PortId(0), 30), Some(third));
        assert_eq!(r.add_subinterface(PortId(7), 1), None, "no such port");
        assert!(!r.remove_subinterface(0), "physical interfaces stay");
        assert!(r.remove_subinterface(third));
        assert_eq!(r.interfaces().len(), 2 + 2 + 1);
    });
}

#[test]
fn removing_a_subinterface_drops_its_route() {
    let (mut w, r, _) = rig();
    let before = w.device::<Router>(r).fib().routes().count();
    w.with_device::<Router, _>(r, |r, _| assert!(r.remove_subinterface(2)));
    assert_eq!(w.device::<Router>(r).fib().routes().count(), before - 1);
}

#[test]
fn subinterface_config_survives_a_save_and_reload() {
    const LAB: &str = r#"format = "labsim-lab"
version = 1

[[device]]
name = "r1"
template = "router-2g"
pos = [0, 0]
"#;
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    let id = lab.devices[0].id;
    let mut sh = Shell::router(id);
    for l in [
        "enable",
        "configure terminal",
        "interface g0/0",
        "no shutdown",
        "interface g0/0.10",
        "encapsulation dot1q 10",
        "ip address 10.0.10.1 255.255.255.0",
        "interface g0/0.99",
        "encapsulation dot1q 99 native",
        "ip address 10.0.99.1 255.255.255.0",
        "shutdown",
        "end",
    ] {
        assert_eq!(sh.exec(&mut lab.world, l).unwrap(), "", "{l}");
    }
    let cfg = running_config(lab.world.device::<Router>(id));
    assert!(cfg.contains("encapsulation dot1q 99 native"), "{cfg}");
    let text = format!("{LAB}startup_config = '''\n{cfg}\n'''\n");
    let again = load_and_build(&text, Trace::new()).unwrap();
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    assert_eq!(
        running_config(again.world.device::<Router>(again.devices[0].id)),
        cfg
    );
}

#[test]
fn bad_subinterface_names_are_refused() {
    let mut lab = load_and_build(
        "format = \"labsim-lab\"\nversion = 1\n[[device]]\nname = \"r1\"\ntemplate = \"router-2g\"\npos = [0, 0]\n",
        Trace::new(),
    )
    .unwrap();
    let mut sh = Shell::router(lab.devices[0].id);
    for l in ["enable", "configure terminal"] {
        sh.exec(&mut lab.world, l);
    }
    for bad in ["g0/5.10", "g0/0.", "g0/0.x", "g0/0.70000", "x0/0.1"] {
        let out = sh
            .exec(&mut lab.world, &format!("interface {bad}"))
            .unwrap();
        assert!(out.contains("No such interface"), "{bad}: {out}");
    }
    let out = sh.exec(&mut lab.world, "no interface g0/0.10").unwrap();
    assert!(out.contains("No such interface"), "{out}");
    assert_eq!(sh.prompt(&lab.world), "r1(config)#", "still in config mode");
}
