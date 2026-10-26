//! DHCP on the wire: a client, a server, a relay, and the client's state machine.

use engine::World;
use kernel::{DeviceId, LinkConfig, PortId, SimTime, Trace};
use netdev::{Ctx, Device, DeviceEvent, DhcpState, Host, L2Switch, Router};
use std::any::Any;
use wire::{
    DhcpMessage, DhcpOp, DhcpType, EtherType, EthernetFrame, IpProto, Ipv4Addr, Ipv4Packet,
    MacAddr, UdpDatagram,
};

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

fn client(n: u8) -> Host {
    Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, n, 1]),
        Ipv4Addr::UNSPECIFIED,
        0,
    )
}

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// Configure interface `port` of a router with an address and bring it up.
fn address(w: &mut World, r: DeviceId, port: u16, a: Ipv4Addr, len: u8) {
    w.with_device::<Router, _>(r, |r, _| {
        r.set_address(PortId(port), Some((a, len)));
        r.set_shutdown(PortId(port), false);
    });
}

fn pool(w: &mut World, r: DeviceId, net: Ipv4Addr, router: Ipv4Addr) {
    w.with_device::<Router, _>(r, |r, _| {
        let p = r.dhcp.pool_mut("lan");
        p.network = Some((net, 24));
        p.default_router = Some(router);
        p.dns = vec![ip(8, 8, 8, 8)];
        r.dhcp.exclude(router, router);
    });
}

fn start(w: &mut World, h: DeviceId) {
    w.with_device::<Host, _>(h, |h, c| h.dhcp_start(c));
}

fn state(w: &World, h: DeviceId) -> DhcpState {
    w.device::<Host>(h).dhcp.state
}

fn ping(w: &mut World, from: DeviceId, to: Ipv4Addr) -> bool {
    let before = w.device::<Host>(from).ping_results.len();
    w.with_device::<Host, _>(from, |h, c| h.ping(c, to));
    w.run_to_idle();
    w.device::<Host>(from).ping_results.len() > before
}

/// pc1 and pc2 behind a switch with the router `r1` (10.0.0.1/24, pool 10.0.0.0/24).
fn lan() -> (World, [DeviceId; 2], DeviceId) {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(4));
    let r1 = w.add_device(Router::new("r1", 1, 2));
    let pcs = [w.add_device(client(1)), w.add_device(client(2))];
    for (i, pc) in pcs.iter().enumerate() {
        w.connect((*pc, 0), (sw, i as u16), LinkConfig::FAST_ETHERNET);
    }
    w.connect((r1, 0), (sw, 3), LinkConfig::FAST_ETHERNET);
    address(&mut w, r1, 0, ip(10, 0, 0, 1), 24);
    pool(&mut w, r1, ip(10, 0, 0, 0), ip(10, 0, 0, 1));
    (w, pcs, r1)
}

#[test]
fn a_client_gets_an_address_gateway_and_dns_from_the_router() {
    let (mut w, pcs, r1) = lan();
    start(&mut w, pcs[0]);
    w.run_to_idle();
    let h = w.device::<Host>(pcs[0]);
    assert_eq!(h.dhcp.state, DhcpState::Bound);
    assert_eq!((h.ip, h.prefix_len), (ip(10, 0, 0, 2), 24));
    assert_eq!(h.gateway, Some(ip(10, 0, 0, 1)));
    assert_eq!(h.dns, [ip(8, 8, 8, 8)]);
    assert_eq!(h.dhcp.server.map(|s| s.0), Some(ip(10, 0, 0, 1)));
    let left = h.dhcp_lease_left(w.kernel.now()).unwrap();
    assert!((86_390..=86_400).contains(&left), "{left}");
    let bindings = w.with_device::<Router, _>(r1, |r, c| r.dhcp.bindings(c.now()));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].ip, ip(10, 0, 0, 2));
    assert!(
        ping(&mut w, pcs[0], ip(10, 0, 0, 1)),
        "and it can use the address"
    );
}

