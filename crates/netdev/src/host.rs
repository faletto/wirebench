use crate::{record_probe, Ctx, Device, DeviceEvent, ProbeBody, ProbeReply, Resolver};
use kernel::{PortId, SimTime};
use std::any::Any;
use std::collections::BTreeMap;
use wire::{
    ArpOp, ArpPacket, DhcpMessage, DhcpOp, DhcpType, EtherType, EthernetFrame, IcmpEcho, IcmpError,
    IcmpErrorKind, IpProto, Ipv4Addr, Ipv4Packet, MacAddr, UdpDatagram,
};

const PORT: PortId = PortId(0);
/// ICMP identifier of the echo requests `ping` sends; replies with another one are not pings.
const PING_IDENT: u16 = 1;
const ARP_TIMEOUT: SimTime = SimTime::from_secs(1);

/// Timer tokens with this bit set belong to the DHCP client; the low bits are the generation of
/// the state they were set in. ARP timer tokens are 32-bit addresses, so the two never clash.
const DHCP_TIMER: u64 = 1 << 62;
/// First retransmission delay for DISCOVER and REQUEST; it doubles each time up to a cap.
const DHCP_RETRY: SimTime = SimTime::from_secs(2);
const DHCP_RETRY_CAP: SimTime = SimTime::from_secs(16);
/// DISCOVERs sent (the first plus retries at 2, 6, 14 and 30 seconds) before the client gives up
/// until told to try again. Keeping it finite keeps a lab with no server able to go idle.
const DHCP_DISCOVER_TRIES: u8 = 5;
/// REQUESTs sent without an answer before the client starts over with DISCOVER.
const DHCP_REQUEST_TRIES: u8 = 3;
/// A renewing client waits at least this long between attempts (RFC 2131 section 4.4.5).
const DHCP_RENEW_MIN: SimTime = SimTime::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingResult {
    pub seq: u16,
    pub rtt: SimTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpState {
    /// Not using DHCP (static address), or released.
    Off,
    /// DISCOVER sent, waiting for offers.
    Selecting,
    /// REQUEST sent, waiting for the server's answer.
    Requesting,
    Bound,
    /// Past T1: asking the leasing server to extend the lease.
    Renewing,
    /// Past T2: asking any server.
    Rebinding,
}

/// The DHCP client's state (RFC 2131 section 4.4).
#[derive(Clone, Debug)]
pub struct DhcpClient {
    /// `ipconfig dhcp` is in effect: the host takes its address from a server.
    pub enabled: bool,
    pub state: DhcpState,
    /// Ask servers to reply by broadcast (the BROADCAST flag). Off by default: this host can
    /// receive a unicast reply before it has an address.
    pub broadcast_flag: bool,
    /// The server that holds the lease, and the hardware address to reach it at.
    pub server: Option<(Ipv4Addr, MacAddr)>,
    pub lease_end: Option<SimTime>,
    xid: u32,
    /// Bumped on every state change so timers set for an earlier state are ignored.
    generation: u32,
    exchanges: u32,
    tries: u8,
    started: SimTime,
    t1: SimTime,
    t2: SimTime,
    offered: Option<(Ipv4Addr, Ipv4Addr)>,
}

impl Default for DhcpClient {
    fn default() -> Self {
        DhcpClient {
            enabled: false,
            state: DhcpState::Off,
            broadcast_flag: false,
            server: None,
            lease_end: None,
            xid: 0,
            generation: 0,
            exchanges: 0,
            tries: 0,
            started: SimTime::ZERO,
            t1: SimTime::ZERO,
            t2: SimTime::ZERO,
            offered: None,
        }
    }
}

/// An end host: one NIC, ARP cache, ICMP echo responder and a programmatic ping.
#[derive(Debug)]
pub struct Host {
    pub name: String,
    pub mac: MacAddr,
    pub ip: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub arp_ttl: SimTime,
    arp: Resolver,
    sent: BTreeMap<u16, SimTime>,
    next_seq: u16,
    ip_ident: u16,
    pub ping_results: Vec<PingResult>,
    /// ICMP errors received (time exceeded, unreachable), oldest first.
    pub icmp_errors: Vec<IcmpErrorKind>,
    /// TTL for locally originated packets.
    pub ttl: u8,
    /// DNS servers, from the lab file or DHCP. Informational: there is no resolver yet.
    pub dns: Vec<Ipv4Addr>,
    pub dhcp: DhcpClient,
    /// Replies to probes this host sent (traceroute), oldest first.
    pub probes: Vec<ProbeReply>,
}

impl Host {
    pub fn new(name: &str, mac: MacAddr, ip: Ipv4Addr, prefix_len: u8) -> Self {
        Host {
            name: name.to_string(),
            mac,
            ip,
            prefix_len,
            gateway: None,
            arp_ttl: SimTime::from_secs(4 * 3600),
            arp: Resolver::default(),
            sent: BTreeMap::new(),
            next_seq: 1,
            ip_ident: 1,
            ping_results: Vec::new(),
            icmp_errors: Vec::new(),
            ttl: 64,
            dns: Vec::new(),
            dhcp: DhcpClient::default(),
            probes: Vec::new(),
        }
    }

    fn mask(&self) -> u32 {
        if self.prefix_len == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix_len as u32)
        }
    }

    fn next_hop(&self, dst: Ipv4Addr) -> Option<Ipv4Addr> {
        if dst.to_u32() & self.mask() == self.ip.to_u32() & self.mask() {
            Some(dst)
        } else {
            self.gateway
        }
    }

    pub fn arp_lookup(&self, ip: Ipv4Addr, now: SimTime) -> Option<MacAddr> {
        self.arp.lookup(ip, now)
    }

    /// Send one echo request. The result shows up in `ping_results` if a reply arrives.
    pub fn ping(&mut self, ctx: &mut Ctx, dst: Ipv4Addr) -> u16 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.sent.insert(seq, ctx.now());
        let echo = IcmpEcho {
            reply: false,
            ident: PING_IDENT,
            seq,
            data: vec![0x61; 32],
        };
        self.send_ip(ctx, dst, IpProto::ICMP, echo.emit());
        seq
    }

    /// Send an echo request with the given TTL (a traceroute probe). It is not counted as a ping:
    /// the answer shows up in [`Host::probes`].
    pub fn send_echo_probe(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, ttl: u8, ident: u16, seq: u16) {
        let echo = IcmpEcho {
            reply: false,
            ident,
            seq,
            data: vec![0x61; 32],
        };
        self.send_ip_ttl(ctx, dst, IpProto::ICMP, echo.emit(), ttl);
    }

    fn send_ip(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, proto: IpProto, payload: Vec<u8>) {
        let ttl = self.ttl;
        self.send_ip_ttl(ctx, dst, proto, payload, ttl);
    }

    fn send_ip_ttl(
        &mut self,
        ctx: &mut Ctx,
        dst: Ipv4Addr,
        proto: IpProto,
        payload: Vec<u8>,
        ttl: u8,
    ) {
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.ip_ident,
            flags_frag: 0,
            ttl,
            proto,
            src: self.ip,
            dst,
            payload,
        };
        self.ip_ident = self.ip_ident.wrapping_add(1);
        let Some(hop) = self.next_hop(dst) else {
            ctx.note("no route");
            return;
        };
        if let Some(mac) = self.arp_lookup(hop, ctx.now()) {
            self.emit_ip(ctx, mac, &pkt);
            return;
        }
        if self.arp.queue(hop, pkt) {
            let req = ArpPacket {
                op: ArpOp::Request,
                sender_mac: self.mac,
                sender_ip: self.ip,
                target_mac: MacAddr::default(),
                target_ip: hop,
            };
            self.emit(ctx, MacAddr::BROADCAST, EtherType::ARP, req.emit());
            ctx.schedule(ARP_TIMEOUT, hop.to_u32() as u64);
        }
    }

    fn emit_ip(&self, ctx: &mut Ctx, dst_mac: MacAddr, pkt: &Ipv4Packet) {
        self.emit(ctx, dst_mac, EtherType::IPV4, pkt.emit());
    }

    fn emit(&self, ctx: &mut Ctx, dst: MacAddr, et: EtherType, mut payload: Vec<u8>) {
        if payload.len() < 46 {
            payload.resize(46, 0);
        }
        let f = EthernetFrame {
            dst,
            src: self.mac,
            vlan: None,
            ethertype: et,
            payload,
        };
        ctx.transmit(PORT, f.emit());
    }

    fn on_arp(&mut self, ctx: &mut Ctx, p: ArpPacket) {
        if p.target_ip != self.ip {
            return;
        }
        let expires = ctx.now() + self.arp_ttl;
        self.arp.learn(p.sender_ip, p.sender_mac, expires);
        if p.op == ArpOp::Request {
            let reply = ArpPacket {
                op: ArpOp::Reply,
                sender_mac: self.mac,
                sender_ip: self.ip,
                target_mac: p.sender_mac,
                target_ip: p.sender_ip,
            };
            self.emit(ctx, p.sender_mac, EtherType::ARP, reply.emit());
        }
        for pkt in self.arp.take_pending(p.sender_ip) {
            self.emit_ip(ctx, p.sender_mac, &pkt);
        }
    }

    fn on_ip(&mut self, ctx: &mut Ctx, p: Ipv4Packet) {
        if p.dst != self.ip && p.dst != Ipv4Addr::BROADCAST {
            return;
        }
        // A UDP packet for a port nobody listens on: say so, as every host does. This is what
        // ends a UDP traceroute at its destination.
        if p.proto == IpProto::UDP && p.dst == self.ip && self.ip != Ipv4Addr::UNSPECIFIED {
            let err = IcmpError {
                kind: IcmpErrorKind::DestUnreachable(3),
                original: IcmpError::quote(&p.emit()),
            };
            self.send_ip(ctx, p.src, IpProto::ICMP, err.emit());
            return;
        }
        if p.proto != IpProto::ICMP {
            return;
        }
        if let Ok(e) = IcmpError::parse(&p.payload) {
            self.icmp_errors.push(e.kind);
            record_probe(
                &mut self.probes,
                ProbeReply {
                    at: ctx.now(),
                    from: p.src,
                    body: ProbeBody::Error {
                        kind: e.kind,
                        original: e.original,
                    },
                },
            );
            return;
        }
        let Ok(echo) = IcmpEcho::parse(&p.payload) else {
            return;
        };
        if !echo.reply {
            let reply = IcmpEcho {
                reply: true,
                ..echo
            };
            self.send_ip(ctx, p.src, IpProto::ICMP, reply.emit());
            return;
        }
        record_probe(
            &mut self.probes,
            ProbeReply {
                at: ctx.now(),
                from: p.src,
                body: ProbeBody::EchoReply {
                    ident: echo.ident,
                    seq: echo.seq,
                },
            },
        );
        if echo.ident == PING_IDENT {
            if let Some(t0) = self.sent.remove(&echo.seq) {
                self.ping_results.push(PingResult {
                    seq: echo.seq,
                    rtt: SimTime(ctx.now().0 - t0.0),
                });
            }
        }
    }
}

