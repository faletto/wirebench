use crate::fib::{mask, Fib, Route, RouteKind};
use crate::{Ctx, Device, DeviceEvent, Resolver};
use kernel::{PortId, SimTime};
use std::any::Any;
use wire::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, IcmpEcho, IcmpError, IcmpErrorKind, IpProto,
    Ipv4Addr, Ipv4Packet, MacAddr, VlanTag,
};

const ARP_TIMEOUT: SimTime = SimTime::from_secs(1);
const ARP_TTL: SimTime = SimTime::from_secs(4 * 3600);

/// A logical interface. The first `n_ports` are the physical ports, in port order; subinterfaces
/// follow, in the order they were created. Routes and ARP refer to interfaces by index (carried
/// in a `PortId`), which is the port number for a physical interface.
#[derive(Clone, Debug)]
pub struct Interface {
    pub mac: MacAddr,
    pub addr: Option<(Ipv4Addr, u8)>,
    /// Administratively down interfaces neither forward nor appear in the FIB.
    pub shutdown: bool,
    /// Set on subinterfaces (`interface g0/0.10`).
    pub sub: Option<SubInterface>,
    /// Set on a switch virtual interface: the VLAN it routes for (`interface vlan 10`).
    pub svi: Option<u16>,
    /// The link layer is down although the interface is not shut down. A switch sets this on an
    /// SVI whose VLAN has no port in it.
    pub line_down: bool,
}

/// An 802.1Q subinterface: traffic for one VLAN on a physical port, sharing the port's MAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubInterface {
    pub parent: PortId,
    /// The number after the dot in the name. Unrelated to the VLAN id.
    pub number: u16,
    /// VLAN carried, set by `encapsulation dot1q`. Without it the interface carries nothing.
    pub vlan: Option<u16>,
    /// Untagged frames belong to this subinterface instead of the physical one.
    pub native: bool,
}

/// A frame the router wants sent on logical interface `iface`; the owner puts it on a wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub iface: usize,
    pub dst: MacAddr,
    pub ethertype: EtherType,
    pub payload: Vec<u8>,
}

/// Why an `encapsulation dot1q` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncapError {
    /// Not a subinterface.
    NotSubinterface,
    /// Another subinterface of the same port already carries that VLAN.
    InUse,
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
    n_ports: u16,
    ifaces: Vec<Interface>,
    statics: Vec<StaticRoute>,
    fib: Fib,
    arp: Resolver,
    ident: u16,
    /// Forward packets that are not for the router itself. A layer 3 switch starts with this off
    /// (`ip routing` turns it on); it then only answers for its own addresses.
    routing: bool,
    /// With routing off, where traffic for other networks goes (`ip default-gateway`).
    default_gateway: Option<Ipv4Addr>,
    outbox: Vec<Outgoing>,
}

impl Router {
    /// `unit` makes interface MACs unique across routers in a lab (02:aa:00:00:<unit>:<port>).
    pub fn new(name: &str, unit: u8, n_ports: u16) -> Self {
        let ifaces = (0..n_ports)
            .map(|p| Interface {
                mac: MacAddr([2, 0xaa, 0, 0, unit, p as u8]),
                addr: None,
                shutdown: true,
                sub: None,
                svi: None,
                line_down: false,
            })
            .collect();
        Router {
            name: name.to_string(),
            n_ports,
            ifaces,
            statics: Vec::new(),
            fib: Fib::default(),
            arp: Resolver::default(),
            ident: 1,
            routing: true,
            default_gateway: None,
            outbox: Vec::new(),
        }
    }

    /// Every logical interface: the physical ports, then subinterfaces.
    pub fn interfaces(&self) -> &[Interface] {
        &self.ifaces
    }

    /// Number of physical ports.
    pub fn n_ports(&self) -> u16 {
        self.n_ports
    }

    /// The interface's address if it is configured and up (its own and its port's shutdown flags
    /// both clear).
    pub fn live(&self, i: usize) -> Option<(Ipv4Addr, u8)> {
        let ifc = self.ifaces.get(i)?;
        let parent_down = ifc
            .sub
            .is_some_and(|s| self.ifaces[s.parent.0 as usize].shutdown);
        if ifc.shutdown || ifc.line_down || parent_down {
            None
        } else {
            ifc.addr
        }
    }

    /// Whether the interface is administratively up, with or without an address.
    pub fn is_up(&self, i: usize) -> bool {
        self.ifaces.get(i).is_some_and(|ifc| {
            !ifc.shutdown
                && !ifc.line_down
                && ifc
                    .sub
                    .is_none_or(|s| !self.ifaces[s.parent.0 as usize].shutdown)
        })
    }