#[test]
fn two_clients_get_different_addresses_and_can_reach_each_other() {
    let (mut w, pcs, _) = lan();
    start(&mut w, pcs[0]);
    start(&mut w, pcs[1]);
    w.run_to_idle();
    let a = w.device::<Host>(pcs[0]).ip;
    let b = w.device::<Host>(pcs[1]).ip;
    assert_ne!(a, b);
    assert_eq!(state(&w, pcs[1]), DhcpState::Bound);
    assert!(ping(&mut w, pcs[0], b));
}

#[test]
fn the_broadcast_flag_makes_the_server_answer_by_broadcast() {
    let (mut w, pcs, _) = lan();
    w.with_device::<Host, _>(pcs[0], |h, _| h.dhcp.broadcast_flag = true);
    start(&mut w, pcs[0]);
    w.run_to_idle();
    assert_eq!(state(&w, pcs[0]), DhcpState::Bound);
}

#[test]
fn the_lease_is_renewed_at_t1_and_the_binding_stays() {
    let (mut w, pcs, r1) = lan();
    start(&mut w, pcs[0]);
    w.run_to_idle();
    let first_end = w.device::<Host>(pcs[0]).dhcp.lease_end.unwrap();
    // Just before T1 (half the lease): still bound. Just after: renewed, new end time.
    w.run_until(secs(43_000));
    assert_eq!(state(&w, pcs[0]), DhcpState::Bound);
    w.run_until(secs(43_300));
    let h = w.device::<Host>(pcs[0]);
    assert_eq!(h.dhcp.state, DhcpState::Bound, "renewed and bound again");
    assert!(
        h.dhcp.lease_end.unwrap() > first_end,
        "the lease was extended"
    );
    assert_eq!(h.ip, ip(10, 0, 0, 2), "same address");
    let bound = w.with_device::<Router, _>(r1, |r, c| r.dhcp.bindings(c.now()));
    assert_eq!(bound.len(), 1);
}

#[test]
fn without_a_server_the_lease_runs_out_and_the_client_starts_over() {
    let (mut w, pcs, r1) = lan();
    start(&mut w, pcs[0]);
    w.run_to_idle();
    // The router disappears: renewal at T1, rebinding at T2, expiry at the end of the lease.
    w.with_device::<Router, _>(r1, |r, _| r.set_shutdown(PortId(0), true));
    w.run_until(secs(60_000));
    assert!(
        matches!(
            state(&w, pcs[0]),
            DhcpState::Renewing | DhcpState::Rebinding
        ),
        "{:?}",
        state(&w, pcs[0])
    );
    assert_eq!(
        w.device::<Host>(pcs[0]).ip,
        ip(10, 0, 0, 2),
        "still holds the address"
    );
    w.run_until(secs(86_000 + 1_000));
    let h = w.device::<Host>(pcs[0]);
    assert_eq!(h.ip, Ipv4Addr::UNSPECIFIED, "the address is gone");
    assert_eq!(h.dhcp.state, DhcpState::Selecting);
    assert!(h.gateway.is_none() && h.dns.is_empty());
}

#[test]
fn release_frees_the_address_on_the_server() {
    let (mut w, pcs, r1) = lan();
    start(&mut w, pcs[0]);
    w.run_to_idle();
    w.with_device::<Host, _>(pcs[0], |h, c| h.dhcp_release(c));
    w.run_to_idle();
    let h = w.device::<Host>(pcs[0]);
    assert_eq!(
        (h.ip, h.dhcp.state),
        (Ipv4Addr::UNSPECIFIED, DhcpState::Off)
    );
    assert!(h.dhcp.enabled, "still a DHCP client, just without a lease");
    assert!(w
        .with_device::<Router, _>(r1, |r, c| r.dhcp.bindings(c.now()))
        .is_empty());
    // And it can ask again.
    w.with_device::<Host, _>(pcs[0], |h, c| h.dhcp_renew(c));
    w.run_to_idle();
    assert_eq!(w.device::<Host>(pcs[0]).ip, ip(10, 0, 0, 2));
}

