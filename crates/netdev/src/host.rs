use crate::{Ctx, Device, DeviceEvent, Resolver};
use kernel::{PortId, SimTime};
use std::any::Any;
use std::collections::BTreeMap;
use wire::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, IcmpEcho, IcmpError, IcmpErrorKind, IpProto,
    Ipv4Addr, Ipv4Packet, MacAddr,
};

const PORT: PortId = PortId(0);
const ARP_TIMEOUT: SimTime = SimTime::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingResult {
    pub seq: u16,
    pub rtt: SimTime,
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
            ident: 1,
            seq,
            data: vec![0x61; 32],
        };
        self.send_ip(ctx, dst, IpProto::ICMP, echo.emit());
        seq
    }

    fn send_ip(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, proto: IpProto, payload: Vec<u8>) {
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.ip_ident,
            flags_frag: 0,
            ttl: self.ttl,
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
        if p.proto != IpProto::ICMP {
            return;
        }
        if let Ok(e) = IcmpError::parse(&p.payload) {
            self.icmp_errors.push(e.kind);
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
        } else if let Some(t0) = self.sent.remove(&echo.seq) {
            self.ping_results.push(PingResult {
                seq: echo.seq,
                rtt: SimTime(ctx.now().0 - t0.0),
            });
        }
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
                            self.on_ip(ctx, p);
                        }
                    }
                    _ => {}
                }
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
