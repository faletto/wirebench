//! IP access lists: standard and extended, numbered and named. Pure data and matching, no
//! devices and no I/O. A router holds an [`AclSet`] and asks it for a verdict on each packet
//! (see `provenance/acls.md`).

use wire::{IpProto, Ipv4Addr, Ipv4Packet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AclKind {
    Standard,
    Extended,
}

/// An address and the wildcard bits that say which of its bits do not matter (a 1 is "any").
/// The address is kept with the ignored bits cleared, as IOS does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AddrSpec {
    pub addr: Ipv4Addr,
    pub wild: Ipv4Addr,
}

impl AddrSpec {
    pub const ANY: AddrSpec = AddrSpec {
        addr: Ipv4Addr([0; 4]),
        wild: Ipv4Addr([255; 4]),
    };

    pub fn host(a: Ipv4Addr) -> Self {
        AddrSpec {
            addr: a,
            wild: Ipv4Addr([0; 4]),
        }
    }

    pub fn new(addr: Ipv4Addr, wild: Ipv4Addr) -> Self {
        AddrSpec {
            addr: Ipv4Addr::from_u32(addr.to_u32() & !wild.to_u32()),
            wild,
        }
    }

    pub fn matches(&self, a: Ipv4Addr) -> bool {
        (a.to_u32() ^ self.addr.to_u32()) & !self.wild.to_u32() == 0
    }

    fn is_any(&self) -> bool {
        self.wild.to_u32() == u32::MAX
    }

