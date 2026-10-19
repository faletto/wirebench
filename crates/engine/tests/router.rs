use engine::chain::{static_chain, static_chain_ping};
use kernel::PortId;
use netdev::{Host, RouteKind, Router};
use wire::{IcmpErrorKind, Ipv4Addr};

#[test]
fn pc_router_router_pc_ping_golden() {
    let c = static_chain_ping();
    let h = c.world.device::<Host>(c.pc1);
    assert_eq!(h.ping_results.len(), 1, "one reply expected");
    assert!(h.icmp_errors.is_empty());
    assert_eq!(c.world.trace_hash(), BLESSED);
}

// Pinned in testkit/determinism.txt too. Re-bless deliberately.
const BLESSED: u64 = 11618600247012007814;

fn ping_with_ttl(ttl: u8, dst: Ipv4Addr) -> Vec<IcmpErrorKind> {
    let mut c = static_chain();
    c.world.with_device::<Host, _>(c.pc1, |h, ctx| {
        h.ttl = ttl;
        h.ping(ctx, dst)
    });
    c.world.run_to_idle();
    let h = c.world.device::<Host>(c.pc1);
    assert!(h.ping_results.is_empty());
    h.icmp_errors.clone()
}

#[test]
fn ttl_one_expires_at_first_router() {
    assert_eq!(
        ping_with_ttl(1, Ipv4Addr::new(10, 0, 2, 2)),
        vec![IcmpErrorKind::TimeExceeded]
    );
}

#[test]
fn ttl_two_expires_at_second_router_and_error_returns() {
    assert_eq!(
        ping_with_ttl(2, Ipv4Addr::new(10, 0, 2, 2)),
        vec![IcmpErrorKind::TimeExceeded]
    );
}

#[test]
fn unknown_destination_gets_net_unreachable() {
    assert_eq!(
        ping_with_ttl(64, Ipv4Addr::new(10, 9, 9, 9)),
        vec![IcmpErrorKind::DestUnreachable(0)]
    );
}

#[test]
fn router_answers_ping_to_its_own_address() {
    let mut c = static_chain();
    c.world
        .with_device::<Host, _>(c.pc1, |h, ctx| h.ping(ctx, Ipv4Addr::new(10, 0, 12, 2)));
    c.world.run_to_idle();
    assert_eq!(c.world.device::<Host>(c.pc1).ping_results.len(), 1);
}

#[test]
fn shutdown_interface_removes_routes_and_breaks_ping() {
    let mut c = static_chain();
    c.world
        .with_device::<Router, _>(c.r1, |r, _| r.set_shutdown(PortId(1), true));
    // r1 lost its connected 10.0.12.0/30, so the static via 10.0.12.2 no longer installs.
    let r1 = c.world.device::<Router>(c.r1);
    assert!(r1.fib().routes().all(|r| r.kind == RouteKind::Connected));
    assert_eq!(r1.fib().routes().count(), 1);
    c.world
        .with_device::<Host, _>(c.pc1, |h, ctx| h.ping(ctx, Ipv4Addr::new(10, 0, 2, 2)));
    c.world.run_to_idle();
    let h = c.world.device::<Host>(c.pc1);
    assert!(h.ping_results.is_empty());
    assert_eq!(h.icmp_errors, vec![IcmpErrorKind::DestUnreachable(0)]);
}

#[test]
fn chain_is_reproducible() {
    assert_eq!(
        static_chain_ping().world.trace_hash(),
        static_chain_ping().world.trace_hash()
    );
}
