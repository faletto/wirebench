//! Probes on the wire: per-hop TTL expiry, who answers, and from which address.

use engine::chain::static_chain;
use engine::World;
use kernel::{DeviceId, SimTime};
use netdev::{Host, ProbeBody, ProbeReply, Router};
use wire::{IcmpErrorKind, Ipv4Addr};

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

const ID: u16 = 0x7472;

fn host_echo(w: &mut World, h: DeviceId, dst: Ipv4Addr, ttl: u8, seq: u16) {
    w.with_device::<Host, _>(h, |h, c| h.send_echo_probe(c, dst, ttl, ID, seq));
    w.run_to_idle();
}

fn router_udp(w: &mut World, r: DeviceId, dst: Ipv4Addr, ttl: u8, dport: u16) -> bool {
    let sent = w.with_device::<Router, _>(r, |r, c| {
        let ok = r.send_udp_probe(c, dst, ttl, dport);
        r.flush_outbox(c);
        ok
    });
    w.run_to_idle();
    sent
}

fn router_echo(w: &mut World, r: DeviceId, dst: Ipv4Addr, ttl: u8, seq: u16) -> bool {
    let sent = w.with_device::<Router, _>(r, |r, c| {
        let ok = r.send_echo_probe(c, dst, ttl, ID, seq);
        r.flush_outbox(c);
        ok
    });
    w.run_to_idle();
    sent
}

fn host_log(w: &World, h: DeviceId) -> Vec<ProbeReply> {
    w.device::<Host>(h).probes.clone()
}

fn router_log(w: &World, r: DeviceId) -> Vec<ProbeReply> {
    w.device::<Router>(r).probes.clone()
}

fn kind(r: &ProbeReply) -> Option<IcmpErrorKind> {
    match &r.body {
        ProbeBody::Error { kind, .. } => Some(*kind),
        _ => None,
    }
}

#[test]
fn a_hosts_echo_probes_expire_hop_by_hop_and_the_last_one_is_answered() {
    let mut c = static_chain();
    let dst = ip(10, 0, 2, 2);
    for ttl in 1..=3u8 {
        host_echo(&mut c.world, c.pc1, dst, ttl, u16::from(ttl));
    }
    let log = host_log(&c.world, c.pc1);
    assert_eq!(log.len(), 3);
    // Each router answers from the address of the interface the probe came in on.
    assert_eq!(
        (kind(&log[0]), log[0].from),
        (Some(IcmpErrorKind::TimeExceeded), ip(10, 0, 1, 1))
    );
    assert_eq!(
        (kind(&log[1]), log[1].from),
        (Some(IcmpErrorKind::TimeExceeded), ip(10, 0, 12, 2))
    );
    assert_eq!(log[2].body, ProbeBody::EchoReply { ident: ID, seq: 3 });
    assert_eq!(log[2].from, dst);
    // The expired probes quote enough to be matched to what was sent.
    assert_eq!(log[0].echo_id(), Some((ID, 1)));
    assert_eq!(log[1].echo_id(), Some((ID, 2)));
    assert_eq!(log[2].echo_id(), Some((ID, 3)));
    // Times only ever go forward, and farther hops take longer.
    assert!(log[0].at < log[1].at && log[1].at < log[2].at);
}

#[test]
fn probes_do_not_count_as_pings() {
    let mut c = static_chain();
    host_echo(&mut c.world, c.pc1, ip(10, 0, 2, 2), 8, 1);
    assert!(c.world.device::<Host>(c.pc1).ping_results.is_empty());
    // And a real ping still works alongside.
    c.world
        .with_device::<Host, _>(c.pc1, |h, ctx| h.ping(ctx, ip(10, 0, 2, 2)));
    c.world.run_to_idle();
    assert_eq!(c.world.device::<Host>(c.pc1).ping_results.len(), 1);
}

#[test]
fn a_routers_udp_probes_end_with_port_unreachable_from_the_destination_host() {
    let mut c = static_chain();
    let dst = ip(10, 0, 2, 2);
    assert!(router_udp(&mut c.world, c.r1, dst, 1, 33_434));
    assert!(router_udp(&mut c.world, c.r1, dst, 2, 33_435));
    let log = router_log(&c.world, c.r1);
    assert_eq!(log.len(), 2);
    assert_eq!(
        (kind(&log[0]), log[0].from),
        (Some(IcmpErrorKind::TimeExceeded), ip(10, 0, 12, 2))
    );
    assert_eq!(log[0].quoted_udp_dport(), Some(33_434));
    assert_eq!(
        (kind(&log[1]), log[1].from),
        (Some(IcmpErrorKind::DestUnreachable(3)), dst),
        "the host says port unreachable"
    );
    assert_eq!(log[1].quoted_udp_dport(), Some(33_435));
}