    /// Find or create subinterface `parent.number`; returns its interface index.
    pub fn add_subinterface(&mut self, parent: PortId, number: u16) -> Option<usize> {
        if parent.0 >= self.n_ports {
            return None;
        }
        let found = self.ifaces.iter().position(|i| {
            i.sub
                .is_some_and(|s| s.parent == parent && s.number == number)
        });
        if found.is_some() {
            return found;
        }
        self.ifaces.push(Interface {
            mac: self.ifaces[parent.0 as usize].mac,
            addr: None,
            shutdown: false,
            svi: None,
            line_down: false,
            sub: Some(SubInterface {
                parent,
                number,
                vlan: None,
                native: false,
            }),
        });
        Some(self.ifaces.len() - 1)
    }

    /// Delete a subinterface. Interface indexes above it shift down by one. Physical interfaces
    /// cannot be removed.
    pub fn remove_subinterface(&mut self, i: usize) -> bool {
        if self.ifaces.get(i).is_none_or(|ifc| ifc.sub.is_none()) {
            return false;
        }
        self.ifaces.remove(i);
        self.rebuild_fib();
        true
    }

    /// Find or create the switch virtual interface for `vlan`, with the given MAC address.
    pub fn add_svi(&mut self, vlan: u16, mac: MacAddr) -> usize {
        if let Some(i) = self.svi_index(vlan) {
            return i;
        }
        self.ifaces.push(Interface {
            mac,
            addr: None,
            shutdown: false,
            sub: None,
            svi: Some(vlan),
            line_down: false,
        });
        self.ifaces.len() - 1
    }

    pub fn svi_index(&self, vlan: u16) -> Option<usize> {
        self.ifaces.iter().position(|i| i.svi == Some(vlan))
    }

    /// Delete the SVI for `vlan`; interface indexes above it shift down by one.
    pub fn remove_svi(&mut self, vlan: u16) -> bool {
        let Some(i) = self.svi_index(vlan) else {
            return false;
        };
        self.ifaces.remove(i);
        self.rebuild_fib();
        true
    }

    /// Mark interface `i` as having (or lacking) a working link layer.
    pub fn set_line_down(&mut self, i: usize, down: bool) {
        if let Some(ifc) = self.ifaces.get_mut(i) {
            if ifc.line_down != down {
                ifc.line_down = down;
                self.rebuild_fib();
            }
        }
    }

    pub fn routing(&self) -> bool {
        self.routing
    }

    pub fn set_routing(&mut self, on: bool) {
        self.routing = on;
        self.rebuild_fib();
    }

    pub fn default_gateway(&self) -> Option<Ipv4Addr> {
        self.default_gateway
    }

    pub fn set_default_gateway(&mut self, gw: Option<Ipv4Addr>) {
        self.default_gateway = gw;
        self.rebuild_fib();
    }

    /// Frames queued for sending since the last call.
    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    /// `encapsulation dot1q <vlan> [native]` on a subinterface, or `None` to remove it.
    pub fn set_encapsulation(
        &mut self,
        i: usize,
        vlan: Option<u16>,
        native: bool,
    ) -> Result<(), EncapError> {
        let Some(sub) = self.ifaces.get(i).and_then(|ifc| ifc.sub) else {
            return Err(EncapError::NotSubinterface);
        };
        if let Some(v) = vlan {
            let clash = self.ifaces.iter().enumerate().any(|(j, o)| {
                j != i
                    && o.sub
                        .is_some_and(|s| s.parent == sub.parent && s.vlan == Some(v))
            });
            if clash {
                return Err(EncapError::InUse);
            }
        }
        if let Some(s) = self.ifaces[i].sub.as_mut() {
            s.vlan = vlan;
            s.native = native && vlan.is_some();
        }
        Ok(())
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
        if let Some(ifc) = self.ifaces.get_mut(port.0 as usize) {
            ifc.addr = addr;
        }
        self.rebuild_fib();
    }

