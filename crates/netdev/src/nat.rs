//! Network address translation: static NAT, dynamic NAT from a pool, and PAT (overload), for
//! traffic crossing a router from an `ip nat inside` interface to an `ip nat outside` one.
//!
//! This holds the state (rules, pools, the translation table) and rewrites packets. When to
//! translate, and which access lists permit it, is the router's business.
//!
//! Simplifications, written down in `provenance/nat.md`: a mapping is keyed by the inside host's
//! address and port (and protocol) alone, so one inside socket keeps one global port whatever it
//! talks to (endpoint-independent mapping); only TCP, UDP and ICMP echo are translated; ICMP
//! errors are translated on their way in (so traceroute through PAT works) but not on their way out;
//! addresses inside application data (FTP and the like) are not touched.

use kernel::SimTime;
use wire::{
    internet_checksum, IcmpEcho, IcmpError, IpProto, Ipv4Addr, Ipv4Packet, TcpSegment, UdpDatagram,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Inside,
    Outside,
}

pub const ICMP_TIMEOUT: SimTime = SimTime::from_secs(60);
pub const UDP_TIMEOUT: SimTime = SimTime::from_secs(300);
pub const TCP_TIMEOUT: SimTime = SimTime::from_secs(86_400);
/// After a FIN or RST has crossed.
pub const TCP_FINRST_TIMEOUT: SimTime = SimTime::from_secs(60);
/// Dynamic one-to-one entries (no ports): `ip nat translation timeout`'s default of a day.
pub const SIMPLE_TIMEOUT: SimTime = SimTime::from_secs(86_400);

const ICMP: u8 = 1;
const TCP: u8 = 6;
const UDP: u8 = 17;

/// `ip nat pool`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub name: String,
    pub start: Ipv4Addr,
    pub end: Ipv4Addr,
    pub prefix_len: u8,
}

impl Pool {
    pub fn addrs(&self) -> Vec<Ipv4Addr> {
        (self.start.to_u32()..=self.end.to_u32())
            .map(Ipv4Addr::from_u32)
            .collect()
    }
}

/// Where a dynamic rule takes its global addresses from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pool(String),
    /// The address of this interface (index into the router's interfaces).
    Interface(usize),
}

/// `ip nat inside source list <acl> pool|interface ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub acl: String,
    pub target: Target,
    pub overload: bool,
}

/// `ip nat inside source static ...`. `proto` 0 means every protocol and `port` 0 means no port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Static {
    pub proto: u8,
    pub local: (Ipv4Addr, u16),
    pub global: (Ipv4Addr, u16),
}

/// One translation in the table.
#[derive(Clone, Debug)]
pub struct Entry {
    /// 0 for a one-to-one entry that covers every protocol.
    pub proto: u8,
    pub local: (Ipv4Addr, u16),
    pub global: (Ipv4Addr, u16),
    /// The far end of the conversation that made it (for `show`), if it has one.
    pub remote: Option<(Ipv4Addr, u16)>,
    /// The rule's access list, to drop the entry with the rule.
    pub acl: String,
    pub expires: SimTime,
    /// A FIN or RST has crossed: whatever follows only keeps it for [`TCP_FINRST_TIMEOUT`].
    pub closing: bool,
}

impl Entry {
    /// A packet used this entry: push its expiry out, or bring it close if the connection ends.
    fn touch(&mut self, now: SimTime, proto: u8, flags: u8, t: &Timeouts) {
        if proto == TCP && flags & (TcpSegment::FIN | TcpSegment::RST) != 0 {
            self.closing = true;
        }
        self.expires = now
            + if self.closing {
                t.finrst
            } else {
                t.of(self.proto)
            };
    }
}

/// The five things NAT looks at in a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flow {
    pub proto: u8,
    pub src: (Ipv4Addr, u16),
    pub dst: (Ipv4Addr, u16),
    pub flags: u8,
}

