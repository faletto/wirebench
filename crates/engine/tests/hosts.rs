use engine::World;
use kernel::{LinkConfig, SimTime, Trace};
use netdev::{Host, L2Switch};
use proptest::prelude::*;
use wire::{Ipv4Addr, MacAddr};

fn host(n: u8) -> Host {
    Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, 0, n]),
        Ipv4Addr::new(10, 0, 0, n),
        24,
    )
}

fn two_hosts() -> (World, kernel::DeviceId, kernel::DeviceId) {
    let mut w = World::new(Trace::keeping());
    let a = w.add_device(host(1));
    let b = w.add_device(host(2));
    w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
    (w, a, b)
}

#[test]
fn two_hosts_ping_golden() {
    let (w, a, _b) = engine::scenarios::two_hosts_ping();
    let r = w.device::<Host>(a).ping_results.clone();
    assert_eq!(r.len(), 1);
    // ARP req + reply are 60 B (4.8us ser + 1us prop); echo req + reply are 74 B (5.92us + 1us).
    assert_eq!(r[0].rtt, SimTime(2 * 5_800 + 2 * 6_920));
    assert_eq!(w.trace_hash(), BLESSED);
}

// Re-bless deliberately when the canonical trace or protocol behavior changes.
const BLESSED: u64 = 1500696035514225285;

#[test]
fn ping_trace_is_reproducible() {
    let run = || {
        let (mut w, a, _) = two_hosts();
        w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
        w.run_to_idle();
        w.trace_hash()
    };
    assert_eq!(run(), run());
}

#[test]
fn arp_entry_expires() {
    let (mut w, a, _) = two_hosts();
    w.with_device::<Host, _>(a, |h, _| h.arp_ttl = SimTime::from_secs(10));
    w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    let ip = Ipv4Addr::new(10, 0, 0, 2);
    let t = w.kernel.now();
    assert!(w.device::<Host>(a).arp_lookup(ip, t).is_some());
    assert!(w
        .device::<Host>(a)
        .arp_lookup(ip, t + SimTime::from_secs(11))
        .is_none());
}

#[test]
fn ping_unreachable_times_out_arp() {
    let mut w = World::new(Trace::new());
    let a = w.add_device(host(1));
    let b = w.add_device(host(2));
    let l = w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
    w.kernel.set_link_up(l, false);
    w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    assert!(w.device::<Host>(a).ping_results.is_empty());
}

fn three_behind_switch() -> (World, [kernel::DeviceId; 3], kernel::DeviceId) {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(3));
    let hs = [1u8, 2, 3].map(|n| w.add_device(host(n)));
    for (i, h) in hs.iter().enumerate() {
        w.connect((*h, 0), (sw, i as u16), LinkConfig::FAST_ETHERNET);
    }
    (w, hs, sw)
}

#[test]
fn switch_learns_then_unicasts() {
    let (mut w, hs, sw) = three_behind_switch();
    w.with_device::<Host, _>(hs[0], |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    assert_eq!(w.device::<Host>(hs[0]).ping_results.len(), 1);
    let t = w.kernel.now();
    let table = w.device::<L2Switch>(sw).mac_table(t);
    assert_eq!(table.len(), 2);
}

#[test]
fn unknown_unicast_floods() {
    // Host 1 pings a never-seen host 3 IP before anyone has spoken: ARP broadcast reaches all.
    let (mut w, hs, _) = three_behind_switch();
    w.with_device::<Host, _>(hs[0], |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 3)));
    w.run_to_idle();
    assert_eq!(w.device::<Host>(hs[0]).ping_results.len(), 1);
}

#[test]
fn mac_table_ages_out() {
    let (mut w, hs, sw) = three_behind_switch();
    w.with_device::<Host, _>(hs[0], |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    let t = w.kernel.now() + SimTime::from_secs(301);
    assert!(w.device::<L2Switch>(sw).mac_table(t).is_empty());
}

proptest! {
    /// A frame is never sent back out its ingress port: with one host per port, a host
    /// never receives a frame that it sourced itself (hosts count unexpected rx via ARP cache).
    #[test]
    fn never_sends_back_out_ingress(pings in proptest::collection::vec((0usize..3, 1u8..4), 1..8)) {
        let (mut w, hs, _) = three_behind_switch();
        for (from, to) in pings {
            let dst = Ipv4Addr::new(10, 0, 0, to);
            w.with_device::<Host, _>(hs[from], |h, c| h.ping(c, dst));
            w.run_to_idle();
        }
        // Hosts only learn ARP entries for targeted requests; a reflected own-frame would
        // make a host ARP-resolve itself, since sender_ip == ip == target_ip for a loopback.
        for (i, h) in hs.iter().enumerate() {
            let own = Ipv4Addr::new(10, 0, 0, i as u8 + 1);
            let t = w.kernel.now();
            prop_assert!(w.device::<Host>(*h).arp_lookup(own, t).is_none());
        }
    }
}
