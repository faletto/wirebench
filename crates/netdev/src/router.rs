use crate::discovery::{Discovery, Identity, PortInfo as DiscPort};
use crate::fib::{mask, Fib, Route, RouteKind};
use crate::syslog::{LineSeen, Logging, ERRORS, NOTIFICATIONS};
use crate::{civil_from_unix, MONTHS};
use crate::{
    record_probe, AclPacket, AclSet, Clock, Ctx, Device, DeviceEvent, DhcpServer, FhrpGroup,
    FhrpKind, FhrpMsg, FhrpProto, FhrpState, Mgmt, Nat, NatFlow, NatSide, NatTarget, Ntp, Ospf,
    OspfIfCfg, OspfIfView, OspfPathKind, OspfSource, OspfTx, ProbeBody, ProbeReply, Resolver, Tcp,
    TcpTx,
};
use kernel::{PortId, SimTime};
use std::any::Any;
use wire::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, IcmpEcho, IcmpError, IcmpErrorKind, IpProto,
    Ipv4Addr, Ipv4Packet, MacAddr, OspfPacket, UdpDatagram, VlanTag, ALL_DR_ROUTERS,
    ALL_SPF_ROUTERS,
};
use wire::{DhcpMessage, DhcpOp, DhcpType};

const ARP_TIMEOUT: SimTime = SimTime::from_secs(1);
/// At most one ICMP destination-unreachable per this long for traffic the router forwards
/// (`ip icmp rate-limit unreachable`'s default of 500 ms). Time-exceeded is not limited.
const UNREACHABLE_GAP: SimTime = SimTime::from_millis(500);
/// The timer token of OSPF's one-second upkeep (ARP's tokens are 32-bit addresses).
const OSPF_TICK: u64 = 1 << 40;
/// The timer token of the neighbour discovery upkeep.
const DISC_TICK: u64 = 1 << 41;
/// The timer token of NTP polling.
const NTP_TICK: u64 = 1 << 42;
/// The timer token of the first hop redundancy upkeep.
const FHRP_TICK: u64 = 1 << 43;
/// The timer token of the interface watcher that feeds the log.
pub(crate) const LOG_TICK: u64 = 1 << 45;
/// The timer token of TCP retransmission.
const TCP_TICK: u64 = 1 << 44;
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
    /// `ip helper-address`: where DHCP requests heard on this interface are relayed.
    pub helpers: Vec<Ipv4Addr>,
    /// The link layer is down although the interface is not shut down. A switch sets this on an
    /// SVI whose VLAN has no port in it.
    pub line_down: bool,
    /// `ip access-group <acl> in`: the access list applied to packets arriving on this interface.
    pub acl_in: Option<String>,
    /// `ip access-group <acl> out`: applied to packets forwarded out of it (never to packets the
    /// router itself sends).
    pub acl_out: Option<String>,
    /// `ip nat inside` / `ip nat outside`.
    pub nat: Option<NatSide>,
    /// `ip ospf ...` and `passive-interface`.
    pub ospf: OspfIfCfg,
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
    /// Send from this MAC address instead of the interface's own (a virtual router address).
    pub src: Option<MacAddr>,
}

/// One first hop redundancy group on one logical interface.
#[derive(Clone, Debug)]
pub struct FhrpEntry {
    pub iface: usize,
    pub g: FhrpGroup,
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
#[derive(Debug, Clone)]
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
    /// Passwords, banner and lines (see [`Mgmt`]).
    pub mgmt: Mgmt,
    /// The saved configuration: what `copy running-config startup-config` stored.
    pub startup: Option<String>,
    /// The DHCP server: pools, exclusions and leases. Answers clients on interfaces without a
    /// `helper-address`.
    pub dhcp: DhcpServer,
    /// Access lists, by name or number (see [`Router::acl_permits`]).
    pub acls: AclSet,
    /// `router ospf`: the protocol state, if configured.
    pub ospf: Option<Ospf>,
    ospf_timer: bool,
    /// CDP and LLDP.
    pub disc: Discovery,
    disc_timer: bool,
    /// The device clock (`clock set`, NTP) and NTP itself.
    pub clock: Clock,
    pub ntp: Ntp,
    ntp_timer: bool,
    /// The message log, syslog hosts and trap level.
    pub log: Logging,
    log_timer: bool,
    /// HSRP and VRRP groups.
    pub fhrp: Vec<FhrpEntry>,
    fhrp_timer: bool,
    /// TCP connections and UDP sockets (telnet and SSH sessions live here).
    pub tcp: Tcp,
    /// Address translation (`ip nat ...`).
    pub nat: Nat,
    /// Set by a switch for its layer 3 half: vty ports and `access-class`, which the switch holds.
    vty_policy: Option<(Vec<u16>, Option<String>)>,
    tcp_armed: Option<SimTime>,
    /// Source MAC of the frame being handled (OSPF learns its neighbours' addresses from it).
    rx_src_mac: MacAddr,
    /// Replies to probes this router sent (pings, traceroutes), oldest first.
    pub probes: Vec<ProbeReply>,
    /// No forwarding-plane unreachable is sent before this time.
    unreachable_after: SimTime,
}