#[test]
fn a_static_address_turns_dhcp_off() {
    let (mut w, pcs, _) = lan();
    start(&mut w, pcs[0]);
    w.run_to_idle();
    w.with_device::<Host, _>(pcs[0], |h, _| {
        h.dhcp_disable();
        h.ip = ip(10, 0, 0, 99);
    });
    // The old renewal timer fires into a client that no longer cares.
    w.run_until(secs(50_000));
    assert_eq!(w.device::<Host>(pcs[0]).ip, ip(10, 0, 0, 99));
    assert_eq!(state(&w, pcs[0]), DhcpState::Off);
}

#[test]
fn a_disabled_service_or_missing_pool_leaves_the_client_waiting() {
    let (mut w, pcs, r1) = lan();
    w.with_device::<Router, _>(r1, |r, _| r.dhcp.enabled = false);
    start(&mut w, pcs[0]);
    w.run_to_idle();
    assert_eq!(state(&w, pcs[0]), DhcpState::Selecting);
    assert_eq!(w.device::<Host>(pcs[0]).ip, Ipv4Addr::UNSPECIFIED);
}

/// pc1 - sw - r1 (relay, 10.0.1.1/24 and 10.0.12.1/30) - r2 (server, 10.0.12.2/30).
fn relayed() -> (World, DeviceId, DeviceId, DeviceId) {
    let mut w = World::new(Trace::new());
    let sw = w.add_device(L2Switch::new(4));
    let r1 = w.add_device(Router::new("r1", 1, 2));
    let r2 = w.add_device(Router::new("r2", 2, 2));
    let pc = w.add_device(client(1));
    w.connect((pc, 0), (sw, 0), LinkConfig::FAST_ETHERNET);
    w.connect((r1, 0), (sw, 3), LinkConfig::FAST_ETHERNET);
    w.connect((r1, 1), (r2, 0), LinkConfig::FAST_ETHERNET);
    address(&mut w, r1, 0, ip(10, 0, 1, 1), 24);
    address(&mut w, r1, 1, ip(10, 0, 12, 1), 30);
    address(&mut w, r2, 0, ip(10, 0, 12, 2), 30);
    w.with_device::<Router, _>(r2, |r, _| {
        // The server needs a way back to the relay's LAN, and a pool for it.
        r.add_static(netdev::StaticRoute {
            net: ip(10, 0, 1, 0),
            len: 24,
            via: ip(10, 0, 12, 1),
        });
    });
    pool(&mut w, r2, ip(10, 0, 1, 0), ip(10, 0, 1, 1));
    w.with_device::<Router, _>(r1, |r, _| r.add_helper(0, ip(10, 0, 12, 2)));
    (w, pc, r1, r2)
}

#[test]
fn a_relay_carries_the_exchange_to_a_server_on_another_network() {
    let (mut w, pc, r1, r2) = relayed();
    start(&mut w, pc);
    w.run_to_idle();
    let (state, addr, gateway, server) = {
        let h = w.device::<Host>(pc);
        (h.dhcp.state, h.ip, h.gateway, h.dhcp.server.map(|s| s.0))
    };
    assert_eq!(state, DhcpState::Bound, "{state:?}");
    assert_eq!(
        addr,
        ip(10, 0, 1, 2),
        "an address from the pool the relay's giaddr selects"
    );
    assert_eq!(gateway, Some(ip(10, 0, 1, 1)));
    assert!(
        w.with_device::<Router, _>(r1, |r, c| r.dhcp.bindings(c.now()))
            .is_empty(),
        "the relay keeps no leases"
    );
    assert_eq!(
        w.with_device::<Router, _>(r2, |r, c| r.dhcp.bindings(c.now()))
            .len(),
        1
    );
    // Server id is the server's own address, so renewals later go straight to it.
    assert_eq!(server, Some(ip(10, 0, 12, 2)));
}

#[test]
fn a_relay_with_no_reachable_server_gives_the_client_nothing_and_it_stops_trying() {
    let (mut w, pc, _, r2) = relayed();
    w.with_device::<Router, _>(r2, |r, _| r.set_shutdown(PortId(0), true));
    start(&mut w, pc);
    w.run_to_idle();
    assert_eq!(state(&w, pc), DhcpState::Selecting);
    assert!(
        w.kernel.now() < secs(60),
        "the retries end well inside a minute"
    );
}

