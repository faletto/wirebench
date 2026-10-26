//! Switch virtual interfaces: a switch that answers for, and routes between, its VLANs.

use engine::World;
use kernel::{DeviceId, LinkConfig, PortId, SimTime, Trace};
use netdev::{Ctx, Device, DeviceEvent, Host, L2Switch};
use std::any::Any;
use wire::{EthernetFrame, Ipv4Addr, MacAddr};

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

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

fn host(n: u8, net: u8) -> Host {
    let mut h = Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, 0, n]),
        ip(10, 0, net, n),
        24,
    );
    h.gateway = Some(ip(10, 0, net, 1));
    h
}

fn svi(w: &mut World, sw: DeviceId, vlan: u16, net: u8) -> usize {
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.add_vlan(vlan);
        let i = s.add_svi(vlan);
        s.l3_mut()
            .set_address(PortId(i as u16), Some((ip(10, 0, net, 1), 24)));
        i
    })
}

fn ping(w: &mut World, from: DeviceId, to: Ipv4Addr) -> bool {
    let before = w.device::<Host>(from).ping_results.len();
    w.with_device::<Host, _>(from, |h, c| h.ping(c, to));
    w.run_to_idle();
    w.device::<Host>(from).ping_results.len() > before
}

/// pc1 (10.0.10.2) in VLAN 10 on port 0, pc2 (10.0.20.2) in VLAN 20 on port 1, SVIs for both.
fn lab() -> (World, DeviceId, [DeviceId; 2]) {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(4));
    let hs = [w.add_device(host(2, 10)), w.add_device(host(2, 20))];
    for (i, h) in hs.iter().enumerate() {
        w.connect((*h, 0), (sw, i as u16), LinkConfig::FAST_ETHERNET);
        w.with_device::<L2Switch, _>(sw, |s, _| {
            s.add_vlan(10 * (i as u16 + 1));
            s.configure_port(PortId(i as u16), |c| c.access_vlan = 10 * (i as u16 + 1));
        });
    }
    svi(&mut w, sw, 10, 10);
    svi(&mut w, sw, 20, 20);
    (w, sw, hs)
}

#[test]
fn with_ip_routing_the_switch_routes_between_its_vlans() {
    let (mut w, sw, hs) = lab();
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_routing(true));
    assert!(
        ping(&mut w, hs[0], ip(10, 0, 20, 2)),
        "pc1 to pc2 through the switch"
    );
    assert!(ping(&mut w, hs[1], ip(10, 0, 10, 2)), "and back");
}

#[test]
fn without_ip_routing_the_switch_answers_for_itself_but_does_not_forward() {
    let (mut w, _, hs) = lab();
    assert!(ping(&mut w, hs[0], ip(10, 0, 10, 1)), "its own SVI answers");
    assert!(ping(&mut w, hs[1], ip(10, 0, 20, 1)));
    assert!(
        !ping(&mut w, hs[0], ip(10, 0, 20, 2)),
        "no routing between VLANs"
    );
}

#[test]
fn an_svi_is_down_when_its_vlan_has_no_port_and_comes_back_with_one() {
    let (mut w, sw, hs) = lab();
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_routing(true));
    // Move pc2's port out of VLAN 20: nothing carries it, so SVI 20 goes down.
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.configure_port(PortId(1), |c| c.access_vlan = 10)
    });
    let i = w.device::<L2Switch>(sw).l3().svi_index(20).unwrap();
    assert!(!w.device::<L2Switch>(sw).l3().is_up(i));
    assert!(w.device::<L2Switch>(sw).l3().fib().routes().count() < 2);
    assert!(!ping(&mut w, hs[0], ip(10, 0, 20, 2)));
    w.with_device::<L2Switch, _>(sw, |s, _| {
        s.configure_port(PortId(1), |c| c.access_vlan = 20)
    });
    assert!(w.device::<L2Switch>(sw).l3().is_up(i));
    assert!(ping(&mut w, hs[0], ip(10, 0, 20, 2)));
}

#[test]
fn deleting_the_vlan_takes_its_svi_down_without_deleting_it() {
    let (mut w, sw, _) = lab();
    w.with_device::<L2Switch, _>(sw, |s, _| s.remove_vlan(10));
    let i = w.device::<L2Switch>(sw).l3().svi_index(10).unwrap();
    assert!(!w.device::<L2Switch>(sw).l3().is_up(i));
    w.with_device::<L2Switch, _>(sw, |s, _| s.add_vlan(10));
    assert!(w.device::<L2Switch>(sw).l3().is_up(i));
}

#[test]
fn frames_addressed_to_an_svi_are_not_flooded_to_other_ports() {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(3));
    let pc = w.add_device(host(2, 10));
    let tap = w.add_device(Tap::default());
    w.connect((pc, 0), (sw, 0), LinkConfig::FAST_ETHERNET);
    w.connect((tap, 0), (sw, 1), LinkConfig::FAST_ETHERNET);
    for p in 0..2 {
        w.with_device::<L2Switch, _>(sw, |s, _| {
            s.add_vlan(10);
            s.configure_port(PortId(p), |c| c.access_vlan = 10)
        });
    }
    svi(&mut w, sw, 10, 10);
    assert!(ping(&mut w, pc, ip(10, 0, 10, 1)));
    let svi_mac = w.device::<L2Switch>(sw).l3().interfaces()[0].mac;
    let heard: Vec<EthernetFrame> = w
        .device::<Tap>(tap)
        .seen
        .iter()
        .map(|f| EthernetFrame::parse(f).unwrap())
        .collect();
    assert!(
        heard.iter().all(|f| f.dst != svi_mac),
        "unicast to the SVI stayed inside the switch"
    );
    assert!(
        heard.iter().any(|f| f.dst.is_broadcast()),
        "the ARP request was a broadcast, so the tap hears it"
    );
    assert!(
        heard.iter().all(|f| f.src != svi_mac),
        "the SVI's replies are unicast to the learned port, not flooded"
    );
}

#[test]
fn the_default_gateway_applies_only_while_routing_is_off() {
    let (mut w, sw, _) = lab();
    let far = ip(8, 8, 8, 8);
    let gw = ip(10, 0, 10, 254);
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_default_gateway(Some(gw)));
    let has = |w: &World| w.device::<L2Switch>(sw).l3().fib().lookup(far).is_some();
    assert!(has(&w), "management traffic leaves through the gateway");
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_routing(true));
    assert!(!has(&w), "a routing switch ignores it");
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_routing(false));
    assert!(has(&w));
}

#[test]
fn an_unanswered_arp_on_an_svi_times_out_and_the_switch_keeps_working() {
    let (mut w, sw, hs) = lab();
    w.with_device::<L2Switch, _>(sw, |s, _| s.l3_mut().set_routing(true));
    // pc1 asks for a host in VLAN 20 that does not exist: the SVI ARPs, nobody answers, and the
    // timer (delivered to the switch) clears the pending packet.
    assert!(!ping(&mut w, hs[0], ip(10, 0, 20, 77)));
    assert!(w.kernel.now() > SimTime::from_secs(1));
    assert!(
        ping(&mut w, hs[0], ip(10, 0, 20, 2)),
        "later traffic is unaffected"
    );
}