    fn is_host(&self) -> bool {
        self.wild.to_u32() == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortSpec {
    Any,
    Eq(u16),
    Neq(u16),
    Lt(u16),
    Gt(u16),
    Range(u16, u16),
}

impl PortSpec {
    pub fn matches(&self, p: u16) -> bool {
        match *self {
            PortSpec::Any => true,
            PortSpec::Eq(x) => p == x,
            PortSpec::Neq(x) => p != x,
            PortSpec::Lt(x) => p < x,
            PortSpec::Gt(x) => p > x,
            PortSpec::Range(a, b) => a <= p && p <= b,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IcmpSpec {
    Any,
    Type(u8),
    TypeCode(u8, u8),
}

/// One permit or deny line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    pub permit: bool,
    /// `None` is `ip`: any protocol.
    pub proto: Option<u8>,
    pub src: AddrSpec,
    pub sport: PortSpec,
    pub dst: AddrSpec,
    pub dport: PortSpec,
    /// TCP segments with ACK or RST set.
    pub established: bool,
    pub icmp: IcmpSpec,
}

impl Rule {
    fn has_l4(&self) -> bool {
        self.sport != PortSpec::Any
            || self.dport != PortSpec::Any
            || self.established
            || self.icmp != IcmpSpec::Any
    }

    /// Whether this rule applies to `p`.
    pub fn matches(&self, p: &Packet) -> bool {
        if self.proto.is_some_and(|x| x != p.proto)
            || !self.src.matches(p.src)
            || !self.dst.matches(p.dst)
        {
            return false;
        }
        if p.later_fragment {
            // A later fragment has no ports or flags to look at. A permit line with such
            // conditions lets it through and a deny line does not catch it [verify].
            return !self.has_l4() || self.permit;
        }
        let port_ok = |spec: PortSpec, port: Option<u16>| match (spec, port) {
            (PortSpec::Any, _) => true,
            (s, Some(p)) => s.matches(p),
            (_, None) => false,
        };
        port_ok(self.sport, p.sport)
            && port_ok(self.dport, p.dport)
            && (!self.established || p.ack_or_rst)
            && match (self.icmp, p.icmp) {
                (IcmpSpec::Any, _) => true,
                (IcmpSpec::Type(t), Some((pt, _))) => t == pt,
                (IcmpSpec::TypeCode(t, c), Some((pt, pc))) => t == pt && c == pc,
                (_, None) => false,
            }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Body {
    Rule(Rule),
    Remark(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub seq: u32,
    pub body: Body,
    /// Packets this line decided.
    pub hits: u64,
}

/// What an access list looks at in a packet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Packet {
    pub proto: u8,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub sport: Option<u16>,
    pub dport: Option<u16>,
    pub ack_or_rst: bool,
    pub icmp: Option<(u8, u8)>,
    pub later_fragment: bool,
}

impl Packet {
    pub fn from_ipv4(p: &Ipv4Packet) -> Self {
        let b = &p.payload;
        let later_fragment = p.flags_frag & 0x1fff != 0;
        let ports =
            !later_fragment && b.len() >= 4 && matches!(p.proto, IpProto::TCP | IpProto::UDP);
        Packet {
            proto: p.proto.0,
            src: p.src,
            dst: p.dst,
            sport: ports.then(|| u16::from_be_bytes([b[0], b[1]])),
            dport: ports.then(|| u16::from_be_bytes([b[2], b[3]])),
            ack_or_rst: !later_fragment
                && p.proto == IpProto::TCP
                && b.len() > 13
                && b[13] & 0x14 != 0,
            icmp: (!later_fragment && p.proto == IpProto::ICMP && b.len() >= 2)
                .then(|| (b[0], b[1])),
            later_fragment,
        }
    }
}

/// Largest sequence number.
pub const MAX_SEQ: u32 = 2_147_483_647;
const SEQ_STEP: u32 = 10;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Acl {
    pub kind: AclKind,
    /// In sequence order.
    pub entries: Vec<Entry>,
}

/// Why a line could not be added.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddError {
    DuplicateSeq,
    SeqOverflow,
}

impl Acl {
    pub fn new(kind: AclKind) -> Self {
        Acl {
            kind,
            entries: Vec::new(),
        }
    }

    /// Add a line at `seq`, or ten after the last one. Returns its number.
    pub fn add(&mut self, seq: Option<u32>, body: Body) -> Result<u32, AddError> {
        let seq = match seq {
            Some(s) => s,
            None => match self.entries.last() {
                None => SEQ_STEP,
                Some(e) if e.seq > MAX_SEQ - SEQ_STEP => return Err(AddError::SeqOverflow),
                Some(e) => e.seq + SEQ_STEP,
            },
        };
        match self.entries.binary_search_by_key(&seq, |e| e.seq) {
            Ok(_) => Err(AddError::DuplicateSeq),
            Err(at) => {
                self.entries.insert(at, Entry { seq, body, hits: 0 });
                Ok(seq)
            }
        }
    }

    pub fn remove_seq(&mut self, seq: u32) -> bool {
        let n = self.entries.len();
        self.entries.retain(|e| e.seq != seq);
        self.entries.len() != n
    }

    /// Remove the first line with this content (`no permit ip any any`).
    pub fn remove_body(&mut self, body: &Body) -> bool {
        match self.entries.iter().position(|e| &e.body == body) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn clear_hits(&mut self) {
        for e in &mut self.entries {
            e.hits = 0;
        }
    }

    /// The verdict for `p` without counting it: the first matching line decides; no match is a
    /// deny.
    pub fn verdict(&self, p: &Packet) -> bool {
        self.entries
            .iter()
            .find_map(|e| match &e.body {
                Body::Rule(r) if r.matches(p) => Some(r.permit),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// The verdict for `p`, counting it against the line that decided it.
    pub fn check(&mut self, p: &Packet) -> bool {
        for e in &mut self.entries {
            if let Body::Rule(r) = &e.body {
                if r.matches(p) {
                    e.hits += 1;
                    return r.permit;
                }
            }
        }
        false
    }

    /// Whether every sequence number is what adding the lines in order would have given.
    fn default_numbering(&self) -> bool {
        self.entries
            .iter()
            .enumerate()
            .all(|(i, e)| e.seq == SEQ_STEP * (i as u32 + 1))
    }
}

/// A list of that name exists and is of the other kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KindClash;

/// All the access lists of a device, by name (a number is a name too), in the order created.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct AclSet {
    lists: Vec<(String, Acl)>,
}

impl AclSet {
    pub fn get(&self, name: &str) -> Option<&Acl> {
        self.lists.iter().find(|(n, _)| n == name).map(|(_, a)| a)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Acl> {
        self.lists
            .iter_mut()
            .find(|(n, _)| n == name)
            .map(|(_, a)| a)
    }

    /// The list called `name`, created as `kind` if there is none. `Err` if there is one of the
    /// other kind.
    pub fn open(&mut self, name: &str, kind: AclKind) -> Result<&mut Acl, KindClash> {
        if self.get(name).is_none() {
            self.lists.push((name.to_string(), Acl::new(kind)));
        }
        let acl = self.get_mut(name).expect("just made");
        if acl.kind == kind {
            Ok(acl)
        } else {
            Err(KindClash)
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.lists.len();
        self.lists.retain(|(x, _)| x != name);
        self.lists.len() != n
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Acl)> {
        self.lists.iter().map(|(n, a)| (n.as_str(), a))
    }

    pub fn clear_hits(&mut self, name: Option<&str>) {
        for (n, a) in &mut self.lists {
            if name.is_none_or(|x| x == n) {
                a.clear_hits();
            }
        }
    }
}

/// The kind a list number stands for: 1-99 and 1300-1999 standard, 100-199 and 2000-2699
/// extended.
pub fn kind_of_number(n: u32) -> Option<AclKind> {
    match n {
        1..=99 | 1300..=1999 => Some(AclKind::Standard),
        100..=199 | 2000..=2699 => Some(AclKind::Extended),
        _ => None,
    }
}

// ---- names ----

const PROTOCOLS: &[(&str, u8)] = &[
    ("icmp", 1),
    ("tcp", 6),
    ("udp", 17),
    ("gre", 47),
    ("eigrp", 88),
    ("ospf", 89),
];

const PORTS: &[(&str, u16)] = &[
    ("echo", 7),
    ("discard", 9),
    ("daytime", 13),
    ("ftp-data", 20),
    ("ftp", 21),
    ("telnet", 23),
    ("smtp", 25),
    ("time", 37),
    ("domain", 53),
    ("bootps", 67),
    ("bootpc", 68),
    ("tftp", 69),
    ("www", 80),
    ("pop3", 110),
    ("ntp", 123),
    ("snmp", 161),
    ("snmptrap", 162),
    ("bgp", 179),
    ("https", 443),
    ("syslog", 514),
];

const ICMP_NAMES: &[(&str, IcmpSpec)] = &[
    ("echo-reply", IcmpSpec::Type(0)),
    ("unreachable", IcmpSpec::Type(3)),
    ("net-unreachable", IcmpSpec::TypeCode(3, 0)),
    ("host-unreachable", IcmpSpec::TypeCode(3, 1)),
    ("protocol-unreachable", IcmpSpec::TypeCode(3, 2)),
    ("port-unreachable", IcmpSpec::TypeCode(3, 3)),
    ("administratively-prohibited", IcmpSpec::TypeCode(3, 13)),
    ("redirect", IcmpSpec::Type(5)),
    ("echo", IcmpSpec::Type(8)),
    ("time-exceeded", IcmpSpec::Type(11)),
];

// ---- parsing ----

/// A line that could not be read; the text is what the CLI shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError(pub String);

fn bad<T>(near: Option<&str>) -> Result<T, ParseError> {
    Err(ParseError(match near {
        Some(t) => format!("% Invalid input detected at or near '{t}'."),
        None => "% Incomplete command.".to_string(),
    }))
}

struct Tokens<'a> {
    toks: &'a [&'a str],
    at: usize,
}

impl<'a> Tokens<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.toks.get(self.at).copied()
    }
    fn next(&mut self) -> Option<&'a str> {
        let t = self.peek();
        self.at += 1;
        t
    }
    fn need(&mut self) -> Result<&'a str, ParseError> {
        self.next().ok_or(()).or_else(|_| bad(None))
    }
}

/// Dotted decimal, four numbers of 0 to 255.
fn dotted(t: &str) -> Option<Ipv4Addr> {
    let mut o = [0u8; 4];
    let mut parts = t.split('.');
    for b in &mut o {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *b = p.parse().ok()?;
    }
    parts.next().is_none().then_some(Ipv4Addr(o))
}

fn ip(t: &str) -> Result<Ipv4Addr, ParseError> {
    dotted(t).ok_or(()).or_else(|_| bad(Some(t)))
}

fn addr(it: &mut Tokens) -> Result<AddrSpec, ParseError> {
    let t = it.need()?;
    match t {
        "any" => Ok(AddrSpec::ANY),
        "host" => Ok(AddrSpec::host(ip(it.need()?)?)),
        _ => {
            let a = ip(t)?;
            // A wildcard may follow; without one the address stands for itself.
            match it.peek().and_then(dotted) {
                Some(w) => {
                    it.next();
                    Ok(AddrSpec::new(a, w))
                }
                None => Ok(AddrSpec::host(a)),
            }
        }
    }
}

fn port(t: &str) -> Result<u16, ParseError> {
    if let Some((_, n)) = PORTS.iter().find(|(name, _)| *name == t) {
        return Ok(*n);
    }
    t.parse::<u16>().or_else(|_| bad(Some(t)))
}

fn port_spec(it: &mut Tokens) -> Result<PortSpec, ParseError> {
    let Some(op) = it.peek() else {
        return Ok(PortSpec::Any);
    };
    let spec = match op {
        "eq" | "neq" | "lt" | "gt" => {
            it.next();
            let p = port(it.need()?)?;
            match op {
                "eq" => PortSpec::Eq(p),
                "neq" => PortSpec::Neq(p),
                "lt" => PortSpec::Lt(p),
                _ => PortSpec::Gt(p),
            }
        }
        "range" => {
            it.next();
            let (a, b) = (port(it.need()?)?, port(it.need()?)?);
            if a > b {
                return Err(ParseError(
                    "% The second port of a range may not be below the first.".to_string(),
                ));
            }
            PortSpec::Range(a, b)
        }
        _ => PortSpec::Any,
    };
    Ok(spec)
}

fn icmp_spec(it: &mut Tokens) -> Result<IcmpSpec, ParseError> {
    let Some(t) = it.peek() else {
        return Ok(IcmpSpec::Any);
    };
    if let Some((_, s)) = ICMP_NAMES.iter().find(|(n, _)| *n == t) {
        it.next();
        return Ok(*s);
    }
    let Ok(ty) = t.parse::<u8>() else {
        return Ok(IcmpSpec::Any);
    };
    it.next();
    match it.peek().and_then(|c| c.parse::<u8>().ok()) {
        Some(c) => {
            it.next();
            Ok(IcmpSpec::TypeCode(ty, c))
        }
        None => Ok(IcmpSpec::Type(ty)),
    }
}

/// Read `permit|deny ...` (without any sequence number) for a list of `kind`.
pub fn parse_rule(kind: AclKind, toks: &[&str]) -> Result<Rule, ParseError> {
    let mut it = Tokens { toks, at: 0 };
    let permit = match it.need()? {
        "permit" => true,
        "deny" => false,
        t => return bad(Some(t)),
    };
    let rule = match kind {
        AclKind::Standard => Rule {
            permit,
            proto: None,
            src: addr(&mut it)?,
            sport: PortSpec::Any,
            dst: AddrSpec::ANY,
            dport: PortSpec::Any,
            established: false,
            icmp: IcmpSpec::Any,
        },
        AclKind::Extended => {
            let p = it.need()?;
            let proto = if p == "ip" {
                None
            } else if let Some((_, n)) = PROTOCOLS.iter().find(|(name, _)| *name == p) {
                Some(*n)
            } else {
                Some(p.parse::<u8>().or_else(|_| bad(Some(p)))?)
            };
            let l4 = matches!(proto, Some(6) | Some(17));
            let src = addr(&mut it)?;
            let sport = if l4 {
                port_spec(&mut it)?
            } else {
                PortSpec::Any
            };
            let dst = addr(&mut it)?;
            let dport = if l4 {
                port_spec(&mut it)?
            } else {
                PortSpec::Any
            };
            let established = if proto == Some(6) && it.peek() == Some("established") {
                it.next();
                true
            } else {
                false
            };
            let icmp = if proto == Some(1) {
                icmp_spec(&mut it)?
            } else {
                IcmpSpec::Any
            };
            Rule {
                permit,
                proto,
                src,
                sport,
                dst,
                dport,
                established,
                icmp,
            }
        }
    };
    match it.next() {
        None => Ok(rule),
        Some("log") | Some("log-input") => Err(ParseError(
            "% Logging of matches is not supported.".to_string(),
        )),
        Some(t) => bad(Some(t)),
    }
}

// ---- rendering ----

fn render_addr(a: &AddrSpec) -> String {
    if a.is_any() {
        "any".to_string()
    } else if a.is_host() {
        format!("host {}", a.addr)
    } else {
        format!("{} {}", a.addr, a.wild)
    }
}

fn render_port(s: PortSpec) -> String {
    let name = |p: u16| {
        PORTS
            .iter()
            .find(|(_, n)| *n == p)
            .map_or(p.to_string(), |(n, _)| n.to_string())
    };
    match s {
        PortSpec::Any => String::new(),
        PortSpec::Eq(p) => format!(" eq {}", name(p)),
        PortSpec::Neq(p) => format!(" neq {}", name(p)),
        PortSpec::Lt(p) => format!(" lt {}", name(p)),
        PortSpec::Gt(p) => format!(" gt {}", name(p)),
        PortSpec::Range(a, b) => format!(" range {} {}", name(a), name(b)),
    }
}

impl Rule {
    /// The line as typed into a configuration (without a sequence number).
    pub fn render(&self, kind: AclKind) -> String {
        let action = if self.permit { "permit" } else { "deny" };
        match kind {
            AclKind::Standard => {
                // A host is written without `host` here, as IOS prints it.
                let src = if self.src.is_host() {
                    self.src.addr.to_string()
                } else {
                    render_addr(&self.src)
                };
                format!("{action} {src}")
            }
            AclKind::Extended => {
                let proto = match self.proto {
                    None => "ip".to_string(),
                    Some(n) => PROTOCOLS
                        .iter()
                        .find(|(_, x)| *x == n)
                        .map_or(n.to_string(), |(name, _)| name.to_string()),
                };
                let mut o = format!(
                    "{action} {proto} {}{} {}{}",
                    render_addr(&self.src),
                    render_port(self.sport),
                    render_addr(&self.dst),
                    render_port(self.dport),
                );
                if self.established {
                    o += " established";
                }
                match self.icmp {
                    IcmpSpec::Any => {}
                    s => match ICMP_NAMES.iter().find(|(_, x)| *x == s) {
                        Some((n, _)) => o += &format!(" {n}"),
                        None => match s {
                            IcmpSpec::Type(t) => o += &format!(" {t}"),
                            IcmpSpec::TypeCode(t, c) => o += &format!(" {t} {c}"),
                            IcmpSpec::Any => {}
                        },
                    },
                }
                o
            }
        }
    }

    /// The line as `show access-lists` prints it: a standard line with a wildcard reads
    /// `permit 10.0.0.0, wildcard bits 0.0.0.255`.
    fn show(&self, kind: AclKind) -> String {
        if kind == AclKind::Standard && !self.src.is_host() && !self.src.is_any() {
            let action = if self.permit { "permit" } else { "deny" };
            return format!(
                "{action} {}, wildcard bits {}",
                self.src.addr, self.src.wild
            );
        }
        self.render(kind)
    }
}

fn is_number(name: &str) -> bool {
    name.parse::<u32>()
        .is_ok_and(|n| kind_of_number(n).is_some())
}

/// The `running-config` text for all lists, numbered ones as `access-list N ...` lines (or as
/// a block if their numbering was edited), named ones as `ip access-list` blocks.
pub fn config_text(set: &AclSet) -> String {
    let mut o = String::new();
    for (name, acl) in set.iter() {
        let kind = match acl.kind {
            AclKind::Standard => "standard",
            AclKind::Extended => "extended",
        };
        if is_number(name) && acl.default_numbering() {
            for e in &acl.entries {
                match &e.body {
                    Body::Rule(r) => o += &format!("access-list {name} {}\n", r.render(acl.kind)),
                    Body::Remark(t) => o += &format!("access-list {name} remark {t}\n"),
                }
            }
            continue;
        }
        o += &format!("ip access-list {kind} {name}\n");
        let mut expected = SEQ_STEP;
        for e in &acl.entries {
            let seq = if e.seq == expected {
                String::new()
            } else {
                format!("{} ", e.seq)
            };
            expected = e.seq.saturating_add(SEQ_STEP);
            match &e.body {
                Body::Rule(r) => o += &format!(" {seq}{}\n", r.render(acl.kind)),
                Body::Remark(t) => o += &format!(" {seq}remark {t}\n"),
            }
        }
    }
    o
}

/// `show access-lists`, for one list or all.
pub fn show(set: &AclSet, only: Option<&str>) -> String {
    let mut out = Vec::new();
    for (name, acl) in set.iter() {
        if only.is_some_and(|x| x != name) {
            continue;
        }
        let kind = match acl.kind {
            AclKind::Standard => "Standard",
            AclKind::Extended => "Extended",
        };
        let mut o = format!("{kind} IP access list {name}");
        for e in &acl.entries {
            // Remarks are not listed here, only in the configuration.
            let Body::Rule(r) = &e.body else { continue };
            o += &format!("\n    {} {}", e.seq, r.show(acl.kind));
            match e.hits {
                0 => {}
                1 => o += " (1 match)",
                n => o += &format!(" ({n} matches)"),
            }
        }
        out.push(o);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rule(kind: AclKind, s: &str) -> Rule {
        let toks: Vec<&str> = s.split_whitespace().collect();
        parse_rule(kind, &toks).unwrap()
    }

    fn pkt(proto: u8, src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> Packet {
        Packet {
            proto,
            src: Ipv4Addr(src),
            dst: Ipv4Addr(dst),
            sport: matches!(proto, 6 | 17).then_some(sport),
            dport: matches!(proto, 6 | 17).then_some(dport),
            ack_or_rst: false,
            icmp: None,
            later_fragment: false,
        }
    }

    #[test]
    fn wildcard_matching_and_normalising() {
        let r = rule(AclKind::Standard, "permit 10.1.2.3 0.0.255.255");
        assert_eq!(r.src.addr, Ipv4Addr([10, 1, 0, 0]));
        assert!(r.src.matches(Ipv4Addr([10, 1, 77, 88])));
        assert!(!r.src.matches(Ipv4Addr([10, 2, 0, 0])));
        // Wildcards need not be contiguous.
        let odd = AddrSpec::new(Ipv4Addr([10, 0, 0, 1]), Ipv4Addr([0, 255, 0, 0]));
        assert!(odd.matches(Ipv4Addr([10, 9, 0, 1])));
        assert!(!odd.matches(Ipv4Addr([10, 9, 0, 2])));
    }

    #[test]
    fn first_match_wins_and_the_end_denies() {
        let mut a = Acl::new(AclKind::Extended);
        a.add(
            None,
            Body::Rule(rule(AclKind::Extended, "deny tcp host 10.0.0.5 any eq www")),
        )
        .unwrap();
        a.add(
            None,
            Body::Rule(rule(
                AclKind::Extended,
                "permit tcp 10.0.0.0 0.0.0.255 any eq www",
            )),
        )
        .unwrap();
        assert!(!a.check(&pkt(6, [10, 0, 0, 5], [1, 1, 1, 1], 1000, 80)));
        assert!(a.check(&pkt(6, [10, 0, 0, 6], [1, 1, 1, 1], 1000, 80)));
        assert!(!a.check(&pkt(6, [10, 0, 0, 6], [1, 1, 1, 1], 1000, 443)));
        assert_eq!(
            a.entries.iter().map(|e| e.hits).collect::<Vec<_>>(),
            [1, 1],
            "the implicit deny at the end is not counted"
        );
    }

    #[test]
    fn established_icmp_and_ports() {
        let r = rule(AclKind::Extended, "permit tcp any any established");
        let mut p = pkt(6, [1, 1, 1, 1], [2, 2, 2, 2], 80, 5000);
        assert!(!r.matches(&p));
        p.ack_or_rst = true;
        assert!(r.matches(&p));
        let r = rule(AclKind::Extended, "permit icmp any any echo");
        let mut p = pkt(1, [1, 1, 1, 1], [2, 2, 2, 2], 0, 0);
        p.icmp = Some((8, 0));
        assert!(r.matches(&p));
        p.icmp = Some((0, 0));
        assert!(!r.matches(&p));
        let r = rule(AclKind::Extended, "deny udp any range 1000 2000 any gt 100");
        assert!(r.matches(&pkt(17, [1, 1, 1, 1], [2, 2, 2, 2], 1500, 101)));
        assert!(!r.matches(&pkt(17, [1, 1, 1, 1], [2, 2, 2, 2], 1500, 100)));
    }

    #[test]
    fn later_fragments_pass_permits_with_conditions_not_denies() {
        let mut p = pkt(6, [1, 1, 1, 1], [2, 2, 2, 2], 0, 0);
        p.sport = None;
        p.dport = None;
        p.later_fragment = true;
        assert!(rule(AclKind::Extended, "permit tcp any any eq www").matches(&p));
        assert!(!rule(AclKind::Extended, "deny tcp any any eq www").matches(&p));
        assert!(rule(AclKind::Extended, "deny tcp any any").matches(&p));
    }

    #[test]
    fn sequence_numbers() {
        let mut a = Acl::new(AclKind::Standard);
        let r = || Body::Rule(rule(AclKind::Standard, "permit any"));
        assert_eq!(a.add(None, r()), Ok(10));
        assert_eq!(a.add(None, r()), Ok(20));
        assert_eq!(a.add(Some(15), r()), Ok(15));
        assert_eq!(a.add(Some(15), r()), Err(AddError::DuplicateSeq));
        assert_eq!(
            a.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [10, 15, 20]
        );
        assert_eq!(a.add(None, r()), Ok(30));
        assert!(a.remove_seq(15));
        assert!(!a.remove_seq(15));
        let mut full = Acl::new(AclKind::Standard);
        full.add(Some(MAX_SEQ), r()).unwrap();
        assert_eq!(full.add(None, r()), Err(AddError::SeqOverflow));
    }

    #[test]
    fn errors_name_the_trouble() {
        let e = |k, s: &str| {
            let toks: Vec<&str> = s.split_whitespace().collect();
            parse_rule(k, &toks).unwrap_err().0
        };
        assert!(e(AclKind::Extended, "permit foo any any").contains("'foo'"));
        assert!(e(AclKind::Extended, "permit ip any").contains("Incomplete"));
        assert!(e(AclKind::Extended, "permit ip any any extra").contains("'extra'"));
        assert!(e(AclKind::Extended, "permit tcp any any range 90 80").contains("range"));
        assert!(e(AclKind::Standard, "permit 300.1.1.1").contains("'300.1.1.1'"));
        assert!(e(AclKind::Extended, "permit ip any any log").contains("Logging"));
    }

    #[test]
    fn config_and_show_text() {
        let mut set = AclSet::default();
        let a = set.open("101", AclKind::Extended).unwrap();
        a.add(None, Body::Remark("web only".into())).unwrap();
        a.add(
            None,
            Body::Rule(rule(
                AclKind::Extended,
                "permit tcp 10.0.0.0 0.0.0.255 any eq 80",
            )),
        )
        .unwrap();
        let s = set.open("1", AclKind::Standard).unwrap();
        s.add(
            None,
            Body::Rule(rule(AclKind::Standard, "deny host 10.0.0.9")),
        )
        .unwrap();
        s.add(
            None,
            Body::Rule(rule(AclKind::Standard, "permit 10.0.0.0 0.0.0.255")),
        )
        .unwrap();
        s.entries[1].hits = 2;
        let n = set.open("EDGE", AclKind::Extended).unwrap();
        n.add(
            Some(5),
            Body::Rule(rule(AclKind::Extended, "deny icmp any any")),
        )
        .unwrap();
        assert_eq!(
            config_text(&set),
            "access-list 101 remark web only\n\
             access-list 101 permit tcp 10.0.0.0 0.0.0.255 any eq www\n\
             access-list 1 deny 10.0.0.9\n\
             access-list 1 permit 10.0.0.0 0.0.0.255\n\
             ip access-list extended EDGE\n 5 deny icmp any any\n"
        );
        assert_eq!(
            show(&set, Some("1")),
            "Standard IP access list 1\n    10 deny 10.0.0.9\n    20 permit 10.0.0.0, wildcard bits 0.0.0.255 (2 matches)"
        );
        // A numbered list whose numbering was edited prints as a block.
        set.get_mut("101").unwrap().remove_seq(10);
        assert!(config_text(&set).contains("ip access-list extended 101\n 20 permit tcp"));
    }

    // ---- properties ----

    fn arb_addr() -> impl Strategy<Value = AddrSpec> {
        // Few distinct values, so that random packets hit the rules.
        let octet = prop_oneof![Just(0u8), Just(1), Just(2), Just(255)];
        let wild = prop_oneof![Just(0u8), Just(0), Just(255), Just(3), Just(0xf0),];
        (
            proptest::array::uniform4(octet),
            proptest::array::uniform4(wild),
        )
            .prop_map(|(a, w)| AddrSpec::new(Ipv4Addr(a), Ipv4Addr(w)))
    }

    fn arb_port() -> impl Strategy<Value = PortSpec> {
        let p = prop_oneof![
            Just(0u16),
            Just(1),
            Just(80),
            Just(81),
            Just(443),
            Just(65535)
        ];
        prop_oneof![
            Just(PortSpec::Any),
            p.clone().prop_map(PortSpec::Eq),
            p.clone().prop_map(PortSpec::Neq),
            p.clone().prop_map(PortSpec::Lt),
            p.clone().prop_map(PortSpec::Gt),
            (p.clone(), p).prop_map(|(a, b)| PortSpec::Range(a.min(b), a.max(b))),
        ]
    }

    fn arb_rule() -> impl Strategy<Value = Rule> {
        let proto = prop_oneof![
            Just(None),
            Just(Some(1u8)),
            Just(Some(6)),
            Just(Some(17)),
            Just(Some(89))
        ];
        let icmp = prop_oneof![
            Just(IcmpSpec::Any),
            Just(IcmpSpec::Type(8)),
            Just(IcmpSpec::Type(3)),
            Just(IcmpSpec::TypeCode(3, 13)),
        ];
        (
            any::<bool>(),
            proto,
            arb_addr(),
            arb_port(),
            arb_addr(),
            arb_port(),
            any::<bool>(),
            icmp,
        )
            .prop_map(
                |(permit, proto, src, sport, dst, dport, established, icmp)| {
                    // Only what the syntax can say: ports for tcp/udp, `established` for tcp, a
                    // type for icmp.
                    let l4 = matches!(proto, Some(6) | Some(17));
                    Rule {
                        permit,
                        proto,
                        src,
                        sport: if l4 { sport } else { PortSpec::Any },
                        dst,
                        dport: if l4 { dport } else { PortSpec::Any },
                        established: proto == Some(6) && established,
                        icmp: if proto == Some(1) {
                            icmp
                        } else {
                            IcmpSpec::Any
                        },
                    }
                },
            )
    }

    fn arb_packet() -> impl Strategy<Value = Packet> {
        let octet = prop_oneof![Just(0u8), Just(1), Just(2), Just(255), Just(16)];
        let port = prop_oneof![
            Just(0u16),
            Just(1),
            Just(80),
            Just(81),
            Just(443),
            Just(65535)
        ];
        (
            prop_oneof![Just(1u8), Just(6), Just(17), Just(89)],
            proptest::array::uniform4(octet.clone()),
            proptest::array::uniform4(octet),
            port.clone(),
            port,
            any::<bool>(),
            prop_oneof![Just((8u8, 0u8)), Just((0, 0)), Just((3, 13)), Just((3, 1))],
            prop_oneof![9 => Just(false), 1 => Just(true)],
        )
            .prop_map(|(proto, s, d, sp, dp, ar, ic, frag)| {
                let l4 = matches!(proto, 6 | 17) && !frag;
                Packet {
                    proto,
                    src: Ipv4Addr(s),
                    dst: Ipv4Addr(d),
                    sport: l4.then_some(sp),
                    dport: l4.then_some(dp),
                    ack_or_rst: proto == 6 && !frag && ar,
                    icmp: (proto == 1 && !frag).then_some(ic),
                    later_fragment: frag,
                }
            })
    }

    /// An independent reading of the rules: bit by bit, with no shared helpers.
    fn reference(rules: &[Rule], p: &Packet) -> Option<(usize, bool)> {
        let addr_ok = |spec: &AddrSpec, a: Ipv4Addr| {
            (0..32).all(|bit| {
                let w = (spec.wild.to_u32() >> bit) & 1;
                let x = (spec.addr.to_u32() >> bit) & 1;
                let y = (a.to_u32() >> bit) & 1;
                w == 1 || x == y
            })
        };
        let port_ok = |spec: PortSpec, port: Option<u16>| -> bool {
            let Some(q) = port else {
                return spec == PortSpec::Any;
            };
            match spec {
                PortSpec::Any => true,
                PortSpec::Eq(x) => q == x,
                PortSpec::Neq(x) => q != x,
                PortSpec::Lt(x) => q < x,
                PortSpec::Gt(x) => q > x,
                PortSpec::Range(a, b) => (a..=b).contains(&q),
            }
        };
        for (i, r) in rules.iter().enumerate() {
            let l3 = (r.proto.is_none() || r.proto == Some(p.proto))
                && addr_ok(&r.src, p.src)
                && addr_ok(&r.dst, p.dst);
            let hit = if p.later_fragment {
                let conditions = r.sport != PortSpec::Any
                    || r.dport != PortSpec::Any
                    || r.established
                    || r.icmp != IcmpSpec::Any;
                l3 && (!conditions || r.permit)
            } else {
                l3 && port_ok(r.sport, p.sport)
                    && port_ok(r.dport, p.dport)
                    && (!r.established || p.ack_or_rst)
                    && match r.icmp {
                        IcmpSpec::Any => true,
                        IcmpSpec::Type(t) => p.icmp.is_some_and(|(a, _)| a == t),
                        IcmpSpec::TypeCode(t, c) => p.icmp == Some((t, c)),
                    }
            };
            if hit {
                return Some((i, r.permit));
            }
        }
        None
    }

    proptest! {
        #[test]
        fn evaluation_agrees_with_the_reference(
            rules in proptest::collection::vec(arb_rule(), 0..8),
            packets in proptest::collection::vec(arb_packet(), 1..24),
        ) {
            let mut acl = Acl::new(AclKind::Extended);
            for r in &rules {
                acl.add(None, Body::Rule(r.clone())).unwrap();
            }
            let mut expected_hits = vec![0u64; rules.len()];
            for p in &packets {
                let first = reference(&rules, p);
                let v = first.is_some_and(|(_, permit)| permit);
                prop_assert_eq!(acl.verdict(p), v);
                prop_assert_eq!(acl.check(p), v);
                if let Some((i, _)) = first {
                    expected_hits[i] += 1;
                }
            }
            // Each packet is counted once, on the line that decided it.
            let hits: Vec<u64> = acl.entries.iter().map(|e| e.hits).collect();
            prop_assert_eq!(hits, expected_hits);
        }

        #[test]
        fn a_rule_reads_back_as_it_prints(r in arb_rule()) {
            let text = r.render(AclKind::Extended);
            let toks: Vec<&str> = text.split_whitespace().collect();
            prop_assert_eq!(parse_rule(AclKind::Extended, &toks).unwrap(), r);
        }

        #[test]
        fn a_standard_rule_reads_back_as_it_prints(permit in any::<bool>(), src in arb_addr()) {
            let r = Rule {
                permit, proto: None, src, sport: PortSpec::Any, dst: AddrSpec::ANY,
                dport: PortSpec::Any, established: false, icmp: IcmpSpec::Any,
            };
            let text = r.render(AclKind::Standard);
            let toks: Vec<&str> = text.split_whitespace().collect();
            prop_assert_eq!(parse_rule(AclKind::Standard, &toks).unwrap(), r);
        }

        #[test]
        fn parsing_never_panics(toks in proptest::collection::vec("[a-z0-9.]{0,9}", 0..10), std in any::<bool>()) {
            let toks: Vec<&str> = toks.iter().map(String::as_str).collect();
            let _ = parse_rule(if std { AclKind::Standard } else { AclKind::Extended }, &toks);
        }
    }
}