// ---- the client against a scripted server ----

#[derive(Default)]
struct FakeServer {
    seen: Vec<DhcpMessage>,
    frames: Vec<Vec<u8>>,
}

impl Device for FakeServer {
    fn handle(&mut self, _: &mut Ctx, ev: DeviceEvent) {
        if let DeviceEvent::FrameRx { frame, .. } = ev {
            if let Some(m) = dhcp_of(&frame) {
                self.seen.push(m);
            }
            self.frames.push(frame);
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn dhcp_of(frame: &[u8]) -> Option<DhcpMessage> {
    let f = EthernetFrame::parse(frame).ok()?;
    let p = Ipv4Packet::parse(&f.payload).ok()?;
    let u = UdpDatagram::parse(&p.payload).ok()?;
    DhcpMessage::parse(&u.payload).ok()
}

const SERVER_MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 0x99]);
const SERVER_IP: Ipv4Addr = Ipv4Addr([10, 9, 9, 1]);

fn answer(w: &mut World, srv: DeviceId, to: &DhcpMessage, kind: DhcpType, yiaddr: Ipv4Addr) {
    let reply = DhcpMessage {
        op: DhcpOp::Reply,
        xid: to.xid,
        chaddr: to.chaddr,
        yiaddr,
        msg_type: Some(kind),
        server_id: Some(SERVER_IP),
        lease_secs: Some(1_000),
        subnet_mask: Some(ip(255, 255, 0, 0)),
        routers: vec![ip(10, 9, 9, 254)],
        ..DhcpMessage::default()
    };
    let udp = UdpDatagram {
        src_port: 67,
        dst_port: 68,
        payload: reply.emit(),
    };
    let pkt = Ipv4Packet {
        dscp_ecn: 0,
        ident: 1,
        flags_frag: 0,
        ttl: 64,
        proto: IpProto::UDP,
        src: SERVER_IP,
        dst: yiaddr,
        payload: udp.emit(SERVER_IP, yiaddr),
    };
    let frame = EthernetFrame {
        dst: to.chaddr,
        src: SERVER_MAC,
        vlan: None,
        ethertype: EtherType::IPV4,
        payload: pkt.emit(),
    }
    .emit();
    w.with_device::<FakeServer, _>(srv, |_, c| c.transmit(PortId(0), frame));
    // Let the reply land and the client answer, but not so long that its timers fire.
    let t = w.kernel.now() + SimTime::from_millis(100);
    w.run_until(t);
}

fn fake() -> (World, DeviceId, DeviceId) {
    let mut w = World::new(Trace::new());
    let srv = w.add_device(FakeServer::default());
    let pc = w.add_device(client(1));
    w.connect((pc, 0), (srv, 0), LinkConfig::FAST_ETHERNET);
    (w, pc, srv)
}

fn last(w: &World, srv: DeviceId) -> DhcpMessage {
    w.device::<FakeServer>(srv)
        .seen
        .last()
        .cloned()
        .expect("a message")
}

#[test]
fn the_client_follows_discover_offer_request_ack() {
    let (mut w, pc, srv) = fake();
    start(&mut w, pc);
    w.run_until(secs(1));
    let d = last(&w, srv);
    assert_eq!(d.msg_type, Some(DhcpType::Discover));
    assert_eq!(d.chaddr, MacAddr([2, 0, 0, 0, 1, 1]));
    assert_eq!(d.param_request, [1, 3, 6, 51, 58, 59]);
    answer(&mut w, srv, &d, DhcpType::Offer, ip(10, 9, 9, 50));
    let r = last(&w, srv);
    assert_eq!(r.msg_type, Some(DhcpType::Request));
    assert_eq!(r.requested_ip, Some(ip(10, 9, 9, 50)));
    assert_eq!(r.server_id, Some(SERVER_IP));
    assert_eq!(r.xid, d.xid, "same transaction");
    answer(&mut w, srv, &r, DhcpType::Ack, ip(10, 9, 9, 50));
    let h = w.device::<Host>(pc);
    assert_eq!((h.ip, h.prefix_len), (ip(10, 9, 9, 50), 16));
    assert_eq!(h.gateway, Some(ip(10, 9, 9, 254)));
    assert_eq!(h.dhcp.server, Some((SERVER_IP, SERVER_MAC)));
}

#[test]
fn a_nak_sends_the_client_back_to_discovery_with_a_new_transaction() {
    let (mut w, pc, srv) = fake();
    start(&mut w, pc);
    w.run_until(secs(1));
    let d = last(&w, srv);
    answer(&mut w, srv, &d, DhcpType::Offer, ip(10, 9, 9, 50));
    let r = last(&w, srv);
    answer(&mut w, srv, &r, DhcpType::Nak, Ipv4Addr::UNSPECIFIED);
    let again = last(&w, srv);
    assert_eq!(again.msg_type, Some(DhcpType::Discover));
    assert_ne!(again.xid, d.xid, "a new exchange has a new id");
    assert_eq!(w.device::<Host>(pc).ip, Ipv4Addr::UNSPECIFIED);
    assert_eq!(state(&w, pc), DhcpState::Selecting);
}

#[test]
fn replies_for_another_transaction_or_client_are_ignored() {
    let (mut w, pc, srv) = fake();
    start(&mut w, pc);
    w.run_until(secs(1));
    let d = last(&w, srv);
    let mut wrong_xid = d.clone();
    wrong_xid.xid ^= 1;
    answer(&mut w, srv, &wrong_xid, DhcpType::Offer, ip(10, 9, 9, 50));
    let mut other = d.clone();
    other.chaddr = MacAddr([2, 0, 0, 0, 7, 7]);
    answer(&mut w, srv, &other, DhcpType::Offer, ip(10, 9, 9, 51));
    assert_eq!(
        state(&w, pc),
        DhcpState::Selecting,
        "neither offer was for this client"
    );
    answer(&mut w, srv, &d, DhcpType::Offer, ip(10, 9, 9, 52));
    assert_eq!(state(&w, pc), DhcpState::Requesting);
}

#[test]
fn discover_is_retried_with_growing_gaps_and_then_given_up() {
    let (mut w, pc, srv) = fake();
    start(&mut w, pc);
    let before = w.kernel.now();
    w.run_to_idle();
    let discovers: Vec<_> = w
        .device::<FakeServer>(srv)
        .seen
        .iter()
        .filter(|m| m.msg_type == Some(DhcpType::Discover))
        .collect();
    assert_eq!(discovers.len(), 5, "the first plus four retries");
    assert!(
        discovers.iter().all(|m| m.xid == discovers[0].xid),
        "retries keep the id"
    );
    let end = w.kernel.now();
    assert!(
        end.0 - before.0 > 25_000_000_000 && end < secs(40),
        "{end:?}"
    );
    assert_eq!(state(&w, pc), DhcpState::Selecting);
    // The shell can start it again.
    start(&mut w, pc);
    w.run_to_idle();
    assert_eq!(
        w.device::<FakeServer>(srv)
            .seen
            .iter()
            .filter(|m| m.msg_type == Some(DhcpType::Discover))
            .count(),
        10
    );
}

#[test]
fn an_unanswered_request_falls_back_to_discovery() {
    let (mut w, pc, srv) = fake();
    start(&mut w, pc);
    w.run_until(secs(1));
    let d = last(&w, srv);
    answer(&mut w, srv, &d, DhcpType::Offer, ip(10, 9, 9, 50));
    // The server never acks: REQUESTs repeat, then the client starts again.
    w.run_until(secs(20));
    let kinds: Vec<_> = w
        .device::<FakeServer>(srv)
        .seen
        .iter()
        .map(|m| m.msg_type.unwrap())
        .collect();
    let requests = kinds.iter().filter(|k| **k == DhcpType::Request).count();
    assert_eq!(requests, 3);
    assert!(
        kinds.iter().filter(|k| **k == DhcpType::Discover).count() >= 2,
        "{kinds:?}"
    );
}

#[test]
fn the_exchange_is_deterministic_and_visible_in_the_inspector() {
    let run = || {
        let (mut w, pcs, _) = lan();
        start(&mut w, pcs[0]);
        w.run_to_idle();
        w.trace_hash()
    };
    assert_eq!(run(), run());
    let (mut w, pcs, _) = lan();
    start(&mut w, pcs[0]);
    let mut seen = Vec::new();
    while let Some(engine::Delivered::Frame { frame, .. }) = w.step() {
        seen.push(wire::summarize(&frame));
    }
    let want = [
        "DHCP discover",
        "DHCP offer 10.0.0.2",
        "DHCP request for 10.0.0.2",
        "DHCP ack 10.0.0.2",
    ];
    for w in want {
        assert!(
            seen.iter().any(|s| s.starts_with(w)),
            "no {w:?} in {seen:#?}"
        );
    }
}

// ---- how a server addresses its replies (RFC 2131 section 4.1) ----

/// One DHCP frame seen on the wire.
struct Seen {
    l2_dst: MacAddr,
    src: Ipv4Addr,
    dst: Ipv4Addr,
    ports: (u16, u16),
    msg: DhcpMessage,
}

const CLIENT_MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 0x77]);