impl Flow {
    /// None for anything NAT does not translate (other protocols, later fragments, ICMP that is
    /// not an echo).
    pub fn of(p: &Ipv4Packet) -> Option<Flow> {
        if p.flags_frag & 0x1fff != 0 {
            return None;
        }
        match p.proto {
            IpProto::TCP => {
                let s = TcpSegment::parse(&p.payload).ok()?;
                Some(Flow {
                    proto: TCP,
                    src: (p.src, s.src_port),
                    dst: (p.dst, s.dst_port),
                    flags: s.flags,
                })
            }
            IpProto::UDP => {
                let u = UdpDatagram::parse(&p.payload).ok()?;
                Some(Flow {
                    proto: UDP,
                    src: (p.src, u.src_port),
                    dst: (p.dst, u.dst_port),
                    flags: 0,
                })
            }
            IpProto::ICMP => {
                let e = IcmpEcho::parse(&p.payload).ok()?;
                Some(Flow {
                    proto: ICMP,
                    src: (p.src, e.ident),
                    dst: (p.dst, e.ident),
                    flags: 0,
                })
            }
            _ => None,
        }
    }
}

/// Counters for `show ip nat statistics`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub expired: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Nat {
    pub pools: Vec<Pool>,
    pub rules: Vec<Rule>,
    pub statics: Vec<Static>,
    /// `ip nat outside source static <global> <local>`: (outside global, outside local).
    pub outside: Vec<(Ipv4Addr, Ipv4Addr)>,
    pub timeouts: Timeouts,
    table: Vec<Entry>,
    pub stats: Stats,
}

/// `ip nat translation ... timeout`: how long an idle translation is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub icmp: SimTime,
    pub udp: SimTime,
    pub tcp: SimTime,
    pub finrst: SimTime,
    /// One-to-one dynamic entries, which cover every protocol.
    pub simple: SimTime,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            icmp: ICMP_TIMEOUT,
            udp: UDP_TIMEOUT,
            tcp: TCP_TIMEOUT,
            finrst: TCP_FINRST_TIMEOUT,
            simple: SIMPLE_TIMEOUT,
        }
    }
}

impl Timeouts {
    fn of(&self, proto: u8) -> SimTime {
        match proto {
            ICMP => self.icmp,
            UDP => self.udp,
            TCP => self.tcp,
            _ => self.simple,
        }
    }
}

fn first_port(wanted: u16) -> u16 {
    match wanted {
        0..=511 => 1,
        512..=1023 => 512,
        _ => 1024,
    }
}

fn last_port(wanted: u16) -> u16 {
    match wanted {
        0..=511 => 511,
        512..=1023 => 1023,
        _ => 65_535,
    }
}

impl Nat {
    /// Whether anything is configured; the router skips NAT entirely when not.
    pub fn is_configured(&self) -> bool {
        !self.rules.is_empty() || !self.statics.is_empty() || !self.outside.is_empty()
    }

    pub fn pool(&self, name: &str) -> Option<&Pool> {
        self.pools.iter().find(|p| p.name == name)
    }