// ---- DHCP client ----

impl Host {
    /// `ipconfig dhcp`: forget the current address and ask a server for one.
    pub fn dhcp_start(&mut self, ctx: &mut Ctx) {
        self.dhcp.enabled = true;
        self.deconfigure();
        self.begin_discovery(ctx);
    }

    /// Stop using DHCP (a static address was set). The address and gateway are left alone.
    pub fn dhcp_disable(&mut self) {
        self.dhcp.enabled = false;
        self.dhcp.state = DhcpState::Off;
        self.dhcp.generation += 1;
        self.dhcp.server = None;
        self.dhcp.lease_end = None;
    }

    /// `ipconfig release`: tell the server, drop the address, and stop until told to renew.
    pub fn dhcp_release(&mut self, ctx: &mut Ctx) {
        if let (true, Some((server, mac))) = (self.ip != Ipv4Addr::UNSPECIFIED, self.dhcp.server) {
            if self.dhcp.enabled {
                let mut m = self.dhcp_message(ctx.now(), DhcpType::Release);
                m.ciaddr = self.ip;
                m.server_id = Some(server);
                m.param_request.clear();
                m.flags = 0;
                self.dhcp_send(ctx, &m, self.ip, server, mac);
            }
        }
        self.deconfigure();
        self.dhcp.state = DhcpState::Off;
        self.dhcp.generation += 1;
        self.dhcp.server = None;
        self.dhcp.lease_end = None;
    }