/// A router at 10.0.0.1/24 with a pool, and a probe on its port that plays a client by hand.
fn probe_rig() -> (World, DeviceId) {
    let mut w = World::new(Trace::new());
    let r1 = w.add_device(Router::new("r1", 1, 2));
    let probe = w.add_device(FakeServer::default());
    w.connect((probe, 0), (r1, 0), LinkConfig::FAST_ETHERNET);
    address(&mut w, r1, 0, ip(10, 0, 0, 1), 24);
    pool(&mut w, r1, ip(10, 0, 0, 0), ip(10, 0, 0, 1));
    (w, probe)
}

fn client_msg(kind: DhcpType, broadcast: bool) -> DhcpMessage {
    let mut m = DhcpMessage {
        xid: 7,
        chaddr: CLIENT_MAC,
        msg_type: Some(kind),
        ..DhcpMessage::default()
    };
    m.set_broadcast(broadcast);
    m
}

/// Send `m` to the router as a client would and return what came back as
/// (layer 2 destination, IP source, IP destination, UDP ports, message).
fn exchange(
    w: &mut World,
    probe: DeviceId,
    m: &DhcpMessage,
    l2: MacAddr,
    src: Ipv4Addr,
    dst: Ipv4Addr,
) -> Vec<Seen> {
    let udp = UdpDatagram {
        src_port: 68,
        dst_port: 67,
        payload: m.emit(),
    };
    let pkt = Ipv4Packet {
        dscp_ecn: 0,
        ident: 1,
        flags_frag: 0,
        ttl: 64,
        proto: IpProto::UDP,
        src,
        dst,
        payload: udp.emit(src, dst),
    };
    let frame = EthernetFrame {
        dst: l2,
        src: CLIENT_MAC,
        vlan: None,
        ethertype: EtherType::IPV4,
        payload: pkt.emit(),
    }
    .emit();
    w.with_device::<FakeServer, _>(probe, |p, c| {
        p.frames.clear();
        c.transmit(PortId(0), frame)
    });
    w.run_to_idle();
    w.device::<FakeServer>(probe)
        .frames
        .iter()
        .filter_map(|f| {
            let e = EthernetFrame::parse(f).ok()?;
            let p = Ipv4Packet::parse(&e.payload).ok()?;
            let u = UdpDatagram::parse(&p.payload).ok()?;
            let d = DhcpMessage::parse(&u.payload).ok()?;
            Some(Seen {
                l2_dst: e.dst,
                src: p.src,
                dst: p.dst,
                ports: (u.src_port, u.dst_port),
                msg: d,
            })
        })
        .collect()
}