const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;
/// A relayed request that has already been through this many relays is dropped (RFC 1542).
const MAX_RELAY_HOPS: u8 = 16;
/// Source port of the UDP packets traceroute sends.
const PROBE_SPORT: u16 = 49_152;

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
                helpers: Vec::new(),
                line_down: false,
                acl_in: None,
                acl_out: None,
                nat: None,
                ospf: OspfIfCfg::default(),
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
            mgmt: Mgmt::default(),
            startup: None,
            dhcp: DhcpServer::default(),
            acls: AclSet::default(),
            ospf: None,
            ospf_timer: false,
            disc: Discovery::new(n_ports as usize),
            disc_timer: false,
            clock: Clock::default(),
            ntp: Ntp::default(),
            ntp_timer: false,
            log: Logging::default(),
            log_timer: false,
            tcp: Tcp::new(u32::from(unit) * 0x0101_0101),
            vty_policy: None,
            nat: Nat::default(),
            tcp_armed: None,
            fhrp: Vec::new(),
            fhrp_timer: false,
            rx_src_mac: MacAddr::default(),
            probes: Vec::new(),
            unreachable_after: SimTime::ZERO,
        }
    }

    /// Back to what a router is before anyone configures it: default name, nothing addressed,
    /// every port shut down, no routes, passwords or banner, empty ARP cache. The saved
    /// configuration survives (it is what a reload restores).
    pub fn factory_reset(&mut self) {
        self.name = "Router".to_string();
        self.ifaces.truncate(self.n_ports as usize);
        for ifc in &mut self.ifaces {
            ifc.addr = None;
            ifc.shutdown = true;
            ifc.line_down = false;
        }
        self.statics.clear();
        self.arp = Resolver::default();
        self.outbox.clear();
        self.routing = true;
        self.default_gateway = None;
        // The key pair lives in private storage on a real device: reload and erase keep it.
        let rsa = self.mgmt.rsa_bits;
        self.mgmt = Mgmt::default();
        self.mgmt.rsa_bits = rsa;
        self.dhcp = DhcpServer::default();
        self.acls = AclSet::default();
        self.ospf = None;
        self.nat = Nat::default();
        self.disc = Discovery::new(self.n_ports as usize);
        self.ntp.reset();
        self.log.reset();
        self.fhrp.clear();
        self.probes.clear();
        self.unreachable_after = SimTime::ZERO;
        for ifc in &mut self.ifaces {
            ifc.helpers.clear();
            ifc.acl_in = None;
            ifc.acl_out = None;
            ifc.nat = None;
            ifc.ospf = OspfIfCfg::default();
        }
        self.rebuild_fib();
    }

    /// `ip access-group`: set (or with `None` clear) the list applied to interface `i` in one
    /// direction.
    pub fn set_acl(&mut self, i: usize, inbound: bool, name: Option<String>) {
        let ifc = &mut self.ifaces[i];
        if inbound {
            ifc.acl_in = name;
        } else {
            ifc.acl_out = name;
        }
    }

    /// `ip nat inside` / `ip nat outside` on interface `i`; `None` takes it off.
    pub fn set_nat_side(&mut self, i: usize, side: Option<NatSide>) {
        self.ifaces[i].nat = side;
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
            helpers: Vec::new(),
            line_down: false,
            acl_in: None,
            acl_out: None,
            nat: None,
            ospf: OspfIfCfg::default(),
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

    /// The address a probe to `dst` leaves from: the interface the route to it uses.
    fn source_for(&self, dst: Ipv4Addr) -> Option<Ipv4Addr> {
        let r = self.fib.lookup(dst)?;
        self.live(r.port.0 as usize).map(|(a, _)| a)
    }

    fn log_reply(&mut self, now: SimTime, from: Ipv4Addr, body: ProbeBody) {
        record_probe(
            &mut self.probes,
            ProbeReply {
                at: now,
                from,
                body,
            },
        );
    }

    /// Send a UDP packet with the given TTL to `dst`:`dport` (a traceroute probe). False if
    /// there is no route; to one of this router's own addresses the answer is immediate.
    pub fn send_udp_probe(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, ttl: u8, dport: u16) -> bool {
        if self.owns(dst) {
            let probe = Ipv4Packet {
                dscp_ecn: 0,
                ident: 0,
                flags_frag: 0,
                ttl,
                proto: IpProto::UDP,
                src: dst,
                dst,
                payload: UdpDatagram {
                    src_port: PROBE_SPORT,
                    dst_port: dport,
                    payload: vec![0; 12],
                }
                .emit(dst, dst),
            };
            let original = IcmpError::quote(&probe.emit());
            let kind = IcmpErrorKind::DestUnreachable(3);
            self.log_reply(ctx.now(), dst, ProbeBody::Error { kind, original });
            return true;
        }
        let Some(src) = self.source_for(dst) else {
            return false;
        };
        let payload = vec![0u8; 12];
        let udp = UdpDatagram {
            src_port: PROBE_SPORT,
            dst_port: dport,
            payload,
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl,
            proto: IpProto::UDP,
            src,
            dst,
            payload: udp.emit(src, dst),
        };
        self.route(ctx, pkt);
        true
    }

    /// Send an ICMP echo request with the given TTL. Same conventions as [`Router::send_udp_probe`].
    pub fn send_echo_probe(
        &mut self,
        ctx: &mut Ctx,
        dst: Ipv4Addr,
        ttl: u8,
        ident: u16,
        seq: u16,
    ) -> bool {
        if self.owns(dst) {
            self.log_reply(ctx.now(), dst, ProbeBody::EchoReply { ident, seq });
            return true;
        }
        let Some(src) = self.source_for(dst) else {
            return false;
        };
        let echo = IcmpEcho {
            reply: false,
            ident,
            seq,
            data: vec![0x61; 32],
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl,
            proto: IpProto::ICMP,
            src,
            dst,
            payload: echo.emit(),
        };
        self.route(ctx, pkt);
        true
    }

    /// Put frames queued by the probe functions on the wire. (A switch sends its own.)
    pub fn flush_outbox(&mut self, ctx: &mut Ctx) {
        self.flush(ctx);
    }

    /// `ip helper-address` on interface `i`.
    pub fn add_helper(&mut self, i: usize, server: Ipv4Addr) {
        if let Some(ifc) = self.ifaces.get_mut(i) {
            if !ifc.helpers.contains(&server) {
                ifc.helpers.push(server);
            }
        }
    }

    pub fn remove_helper(&mut self, i: usize, server: Ipv4Addr) -> bool {
        self.ifaces.get_mut(i).is_some_and(|ifc| {
            let before = ifc.helpers.len();
            ifc.helpers.retain(|h| *h != server);
            ifc.helpers.len() != before
        })
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
            helpers: Vec::new(),
            line_down: false,
            acl_in: None,
            acl_out: None,
            nat: None,
            ospf: OspfIfCfg::default(),
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
                    metric: 0,
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
                        metric: 0,
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
                    metric: 0,
                });
            }
        }
        if self.routing {
            if let Some(ospf) = &self.ospf {
                for r in ospf.routes() {
                    if let Some(c) = connected
                        .lookup(r.via)
                        .filter(|c| c.kind == RouteKind::Connected)
                    {
                        fib.insert(Route {
                            net: r.net,
                            len: r.len,
                            via: Some(r.via),
                            port: c.port,
                            kind: match r.kind {
                                OspfPathKind::Intra => RouteKind::Ospf,
                                OspfPathKind::Inter => RouteKind::OspfInter,
                                OspfPathKind::External1 => RouteKind::OspfExt1,
                                OspfPathKind::External2 => RouteKind::OspfExt2,
                            },
                            metric: r.cost,
                        });
                    }
                }
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
            || self.active_vip(ip).is_some()
    }

    fn emit(&mut self, port: PortId, dst: MacAddr, ethertype: EtherType, payload: Vec<u8>) {
        self.emit_from(port, None, dst, ethertype, payload);
    }

    fn emit_from(
        &mut self,
        port: PortId,
        src: Option<MacAddr>,
        dst: MacAddr,
        ethertype: EtherType,
        mut payload: Vec<u8>,
    ) {
        if payload.len() < 46 {
            payload.resize(46, 0);
        }
        self.outbox.push(Outgoing {
            iface: port.0 as usize,
            dst,
            ethertype,
            payload,
            src,
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
                src: o.src.unwrap_or(ifc.mac),
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
        let Some((src, _)) = self.live(ingress.0 as usize) else {
            return;
        };
        self.send_icmp_error_from(ctx, src, offending, kind);
    }

    /// Send an ICMP error about `offending` from address `src`.
    fn send_icmp_error_from(
        &mut self,
        ctx: &mut Ctx,
        src: Ipv4Addr,
        offending: &Ipv4Packet,
        kind: IcmpErrorKind,
    ) {
        // Never answer an ICMP error with another error.
        if offending.proto == IpProto::ICMP
            && !matches!(offending.payload.first(), Some(0) | Some(8))
        {
            return;
        }
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

    /// An ICMP packet addressed to this router: answer an echo request, remember anything that
    /// looks like an answer to something it sent.
    fn on_icmp_to_me(&mut self, ctx: &mut Ctx, p: &Ipv4Packet) {
        let now = ctx.now();
        if let Ok(echo) = IcmpEcho::parse(&p.payload) {
            if echo.reply {
                self.log_reply(
                    now,
                    p.src,
                    ProbeBody::EchoReply {
                        ident: echo.ident,
                        seq: echo.seq,
                    },
                );
            } else {
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
        } else if let Ok(e) = IcmpError::parse(&p.payload) {
            self.log_reply(
                now,
                p.src,
                ProbeBody::Error {
                    kind: e.kind,
                    original: e.original,
                },
            );
        }
    }

    /// Whether a forwarding-plane unreachable may be sent now (and if so, start the gap).
    fn unreachable_allowed(&mut self, now: SimTime) -> bool {
        if now < self.unreachable_after {
            return false;
        }
        self.unreachable_after = now + UNREACHABLE_GAP;
        true
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
        // The active router of a first hop redundancy group answers for the virtual address with
        // the virtual MAC.
        let virt = self
            .fhrp
            .iter()
            .find(|e| e.iface == port.0 as usize && e.g.is_active() && e.g.vip == p.target_ip)
            .map(|e| e.g.vmac());
        // On an outside interface the router also answers for the global addresses it translates
        // (a static mapping's, or one in use), so the neighbour needs no route to them.
        let proxy = self.ifaces[port.0 as usize].nat == Some(NatSide::Outside)
            && (self.nat.is_global(p.target_ip) || self.nat_virtual(p.target_ip));
        if p.target_ip != my_ip && virt.is_none() && !proxy {
            return;
        }
        self.arp
            .learn(p.sender_ip, p.sender_mac, ctx.now() + ARP_TTL);
        if p.op == ArpOp::Request {
            let mac = virt.unwrap_or(self.ifaces[port.0 as usize].mac);
            let reply = ArpPacket {
                op: ArpOp::Reply,
                sender_mac: mac,
                sender_ip: p.target_ip,
                target_mac: p.sender_mac,
                target_ip: p.sender_ip,
            };
            self.emit_from(port, Some(mac), p.sender_mac, EtherType::ARP, reply.emit());
        }
        for pkt in self.arp.take_pending(p.sender_ip) {
            // Re-route rather than emit directly: the FIB may have changed while waiting.
            self.route(ctx, pkt);
        }
    }

    /// Whether the access list `name` lets `p` through. A name with no list behind it permits
    /// everything, as IOS does.
    fn acl_permits(&mut self, name: Option<&str>, p: &Ipv4Packet) -> bool {
        match name.and_then(|n| self.acls.get_mut(n)) {
            Some(acl) => acl.check(&AclPacket::from_ipv4(p)),
            None => true,
        }
    }

    /// Tell the sender of a packet an access list refused it: ICMP "administratively prohibited"
    /// (type 3 code 13), from the address of the interface it came in on. Not for broadcasts,
    /// packets from no address, or later fragments, and no faster than other unreachables.
    fn acl_refused(&mut self, ctx: &mut Ctx, ingress: PortId, p: &Ipv4Packet) {
        if p.dst == Ipv4Addr::BROADCAST
            || p.src == Ipv4Addr::UNSPECIFIED
            || p.flags_frag & 0x1fff != 0
            || !self.unreachable_allowed(ctx.now())
        {
            return;
        }
        self.send_icmp_error(ctx, ingress, p, IcmpErrorKind::DestUnreachable(13));
    }

    fn on_ip(&mut self, ctx: &mut Ctx, ingress: PortId, mut p: Ipv4Packet) {
        // Inbound access list first: it sees everything that arrives, including packets for the
        // router itself.
        let acl_in = self.ifaces[ingress.0 as usize].acl_in.clone();
        if !self.acl_permits(acl_in.as_deref(), &p) {
            self.acl_refused(ctx, ingress, &p);
            return;
        }
        // From outside, a packet for a translated address goes back to the inside host before
        // anything else looks at it (the address may be the router's own, for PAT).
        if self.nat.is_configured() && self.ifaces[ingress.0 as usize].nat == Some(NatSide::Outside)
        {
            if let Some(q) = self.nat_inbound(ctx.now(), &p) {
                p = q;
            }
        }
        if p.proto == IpProto::OSPF {
            // Never forwarded and never answered with an error.
            if self.ospf.is_some()
                && (p.dst == ALL_SPF_ROUTERS || p.dst == ALL_DR_ROUTERS || self.owns(p.dst))
            {
                self.ospf_receive(ctx, ingress, &p);
            }
            return;
        }
        if p.proto == IpProto::UDP && p.dst == wire::HSRP_GROUP {
            self.on_hsrp(ctx, ingress, &p);
            return;
        }
        if p.proto == IpProto(wire::VRRP_PROTO) && p.dst == wire::VRRP_GROUP {
            self.on_vrrp(ctx, ingress, &p);
            return;
        }
        // Multicast is not routed.
        if (224..240).contains(&p.dst.0[0]) {
            return;
        }
        if p.proto == IpProto::UDP
            && (p.dst == Ipv4Addr::BROADCAST || self.owns(p.dst))
            && self.on_dhcp(ctx, ingress, &p)
        {
            return;
        }
        if p.proto == IpProto::UDP && self.owns(p.dst) && self.on_ntp(ctx, &p) {
            return;
        }
        if p.proto == IpProto::UDP && self.owns(p.dst) && self.on_udp_socket(&p) {
            return;
        }
        if self.owns(p.dst) {
            match p.proto {
                IpProto::ICMP => self.on_icmp_to_me(ctx, &p),
                IpProto::TCP => self.on_tcp(ctx, &p),
                // A UDP packet for a port nothing listens on: tell the sender. (DHCP was taken
                // above.) This is what ends a UDP traceroute at its destination.
                IpProto::UDP => {
                    // From the address that was probed, as RFC 1122 asks.
                    let src = p.dst;
                    self.send_icmp_error_from(ctx, src, &p, IcmpErrorKind::DestUnreachable(3));
                }
                _ => {}
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
        let Some(out) = self.fib.lookup(p.dst).map(|r| r.port) else {
            if self.unreachable_allowed(ctx.now()) {
                self.send_icmp_error(ctx, ingress, &p, IcmpErrorKind::DestUnreachable(0));
            }
            return;
        };
        p.ttl -= 1;
        // Inside to outside: translate the source once the route is known, before the outbound
        // access list sees the packet.
        if self.nat.is_configured()
            && self.ifaces[ingress.0 as usize].nat == Some(NatSide::Inside)
            && self.ifaces[out.0 as usize].nat == Some(NatSide::Outside)
        {
            match self.nat_outbound(ctx, ingress, p) {
                Some(q) => p = q,
                None => return,
            }
        }
        // The outbound access list judges forwarded packets only, after the routing decision.
        let acl_out = self.ifaces[out.0 as usize].acl_out.clone();
        if !self.acl_permits(acl_out.as_deref(), &p) {
            self.acl_refused(ctx, ingress, &p);
            return;
        }
        self.route(ctx, p);
    }
}

// ---- DHCP: server and relay ----

impl Router {
    /// A UDP packet to this router or to the broadcast address. Returns true if it was DHCP
    /// (port 67), handled or deliberately dropped.
    fn on_dhcp(&mut self, ctx: &mut Ctx, ingress: PortId, p: &Ipv4Packet) -> bool {
        let Ok(u) = UdpDatagram::parse(&p.payload) else {
            return false;
        };
        if u.dst_port != DHCP_SERVER_PORT {
            return false;
        }
        let Ok(msg) = DhcpMessage::parse(&u.payload) else {
            return true;
        };
        let i = ingress.0 as usize;
        let Some((iface_ip, _)) = self.live(i) else {
            return true;
        };
        match msg.op {
            DhcpOp::Request => {
                // A relayed request is addressed to us; answer from the address it was sent to.
                let server_ip = if self.owns(p.dst) { p.dst } else { iface_ip };
                self.dhcp_from_client(ctx, i, iface_ip, server_ip, msg)
            }
            DhcpOp::Reply => self.dhcp_from_server(ctx, msg),
        }
        true
    }

    fn dhcp_from_client(
        &mut self,
        ctx: &mut Ctx,
        i: usize,
        iface_ip: Ipv4Addr,
        server_ip: Ipv4Addr,
        mut msg: DhcpMessage,
    ) {
        if msg.hops >= MAX_RELAY_HOPS {
            return;
        }
        let helpers = self.ifaces[i].helpers.clone();
        if !helpers.is_empty() {
            // Relay: stamp where it came from and pass it on. A request that already has a relay
            // address was relayed once; this router does not relay it again.
            if msg.giaddr != Ipv4Addr::UNSPECIFIED {
                return;
            }
            msg.giaddr = iface_ip;
            msg.hops += 1;
            let bytes = msg.emit();
            for h in helpers {
                self.send_udp(ctx, iface_ip, h, DHCP_SERVER_PORT, DHCP_SERVER_PORT, &bytes);
            }
            return;
        }
        let link = if msg.giaddr != Ipv4Addr::UNSPECIFIED {
            msg.giaddr
        } else {
            iface_ip
        };
        let own: Vec<Ipv4Addr> = (0..self.ifaces.len())
            .filter_map(|k| self.live(k).map(|a| a.0))
            .collect();
        let Some(reply) = self.dhcp.serve(ctx.now(), &msg, server_ip, link, &own) else {
            return;
        };
        if reply.giaddr != Ipv4Addr::UNSPECIFIED {
            // Back to the relay, which delivers it to the client.
            self.send_udp(
                ctx,
                server_ip,
                reply.giaddr,
                DHCP_SERVER_PORT,
                DHCP_SERVER_PORT,
                &reply.emit(),
            );
        } else {
            self.dhcp_to_client(i, server_ip, &reply);
        }
    }

    /// A server's reply reached us because we relayed the request: hand it to the client on the
    /// interface that carries the relay address.
    fn dhcp_from_server(&mut self, _ctx: &mut Ctx, msg: DhcpMessage) {
        let Some(k) =
            (0..self.ifaces.len()).find(|k| self.live(*k).map(|a| a.0) == Some(msg.giaddr))
        else {
            return;
        };
        let src = msg.giaddr;
        self.dhcp_to_client(k, src, &msg);
    }

    /// Put a reply on the client's link, addressed as RFC 2131 section 4.1 says: a NAK is
    /// broadcast, a renewing client is answered at its own address, a client that asked for
    /// broadcast gets one, anyone else gets the offered address by unicast to its hardware address
    /// (it cannot ARP before it has an address).
    fn dhcp_to_client(&mut self, i: usize, src: Ipv4Addr, reply: &DhcpMessage) {
        let (dst_ip, dst_mac) = if reply.msg_type == Some(DhcpType::Nak) {
            (Ipv4Addr::BROADCAST, MacAddr::BROADCAST)
        } else if reply.ciaddr != Ipv4Addr::UNSPECIFIED {
            (reply.ciaddr, reply.chaddr)
        } else if reply.broadcast() || reply.yiaddr == Ipv4Addr::UNSPECIFIED {
            (Ipv4Addr::BROADCAST, MacAddr::BROADCAST)
        } else {
            (reply.yiaddr, reply.chaddr)
        };
        let udp = UdpDatagram {
            src_port: DHCP_SERVER_PORT,
            dst_port: DHCP_CLIENT_PORT,
            payload: reply.emit(),
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl: 64,
            proto: IpProto::UDP,
            src,
            dst: dst_ip,
            payload: udp.emit(src, dst_ip),
        };
        self.emit(PortId(i as u16), dst_mac, EtherType::IPV4, pkt.emit());
    }

    /// Originate a UDP packet and route it like any other.
    fn send_udp(
        &mut self,
        ctx: &mut Ctx,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        sport: u16,
        dport: u16,
        payload: &[u8],
    ) {
        let udp = UdpDatagram {
            src_port: sport,
            dst_port: dport,
            payload: payload.to_vec(),
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl: 255,
            proto: IpProto::UDP,
            src,
            dst,
            payload: udp.emit(src, dst),
        };
        self.route(ctx, pkt);
    }
}

// ---- address translation ----

impl Router {
    /// Whether `ip` is a virtual address one of the inside destination rules covers. The rule's
    /// list is read against the destination, so it is tried as if `ip` were a source.
    fn nat_virtual(&mut self, ip: Ipv4Addr) -> bool {
        for rule in self.nat.dest_rules.clone() {
            let probe = Ipv4Packet {
                dscp_ecn: 0,
                ident: 0,
                flags_frag: 0,
                ttl: 1,
                proto: IpProto::TCP,
                src: ip,
                dst: ip,
                payload: vec![],
            };
            if self.acls.get(&rule.acl).is_some() && self.acl_permits(Some(&rule.acl), &probe) {
                return true;
            }
        }
        false
    }

    /// A packet from outside: an existing translation, or for a new connection to a virtual
    /// address, the next real host of its pool.
    fn nat_inbound(&mut self, now: SimTime, p: &Ipv4Packet) -> Option<Ipv4Packet> {
        if let Some(q) = self.nat.inbound(now, p) {
            return Some(q);
        }
        let f = NatFlow::of(p)?;
        // Only a SYN opens a connection; a stray segment does not.
        if f.proto != 6
            || f.flags & (wire::TcpSegment::SYN | wire::TcpSegment::ACK) != wire::TcpSegment::SYN
        {
            return None;
        }
        for (i, rule) in self.nat.dest_rules.clone().into_iter().enumerate() {
            // The list names the address being called, not the caller.
            let mut probe = p.clone();
            probe.src = p.dst;
            if self.acls.get(&rule.acl).is_none() || !self.acl_permits(Some(&rule.acl), &probe) {
                continue;
            }
            let addrs = self
                .nat
                .pool(&rule.pool)
                .map(|x| x.addrs())
                .unwrap_or_default();
            let real = self.nat.create_dest(now, &f, i, &addrs)?;
            self.nat.stats.misses += 1;
            return Nat::apply_in(p, &f, real);
        }
        None
    }

    /// Translate a packet from an inside interface leaving by an outside one. Without a rule
    /// that covers it the packet goes on as it is; when the rule has no address left it is dropped
    /// and the sender told host unreachable.
    fn nat_outbound(
        &mut self,
        ctx: &mut Ctx,
        ingress: PortId,
        p: Ipv4Packet,
    ) -> Option<Ipv4Packet> {
        let now = ctx.now();
        // To an outside local address: the destination becomes the global one first.
        let p = match self.nat.outside_dst(&p) {
            Some(q) => q,
            None => p,
        };
        // An inside host's ICMP error leaves from its global address too.
        if p.proto == IpProto::ICMP {
            if let Some(q) = self.nat.outbound_error(now, &p) {
                return Some(q);
            }
        }
        let Some(f) = NatFlow::of(&p) else {
            return Some(p);
        };
        if let Some(g) = self.nat.find_out(now, &f) {
            self.nat.stats.hits += 1;
            return Nat::apply_out(&p, &f, g).or(Some(p));
        }
        for rule in self.nat.rules.clone() {
            if self.acls.get(&rule.acl).is_none() || !self.acl_permits(Some(&rule.acl), &p) {
                continue;
            }
            let addrs = match &rule.target {
                NatTarget::Pool(n) => self.nat.pool(n).map(|p| p.addrs()).unwrap_or_default(),
                NatTarget::Interface(i) => self.live(*i).map(|(a, _)| vec![a]).unwrap_or_default(),
            };
            return match self.nat.create(now, &f, &rule.acl, &addrs, rule.overload) {
                Some(g) => {
                    self.nat.stats.misses += 1;
                    Nat::apply_out(&p, &f, g)
                }
                None => {
                    self.nat.stats.misses += 1;
                    ctx.note("nat: no address left");
                    if self.unreachable_allowed(now) {
                        self.send_icmp_error(ctx, ingress, &p, IcmpErrorKind::DestUnreachable(1));
                    }
                    None
                }
            };
        }
        Some(p)
    }
}

// ---- TCP and UDP sockets ----

impl Router {
    /// A UDP datagram for a socket an application bound. True if one was.
    fn on_udp_socket(&mut self, p: &Ipv4Packet) -> bool {
        let Ok(u) = UdpDatagram::parse(&p.payload) else {
            return false;
        };
        self.tcp
            .udp_deliver(p.src, u.src_port, u.dst_port, &u.payload)
    }

    /// A TCP segment for one of this router's addresses.
    fn on_tcp(&mut self, ctx: &mut Ctx, p: &Ipv4Packet) {
        let (ports, access_class) = self
            .vty_policy
            .clone()
            .unwrap_or_else(|| (self.mgmt.vty_ports(), self.mgmt.vty.access_class_in.clone()));
        self.tcp.set_listeners(&ports);
        let mut out = vec![];
        if let Ok(seg) = wire::TcpSegment::parse(&p.payload) {
            let new = seg.flags & (wire::TcpSegment::SYN | wire::TcpSegment::ACK)
                == wire::TcpSegment::SYN;
            // `access-class` on the vty lines: a refused caller gets a reset, as if nothing
            // listened.
            if new
                && self.tcp.listening(seg.dst_port)
                && !self.acl_permits(access_class.as_deref(), p)
            {
                self.tcp.set_listeners(&[]);
            }
        }
        self.tcp
            .on_ip(ctx.now(), p.src, p.dst, &p.payload, &mut out);
        self.tcp_emit(ctx, out);
    }

    /// For a switch's layer 3 half: the vty ports and `access-class` the switch has configured.
    pub fn set_vty_policy(&mut self, ports: Vec<u16>, access_class: Option<String>) {
        self.vty_policy = Some((ports, access_class));
    }

    /// Put segments into IP packets and send them; then make sure a retransmission timer is set.
    pub fn tcp_emit(&mut self, ctx: &mut Ctx, out: Vec<TcpTx>) {
        for tx in out {
            let pkt = Ipv4Packet {
                dscp_ecn: 0,
                ident: self.next_ident(),
                flags_frag: 0,
                ttl: 255,
                proto: IpProto::TCP,
                src: tx.src,
                dst: tx.dst,
                payload: tx.seg.emit(tx.src, tx.dst),
            };
            self.route(ctx, pkt);
        }
        if let Some(d) = self.tcp.next_deadline() {
            if self.tcp_armed.is_none_or(|a| d < a || a <= ctx.now()) {
                self.tcp_armed = Some(d);
                ctx.schedule(SimTime(d.0.saturating_sub(ctx.now().0)), TCP_TICK);
            }
        }
    }

    /// Open a connection to `dst`:`dport` from the address that faces it. None if there is no
    /// route, or `dst` is this router. Segments are queued like any probe: a router's caller
    /// follows with [`Router::flush_outbox`], a switch's goes through `originate`.
    pub fn tcp_open(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, dport: u16) -> Option<u32> {
        if self.owns(dst) {
            return None;
        }
        let src = self.source_for(dst)?;
        let mut out = vec![];
        let id = self.tcp.connect(ctx.now(), src, dst, dport, &mut out);
        self.tcp_emit(ctx, out);
        Some(id)
    }

    pub fn tcp_write(&mut self, ctx: &mut Ctx, id: u32, data: &[u8]) -> bool {
        let mut out = vec![];
        let ok = self.tcp.send(ctx.now(), id, data, &mut out);
        self.tcp_emit(ctx, out);
        ok
    }

    pub fn tcp_take(&mut self, ctx: &mut Ctx, id: u32) -> Vec<u8> {
        let mut out = vec![];
        let d = self.tcp.read(ctx.now(), id, &mut out);
        self.tcp_emit(ctx, out);
        d
    }

    pub fn tcp_shut(&mut self, ctx: &mut Ctx, id: u32) {
        let mut out = vec![];
        self.tcp.close(ctx.now(), id, &mut out);
        self.tcp_emit(ctx, out);
    }

    pub fn tcp_abort(&mut self, ctx: &mut Ctx, id: u32) {
        let mut out = vec![];
        self.tcp.abort(id, &mut out);
        self.tcp_emit(ctx, out);
    }

    /// Send a UDP datagram from a bound socket (or any port) to `dst`.
    pub fn udp_send(
        &mut self,
        ctx: &mut Ctx,
        dst: Ipv4Addr,
        sport: u16,
        dport: u16,
        data: &[u8],
    ) -> bool {
        let Some(src) = self.source_for(dst) else {
            return false;
        };
        self.send_udp(ctx, src, dst, sport, dport, data);
        true
    }
}

// ---- first hop redundancy (HSRP, VRRP) ----

fn wire_state(s: u8) -> FhrpState {
    use wire::hsrp_state::*;
    match s {
        ACTIVE => FhrpState::Active,
        STANDBY => FhrpState::Standby,
        SPEAK => FhrpState::Speak,
        LISTEN | LEARN => FhrpState::Listen,
        _ => FhrpState::Init,
    }
}

fn state_code(s: FhrpState) -> u8 {
    use wire::hsrp_state::*;
    match s {
        FhrpState::Active | FhrpState::Master => ACTIVE,
        FhrpState::Standby | FhrpState::Backup => STANDBY,
        FhrpState::Speak => SPEAK,
        FhrpState::Listen => LISTEN,
        FhrpState::Init => INITIAL,
    }
}

impl Router {
    /// The group that is active for the virtual address `ip`.
    fn active_vip(&self, ip: Ipv4Addr) -> Option<usize> {
        self.fhrp
            .iter()
            .position(|e| e.g.is_active() && e.g.vip == ip && self.live(e.iface).is_some())
    }

    /// Whether a frame for MAC `dst` is for interface `i`: its address, broadcast, the multicast
    /// groups of the protocols in use, or the virtual address of a group it is active in.
    pub fn accepts_mac(&self, i: usize, dst: MacAddr) -> bool {
        let ospf_group = self.ospf.is_some()
            && dst.0[..5] == [0x01, 0x00, 0x5e, 0x00, 0x00]
            && matches!(dst.0[5], 5 | 6);
        let fhrp_group = self.fhrp.iter().any(|e| {
            e.iface == i
                && ((e.g.proto == FhrpProto::Hsrp && dst == wire::HSRP_MAC)
                    || (e.g.proto == FhrpProto::Vrrp && dst == wire::VRRP_MAC))
        });
        dst.is_broadcast() || ospf_group || fhrp_group || self.owns_mac(i, dst)
    }

    /// Whether `dst` is this router's own unicast address on `i`, real or virtual: a frame to it is
    /// for this router and must not be bridged on.
    pub fn owns_mac(&self, i: usize, dst: MacAddr) -> bool {
        dst == self.ifaces[i].mac
            || self
                .fhrp
                .iter()
                .any(|e| e.iface == i && e.g.is_active() && e.g.vmac() == dst)
    }

    /// Create or change group `group` of `proto` on interface `i`.
    pub fn fhrp_configure(
        &mut self,
        i: usize,
        proto: FhrpProto,
        group: u8,
        f: impl FnOnce(&mut FhrpGroup),
    ) {
        let idx = match self
            .fhrp
            .iter()
            .position(|e| e.iface == i && e.g.proto == proto && e.g.group == group)
        {
            Some(x) => x,
            None => {
                self.fhrp.push(FhrpEntry {
                    iface: i,
                    g: FhrpGroup::new(proto, group, Ipv4Addr::UNSPECIFIED),
                });
                self.fhrp.len() - 1
            }
        };
        f(&mut self.fhrp[idx].g);
    }

    /// Delete a group, telling the others if this router was in charge.
    pub fn fhrp_remove(&mut self, ctx: &mut Ctx, i: usize, proto: FhrpProto, group: u8) -> bool {
        let Some(idx) = self
            .fhrp
            .iter()
            .position(|e| e.iface == i && e.g.proto == proto && e.g.group == group)
        else {
            return false;
        };
        let mut e = self.fhrp.remove(idx);
        let me = self.live(i).map_or(Ipv4Addr::UNSPECIFIED, |a| a.0);
        let mut out = vec![];
        e.g.step(ctx.now(), me, false, false, &mut out);
        for m in out {
            self.fhrp_send(i, &e.g, me, m);
        }
        true
    }

    pub fn fhrp_kick(&mut self, ctx: &mut Ctx) {
        if !self.fhrp.is_empty() && !self.fhrp_timer {
            self.fhrp_timer = true;
            ctx.schedule_background(SimTime::from_secs(1), FHRP_TICK);
        }
    }

    fn fhrp_tick(&mut self, ctx: &mut Ctx) {
        let now = ctx.now();
        for idx in 0..self.fhrp.len() {
            let i = self.fhrp[idx].iface;
            let live = self.live(i);
            let me = live.map_or(Ipv4Addr::UNSPECIFIED, |a| a.0);
            let vip = self.fhrp[idx].g.vip;
            let up = live.is_some() && vip != Ipv4Addr::UNSPECIFIED;
            let penalty = self.fhrp[idx]
                .g
                .tracks
                .clone()
                .into_iter()
                .filter(|(t, _)| !self.line_up(ctx, *t))
                .fold(0u8, |a, (_, d)| a.saturating_add(d));
            self.fhrp[idx].g.penalty = penalty;
            let mut out = vec![];
            self.fhrp[idx].g.step(now, me, up, vip == me, &mut out);
            self.fhrp_emit(idx, me, out);
        }
        self.fhrp_kick(ctx);
    }

    /// Whether interface `t` is up at the line level: not shut, and cabled to a live link (for a
    /// real port).
    fn line_up(&self, ctx: &Ctx, t: usize) -> bool {
        let Some(ifc) = self.ifaces.get(t) else {
            return false;
        };
        if ifc.shutdown || ifc.line_down {
            return false;
        }
        let phys = ifc
            .sub
            .map(|s| s.parent)
            .or(ifc.svi.is_none().then_some(PortId(t as u16)));
        phys.is_none_or(|p| ctx.port_up(p))
    }

    /// Send what group `idx` produced, and its gratuitous ARP if it has just taken over.
    fn fhrp_emit(&mut self, idx: usize, me: Ipv4Addr, out: Vec<FhrpMsg>) {
        let i = self.fhrp[idx].iface;
        let g = self.fhrp[idx].g.clone();
        for m in out {
            self.fhrp_send(i, &g, me, m);
        }
        if self.fhrp[idx].g.take_garp() {
            let vmac = g.vmac();
            let arp = ArpPacket {
                op: ArpOp::Reply,
                sender_mac: vmac,
                sender_ip: g.vip,
                target_mac: MacAddr::BROADCAST,
                target_ip: g.vip,
            };
            self.emit_from(
                PortId(i as u16),
                Some(vmac),
                MacAddr::BROADCAST,
                EtherType::ARP,
                arp.emit(),
            );
        }
    }

    fn fhrp_send(&mut self, i: usize, g: &FhrpGroup, me: Ipv4Addr, m: FhrpMsg) {
        if me == Ipv4Addr::UNSPECIFIED {
            return;
        }
        let (dst_ip, dst_mac, proto, payload, src_mac) = match g.proto {
            FhrpProto::Hsrp => {
                let h = wire::Hsrp {
                    op: match m.kind {
                        FhrpKind::Hello => wire::HsrpOp::Hello,
                        FhrpKind::Coup => wire::HsrpOp::Coup,
                        FhrpKind::Resign => wire::HsrpOp::Resign,
                    },
                    state: state_code(m.state),
                    hello: m.hello as u8,
                    hold: m.hold as u8,
                    priority: m.priority,
                    group: g.group,
                    vip: g.vip,
                };
                let udp = UdpDatagram {
                    src_port: wire::HSRP_PORT,
                    dst_port: wire::HSRP_PORT,
                    payload: h.emit(),
                }
                .emit(me, wire::HSRP_GROUP);
                // The active router speaks with the virtual address so switches learn where it is.
                let src = (m.state == FhrpState::Active).then(|| g.vmac());
                (wire::HSRP_GROUP, wire::HSRP_MAC, IpProto::UDP, udp, src)
            }
            FhrpProto::Vrrp => {
                let v = wire::Vrrp {
                    group: g.group,
                    priority: if m.kind == FhrpKind::Resign {
                        0
                    } else {
                        m.priority
                    },
                    advert: m.hello as u8,
                    ips: vec![g.vip],
                };
                (
                    wire::VRRP_GROUP,
                    wire::VRRP_MAC,
                    IpProto(wire::VRRP_PROTO),
                    v.emit(),
                    Some(g.vmac()),
                )
            }
        };
        let pkt = Ipv4Packet {
            dscp_ecn: 0xc0,
            ident: self.next_ident(),
            flags_frag: 0,
            ttl: if g.proto == FhrpProto::Vrrp { 255 } else { 1 },
            proto,
            src: me,
            dst: dst_ip,
            payload,
        };
        self.emit_from(
            PortId(i as u16),
            src_mac,
            dst_mac,
            EtherType::IPV4,
            pkt.emit(),
        );
    }

    fn on_hsrp(&mut self, ctx: &mut Ctx, ingress: PortId, p: &Ipv4Packet) {
        let Ok(u) = UdpDatagram::parse(&p.payload) else {
            return;
        };
        if u.dst_port != wire::HSRP_PORT {
            return;
        }
        let Ok(h) = wire::Hsrp::parse(&u.payload) else {
            return;
        };
        let msg = FhrpMsg {
            kind: match h.op {
                wire::HsrpOp::Hello => FhrpKind::Hello,
                wire::HsrpOp::Coup => FhrpKind::Coup,
                wire::HsrpOp::Resign => FhrpKind::Resign,
            },
            state: wire_state(h.state),
            priority: h.priority,
            hello: u32::from(h.hello),
            hold: u32::from(h.hold),
        };
        self.fhrp_receive(ctx, ingress, FhrpProto::Hsrp, h.group, p.src, msg);
    }

    fn on_vrrp(&mut self, ctx: &mut Ctx, ingress: PortId, p: &Ipv4Packet) {
        let Ok(v) = wire::Vrrp::parse(&p.payload) else {
            return;
        };
        // RFC 3768: only packets with TTL 255 are accepted, so they cannot come from afar.
        if p.ttl != 255 {
            return;
        }
        let msg = FhrpMsg {
            kind: if v.priority == 0 {
                FhrpKind::Resign
            } else {
                FhrpKind::Hello
            },
            state: FhrpState::Master,
            priority: v.priority,
            hello: u32::from(v.advert),
            hold: 3 * u32::from(v.advert),
        };
        self.fhrp_receive(ctx, ingress, FhrpProto::Vrrp, v.group, p.src, msg);
    }

    fn fhrp_receive(
        &mut self,
        ctx: &mut Ctx,
        ingress: PortId,
        proto: FhrpProto,
        group: u8,
        from: Ipv4Addr,
        msg: FhrpMsg,
    ) {
        let i = ingress.0 as usize;
        let Some(idx) = self
            .fhrp
            .iter()
            .position(|e| e.iface == i && e.g.proto == proto && e.g.group == group)
        else {
            return;
        };
        let Some((me, _)) = self.live(i) else { return };
        if from == me {
            return;
        }
        let vip = self.fhrp[idx].g.vip;
        let mut out = vec![];
        self.fhrp[idx]
            .g
            .receive(ctx.now(), me, from, vip == me, msg, &mut out);
        self.fhrp_emit(idx, me, out);
    }
}

impl Router {
    /// A frame that arrived for logical interface `i`. Wrong address or interface down: ignored.
    /// Anything the router wants to send is queued; collect it with [`Router::take_outbox`].
    pub fn receive(&mut self, ctx: &mut Ctx, i: usize, f: EthernetFrame) {
        if self.live(i).is_none() {
            return;
        }
        if !self.accepts_mac(i, f.dst) {
            return;
        }
        self.rx_src_mac = f.src;
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
        if token == FHRP_TICK {
            self.fhrp_timer = false;
            self.fhrp_tick(ctx);
            return;
        }
        if token == TCP_TICK {
            self.tcp_armed = None;
            let mut out = vec![];
            self.tcp.tick(ctx.now(), &mut out);
            self.tcp_emit(ctx, out);
            return;
        }
        if token == LOG_TICK {
            self.log_timer = false;
            self.log_tick(ctx);
            return;
        }
        if token == NTP_TICK {
            self.ntp_timer = false;
            self.ntp_tick(ctx);
            return;
        }
        if token == DISC_TICK {
            self.disc_timer = false;
            self.disc_tick(ctx);
            self.disc_kick(ctx);
            return;
        }
        if token == OSPF_TICK {
            self.ospf_tick(ctx);
            return;
        }
        if self.arp.timeout(Ipv4Addr::from_u32(token as u32)) {
            ctx.note("arp timeout");
        }
    }
}

impl Device for Router {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        match ev {
            DeviceEvent::FrameRx { port, frame } => {
                if self.disc.consumes(&frame) {
                    self.disc.receive(ctx.now(), port.0 as usize, &frame);
                    return;
                }
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

    fn start(&mut self, ctx: &mut Ctx) {
        self.disc_kick(ctx);
    }

    fn clone_box(&self) -> Option<Box<dyn Device>> {
        Some(Box::new(self.clone()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---- OSPF glue ----

impl Router {
    /// The interfaces OSPF may use: up and addressed.
    fn ospf_views(&self) -> Vec<OspfIfView> {
        (0..self.ifaces.len())
            .filter_map(|i| {
                self.live(i).map(|(addr, len)| OspfIfView {
                    index: i,
                    addr,
                    len,
                    mac: self.ifaces[i].mac,
                    cfg: self.ifaces[i].ospf,
                })
            })
            .collect()
    }

    /// OSPF was reconfigured: have it originate afresh and take in whatever that did to the
    /// routes.
    pub fn ospf_config_changed(&mut self) {
        self.ospf_sync_default();
        if let Some(o) = &mut self.ospf {
            o.config_changed();
        }
        if self.ospf.as_mut().is_some_and(|o| o.take_routes_changed()) {
            self.rebuild_fib();
        }
    }

    /// `no router ospf`: forget the protocol and every route it taught.
    pub fn remove_ospf(&mut self) {
        self.ospf = None;
        self.rebuild_fib();
    }

    /// Change one interface's OSPF settings.
    pub fn set_ospf_cfg(&mut self, i: usize, f: impl FnOnce(&mut OspfIfCfg)) {
        f(&mut self.ifaces[i].ospf);
    }

    /// The timestamp of a log line: `*Mar  1 00:00:05.123`, with the same flag `show clock` puts
    /// in front (`*` clock not trustworthy, `.` time source lost).
    pub fn log_stamp(&self, now: SimTime) -> String {
        let (_, mo, d, h, mi, s, _) = civil_from_unix(self.clock.local_secs(now));
        let ms = self.clock.now_ns(now).rem_euclid(1_000_000_000) / 1_000_000;
        let trusted = self.clock.set_by_hand || self.clock.ever_synced || self.ntp.master.is_some();
        let flag = if !trusted {
            "*"
        } else if self.ntp.wants_tick() && self.ntp.state().is_none() {
            "."
        } else {
            ""
        };
        format!(
            "{flag}{} {d:>2} {h:02}:{mi:02}:{s:02}.{ms:03}",
            MONTHS[mo - 1]
        )
    }

    /// Log a message (`tag` like `LINK-3-UPDOWN`) and send it to the syslog hosts.
    pub fn syslog(&mut self, ctx: &mut Ctx, sev: u8, tag: &str, text: &str) {
        let stamp = self.log_stamp(ctx.now());
        for (host, payload) in self.log.log(&stamp, sev, tag, text) {
            if let Some(src) = self.source_for(host) {
                self.send_udp(
                    ctx,
                    src,
                    host,
                    wire::SYSLOG_PORT,
                    wire::SYSLOG_PORT,
                    &payload,
                );
            }
        }
    }

    /// The name an interface goes by in messages.
    pub fn if_long_name(&self, i: usize) -> String {
        let ifc = &self.ifaces[i];
        match (ifc.svi, ifc.sub) {
            (Some(v), _) => format!("Vlan{v}"),
            (None, Some(sub)) => format!("GigabitEthernet0/{}.{}", sub.parent.0, sub.number),
            _ => format!("GigabitEthernet0/{i}"),
        }
    }

    /// Start watching interfaces for the log if somebody will read it and it is not running.
    pub fn log_kick(&mut self, ctx: &mut Ctx) {
        if self.log.wants_tick() && !self.log_timer {
            self.log_timer = true;
            ctx.schedule_background(SimTime::from_secs(1), LOG_TICK);
        }
    }

    /// Compare each interface with how it was last seen and log what changed.
    fn log_tick(&mut self, ctx: &mut Ctx) {
        if !self.log.wants_tick() {
            return;
        }
        for i in 0..self.ifaces.len() {
            let name = self.if_long_name(i);
            let now = LineSeen {
                admin_down: self.ifaces[i].shutdown,
                up: self.line_up(ctx, i),
            };
            self.log_watch(ctx, name, now, self.ifaces[i].svi.is_none());
        }
        self.log_kick(ctx);
    }

    /// One interface's state as of now. A change is logged: `%LINK` for a physical port (down by
    /// `shutdown` reads "administratively down") and `%LINEPROTO` for every kind. The first sight
    /// of an interface logs nothing.
    pub(crate) fn log_watch(&mut self, ctx: &mut Ctx, name: String, now: LineSeen, physical: bool) {
        let Some(before) = self.log.seen.insert(name.clone(), now) else {
            return;
        };
        if before == now {
            return;
        }
        let st = if now.up { "up" } else { "down" };
        if physical {
            if now.admin_down {
                let text = format!("Interface {name}, changed state to administratively down");
                self.syslog(ctx, NOTIFICATIONS, "LINK-5-CHANGED", &text);
            } else {
                let text = format!("Interface {name}, changed state to {st}");
                self.syslog(ctx, ERRORS, "LINK-3-UPDOWN", &text);
            }
        }
        if before.up != now.up {
            let text = format!("Line protocol on Interface {name}, changed state to {st}");
            self.syslog(ctx, NOTIFICATIONS, "LINEPROTO-5-UPDOWN", &text);
        }
    }

    /// Start NTP polling if servers are configured and it is not running.
    pub fn ntp_kick(&mut self, ctx: &mut Ctx) {
        if self.ntp.wants_tick() && !self.ntp_timer {
            self.ntp_timer = true;
            ctx.schedule_background(SimTime::from_secs(1), NTP_TICK);
        }
    }

    fn ntp_tick(&mut self, ctx: &mut Ctx) {
        if !self.ntp.wants_tick() {
            return;
        }
        let mut out = vec![];
        self.ntp.tick(ctx.now(), &self.clock, &mut out);
        for (server, payload) in out {
            if let Some(src) = self.source_for(server) {
                self.send_udp(ctx, src, server, wire::NTP_PORT, wire::NTP_PORT, &payload);
            }
        }
        self.ntp_kick(ctx);
    }

    /// A UDP packet for this router: NTP (port 123), answered or taken in. True if it was NTP.
    fn on_ntp(&mut self, ctx: &mut Ctx, p: &Ipv4Packet) -> bool {
        let Ok(u) = UdpDatagram::parse(&p.payload) else {
            return false;
        };
        if u.dst_port != wire::NTP_PORT {
            return false;
        }
        if let Ok(pkt) = wire::NtpPacket::parse(&u.payload) {
            if let Some(reply) = self.ntp.on_packet(ctx.now(), &mut self.clock, p.src, &pkt) {
                self.send_udp(ctx, p.dst, p.src, wire::NTP_PORT, u.src_port, &reply);
            }
        }
        true
    }

    /// Start the one-second neighbour discovery upkeep if it is wanted and not running.
    pub fn disc_kick(&mut self, ctx: &mut Ctx) {
        if self.disc.wants_tick() && !self.disc_timer {
            self.disc_timer = true;
            ctx.schedule_background(SimTime::from_secs(1), DISC_TICK);
        }
    }

    fn disc_tick(&mut self, ctx: &mut Ctx) {
        if !self.disc.wants_tick() {
            return;
        }
        let id = Identity {
            name: self.name.clone(),
            platform: "wirebench router".to_string(),
            software: "wirebench simulated router".to_string(),
            caps: wire::cdp_cap::ROUTER,
            ports: (0..self.n_ports as usize)
                .map(|i| DiscPort {
                    long: format!("GigabitEthernet0/{i}"),
                    short: format!("Gi0/{i}"),
                    mac: self.ifaces[i].mac,
                    live: ctx.port_up(PortId(i as u16)) && !self.ifaces[i].shutdown,
                    addr: self.ifaces[i].addr.map(|a| a.0),
                })
                .collect(),
        };
        let mut out = vec![];
        self.disc.tick(ctx.now(), &id, &mut out);
        for (p, frame) in out {
            ctx.transmit(PortId(p as u16), frame);
        }
    }

    /// Start OSPF's one-second upkeep if the protocol is configured and it is not running.
    pub fn ospf_kick(&mut self, ctx: &mut Ctx) {
        if self.ospf.is_some() && !self.ospf_timer {
            self.ospf_timer = true;
            ctx.schedule_background(SimTime::from_secs(1), OSPF_TICK);
        }
    }

    /// Tell OSPF whether we have a default route that does not come from OSPF itself.
    fn ospf_sync_default(&mut self) {
        let present = self.fib.routes().any(|r| r.len == 0 && !r.kind.is_ospf());
        if let Some(o) = &mut self.ospf {
            o.set_default_route(present);
        }
    }

    fn ospf_tick(&mut self, ctx: &mut Ctx) {
        self.ospf_timer = false;
        self.ospf_sync_default();
        let views = self.ospf_views();
        let mut out = Vec::new();
        if let Some(o) = &mut self.ospf {
            o.tick(ctx.now(), &views, &mut out);
        }
        self.ospf_after(out);
        self.ospf_kick(ctx);
    }

    fn ospf_receive(&mut self, ctx: &mut Ctx, ingress: PortId, p: &Ipv4Packet) {
        let Ok(pkt) = OspfPacket::parse(&p.payload) else {
            return;
        };
        let i = ingress.0 as usize;
        self.ospf_sync_default();
        let views = self.ospf_views();
        let mac = self.rx_src_mac;
        let mut out = Vec::new();
        if let Some(o) = &mut self.ospf {
            // Packets to 224.0.0.6 are for the designated router and its backup only.
            if p.dst == ALL_DR_ROUTERS && !o.is_designated(i) {
                return;
            }
            o.receive(
                ctx.now(),
                &views,
                OspfSource {
                    iface: i,
                    ip: p.src,
                    mac,
                },
                pkt,
                &mut out,
            );
        }
        self.ospf_after(out);
    }

    /// Put the packets the protocol produced on the wire and take in any new routes.
    fn ospf_after(&mut self, out: Vec<OspfTx>) {
        for tx in out {
            let Some((src, _)) = self.live(tx.iface) else {
                continue;
            };
            let pkt = Ipv4Packet {
                dscp_ecn: 0xc0,
                ident: self.next_ident(),
                flags_frag: 0,
                ttl: 1,
                proto: IpProto::OSPF,
                src,
                dst: tx.dst,
                payload: tx.packet.emit(),
            };
            self.emit(
                PortId(tx.iface as u16),
                tx.dst_mac,
                EtherType::IPV4,
                pkt.emit(),
            );
        }
        if self.ospf.as_mut().is_some_and(|o| o.take_routes_changed()) {
            self.rebuild_fib();
        }
    }
}