    /// `ipconfig renew`: extend the lease now, or start over if there is none.
    pub fn dhcp_renew(&mut self, ctx: &mut Ctx) {
        match self.dhcp.state {
            DhcpState::Bound | DhcpState::Renewing | DhcpState::Rebinding => {
                self.enter_renewing(ctx)
            }
            _ => self.dhcp_start(ctx),
        }
    }

    fn deconfigure(&mut self) {
        self.ip = Ipv4Addr::UNSPECIFIED;
        self.prefix_len = 0;
        self.gateway = None;
        self.dns.clear();
    }

    /// Start a new exchange in `state`: fresh transaction id, timers from the last state ignored.
    fn new_exchange(&mut self, now: SimTime, state: DhcpState) {
        let d = &mut self.dhcp;
        d.generation += 1;
        d.exchanges += 1;
        d.state = state;
        d.tries = 0;
        d.started = now;
        let low = u32::from_be_bytes([self.mac.0[2], self.mac.0[3], self.mac.0[4], self.mac.0[5]]);
        d.xid = low ^ d.exchanges.wrapping_mul(0x9e37_79b9);
    }

    fn begin_discovery(&mut self, ctx: &mut Ctx) {
        self.new_exchange(ctx.now(), DhcpState::Selecting);
        self.dhcp.offered = None;
        self.dhcp.server = None;
        self.dhcp.lease_end = None;
        self.send_discover(ctx);
        self.dhcp_schedule(ctx, DHCP_RETRY);
    }

