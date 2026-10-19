use crate::fib::{mask, Fib, Route, RouteKind};
use crate::{Ctx, Device, DeviceEvent, Resolver};
use kernel::{PortId, SimTime};
use std::any::Any;
use wire::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, IcmpEcho, IcmpError, IcmpErrorKind, IpProto,
    Ipv4Addr, Ipv4Packet, MacAddr,
};

const ARP_TIMEOUT: SimTime = SimTime::from_secs(1);
const ARP_TTL: SimTime = SimTime::from_secs(4 * 3600);

#[derive(Clone, Debug)]
pub struct Interface {
    pub mac: MacAddr,
    pub addr: Option<(Ipv4Addr, u8)>,
    /// Administratively down interfaces neither forward nor appear in the FIB.
    pub shutdown: bool,
}

impl Interface {
    fn active_addr(&self) -> Option<(Ipv4Addr, u8)> {
        if self.shutdown {
            None
        } else {
            self.addr
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticRoute {
    pub net: Ipv4Addr,
    pub len: u8,
    pub via: Ipv4Addr,
}

/// An IPv4 router: connected and static routes, ARP, TTL handling and ICMP errors.
#[derive(Debug)]
pub struct Router {
    pub name: String,
    ifaces: Vec<Interface>,
    statics: Vec<StaticRoute>,
    fib: Fib,
    arp: Resolver,
    ident: u16,
}

impl Router {
    /// `unit` makes interface MACs unique across routers in a lab (02:aa:00:00:<unit>:<port>).
    pub fn new(name: &str, unit: u8, n_ports: u16) -> Self {
        let ifaces = (0..n_ports)
            .map(|p| Interface {
                mac: MacAddr([2, 0xaa, 0, 0, unit, p as u8]),
                addr: None,
                shutdown: true,
            })
            .collect();
        Router {
            name: name.to_string(),
            ifaces,
            statics: Vec::new(),
            fib: Fib::default(),
            arp: Resolver::default(),
            ident: 1,
        }
    }

    pub fn interfaces(&self) -> &[Interface] {
        &self.ifaces
    }

    pub fn statics(&self) -> &[StaticRoute] {
        &self.statics
    }

    pub fn fib(&self) -> &Fib {
        &self.fib
    }

    pub fn arp_entries(&self, now: SimTime) -> Vec<(Ipv4Addr, MacAddr)> {
        self.arp.entries(now)
    }

    pub fn set_address(&mut self, port: PortId, addr: Option<(Ipv4Addr, u8)>) {
        self.ifaces[port.0 as usize].addr = addr;
        self.rebuild_fib();
    }

    pub fn set_shutdown(&mut self, port: PortId, down: bool) {
        self.ifaces[port.0 as usize].shutdown = down;
        self.rebuild_fib();
    }

    pub fn add_static(&mut self, r: StaticRoute) {
        if !self.statics.contains(&r) {
            self.statics.push(r);
        }
        self.rebuild_fib();
    }

    pub fn remove_static(&mut self, r: StaticRoute) {
        self.statics.retain(|s| *s != r);
        self.rebuild_fib();
    }

    /// Statics install only while their next hop is reachable through a connected route.
    fn rebuild_fib(&mut self) {
        let mut fib = Fib::default();
        for (i, ifc) in self.ifaces.iter().enumerate() {
            if let Some((ip, len)) = ifc.active_addr() {
                fib.insert(Route {
                    net: Ipv4Addr::from_u32(ip.to_u32() & mask(len)),
                    len,
                    via: None,
                    port: PortId(i as u16),
                    kind: RouteKind::Connected,
                });
            }
        }
        let connected = fib.clone();
        for s in &self.statics {
            if let Some(c) = connected
                .lookup(s.via)
                .filter(|c| c.kind == RouteKind::Connected)
            {
                fib.insert(Route {
                    net: s.net,
                    len: s.len,
                    via: Some(s.via),
                    port: c.port,
                    kind: RouteKind::Static,
                });
            }
        }
        self.fib = fib;
    }

    fn owns(&self, ip: Ipv4Addr) -> bool {
        self.ifaces
            .iter()
            .any(|i| i.active_addr().is_some_and(|(a, _)| a == ip))
    }

    fn emit(&self, ctx: &mut Ctx, port: PortId, dst: MacAddr, et: EtherType, mut payload: Vec<u8>) {
        if payload.len() < 46 {
            payload.resize(46, 0);
        }
        let src = self.ifaces[port.0 as usize].mac;
        let f = EthernetFrame {
            dst,
            src,
            vlan: None,
            ethertype: et,
            payload,
        };
        ctx.transmit(port, f.emit());
    }

    /// Route and transmit an IP packet (forwarded or locally originated).
    fn route(&mut self, ctx: &mut Ctx, pkt: Ipv4Packet) {
        let Some(r) = self.fib.lookup(pkt.dst).copied() else {
            ctx.note("no route");
            return;
        };
        let hop = r.via.unwrap_or(pkt.dst);
        if let Some(mac) = self.arp.lookup(hop, ctx.now()) {
            self.emit(ctx, r.port, mac, EtherType::IPV4, pkt.emit());
            return;
        }
        if self.arp.queue(hop, pkt) {
            let sender_ip = self.ifaces[r.port.0 as usize]
                .active_addr()
                .map_or(Ipv4Addr::UNSPECIFIED, |(a, _)| a);
            let req = ArpPacket {
                op: ArpOp::Request,
                sender_mac: self.ifaces[r.port.0 as usize].mac,
                sender_ip,
                target_mac: MacAddr::default(),
                target_ip: hop,
            };
            self.emit(ctx, r.port, MacAddr::BROADCAST, EtherType::ARP, req.emit());
            ctx.schedule(ARP_TIMEOUT, hop.to_u32() as u64);
        }
    }

    fn send_icmp_error(
        &mut self,
        ctx: &mut Ctx,
        ingress: PortId,
        offending: &Ipv4Packet,
        kind: IcmpErrorKind,
    ) {
        // Never answer an ICMP error with another error.
        if offending.proto == IpProto::ICMP
            && !matches!(offending.payload.first(), Some(0) | Some(8))
        {
            return;
        }
        let Some((src, _)) = self.ifaces[ingress.0 as usize].active_addr() else {
            return;
        };
        let err = IcmpError {
            kind,
            original: IcmpError::quote(&offending.emit()),
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl: 255,
            proto: IpProto::ICMP,
            src,
            dst: offending.src,
            payload: err.emit(),
        };
        self.route(ctx, pkt);
    }

    fn next_ident(&mut self) -> u16 {
        let i = self.ident;
        self.ident = self.ident.wrapping_add(1);
        i
    }

    fn on_arp(&mut self, ctx: &mut Ctx, port: PortId, p: ArpPacket) {
        let Some((my_ip, _)) = self.ifaces[port.0 as usize].active_addr() else {
            return;
        };
        if p.target_ip != my_ip {
            return;
        }
        self.arp
            .learn(p.sender_ip, p.sender_mac, ctx.now() + ARP_TTL);
        if p.op == ArpOp::Request {
            let reply = ArpPacket {
                op: ArpOp::Reply,
                sender_mac: self.ifaces[port.0 as usize].mac,
                sender_ip: my_ip,
                target_mac: p.sender_mac,
                target_ip: p.sender_ip,
            };
            self.emit(ctx, port, p.sender_mac, EtherType::ARP, reply.emit());
        }
        for pkt in self.arp.take_pending(p.sender_ip) {
            // Re-route rather than emit directly: the FIB may have changed while waiting.
            self.route(ctx, pkt);
        }
    }

    fn on_ip(&mut self, ctx: &mut Ctx, ingress: PortId, mut p: Ipv4Packet) {
        if self.owns(p.dst) {
            if p.proto != IpProto::ICMP {
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
                let out = Ipv4Packet {
                    dscp_ecn: 0,
                    ident: self.next_ident(),
                    flags_frag: 0,
                    ttl: 255,
                    proto: IpProto::ICMP,
                    // Reply from the address that was pinged.
                    src: p.dst,
                    dst: p.src,
                    payload: reply.emit(),
                };
                self.route(ctx, out);
            }
            return;
        }
        if p.dst == Ipv4Addr::BROADCAST {
            return;
        }
        if p.ttl <= 1 {
            self.send_icmp_error(ctx, ingress, &p, IcmpErrorKind::TimeExceeded);
            return;
        }
        if self.fib.lookup(p.dst).is_none() {
            self.send_icmp_error(ctx, ingress, &p, IcmpErrorKind::DestUnreachable(0));
            return;
        }
        p.ttl -= 1;
        self.route(ctx, p);
    }
}

impl Device for Router {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        match ev {
            DeviceEvent::FrameRx { port, frame } => {
                let ifc = &self.ifaces[port.0 as usize];
                if ifc.active_addr().is_none() {
                    return;
                }
                let Ok(f) = EthernetFrame::parse(&frame) else {
                    return;
                };
                if f.dst != ifc.mac && !f.dst.is_broadcast() {
                    return;
                }
                match f.ethertype {
                    EtherType::ARP => {
                        if let Ok(p) = ArpPacket::parse(&f.payload) {
                            self.on_arp(ctx, port, p);
                        }
                    }
                    EtherType::IPV4 => {
                        if let Ok(p) = Ipv4Packet::parse(&f.payload) {
                            self.on_ip(ctx, port, p);
                        }
                    }
                    _ => {}
                }
            }
            DeviceEvent::Timer { token, .. } => {
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