    pub fn set_shutdown(&mut self, port: PortId, down: bool) {
        if let Some(ifc) = self.ifaces.get_mut(port.0 as usize) {
            ifc.shutdown = down;
        }
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
        for i in 0..self.ifaces.len() {
            if let Some((ip, len)) = self.live(i) {
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
        if !self.routing {
            if let Some(gw) = self.default_gateway {
                if let Some(c) = connected
                    .lookup(gw)
                    .filter(|c| c.kind == RouteKind::Connected)
                {
                    fib.insert(Route {
                        net: Ipv4Addr::UNSPECIFIED,
                        len: 0,
                        via: Some(gw),
                        port: c.port,
                        kind: RouteKind::Static,
                    });
                }
            }
        }
        for s in self.statics.iter().filter(|_| self.routing) {
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

    /// The interface that receives a frame arriving on physical `port` with VLAN `vid` (if tagged):
    /// the matching subinterface, or for untagged frames the native subinterface, else the port.
    fn classify(&self, port: PortId, vid: Option<u16>) -> Option<PortId> {
        if port.0 >= self.n_ports {
            return None;
        }
        let sub = |want: &dyn Fn(&SubInterface) -> bool| {
            self.ifaces.iter().position(|i| {
                i.sub
                    .is_some_and(|s| s.parent == port && s.vlan.is_some() && want(&s))
            })
        };
        let idx = match vid {
            Some(v) => sub(&|s| !s.native && s.vlan == Some(v))?,
            None => sub(&|s| s.native).unwrap_or(port.0 as usize),
        };
        Some(PortId(idx as u16))
    }

    fn owns(&self, ip: Ipv4Addr) -> bool {
        (0..self.ifaces.len()).any(|i| self.live(i).is_some_and(|(a, _)| a == ip))
    }

    fn emit(&mut self, port: PortId, dst: MacAddr, ethertype: EtherType, mut payload: Vec<u8>) {
        if payload.len() < 46 {
            payload.resize(46, 0);
        }
        self.outbox.push(Outgoing {
            iface: port.0 as usize,
            dst,
            ethertype,
            payload,
        });
    }

    /// Put queued frames on the wire: a subinterface sends on its parent port, tagged unless it is
    /// the native one. SVIs have no wire of their own, so a router drops anything queued for one.
    fn flush(&mut self, ctx: &mut Ctx) {
        for o in std::mem::take(&mut self.outbox) {
            let ifc = &self.ifaces[o.iface];
            let (phys, vlan) = match (ifc.sub, ifc.svi) {
                (_, Some(_)) => continue,
                (Some(s), _) => (
                    s.parent,
                    s.vlan.filter(|_| !s.native).map(|vid| VlanTag {
                        pcp: 0,
                        dei: false,
                        vid,
                    }),
                ),
                (None, None) => (PortId(o.iface as u16), None),
            };
            let f = EthernetFrame {
                dst: o.dst,
                src: ifc.mac,
                vlan,
                ethertype: o.ethertype,
                payload: o.payload,
            };
            ctx.transmit(phys, f.emit());
        }
    }

    /// Route and transmit an IP packet (forwarded or locally originated).
    fn route(&mut self, ctx: &mut Ctx, pkt: Ipv4Packet) {
        let Some(r) = self.fib.lookup(pkt.dst).copied() else {
            ctx.note("no route");
            return;
        };
        let hop = r.via.unwrap_or(pkt.dst);
        if let Some(mac) = self.arp.lookup(hop, ctx.now()) {
            self.emit(r.port, mac, EtherType::IPV4, pkt.emit());
            return;
        }
        if self.arp.queue(hop, pkt) {
            let sender_ip = self
                .live(r.port.0 as usize)
                .map_or(Ipv4Addr::UNSPECIFIED, |(a, _)| a);
            let req = ArpPacket {
                op: ArpOp::Request,
                sender_mac: self.ifaces[r.port.0 as usize].mac,
                sender_ip,
                target_mac: MacAddr::default(),
                target_ip: hop,
            };
            self.emit(r.port, MacAddr::BROADCAST, EtherType::ARP, req.emit());
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
        let Some((src, _)) = self.live(ingress.0 as usize) else {
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
        let Some((my_ip, _)) = self.live(port.0 as usize) else {
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
            self.emit(port, p.sender_mac, EtherType::ARP, reply.emit());
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
        if !self.routing || p.dst == Ipv4Addr::BROADCAST {
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

impl Router {
    /// A frame that arrived for logical interface `i`. Wrong address or interface down: ignored.
    /// Anything the router wants to send is queued; collect it with [`Router::take_outbox`].
    pub fn receive(&mut self, ctx: &mut Ctx, i: usize, f: EthernetFrame) {
        if self.live(i).is_none() {
            return;
        }
        if f.dst != self.ifaces[i].mac && !f.dst.is_broadcast() {
            return;
        }
        let port = PortId(i as u16);
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

    /// An ARP resolution timer fired (the token is the next hop's address).
    pub fn on_timer(&mut self, ctx: &mut Ctx, token: u64) {
        if self.arp.timeout(Ipv4Addr::from_u32(token as u32)) {
            ctx.note("arp timeout");
        }
    }
}

impl Device for Router {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        match ev {
            DeviceEvent::FrameRx { port, frame } => {
                let Ok(f) = EthernetFrame::parse(&frame) else {
                    return;
                };
                // Which logical interface the frame is for: by VLAN tag on this physical port.
                let Some(i) = self.classify(port, f.vlan.map(|t| t.vid)) else {
                    return;
                };
                self.receive(ctx, i.0 as usize, f);
            }
            DeviceEvent::Timer { token, .. } => self.on_timer(ctx, token),
        }
        self.flush(ctx);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