    fn dhcp_message(&self, now: SimTime, kind: DhcpType) -> DhcpMessage {
        let secs = (now.0.saturating_sub(self.dhcp.started.0) / 1_000_000_000).min(65_535);
        DhcpMessage {
            op: DhcpOp::Request,
            xid: self.dhcp.xid,
            secs: secs as u16,
            flags: if self.dhcp.broadcast_flag { 0x8000 } else { 0 },
            chaddr: self.mac,
            msg_type: Some(kind),
            param_request: vec![1, 3, 6, 51, 58, 59],
            ..DhcpMessage::default()
        }
    }

    fn dhcp_send(
        &mut self,
        ctx: &mut Ctx,
        msg: &DhcpMessage,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        dst_mac: MacAddr,
    ) {
        let udp = UdpDatagram {
            src_port: 68,
            dst_port: 67,
            payload: msg.emit(),
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.ip_ident,
            flags_frag: 0,
            ttl: self.ttl,
            proto: IpProto::UDP,
            src,
            dst,
            payload: udp.emit(src, dst),
        };
        self.ip_ident = self.ip_ident.wrapping_add(1);
        self.emit_ip(ctx, dst_mac, &pkt);
    }

    fn dhcp_broadcast(&mut self, ctx: &mut Ctx, msg: &DhcpMessage, src: Ipv4Addr) {
        self.dhcp_send(ctx, msg, src, Ipv4Addr::BROADCAST, MacAddr::BROADCAST);
    }

    fn send_discover(&mut self, ctx: &mut Ctx) {
        let m = self.dhcp_message(ctx.now(), DhcpType::Discover);
        self.dhcp_broadcast(ctx, &m, Ipv4Addr::UNSPECIFIED);
    }

    fn send_selecting_request(&mut self, ctx: &mut Ctx) {
        let Some((addr, server)) = self.dhcp.offered else {
            return;
        };
        let mut m = self.dhcp_message(ctx.now(), DhcpType::Request);
        m.requested_ip = Some(addr);
        m.server_id = Some(server);
        self.dhcp_broadcast(ctx, &m, Ipv4Addr::UNSPECIFIED);
    }