fn discover_from_nobody(w: &mut World, probe: DeviceId, broadcast: bool) -> Vec<Seen> {
    exchange(
        w,
        probe,
        &client_msg(DhcpType::Discover, broadcast),
        MacAddr::BROADCAST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
    )
}

#[test]
fn an_offer_goes_by_unicast_to_the_clients_hardware_address_unless_it_asked_for_broadcast() {
    let (mut w, probe) = probe_rig();
    let r = discover_from_nobody(&mut w, probe, false);
    assert_eq!(r.len(), 1);
    let Seen {
        l2_dst: l2,
        src,
        dst,
        ports,
        msg: m,
    } = &r[0];
    assert_eq!(*l2, CLIENT_MAC, "it cannot ARP, so the server uses chaddr");
    assert_eq!((*src, *dst), (ip(10, 0, 0, 1), ip(10, 0, 0, 2)));
    assert_eq!(*ports, (67, 68));
    assert_eq!(m.msg_type, Some(DhcpType::Offer));
    // The same request with the broadcast flag: everything is broadcast.
    let (mut w, probe) = probe_rig();
    let r = discover_from_nobody(&mut w, probe, true);
    let Seen {
        l2_dst: l2,
        dst,
        msg: m,
        ..
    } = &r[0];
    assert_eq!((*l2, *dst), (MacAddr::BROADCAST, Ipv4Addr::BROADCAST));
    assert!(m.broadcast(), "the flag is echoed");
}