#[test]
fn a_router_answers_a_probe_from_the_address_that_was_probed() {
    let mut c = static_chain();
    // r2's far interface address, reached through its near one.
    assert!(router_udp(&mut c.world, c.r1, ip(10, 0, 2, 1), 5, 33_434));
    let log = router_log(&c.world, c.r1);
    assert_eq!(log.len(), 1);
    assert_eq!(kind(&log[0]), Some(IcmpErrorKind::DestUnreachable(3)));
    assert_eq!(
        log[0].from,
        ip(10, 0, 2, 1),
        "not 10.0.12.2, which it came in on"
    );
}

#[test]
fn a_routers_own_addresses_answer_at_once_and_unroutable_ones_are_refused() {
    let mut c = static_chain();
    assert!(router_udp(&mut c.world, c.r1, ip(10, 0, 1, 1), 1, 33_434));
    assert!(router_echo(&mut c.world, c.r1, ip(10, 0, 12, 1), 1, 4));
    let log = router_log(&c.world, c.r1);
    assert_eq!(kind(&log[0]), Some(IcmpErrorKind::DestUnreachable(3)));
    assert_eq!(log[1].body, ProbeBody::EchoReply { ident: ID, seq: 4 });
    assert!(
        !router_udp(&mut c.world, c.r1, ip(172, 16, 0, 1), 5, 33_434),
        "no route"
    );
    assert!(!router_echo(&mut c.world, c.r1, ip(172, 16, 0, 1), 5, 1));
}

#[test]
fn a_router_ping_gets_an_echo_reply_from_a_host_and_from_another_router() {
    let mut c = static_chain();
    assert!(router_echo(&mut c.world, c.r1, ip(10, 0, 2, 2), 255, 1));
    assert!(router_echo(&mut c.world, c.r1, ip(10, 0, 12, 2), 255, 2));
    let log = router_log(&c.world, c.r1);
    assert_eq!(
        log.iter()
            .filter_map(ProbeReply::echo_id)
            .collect::<Vec<_>>(),
        [(ID, 1), (ID, 2)]
    );
    assert_eq!(log[0].from, ip(10, 0, 2, 2));
    assert_eq!(log[1].from, ip(10, 0, 12, 2));
}

#[test]
fn time_exceeded_is_never_rate_limited() {
    let mut c = static_chain();
    // Three expiring probes in the same instant: all three are answered.
    for seq in 1..=3 {
        c.world.with_device::<Host, _>(c.pc1, |h, ctx| {
            h.send_echo_probe(ctx, ip(10, 0, 2, 2), 1, ID, seq)
        });
    }
    c.world.run_to_idle();
    assert_eq!(host_log(&c.world, c.pc1).len(), 3);
}

#[test]
fn forwarded_unreachables_are_limited_to_one_per_half_second() {
    let mut c = static_chain();
    let nowhere = ip(10, 99, 0, 1);
    let send = |c: &mut engine::chain::Chain, seq| {
        c.world
            .with_device::<Host, _>(c.pc1, |h, ctx| h.send_echo_probe(ctx, nowhere, 64, ID, seq));
    };
    // Two packets for an unroutable address in quick succession.
    send(&mut c, 1);
    send(&mut c, 2);
    c.world.run_to_idle();
    let first = host_log(&c.world, c.pc1);
    assert_eq!(first.len(), 1, "the second unreachable was held back");
    assert_eq!(first[0].from, ip(10, 0, 1, 1));
    assert_eq!(kind(&first[0]), Some(IcmpErrorKind::DestUnreachable(0)));
    // After the gap another is allowed.
    let later = c.world.kernel.now() + SimTime::from_millis(600);
    c.world.run_until(later);
    send(&mut c, 3);
    c.world.run_to_idle();
    assert_eq!(host_log(&c.world, c.pc1).len(), 2);
}

#[test]
fn probes_are_reproducible() {
    let run = || {
        let mut c = static_chain();
        for ttl in 1..=3u8 {
            host_echo(&mut c.world, c.pc1, ip(10, 0, 2, 2), ttl, u16::from(ttl));
        }
        router_udp(&mut c.world, c.r1, ip(10, 0, 2, 2), 2, 33_434);
        c.world.trace_hash()
    };
    assert_eq!(run(), run());
}