    /// A REQUEST to extend the lease: to the leasing server while renewing, to anyone after T2.
    fn send_renewal(&mut self, ctx: &mut Ctx) {
        let mut m = self.dhcp_message(ctx.now(), DhcpType::Request);
        m.ciaddr = self.ip;
        match (self.dhcp.state, self.dhcp.server) {
            (DhcpState::Renewing, Some((server, mac))) => {
                self.dhcp_send(ctx, &m, self.ip, server, mac)
            }
            _ => self.dhcp_broadcast(ctx, &m, self.ip),
        }
    }

    fn dhcp_schedule(&self, ctx: &mut Ctx, after: SimTime) {
        ctx.schedule(after, DHCP_TIMER | u64::from(self.dhcp.generation));
    }

    /// Next retry no later than `until`: half the time left, but not under a minute.
    fn schedule_until(&self, ctx: &mut Ctx, until: SimTime) {
        let left = until.0.saturating_sub(ctx.now().0);
        let wait = (left / 2).max(DHCP_RENEW_MIN.0).min(left).max(1);
        self.dhcp_schedule(ctx, SimTime(wait));
    }

    fn enter_renewing(&mut self, ctx: &mut Ctx) {
        self.new_exchange(ctx.now(), DhcpState::Renewing);
        self.send_renewal(ctx);
        self.schedule_until(ctx, self.dhcp.t2);
    }

    fn retry_delay(&self) -> SimTime {
        let d = DHCP_RETRY.0 << self.dhcp.tries.min(5);
        SimTime(d.min(DHCP_RETRY_CAP.0))
    }

    fn dhcp_timer(&mut self, ctx: &mut Ctx, generation: u32) {
        if !self.dhcp.enabled || generation != self.dhcp.generation {
            return;
        }
        let now = ctx.now();
        match self.dhcp.state {
            DhcpState::Off => {}
            DhcpState::Selecting => {
                self.dhcp.tries += 1;
                self.send_discover(ctx);
                if self.dhcp.tries + 1 < DHCP_DISCOVER_TRIES {
                    self.dhcp_schedule(ctx, self.retry_delay());
                }
            }
            DhcpState::Requesting => {
                if self.dhcp.tries + 1 >= DHCP_REQUEST_TRIES {
                    self.begin_discovery(ctx);
                } else {
                    self.dhcp.tries += 1;
                    self.send_selecting_request(ctx);
                    self.dhcp_schedule(ctx, self.retry_delay());
                }
            }
            DhcpState::Bound => self.enter_renewing(ctx),
            DhcpState::Renewing => {
                if now >= self.dhcp.t2 {
                    self.dhcp.state = DhcpState::Rebinding;
                    self.dhcp.generation += 1;
                }
                self.send_renewal(ctx);
                let until = match self.dhcp.state {
                    DhcpState::Rebinding => self.dhcp.lease_end.unwrap_or(now),
                    _ => self.dhcp.t2,
                };
                self.schedule_until(ctx, until);
            }
            DhcpState::Rebinding => {
                let end = self.dhcp.lease_end.unwrap_or(now);
                if now >= end {
                    // The lease ran out: the address is gone, start over.
                    self.deconfigure();
                    self.begin_discovery(ctx);
                } else {
                    self.send_renewal(ctx);
                    self.schedule_until(ctx, end);
                }
            }
        }
    }