    /// Whether a pool is named by a rule.
    pub fn pool_in_use(&self, name: &str) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(&r.target, Target::Pool(n) if n == name))
    }

    /// Whether `ip` is a global address some translation uses: the router answers ARP for it.
    pub fn is_global(&self, ip: Ipv4Addr) -> bool {
        self.statics.iter().any(|s| s.global.0 == ip) || self.table.iter().any(|e| e.global.0 == ip)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.table
    }

    /// Forget dynamic entries (all of them, or those made by `acl`'s rules).
    pub fn clear_dynamic(&mut self, acl: Option<&str>) -> usize {
        let before = self.table.len();
        self.table.retain(|e| acl.is_some_and(|a| e.acl != a));
        before - self.table.len()
    }

    pub fn expire(&mut self, now: SimTime) {
        let before = self.table.len();
        self.table.retain(|e| e.expires > now);
        self.stats.expired += (before - self.table.len()) as u64;
    }

    fn global_taken(&self, proto: u8, global: (Ipv4Addr, u16)) -> bool {
        self.table
            .iter()
            .any(|e| e.proto == proto && e.global == global)
            || self.statics.iter().any(|s| {
                s.global.0 == global.0
                    && (s.proto == 0 || s.proto == proto)
                    && (s.global.1 == 0 || s.global.1 == global.1)
            })
    }

    /// The global address and port for a packet from inside, if a translation already exists
    /// (dynamic, then static). Refreshes its timer.
    pub fn find_out(&mut self, now: SimTime, f: &Flow) -> Option<(Ipv4Addr, u16)> {
        self.expire(now);
        let timeouts = self.timeouts;
        let hit = self.table.iter_mut().find(|e| {
            (e.proto == f.proto && e.local == f.src) || (e.proto == 0 && e.local.0 == f.src.0)
        });
        if let Some(e) = hit {
            e.remote = Some(f.dst);
            e.touch(now, f.proto, f.flags, &timeouts);
            return Some(e.global);
        }
        self.statics
            .iter()
            .find(|s| {
                s.local.0 == f.src.0
                    && (s.proto == 0 || s.proto == f.proto)
                    && (s.local.1 == 0 || s.local.1 == f.src.1)
            })
            .map(|s| s.global)
    }

    /// Make a translation for a packet from inside that has none. `addrs` are the global
    /// addresses the rule can use. None when there is nothing free.
    pub fn create(
        &mut self,
        now: SimTime,
        f: &Flow,
        acl: &str,
        addrs: &[Ipv4Addr],
        overload: bool,
    ) -> Option<(Ipv4Addr, u16)> {
        let (proto, local, global) = if overload {
            let wanted = f.src.1;
            let mut found = None;
            'addrs: for &a in addrs {
                if !self.global_taken(f.proto, (a, wanted)) {
                    found = Some((a, wanted));
                    break;
                }
                for p in first_port(wanted)..=last_port(wanted) {
                    if !self.global_taken(f.proto, (a, p)) {
                        found = Some((a, p));
                        break 'addrs;
                    }
                }
            }
            (f.proto, f.src, found?)
        } else {
            // One address per inside host, for every protocol.
            let a = addrs
                .iter()
                .copied()
                .find(|a| !self.global_taken(0, (*a, 0)))?;
            (0, (f.src.0, 0), (a, 0))
        };
        self.table.push(Entry {
            proto,
            local,
            global,
            remote: Some(f.dst),
            acl: acl.to_string(),
            expires: now + self.timeouts.of(proto),
            closing: false,
        });
        Some(global)
    }

    /// Rewrite a packet from inside to carry `global` as its source.
    pub fn apply_out(p: &Ipv4Packet, f: &Flow, global: (Ipv4Addr, u16)) -> Option<Ipv4Packet> {
        let port = if global.1 == 0 { f.src.1 } else { global.1 };
        rewrite(p, (global.0, port), f.dst)
    }

    /// A packet from outside: if its source is an outside global address the source becomes the
    /// outside local one, and if it is addressed to a translated address the destination is put
    /// back to the inside host. None if neither applies.
    pub fn inbound(&mut self, now: SimTime, p: &Ipv4Packet) -> Option<Ipv4Packet> {
        self.expire(now);
        let mut cur = p.clone();
        let mut changed = false;
        if let Some(&(_, local)) = self.outside.iter().find(|(g, _)| *g == cur.src) {
            cur = with_addrs(&cur, local, cur.dst);
            changed = true;
        }
        if cur.proto == IpProto::ICMP {
            if let Some(q) = self.inbound_error(now, &cur) {
                return Some(q);
            }
        }
        if let Some(f) = Flow::of(&cur) {
            if let Some(local) = self.find_in(now, &f) {
                self.stats.hits += 1;
                let port = if local.1 == 0 { f.dst.1 } else { local.1 };
                return rewrite(&cur, f.src, (local.0, port));
            }
        }
        changed.then_some(cur)
    }

    /// A packet from inside to an outside local address: its destination becomes the outside
    /// global one.
    pub fn outside_dst(&self, p: &Ipv4Packet) -> Option<Ipv4Packet> {
        let &(global, _) = self.outside.iter().find(|(_, l)| *l == p.dst)?;
        Some(with_addrs(p, p.src, global))
    }

    /// An ICMP error an inside host sends to the outside, about a packet that reached it through a
    /// translation: it leaves from the global address, and the quoted packet shows the global
    /// address too.
    pub fn outbound_error(&mut self, now: SimTime, p: &Ipv4Packet) -> Option<Ipv4Packet> {
        let e = IcmpError::parse(&p.payload).ok()?;
        let o = &e.original;
        if o.len() < 28 || o[0] >> 4 != 4 {
            return None;
        }
        let ihl = usize::from(o[0] & 15) * 4;
        if ihl < 20 || o.len() < ihl + 8 {
            return None;
        }
        let proto = o[9];
        let remote = Ipv4Addr([o[12], o[13], o[14], o[15]]);
        let local = Ipv4Addr([o[16], o[17], o[18], o[19]]);
        // The quoted packet was sent to the inside host: its destination port is the inside one
        // (for an echo, the identifier, which sits where a source port would).
        let (rport_at, lport_at) = if proto == ICMP {
            (ihl + 4, ihl + 4)
        } else {
            (ihl, ihl + 2)
        };
        let rport = u16::from_be_bytes([o[rport_at], o[rport_at + 1]]);
        let lport = u16::from_be_bytes([o[lport_at], o[lport_at + 1]]);
        let f = Flow {
            proto,
            src: (local, lport),
            dst: (remote, rport),
            flags: 0,
        };
        let global = self.find_out(now, &f)?;
        self.stats.hits += 1;
        let mut o = e.original.clone();
        o[16..20].copy_from_slice(&global.0 .0);
        if global.1 != 0 {
            o[lport_at..lport_at + 2].copy_from_slice(&global.1.to_be_bytes());
        }
        o[10] = 0;
        o[11] = 0;
        let c = internet_checksum(&o[..ihl]);
        o[10..12].copy_from_slice(&c.to_be_bytes());
        let mut q = p.clone();
        q.src = global.0;
        q.payload = IcmpError {
            kind: e.kind,
            original: o,
        }
        .emit();
        Some(q)
    }

    fn find_in(&mut self, now: SimTime, f: &Flow) -> Option<(Ipv4Addr, u16)> {
        let timeouts = self.timeouts;
        let hit = self.table.iter_mut().find(|e| {
            (e.proto == f.proto && e.global == f.dst) || (e.proto == 0 && e.global.0 == f.dst.0)
        });
        if let Some(e) = hit {
            e.remote = Some(f.src);
            e.touch(now, f.proto, f.flags, &timeouts);
            return Some(e.local);
        }
        self.statics
            .iter()
            .find(|s| {
                s.global.0 == f.dst.0
                    && (s.proto == 0 || s.proto == f.proto)
                    && (s.global.1 == 0 || s.global.1 == f.dst.1)
            })
            .map(|s| s.local)
    }

    /// An ICMP error addressed to a global address, about a packet that left from it: send it on
    /// to the inside host, with the quoted packet showing the inside address too.
    fn inbound_error(&mut self, now: SimTime, p: &Ipv4Packet) -> Option<Ipv4Packet> {
        let e = IcmpError::parse(&p.payload).ok()?;
        let o = &e.original;
        if o.len() < 28 || o[0] >> 4 != 4 {
            return None;
        }
        let ihl = usize::from(o[0] & 15) * 4;
        if ihl < 20 || o.len() < ihl + 8 {
            return None;
        }
        let proto = o[9];
        let esrc = Ipv4Addr([o[12], o[13], o[14], o[15]]);
        let edst = Ipv4Addr([o[16], o[17], o[18], o[19]]);
        // For TCP and UDP the quoted ports; for an echo request the identifier.
        let at = if proto == ICMP { ihl + 4 } else { ihl };
        let sport = u16::from_be_bytes([o[at], o[at + 1]]);
        let f = Flow {
            proto,
            src: (edst, 0),
            dst: (esrc, sport),
            flags: 0,
        };
        let local = self.find_in(now, &f)?;
        self.stats.hits += 1;
        let mut o = e.original.clone();
        o[12..16].copy_from_slice(&local.0 .0);
        if local.1 != 0 {
            o[at..at + 2].copy_from_slice(&local.1.to_be_bytes());
        }
        o[10] = 0;
        o[11] = 0;
        let c = internet_checksum(&o[..ihl]);
        o[10..12].copy_from_slice(&c.to_be_bytes());
        let mut q = p.clone();
        q.dst = local.0;
        q.payload = IcmpError {
            kind: e.kind,
            original: o,
        }
        .emit();
        Some(q)
    }
}