#[test]
fn a_nak_is_always_broadcast_and_a_renewal_is_answered_at_the_clients_own_address() {
    let (mut w, probe) = probe_rig();
    let offer = discover_from_nobody(&mut w, probe, false).remove(0).msg;
    // Selecting us but asking for an address outside the pool.
    let mut bad = client_msg(DhcpType::Request, false);
    bad.server_id = Some(ip(10, 0, 0, 1));
    bad.requested_ip = Some(ip(192, 168, 1, 1));
    let r = exchange(
        &mut w,
        probe,
        &bad,
        MacAddr::BROADCAST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
    );
    let Seen {
        l2_dst: l2,
        dst,
        msg: m,
        ..
    } = &r[0];
    assert_eq!(m.msg_type, Some(DhcpType::Nak));
    assert_eq!((*l2, *dst), (MacAddr::BROADCAST, Ipv4Addr::BROADCAST));
    // Take the offered address properly, then renew by unicast to the server.
    let mut take = client_msg(DhcpType::Request, false);
    take.server_id = Some(ip(10, 0, 0, 1));
    take.requested_ip = Some(offer.yiaddr);
    let r = exchange(
        &mut w,
        probe,
        &take,
        MacAddr::BROADCAST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
    );
    assert_eq!(r[0].msg.msg_type, Some(DhcpType::Ack));
    let mut renew = client_msg(DhcpType::Request, false);
    renew.ciaddr = offer.yiaddr;
    let r = exchange(
        &mut w,
        probe,
        &renew,
        MacAddr([2, 0xaa, 0, 0, 1, 0]),
        offer.yiaddr,
        ip(10, 0, 0, 1),
    );
    let Seen {
        l2_dst: l2,
        src,
        dst,
        msg: m,
        ..
    } = &r[0];
    assert_eq!(m.msg_type, Some(DhcpType::Ack));
    assert_eq!(
        (*l2, *src, *dst),
        (CLIENT_MAC, ip(10, 0, 0, 1), offer.yiaddr)
    );
    assert_eq!(m.ciaddr, offer.yiaddr, "ciaddr echoes the request");
}

#[test]
fn a_request_to_another_server_gets_no_answer_and_garbage_gets_none_either() {
    let (mut w, probe) = probe_rig();
    let mut other = client_msg(DhcpType::Request, false);
    other.server_id = Some(ip(10, 0, 0, 99));
    other.requested_ip = Some(ip(10, 0, 0, 10));
    assert!(exchange(
        &mut w,
        probe,
        &other,
        MacAddr::BROADCAST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST
    )
    .is_empty());
    // A reply-type message from a "client" is not for the server to answer.
    let mut odd = client_msg(DhcpType::Discover, false);
    odd.op = DhcpOp::Reply;
    odd.giaddr = ip(1, 2, 3, 4);
    assert!(exchange(
        &mut w,
        probe,
        &odd,
        MacAddr::BROADCAST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST
    )
    .is_empty());
}