    /// A UDP packet that might be a server's answer. Returns true if it was one for this client.
    fn try_dhcp(&mut self, ctx: &mut Ctx, p: &Ipv4Packet, from: MacAddr) -> bool {
        if p.proto != IpProto::UDP || self.dhcp.state == DhcpState::Off {
            return false;
        }
        let Ok(u) = UdpDatagram::parse(&p.payload) else {
            return false;
        };
        if u.dst_port != 68 {
            return false;
        }
        let Ok(m) = DhcpMessage::parse(&u.payload) else {
            return false;
        };
        if m.op != DhcpOp::Reply || m.chaddr != self.mac || m.xid != self.dhcp.xid {
            return false;
        }
        let server = m.server_id.unwrap_or(p.src);
        match (self.dhcp.state, m.msg_type) {
            (DhcpState::Selecting, Some(DhcpType::Offer)) => {
                // Take the first offer.
                self.dhcp.offered = Some((m.yiaddr, server));
                self.dhcp.server = Some((server, from));
                self.dhcp.generation += 1;
                self.dhcp.state = DhcpState::Requesting;
                self.dhcp.tries = 0;
                self.send_selecting_request(ctx);
                self.dhcp_schedule(ctx, self.retry_delay());
            }
            (
                DhcpState::Requesting | DhcpState::Renewing | DhcpState::Rebinding,
                Some(DhcpType::Ack),
            ) => self.bind(ctx, &m, server, from),
            (
                DhcpState::Requesting | DhcpState::Renewing | DhcpState::Rebinding,
                Some(DhcpType::Nak),
            ) => {
                self.deconfigure();
                self.begin_discovery(ctx);
            }
            _ => {}
        }
        true
    }

    fn bind(&mut self, ctx: &mut Ctx, m: &DhcpMessage, server: Ipv4Addr, from: MacAddr) {
        let now = ctx.now();
        let secs = |s: u32| SimTime(u64::from(s) * 1_000_000_000);
        let lease = m.lease_secs.unwrap_or(86_400).max(2);
        let t1 = m.t1_secs.filter(|t| *t < lease).unwrap_or(lease / 2);
        let t2 = m.t2_secs.filter(|t| *t > t1 && *t < lease).unwrap_or(
            ((u64::from(lease) * 7 / 8) as u32)
                .max(t1 + 1)
                .min(lease - 1),
        );
        self.ip = m.yiaddr;
        self.prefix_len = m
            .subnet_mask
            .map_or(24, |mask| mask.to_u32().leading_ones() as u8);
        self.gateway = m.routers.first().copied();
        self.dns = m.dns.clone();
        self.dhcp.server = Some((server, from));
        self.dhcp.generation += 1;
        self.dhcp.state = DhcpState::Bound;
        self.dhcp.lease_end = Some(now + secs(lease));
        self.dhcp.t1 = now + secs(t1);
        self.dhcp.t2 = now + secs(t2);
        self.dhcp_schedule(ctx, secs(t1));
        ctx.note("dhcp bound");
    }

    /// Seconds of lease left at `now`, if there is a lease.
    pub fn dhcp_lease_left(&self, now: SimTime) -> Option<u64> {
        self.dhcp
            .lease_end
            .map(|e| e.0.saturating_sub(now.0) / 1_000_000_000)
    }
}

impl Device for Host {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        match ev {
            DeviceEvent::FrameRx { frame, .. } => {
                let Ok(f) = EthernetFrame::parse(&frame) else {
                    return;
                };
                if f.dst != self.mac && !f.dst.is_broadcast() {
                    return;
                }
                match f.ethertype {
                    EtherType::ARP => {
                        if let Ok(p) = ArpPacket::parse(&f.payload) {
                            self.on_arp(ctx, p);
                        }
                    }
                    EtherType::IPV4 => {
                        if let Ok(p) = Ipv4Packet::parse(&f.payload) {
                            // A server's reply may be addressed to an address this host does not
                            // have yet, so DHCP is looked at before the usual address check.
                            if !self.dhcp.enabled || !self.try_dhcp(ctx, &p, f.src) {
                                self.on_ip(ctx, p);
                            }
                        }
                    }
                    _ => {}
                }
            }
            DeviceEvent::Timer { token, .. } if token & DHCP_TIMER != 0 => {
                self.dhcp_timer(ctx, (token & 0xffff_ffff) as u32);
            }
            DeviceEvent::Timer { token, .. } => {
                // ARP resolution timeout for next hop `token`.
                if self.arp.timeout(Ipv4Addr::from_u32(token as u32)) {
                    ctx.note("arp timeout");
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