/// `p` with new addresses only (ports as they were), for the protocols NAT knows.
fn with_addrs(p: &Ipv4Packet, src: Ipv4Addr, dst: Ipv4Addr) -> Ipv4Packet {
    if let Some(f) = Flow::of(p) {
        if let Some(q) = rewrite(p, (src, f.src.1), (dst, f.dst.1)) {
            return q;
        }
    }
    let mut q = p.clone();
    q.src = src;
    q.dst = dst;
    q
}

/// `p` with new addresses and ports in the IP header and the transport header, checksums redone.
fn rewrite(p: &Ipv4Packet, src: (Ipv4Addr, u16), dst: (Ipv4Addr, u16)) -> Option<Ipv4Packet> {
    let mut q = p.clone();
    q.src = src.0;
    q.dst = dst.0;
    q.payload = match p.proto {
        IpProto::TCP => {
            let mut s = TcpSegment::parse(&p.payload).ok()?;
            s.src_port = src.1;
            s.dst_port = dst.1;
            s.emit(src.0, dst.0)
        }
        IpProto::UDP => {
            let mut u = UdpDatagram::parse(&p.payload).ok()?;
            u.src_port = src.1;
            u.dst_port = dst.1;
            u.emit(src.0, dst.0)
        }
        IpProto::ICMP => {
            let mut e = IcmpEcho::parse(&p.payload).ok()?;
            // A request carries the identifier of its source, a reply the one it answers.
            e.ident = if e.reply { dst.1 } else { src.1 };
            e.emit()
        }
        _ => p.payload.clone(),
    };
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
        Ipv4Addr::new(a, b, c, d)
    }

    fn tcp(src: (Ipv4Addr, u16), dst: (Ipv4Addr, u16), flags: u8) -> Ipv4Packet {
        Ipv4Packet {
            dscp_ecn: 0,
            ident: 7,
            flags_frag: 0,
            ttl: 63,
            proto: IpProto::TCP,
            src: src.0,
            dst: dst.0,
            payload: TcpSegment {
                src_port: src.1,
                dst_port: dst.1,
                seq: 1,
                ack: 0,
                flags,
                window: 100,
                payload: b"hi".to_vec(),
            }
            .emit(src.0, dst.0),
        }
    }

    const NOW: SimTime = SimTime::from_secs(10);

    #[test]
    fn pat_keeps_the_source_port_when_free_and_picks_another_when_not() {
        let mut n = Nat::default();
        let global = ip(203, 0, 113, 1);
        let server = (ip(198, 51, 100, 5), 80);
        let a = Flow::of(&tcp((ip(10, 0, 0, 1), 4000), server, TcpSegment::SYN)).unwrap();
        let b = Flow::of(&tcp((ip(10, 0, 0, 2), 4000), server, TcpSegment::SYN)).unwrap();
        assert_eq!(
            n.create(NOW, &a, "1", &[global], true),
            Some((global, 4000))
        );
        let second = n.create(NOW, &b, "1", &[global], true).unwrap();
        assert_eq!(second.0, global);
        assert_ne!(second.1, 4000);
        assert!(second.1 >= 1024, "stays in the same port range");
        // The same inside socket keeps the same mapping.
        assert_eq!(n.find_out(NOW, &a), Some((global, 4000)));
    }

    #[test]
    fn a_translated_packet_has_valid_checksums_and_the_reply_comes_back() {
        let mut n = Nat::default();
        let (local, global, server) = (
            (ip(10, 0, 0, 1), 4000),
            ip(203, 0, 113, 1),
            (ip(198, 51, 100, 5), 80),
        );
        let out = tcp(local, server, TcpSegment::SYN);
        let f = Flow::of(&out).unwrap();
        let g = n.create(NOW, &f, "1", &[global], true).unwrap();
        let sent = Nat::apply_out(&out, &f, g).unwrap();
        assert_eq!(sent.src, global);
        assert!(TcpSegment::checksum_ok(&sent.payload, sent.src, sent.dst));
        // The server answers to the global address.
        let reply = tcp(server, g, TcpSegment::SYN | TcpSegment::ACK);
        let back = n.inbound(NOW, &reply).unwrap();
        assert_eq!((back.dst, back.src), (local.0, server.0));
        assert!(TcpSegment::checksum_ok(&back.payload, back.src, back.dst));
        assert_eq!(TcpSegment::parse(&back.payload).unwrap().dst_port, 4000);
        // Something to an address and port with no translation is left alone.
        assert!(n.inbound(NOW, &tcp(server, (global, 9999), 0)).is_none());
    }

    #[test]
    fn dynamic_pool_nat_gives_each_host_one_address_and_runs_out() {
        let mut n = Nat::default();
        let pool = [ip(203, 0, 113, 10), ip(203, 0, 113, 11)];
        let f =
            |h: u8| Flow::of(&tcp((ip(10, 0, 0, h), 5000), (ip(198, 51, 100, 5), 80), 0)).unwrap();
        assert_eq!(n.create(NOW, &f(1), "1", &pool, false), Some((pool[0], 0)));
        assert_eq!(n.create(NOW, &f(2), "1", &pool, false), Some((pool[1], 0)));
        assert_eq!(n.create(NOW, &f(3), "1", &pool, false), None);
        // Every protocol from host 1 uses its address; the port is untouched.
        assert_eq!(n.find_out(NOW, &f(1)), Some((pool[0], 0)));
        let p = tcp((ip(10, 0, 0, 1), 5000), (ip(198, 51, 100, 5), 80), 0);
        let q = Nat::apply_out(&p, &f(1), (pool[0], 0)).unwrap();
        assert_eq!(TcpSegment::parse(&q.payload).unwrap().src_port, 5000);
    }

    #[test]
    fn entries_expire_and_a_fin_shortens_the_wait() {
        let mut n = Nat::default();
        let g = ip(203, 0, 113, 1);
        let p = tcp((ip(10, 0, 0, 1), 4000), (ip(198, 51, 100, 5), 80), 0);
        let f = Flow::of(&p).unwrap();
        n.create(NOW, &f, "1", &[g], true);
        let fin = Flow {
            flags: TcpSegment::FIN,
            ..f
        };
        n.find_out(NOW, &fin);
        n.expire(NOW + SimTime::from_secs(59));
        assert_eq!(n.entries().len(), 1);
        n.expire(NOW + SimTime::from_secs(61));
        assert!(n.entries().is_empty());
        assert_eq!(n.stats.expired, 1);
    }

    #[test]
    fn timeouts_can_be_changed_and_a_closing_entry_keeps_the_short_one() {
        let mut n = Nat::default();
        n.timeouts.tcp = SimTime::from_secs(120);
        n.timeouts.finrst = SimTime::from_secs(5);
        let p = tcp((ip(10, 0, 0, 1), 4000), (ip(198, 51, 100, 5), 80), 0);
        let f = Flow::of(&p).unwrap();
        n.create(NOW, &f, "1", &[ip(203, 0, 113, 1)], true);
        n.expire(NOW + SimTime::from_secs(119));
        assert_eq!(n.entries().len(), 1);
        n.expire(NOW + SimTime::from_secs(121));
        assert!(n.entries().is_empty());
        n.create(NOW, &f, "1", &[ip(203, 0, 113, 1)], true);
        let fin = Flow {
            flags: TcpSegment::RST,
            ..f
        };
        n.find_out(NOW, &fin);
        // Later traffic does not stretch a closing entry back to the long timeout.
        n.find_out(NOW + SimTime::from_secs(3), &f);
        n.expire(NOW + SimTime::from_secs(9));
        assert!(n.entries().is_empty());
    }

    #[test]
    fn an_error_from_an_inside_host_leaves_from_its_global_address() {
        let mut n = Nat::default();
        n.statics.push(Static {
            proto: UDP,
            local: (ip(10, 0, 0, 5), 5000),
            global: (ip(203, 0, 113, 1), 6000),
        });
        // A datagram from the internet arrived, was translated, and nothing listens: the inside
        // host answers with port unreachable, quoting the packet as it saw it.
        let seen = Ipv4Packet {
            dscp_ecn: 0,
            ident: 3,
            flags_frag: 0,
            ttl: 60,
            proto: IpProto::UDP,
            src: ip(198, 51, 100, 5),
            dst: ip(10, 0, 0, 5),
            payload: UdpDatagram {
                src_port: 3333,
                dst_port: 5000,
                payload: vec![1, 2, 3],
            }
            .emit(ip(198, 51, 100, 5), ip(10, 0, 0, 5)),
        };
        let err = Ipv4Packet {
            dscp_ecn: 0,
            ident: 4,
            flags_frag: 0,
            ttl: 64,
            proto: IpProto::ICMP,
            src: ip(10, 0, 0, 5),
            dst: ip(198, 51, 100, 5),
            payload: IcmpError {
                kind: wire::IcmpErrorKind::DestUnreachable(3),
                original: IcmpError::quote(&seen.emit()),
            }
            .emit(),
        };
        let out = n.outbound_error(NOW, &err).unwrap();
        assert_eq!(out.src, ip(203, 0, 113, 1));
        let e = IcmpError::parse(&out.payload).unwrap();
        assert_eq!(
            &e.original[16..20],
            &[203, 0, 113, 1],
            "quoted destination is the global one"
        );
        assert_eq!(u16::from_be_bytes([e.original[22], e.original[23]]), 6000);
        assert_eq!(&e.original[12..16], &[198, 51, 100, 5]);
    }

    #[test]
    fn outside_source_translates_the_far_hosts_address_for_the_inside() {
        let mut n = Nat::default();
        let (global, local) = (ip(198, 51, 100, 5), ip(10, 99, 0, 5));
        n.outside.push((global, local));
        // From the far host to an inside one: its source becomes the outside local address.
        let from_far = tcp((global, 80), (ip(10, 0, 0, 1), 4000), 0);
        let q = n.inbound(NOW, &from_far).unwrap();
        assert_eq!((q.src, q.dst), (local, ip(10, 0, 0, 1)));
        assert!(TcpSegment::checksum_ok(&q.payload, q.src, q.dst));
        // From inside to the outside local address: the destination becomes the global one.
        let to_far = tcp((ip(10, 0, 0, 1), 4000), (local, 80), 0);
        let q = n.outside_dst(&to_far).unwrap();
        assert_eq!((q.src, q.dst), (ip(10, 0, 0, 1), global));
        assert!(TcpSegment::checksum_ok(&q.payload, q.src, q.dst));
        // Other addresses are left alone.
        assert!(n
            .outside_dst(&tcp((ip(10, 0, 0, 1), 1), (ip(1, 1, 1, 1), 80), 0))
            .is_none());
    }

    #[test]
    fn static_nat_translates_both_ways_for_every_protocol() {
        let mut n = Nat::default();
        n.statics.push(Static {
            proto: 0,
            local: (ip(10, 0, 0, 5), 0),
            global: (ip(203, 0, 113, 5), 0),
        });
        let out = tcp((ip(10, 0, 0, 5), 80), (ip(198, 51, 100, 5), 3333), 0);
        let f = Flow::of(&out).unwrap();
        let g = n.find_out(NOW, &f).unwrap();
        assert_eq!(Nat::apply_out(&out, &f, g).unwrap().src, ip(203, 0, 113, 5));
        let inb = tcp((ip(198, 51, 100, 5), 3333), (ip(203, 0, 113, 5), 80), 0);
        assert_eq!(n.inbound(NOW, &inb).unwrap().dst, ip(10, 0, 0, 5));
    }

    #[test]
    fn a_port_forward_only_matches_its_port() {
        let mut n = Nat::default();
        n.statics.push(Static {
            proto: TCP,
            local: (ip(10, 0, 0, 5), 80),
            global: (ip(203, 0, 113, 1), 8080),
        });
        let to = |port| tcp((ip(198, 51, 100, 5), 3333), (ip(203, 0, 113, 1), port), 0);
        let q = n.inbound(NOW, &to(8080)).unwrap();
        assert_eq!(q.dst, ip(10, 0, 0, 5));
        assert_eq!(TcpSegment::parse(&q.payload).unwrap().dst_port, 80);
        assert!(n.inbound(NOW, &to(8081)).is_none());
    }

    #[test]
    fn echo_requests_are_translated_by_identifier() {
        let mut n = Nat::default();
        let g = ip(203, 0, 113, 1);
        let echo = |src, dst, reply, ident| Ipv4Packet {
            dscp_ecn: 0,
            ident: 1,
            flags_frag: 0,
            ttl: 60,
            proto: IpProto::ICMP,
            src,
            dst,
            payload: IcmpEcho {
                reply,
                ident,
                seq: 1,
                data: vec![1, 2],
            }
            .emit(),
        };
        let a = echo(ip(10, 0, 0, 1), ip(198, 51, 100, 5), false, 77);
        let b = echo(ip(10, 0, 0, 2), ip(198, 51, 100, 5), false, 77);
        let (fa, fb) = (Flow::of(&a).unwrap(), Flow::of(&b).unwrap());
        let ga = n.create(NOW, &fa, "1", &[g], true).unwrap();
        let gb = n.create(NOW, &fb, "1", &[g], true).unwrap();
        assert_ne!(
            ga.1, gb.1,
            "two hosts, same identifier: different global ones"
        );
        let sent = Nat::apply_out(&a, &fa, ga).unwrap();
        assert_eq!(IcmpEcho::parse(&sent.payload).unwrap().ident, ga.1);
        let reply = echo(ip(198, 51, 100, 5), g, true, gb.1);
        let back = n.inbound(NOW, &reply).unwrap();
        assert_eq!(back.dst, ip(10, 0, 0, 2));
        assert_eq!(IcmpEcho::parse(&back.payload).unwrap().ident, 77);
    }

    #[test]
    fn an_icmp_error_about_a_translated_packet_reaches_the_inside_host() {
        let mut n = Nat::default();
        let g = ip(203, 0, 113, 1);
        let p = tcp((ip(10, 0, 0, 1), 4000), (ip(198, 51, 100, 5), 80), 0);
        let f = Flow::of(&p).unwrap();
        let gl = n.create(NOW, &f, "1", &[g], true).unwrap();
        let sent = Nat::apply_out(&p, &f, gl).unwrap();
        // A router beyond answers "time exceeded", quoting what it saw.
        let err = IcmpError {
            kind: wire::IcmpErrorKind::TimeExceeded,
            original: IcmpError::quote(&sent.emit()),
        };
        let reply = Ipv4Packet {
            dscp_ecn: 0,
            ident: 2,
            flags_frag: 0,
            ttl: 60,
            proto: IpProto::ICMP,
            src: ip(198, 51, 100, 1),
            dst: g,
            payload: err.emit(),
        };
        let back = n.inbound(NOW, &reply).unwrap();
        assert_eq!(back.dst, ip(10, 0, 0, 1));
        let e = IcmpError::parse(&back.payload).unwrap();
        // The quoted packet shows the inside address again.
        assert_eq!(&e.original[12..16], &[10, 0, 0, 1]);
        assert_eq!(u16::from_be_bytes([e.original[20], e.original[21]]), 4000);
    }
}
