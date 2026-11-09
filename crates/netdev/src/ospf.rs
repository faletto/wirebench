//! OSPFv2 for one area (RFC 2328): neighbours, designated router election, database exchange,
//! flooding, aging and the shortest path calculation. A pure state machine: it is told the time
//! and which interfaces are up, is handed packets, and returns the packets to send. The router
//! (`router.rs`) does the addressing, framing and routing table. See `provenance/ospf.md`.

use kernel::SimTime;
use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use wire::{
    Dbd, Hello, Ipv4Addr, LsRequest, Lsa, LsaBody, LsaHeader, MacAddr, OspfBody, OspfPacket,
    RouterLink, ALL_DR_ROUTERS, ALL_SPF_ROUTERS, DBD_INIT, DBD_MASTER, DBD_MORE, INITIAL_SEQ,
    LINK_P2P, LINK_STUB, LINK_TRANSIT, LSA_ASBR_SUMMARY, LSA_EXTERNAL, LSA_NETWORK, LSA_ROUTER,
    LSA_SUMMARY, LS_INFINITY, MAX_AGE, OPT_E, ROUTER_B, ROUTER_E,
};

pub const DEFAULT_HELLO: u16 = 10;
/// Reference bandwidth in Mb/s (`auto-cost reference-bandwidth`).
pub const DEFAULT_REF_BW: u32 = 100;
/// Every router interface is gigabit.
const LINK_MBPS: u32 = 1000;
const RXMT_INTERVAL: SimTime = SimTime::from_secs(5);
const LS_REFRESH: u32 = 1800;
const MAX_AGE_DIFF: u32 = 900;
const INF_TRANS_DELAY: u32 = 1;
/// Wait after an LSDB change before the shortest path calculation runs.
const SPF_DELAY: SimTime = SimTime::from_secs(1);
const MTU: u16 = 1500;
/// How many LSA headers or requests go in one packet.
const PER_PACKET: usize = 50;
/// Bytes of LSAs in one Link State Update.
const LSU_BUDGET: usize = 1400;

/// An LSA's type, link state ID and advertising router.
type Key = (u8, Ipv4Addr, Ipv4Addr);
/// A [`Key`] with the area it is flooded in; AS-external LSAs belong to no area ([`AS_SCOPE`]).
type AKey = (u32, u8, Ipv4Addr, Ipv4Addr);
const AS_SCOPE: u32 = u32::MAX;

fn akey(area: u32, k: Key) -> AKey {
    (
        if k.0 == LSA_EXTERNAL { AS_SCOPE } else { area },
        k.0,
        k.1,
        k.2,
    )
}

fn plain(a: &AKey) -> Key {
    (a.1, a.2, a.3)
}

fn key_of(h: &LsaHeader) -> Key {
    (h.ls_type, h.id, h.adv_router)
}

// ---- configuration ----

/// Per-interface settings (`ip ospf ...`, `passive-interface`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IfCfg {
    pub cost: Option<u32>,
    pub priority: u8,
    pub hello: Option<u16>,
    pub dead: Option<u32>,
    pub point_to_point: bool,
    pub passive: bool,
    /// `ip mtu`: the size advertised in Database Description packets. The simulator does no
    /// fragmentation, so this is all it affects.
    pub mtu: Option<u16>,
    /// `ip ospf mtu-ignore`: do not hold a neighbour with a larger MTU back.
    pub mtu_ignore: bool,
}

impl Default for IfCfg {
    fn default() -> Self {
        IfCfg {
            cost: None,
            priority: 1,
            hello: None,
            dead: None,
            point_to_point: false,
            passive: false,
            mtu: None,
            mtu_ignore: false,
        }
    }
}

impl IfCfg {
    pub fn mtu(&self) -> u16 {
        self.mtu.unwrap_or(MTU)
    }

    pub fn hello_interval(&self) -> u16 {
        self.hello.unwrap_or(DEFAULT_HELLO)
    }

    /// Four hellos unless set.
    pub fn dead_interval(&self) -> u32 {
        self.dead.unwrap_or(4 * self.hello_interval() as u32)
    }
}

/// `default-information originate`: advertise a default route to the AS as an external LSA.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DefaultInfo {
    /// Advertise even without a default route of our own.
    pub always: bool,
    pub metric: u32,
    /// Metric type 2 (the default) rather than 1.
    pub e2: bool,
}

impl Default for DefaultInfo {
    fn default() -> Self {
        DefaultInfo {
            always: false,
            metric: 1,
            e2: true,
        }
    }
}

/// What the router tells the protocol about one usable interface (up and addressed).
#[derive(Clone, Copy, Debug)]
pub struct IfView {
    pub index: usize,
    pub addr: Ipv4Addr,
    pub len: u8,
    pub mac: MacAddr,
    pub cfg: IfCfg,
}

/// `network <addr> <wildcard> area <n>`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NetStmt {
    pub net: Ipv4Addr,
    pub wild: Ipv4Addr,
    pub area: u32,
}

impl NetStmt {
    fn covers(&self, a: Ipv4Addr) -> bool {
        (a.to_u32() ^ self.net.to_u32()) & !self.wild.to_u32() == 0
    }
}

/// Where a received packet came from: our interface, the sender's IP and MAC address.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub iface: usize,
    pub ip: Ipv4Addr,
    pub mac: MacAddr,
}

/// A packet to send out of interface `iface`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    pub iface: usize,
    pub dst: Ipv4Addr,
    pub dst_mac: MacAddr,
    pub packet: OspfPacket,
}

/// How a route was learned, in the order OSPF prefers them (RFC 2328 section 11).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum PathKind {
    /// Within an area we are in.
    Intra,
    /// Through an area border router's summary.
    Inter,
    /// External, metric added to the cost of reaching the boundary router.
    External1,
    /// External, metric on its own (the cost of reaching the boundary router only breaks ties).
    External2,
}

/// A route the shortest path calculation found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OspfRoute {
    pub net: Ipv4Addr,
    pub len: u8,
    /// The metric shown: the whole cost, or for a type 2 external its own metric.
    pub cost: u32,
    pub via: Ipv4Addr,
    pub iface: usize,
    pub kind: PathKind,
    /// The area it was learned through.
    pub area: u32,
}

// ---- state ----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum NbrState {
    Down,
    Init,
    TwoWay,
    ExStart,
    Exchange,
    Loading,
    Full,
}

impl NbrState {
    pub fn name(self) -> &'static str {
        match self {
            NbrState::Down => "DOWN",
            NbrState::Init => "INIT",
            NbrState::TwoWay => "2WAY",
            NbrState::ExStart => "EXSTART",
            NbrState::Exchange => "EXCHANGE",
            NbrState::Loading => "LOADING",
            NbrState::Full => "FULL",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IfState {
    Waiting,
    PointToPoint,
    DrOther,
    Backup,
    Dr,
}

impl IfState {
    pub fn name(self) -> &'static str {
        match self {
            IfState::Waiting => "WAITING",
            IfState::PointToPoint => "P2P",
            IfState::DrOther => "DROTHER",
            IfState::Backup => "BDR",
            IfState::Dr => "DR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Nbr {
    pub id: Ipv4Addr,
    pub ip: Ipv4Addr,
    mac: MacAddr,
    pub state: NbrState,
    pub priority: u8,
    /// The DR and BDR this neighbour names in its hellos (interface addresses).
    dr: Ipv4Addr,
    bdr: Ipv4Addr,
    inactive_at: SimTime,
    /// Whether this router is the master of the exchange.
    master: bool,
    dd_seq: u32,
    /// Headers still to be sent in Database Description packets.
    summary: Vec<LsaHeader>,
    /// What this neighbour has that we need.
    request: Vec<LsaHeader>,
    /// LSAs sent to this neighbour and not yet acknowledged.
    retrans: BTreeSet<Key>,
    last_sent: Option<Dbd>,
    last_rcvd: Option<(u8, u8, u32)>,
    next_rxmt: SimTime,
}

impl Nbr {
    /// Seconds until the neighbour is declared dead if it stays quiet.
    pub fn dead_in(&self, now: SimTime) -> u64 {
        self.inactive_at.0.saturating_sub(now.0) / 1_000_000_000
    }

    fn new(id: Ipv4Addr, ip: Ipv4Addr, mac: MacAddr, now: SimTime, dead: u32) -> Self {
        Nbr {
            id,
            ip,
            mac,
            state: NbrState::Down,
            priority: 0,
            dr: Ipv4Addr::UNSPECIFIED,
            bdr: Ipv4Addr::UNSPECIFIED,
            inactive_at: now + SimTime::from_secs(dead as u64),
            master: false,
            dd_seq: 0,
            summary: Vec::new(),
            request: Vec::new(),
            retrans: BTreeSet::new(),
            last_sent: None,
            last_rcvd: None,
            next_rxmt: SimTime::ZERO,
        }
    }

    fn clear_exchange(&mut self) {
        self.summary.clear();
        self.request.clear();
        self.retrans.clear();
        self.last_sent = None;
        self.last_rcvd = None;
    }
}

#[derive(Clone, Debug)]
pub struct OspfIf {
    pub index: usize,
    pub area: u32,
    pub addr: Ipv4Addr,
    pub len: u8,
    mac: MacAddr,
    pub cfg: IfCfg,
    pub state: IfState,
    /// Designated router and backup, as interface addresses.
    pub dr: Ipv4Addr,
    pub bdr: Ipv4Addr,
    pub nbrs: BTreeMap<Ipv4Addr, Nbr>,
    next_hello: SimTime,
    wait_until: Option<SimTime>,
}

impl OspfIf {
    fn net(&self) -> Ipv4Addr {
        Ipv4Addr::from_u32(self.addr.to_u32() & prefix_mask(self.len))
    }

    fn adjacent_wanted(&self, n: &Nbr) -> bool {
        self.cfg.point_to_point
            || matches!(self.state, IfState::Dr | IfState::Backup)
            || n.ip == self.dr
            || n.ip == self.bdr
    }
}

fn prefix_mask(len: u8) -> u32 {
    if len == 0 {
        0
    } else {
        u32::MAX << (32 - len as u32)
    }
}

#[derive(Clone, Debug)]
struct DbEntry {
    /// As installed; `age` is the age at `installed`.
    lsa: Lsa,
    header: LsaHeader,
    installed: SimTime,
    /// A MaxAge copy has been flooded.
    flushed: bool,
}

impl DbEntry {
    fn new(lsa: Lsa, now: SimTime) -> Self {
        let header = lsa.header();
        DbEntry {
            lsa,
            header,
            installed: now,
            flushed: false,
        }
    }

    fn age(&self, now: SimTime) -> u16 {
        let elapsed = (now.0.saturating_sub(self.installed.0) / 1_000_000_000) as u32;
        (self.lsa.age as u32 + elapsed).min(MAX_AGE as u32) as u16
    }

    fn header_at(&self, now: SimTime) -> LsaHeader {
        LsaHeader {
            age: self.age(now),
            ..self.header
        }
    }
}

/// Which of two instances of an LSA is more recent (13.1): `Greater` if `a`.
fn compare(a: &LsaHeader, b: &LsaHeader) -> Ordering {
    a.seq
        .cmp(&b.seq)
        .then(a.checksum.cmp(&b.checksum))
        .then_with(|| match (a.age == MAX_AGE, b.age == MAX_AGE) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ if (a.age as u32).abs_diff(b.age as u32) > MAX_AGE_DIFF => b.age.cmp(&a.age),
            _ => Ordering::Equal,
        })
}

fn multicast_mac(ip: Ipv4Addr) -> MacAddr {
    let v = ip.to_u32();
    MacAddr([
        0x01,
        0x00,
        0x5e,
        ((v >> 16) & 0x7f) as u8,
        (v >> 8) as u8,
        v as u8,
    ])
}

/// The whole protocol state of one router.
#[derive(Clone, Debug)]
pub struct Ospf {
    pub process: u16,
    pub router_id_cfg: Option<Ipv4Addr>,
    pub networks: Vec<NetStmt>,
    /// Mb/s.
    pub ref_bw: u32,
    /// `default-information originate`, if configured.
    pub default_info: Option<DefaultInfo>,
    /// Whether the router has a default route of its own (set by the router).
    has_default: bool,
    rid: Option<Ipv4Addr>,
    ifs: BTreeMap<usize, OspfIf>,
    db: BTreeMap<AKey, DbEntry>,
    /// The summary LSAs the last calculation says an area border router should originate.
    summaries: BTreeMap<AKey, LsaBody>,
    routes: Vec<OspfRoute>,
    routes_changed: bool,
    reoriginate: bool,
    spf_at: Option<SimTime>,
    dd_counter: u32,
    /// A neighbour holds a newer instance of one of our LSAs than we do: the next one we
    /// originate must be numbered above this.
    seq_floor: BTreeMap<AKey, i32>,
}

impl Ospf {
    pub fn new(process: u16) -> Self {
        Ospf {
            process,
            router_id_cfg: None,
            networks: Vec::new(),
            ref_bw: DEFAULT_REF_BW,
            default_info: None,
            has_default: false,
            rid: None,
            ifs: BTreeMap::new(),
            db: BTreeMap::new(),
            summaries: BTreeMap::new(),
            routes: Vec::new(),
            routes_changed: false,
            reoriginate: false,
            spf_at: None,
            dd_counter: 0x1000,
            seq_floor: BTreeMap::new(),
        }
    }

    /// The router ID in use, once there is one.
    pub fn router_id(&self) -> Option<Ipv4Addr> {
        self.rid
    }

    /// The areas this router has an active interface in.
    pub fn areas(&self) -> BTreeSet<u32> {
        self.ifs.values().map(|f| f.area).collect()
    }

    /// Attached to more than one area, one of them the backbone: an area border router.
    pub fn is_abr(&self) -> bool {
        let a = self.areas();
        a.len() > 1 && a.contains(&0)
    }

    /// Originating external LSAs: an AS boundary router.
    pub fn is_asbr(&self) -> bool {
        self.default_active()
    }

    /// Add a `network` statement.
    pub fn add_network(&mut self, n: NetStmt) {
        if !self.networks.contains(&n) {
            self.networks.push(n);
        }
    }

    /// The router now has (or has lost) a default route of its own.
    pub fn set_default_route(&mut self, present: bool) {
        if self.has_default != present {
            self.has_default = present;
            self.reoriginate = true;
        }
    }

    fn default_active(&self) -> bool {
        self.rid.is_some()
            && self
                .default_info
                .is_some_and(|d| d.always || self.has_default)
    }

    pub fn interfaces(&self) -> impl Iterator<Item = &OspfIf> {
        self.ifs.values()
    }

    /// The configuration changed (cost, reference bandwidth, ...): originate again.
    pub fn config_changed(&mut self) {
        self.reoriginate = true;
    }

    /// Whether this router is the DR or BDR on interface `i`.
    pub fn is_designated(&self, i: usize) -> bool {
        self.ifs
            .get(&i)
            .is_some_and(|f| matches!(f.state, IfState::Dr | IfState::Backup))
    }

    pub fn routes(&self) -> &[OspfRoute] {
        &self.routes
    }

    /// Whether the routes changed since this was last asked.
    pub fn take_routes_changed(&mut self) -> bool {
        std::mem::take(&mut self.routes_changed)
    }

    /// Forget everything learned and decided (the process restarts), keeping the configuration.
    pub fn reset(&mut self) {
        self.rid = None;
        self.ifs.clear();
        self.db.clear();
        self.summaries.clear();
        if !self.routes.is_empty() {
            self.routes.clear();
            self.routes_changed = true;
        }
        self.reoriginate = false;
        self.spf_at = None;
        self.seq_floor.clear();
    }

    pub fn cost_of(&self, cfg: &IfCfg) -> u32 {
        cfg.cost.unwrap_or_else(|| (self.ref_bw / LINK_MBPS).max(1))
    }

    /// LSAs for `show ip ospf database`: (area or `None` for AS scope, header with its current
    /// age, body).
    pub fn database(&self, now: SimTime) -> Vec<(Option<u32>, LsaHeader, &LsaBody)> {
        self.db
            .iter()
            .map(|(k, e)| {
                let area = (k.0 != AS_SCOPE).then_some(k.0);
                (area, e.header_at(now), &e.lsa.body)
            })
            .collect()
    }

    // ---- driving ----

    /// One second (or so) has passed: send hellos, time things out, age the database.
    pub fn tick(&mut self, now: SimTime, views: &[IfView], out: &mut Vec<Tx>) {
        self.sync(now, views, out);
        let Some(rid) = self.rid else { return };
        let ids: Vec<usize> = self.ifs.keys().copied().collect();
        for i in ids {
            self.interface_timers(now, i, rid, out);
        }
        self.age_database(now, out);
        self.housekeeping(now, out);
    }

    pub fn receive(
        &mut self,
        now: SimTime,
        views: &[IfView],
        from: Source,
        pkt: OspfPacket,
        out: &mut Vec<Tx>,
    ) {
        let (iface, src) = (from.iface, from.ip);
        self.sync(now, views, out);
        let Some(rid) = self.rid else { return };
        let Some(ifc) = self.ifs.get(&iface) else {
            return;
        };
        if ifc.cfg.passive
            || pkt.router_id == rid
            || pkt.auth_type != 0
            || u32::from_be_bytes(pkt.area.0) != ifc.area
        {
            return;
        }
        if !ifc.cfg.point_to_point && (src.to_u32() ^ ifc.addr.to_u32()) & prefix_mask(ifc.len) != 0
        {
            return;
        }
        let id = pkt.router_id;
        match pkt.body {
            OspfBody::Hello(h) => self.on_hello(now, from, id, h, out),
            OspfBody::Dbd(d) => self.on_dbd(now, iface, id, d, out),
            OspfBody::Request(r) => self.on_request(now, iface, id, r, out),
            OspfBody::Update(l) => self.on_update(now, iface, id, l, out),
            OspfBody::Ack(h) => self.on_ack(now, iface, id, h),
        }
        self.housekeeping(now, out);
    }

    /// Bring the interface set and the router ID in line with what the router says is up.
    fn sync(&mut self, now: SimTime, views: &[IfView], out: &mut Vec<Tx>) {
        if self.rid.is_none() {
            self.rid = self
                .router_id_cfg
                .or_else(|| views.iter().map(|v| v.addr).max());
            if self.rid.is_none() {
                return;
            }
            self.reoriginate = true;
        }
        let mut wanted: BTreeMap<usize, (&IfView, u32)> = BTreeMap::new();
        for v in views {
            if let Some(n) = self.networks.iter().find(|n| n.covers(v.addr)) {
                wanted.insert(v.index, (v, n.area));
            }
        }
        let gone: Vec<usize> = self
            .ifs
            .iter()
            .filter(|(i, f)| {
                wanted.get(i).is_none_or(|(v, area)| {
                    *area != f.area
                        || v.addr != f.addr
                        || v.len != f.len
                        || v.cfg.point_to_point != f.cfg.point_to_point
                        || v.cfg.passive != f.cfg.passive
                })
            })
            .map(|(i, _)| *i)
            .collect();
        for i in gone {
            self.ifs.remove(&i);
            self.reoriginate = true;
        }
        for (i, (v, area)) in wanted {
            match self.ifs.get_mut(&i) {
                Some(f) => {
                    let changed_priority = f.cfg.priority != v.cfg.priority;
                    if f.cfg != v.cfg {
                        self.reoriginate = true;
                    }
                    f.cfg = v.cfg;
                    f.mac = v.mac;
                    if changed_priority && !f.cfg.point_to_point {
                        self.elect(now, i, out);
                    }
                }
                None => {
                    let p2p = v.cfg.point_to_point;
                    let wait = SimTime::from_secs(v.cfg.dead_interval() as u64);
                    let state = if p2p {
                        IfState::PointToPoint
                    } else if v.cfg.priority == 0 {
                        IfState::DrOther
                    } else {
                        IfState::Waiting
                    };
                    self.ifs.insert(
                        i,
                        OspfIf {
                            index: i,
                            area,
                            addr: v.addr,
                            len: v.len,
                            mac: v.mac,
                            cfg: v.cfg,
                            state,
                            dr: Ipv4Addr::UNSPECIFIED,
                            bdr: Ipv4Addr::UNSPECIFIED,
                            nbrs: BTreeMap::new(),
                            next_hello: now,
                            wait_until: (state == IfState::Waiting).then_some(now + wait),
                        },
                    );
                    self.reoriginate = true;
                }
            }
        }
    }

    fn packet(&self, i: usize, body: OspfBody) -> OspfPacket {
        OspfPacket {
            router_id: self.rid.expect("a router ID"),
            area: Ipv4Addr::from_u32(self.ifs.get(&i).map_or(0, |f| f.area)),
            auth_type: 0,
            auth: [0; 8],
            body,
        }
    }

    fn to_nbr(&self, i: usize, n: &Nbr, body: OspfBody) -> Tx {
        Tx {
            iface: i,
            dst: n.ip,
            dst_mac: n.mac,
            packet: self.packet(i, body),
        }
    }

    fn to_group(&self, i: usize, group: Ipv4Addr, body: OspfBody) -> Tx {
        Tx {
            iface: i,
            dst: group,
            dst_mac: multicast_mac(group),
            packet: self.packet(i, body),
        }
    }

    // ---- timers ----

    fn interface_timers(&mut self, now: SimTime, i: usize, rid: Ipv4Addr, out: &mut Vec<Tx>) {
        let Some(f) = self.ifs.get(&i) else { return };
        if f.cfg.passive {
            return;
        }
        if f.state == IfState::Waiting && f.wait_until.is_some_and(|t| now >= t) {
            self.elect(now, i, out);
        }
        // Neighbours that have gone quiet.
        let dead: Vec<Ipv4Addr> = self.ifs[&i]
            .nbrs
            .values()
            .filter(|n| now >= n.inactive_at)
            .map(|n| n.id)
            .collect();
        let mut elect = false;
        for id in dead {
            let f = self.ifs.get_mut(&i).expect("interface");
            f.nbrs.remove(&id);
            elect = true;
            self.reoriginate = true;
        }
        if elect && !self.ifs[&i].cfg.point_to_point && self.ifs[&i].state != IfState::Waiting {
            self.elect(now, i, out);
        }
        // Hello.
        let f = &self.ifs[&i];
        if now >= f.next_hello {
            let hello = Hello {
                mask: Ipv4Addr::from_u32(prefix_mask(f.len)),
                hello_interval: f.cfg.hello_interval(),
                options: OPT_E,
                priority: f.cfg.priority,
                dead_interval: f.cfg.dead_interval(),
                dr: f.dr,
                bdr: f.bdr,
                neighbors: f
                    .nbrs
                    .values()
                    .filter(|n| n.state >= NbrState::Init)
                    .map(|n| n.id)
                    .collect(),
            };
            let tx = self.to_group(i, ALL_SPF_ROUTERS, OspfBody::Hello(hello));
            out.push(tx);
            let next = now + SimTime::from_secs(self.ifs[&i].cfg.hello_interval() as u64);
            self.ifs.get_mut(&i).expect("interface").next_hello = next;
        }
        // Retransmissions.
        let ids: Vec<Ipv4Addr> = self.ifs[&i].nbrs.keys().copied().collect();
        for id in ids {
            let n = &self.ifs[&i].nbrs[&id];
            if now < n.next_rxmt {
                continue;
            }
            match n.state {
                NbrState::ExStart | NbrState::Exchange => {
                    if let Some(d) = n.last_sent.clone() {
                        // The master repeats its packet; a slave waits to be asked again.
                        if n.master || n.state == NbrState::ExStart {
                            let tx = self.to_nbr(i, n, OspfBody::Dbd(d));
                            out.push(tx);
                        }
                    }
                }
                NbrState::Loading => self.send_requests(i, &id, out),
                _ => {}
            }
            self.retransmit_lsas(now, i, &id, out);
            self.ifs
                .get_mut(&i)
                .and_then(|f| f.nbrs.get_mut(&id))
                .expect("neighbour")
                .next_rxmt = now + RXMT_INTERVAL;
        }
        let _ = rid;
    }

    /// Age the database: refresh our own LSAs when they get old, flood and drop the ones that
    /// reached MaxAge.
    fn age_database(&mut self, now: SimTime, out: &mut Vec<Tx>) {
        let Some(rid) = self.rid else { return };
        let keys: Vec<AKey> = self.db.keys().copied().collect();
        for k in keys {
            let Some(e) = self.db.get(&k) else { continue };
            let age = e.age(now) as u32;
            if k.3 == rid && age >= LS_REFRESH && age < MAX_AGE as u32 {
                self.reoriginate = true;
                // A refresh is a new instance even if the contents are the same.
                self.force_refresh(now, k, out);
            } else if age >= MAX_AGE as u32 {
                if !e.flushed {
                    self.db.get_mut(&k).expect("entry").flushed = true;
                    self.flood(now, k, None, out);
                    self.spf_at.get_or_insert(now + SPF_DELAY);
                } else if self.can_remove(&plain(&k)) {
                    self.db.remove(&k);
                }
            }
        }
    }

    /// A MaxAge LSA may leave the database when no neighbour still has to acknowledge it and no
    /// exchange is under way.
    fn can_remove(&self, k: &Key) -> bool {
        self.ifs.values().all(|f| {
            f.nbrs.values().all(|n| {
                !n.retrans.contains(k) && !matches!(n.state, NbrState::Exchange | NbrState::Loading)
            })
        })
    }

    // ---- hello and neighbours ----

    fn on_hello(&mut self, now: SimTime, from: Source, id: Ipv4Addr, h: Hello, out: &mut Vec<Tx>) {
        let (i, src, mac) = (from.iface, from.ip, from.mac);
        let rid = self.rid.expect("router id");
        let f = self.ifs.get_mut(&i).expect("interface");
        let p2p = f.cfg.point_to_point;
        if (!p2p && h.mask != Ipv4Addr::from_u32(prefix_mask(f.len)))
            || h.hello_interval != f.cfg.hello_interval()
            || h.dead_interval != f.cfg.dead_interval()
            || h.options & OPT_E == 0
        {
            return;
        }
        let dead = f.cfg.dead_interval();
        let waiting = f.state == IfState::Waiting;
        let n = f
            .nbrs
            .entry(id)
            .or_insert_with(|| Nbr::new(id, src, mac, now, dead));
        let (old_prio, old_dr, old_bdr) = (n.priority, n.dr, n.bdr);
        n.ip = src;
        n.mac = mac;
        n.priority = h.priority;
        n.dr = h.dr;
        n.bdr = h.bdr;
        n.inactive_at = now + SimTime::from_secs(dead as u64);
        if n.state == NbrState::Down {
            n.state = NbrState::Init;
        }
        let mut changed = n.priority != old_prio
            || (n.dr == src) != (old_dr == src)
            || (n.bdr == src) != (old_bdr == src);
        let mut start_adj = false;
        if h.neighbors.contains(&rid) {
            if n.state == NbrState::Init {
                n.state = NbrState::TwoWay;
                changed = true;
                start_adj = true;
            }
        } else if n.state >= NbrState::TwoWay {
            // It no longer hears us: back to Init.
            let was_full = n.state == NbrState::Full;
            n.state = NbrState::Init;
            n.clear_exchange();
            changed = true;
            if was_full {
                self.reoriginate = true;
            }
        }
        let n = &self.ifs[&i].nbrs[&id];
        let backup_seen = n.state >= NbrState::TwoWay
            && ((n.dr == src && n.bdr == Ipv4Addr::UNSPECIFIED) || n.bdr == src);
        if p2p {
            if start_adj {
                self.start_exstart(now, i, id, out);
            }
            return;
        }
        if waiting {
            if backup_seen {
                self.elect(now, i, out);
            }
        } else if changed {
            self.elect(now, i, out);
        }
        // A neighbour that just became bidirectional may need an adjacency; `elect` covers the
        // ones whose role changed, this the ones it left alone.
        if start_adj {
            self.adj_ok(now, i, id, out);
        }
    }

    /// Choose the designated and backup designated router (9.4).
    fn elect(&mut self, now: SimTime, i: usize, out: &mut Vec<Tx>) {
        let f = &self.ifs[&i];
        let my_ip = f.addr;
        struct Cand {
            ip: Ipv4Addr,
            id: Ipv4Addr,
            prio: u8,
            dr: Ipv4Addr,
            bdr: Ipv4Addr,
        }
        let rid = self.rid.expect("router id");
        let others: Vec<Cand> = f
            .nbrs
            .values()
            .filter(|n| n.state >= NbrState::TwoWay && n.priority > 0)
            .map(|n| Cand {
                ip: n.ip,
                id: n.id,
                prio: n.priority,
                dr: n.dr,
                bdr: n.bdr,
            })
            .collect();
        let (old_dr, old_bdr) = (f.dr, f.bdr);
        let mut me = Cand {
            ip: my_ip,
            id: rid,
            prio: f.cfg.priority,
            dr: f.dr,
            bdr: f.bdr,
        };
        let pick = |cs: Vec<&Cand>| -> Option<Ipv4Addr> {
            cs.into_iter().max_by_key(|c| (c.prio, c.id)).map(|c| c.ip)
        };
        let run = |me: &Cand| -> (Ipv4Addr, Ipv4Addr) {
            let mut all: Vec<&Cand> = others.iter().collect();
            if me.prio > 0 {
                all.push(me);
            }
            let not_dr: Vec<&Cand> = all.iter().copied().filter(|c| c.dr != c.ip).collect();
            let bdr = pick(not_dr.iter().copied().filter(|c| c.bdr == c.ip).collect())
                .or_else(|| pick(not_dr.clone()));
            let dr = pick(all.iter().copied().filter(|c| c.dr == c.ip).collect()).or(bdr);
            (
                dr.unwrap_or(Ipv4Addr::UNSPECIFIED),
                bdr.unwrap_or(Ipv4Addr::UNSPECIFIED),
            )
        };
        let (mut dr, mut bdr) = run(&me);
        // If this router's own role changed, run it again with the new declaration (step 4).
        if (dr == my_ip) != (old_dr == my_ip) || (bdr == my_ip) != (old_bdr == my_ip) {
            me.dr = dr;
            me.bdr = bdr;
            (dr, bdr) = run(&me);
        }
        let f = self.ifs.get_mut(&i).expect("interface");
        f.dr = dr;
        f.bdr = bdr;
        f.wait_until = None;
        f.state = if dr == my_ip {
            IfState::Dr
        } else if bdr == my_ip {
            IfState::Backup
        } else {
            IfState::DrOther
        };
        if (old_dr, old_bdr) != (dr, bdr) {
            self.reoriginate = true;
        }
        // Adjacencies follow from the roles.
        let ids: Vec<Ipv4Addr> = self.ifs[&i].nbrs.keys().copied().collect();
        for id in ids {
            self.adj_ok(now, i, id, out);
        }
    }
}

// ---- adjacencies and database exchange ----

impl Ospf {
    /// Re-evaluate whether the neighbour should be adjacent (10.4).
    fn adj_ok(&mut self, now: SimTime, i: usize, id: Ipv4Addr, out: &mut Vec<Tx>) {
        let Some(f) = self.ifs.get(&i) else { return };
        let Some(n) = f.nbrs.get(&id) else { return };
        let want = f.adjacent_wanted(n);
        match (n.state, want) {
            (NbrState::TwoWay, true) => self.start_exstart(now, i, id, out),
            (s, false) if s >= NbrState::ExStart => {
                let n = self.nbr_mut(i, &id);
                n.state = NbrState::TwoWay;
                n.clear_exchange();
                self.reoriginate = true;
            }
            _ => {}
        }
    }

    fn nbr_mut(&mut self, i: usize, id: &Ipv4Addr) -> &mut Nbr {
        self.ifs
            .get_mut(&i)
            .and_then(|f| f.nbrs.get_mut(id))
            .expect("neighbour")
    }

    /// Begin (or restart) database exchange: claim to be master with an empty packet.
    fn start_exstart(&mut self, now: SimTime, i: usize, id: Ipv4Addr, out: &mut Vec<Tx>) {
        self.dd_counter = self.dd_counter.wrapping_add(0x100);
        let seq = self.dd_counter;
        let mtu = self.ifs[&i].cfg.mtu();
        let was_full = self.nbr_mut(i, &id).state == NbrState::Full;
        let n = self.nbr_mut(i, &id);
        n.state = NbrState::ExStart;
        n.clear_exchange();
        n.master = true;
        n.dd_seq = seq;
        let d = Dbd {
            mtu,
            options: OPT_E,
            flags: DBD_INIT | DBD_MORE | DBD_MASTER,
            seq,
            headers: Vec::new(),
        };
        n.last_sent = Some(d.clone());
        n.next_rxmt = now + RXMT_INTERVAL;
        if was_full {
            self.reoriginate = true;
        }
        let tx = self.to_nbr(i, &self.ifs[&i].nbrs[&id], OspfBody::Dbd(d));
        out.push(tx);
    }

    /// The headers of everything in the database, for the neighbour's summary list. MaxAge
    /// LSAs go on its retransmission list instead (10.3).
    fn summarise(&mut self, now: SimTime, i: usize, id: &Ipv4Addr) {
        let area = self.ifs[&i].area;
        let mut summary = Vec::new();
        let mut retrans = Vec::new();
        for (k, e) in &self.db {
            // The neighbour's area and the LSAs that belong to the whole AS.
            if k.0 != area && k.0 != AS_SCOPE {
                continue;
            }
            let h = e.header_at(now);
            if h.age >= MAX_AGE {
                retrans.push(plain(k));
            } else {
                summary.push(h);
            }
        }
        let n = self.nbr_mut(i, id);
        n.summary = summary;
        n.retrans.extend(retrans);
    }

    /// Send the next Database Description packet (the next chunk of the summary list).
    fn send_dbd(&mut self, now: SimTime, i: usize, id: &Ipv4Addr, out: &mut Vec<Tx>) {
        let mtu = self.ifs[&i].cfg.mtu();
        let n = self.nbr_mut(i, id);
        let take = n.summary.len().min(PER_PACKET);
        let headers: Vec<LsaHeader> = n.summary.drain(..take).collect();
        let mut flags = 0;
        if n.master {
            flags |= DBD_MASTER;
        }
        if !n.summary.is_empty() {
            flags |= DBD_MORE;
        }
        let d = Dbd {
            mtu,
            options: OPT_E,
            flags,
            seq: n.dd_seq,
            headers,
        };
        n.last_sent = Some(d.clone());
        n.next_rxmt = now + RXMT_INTERVAL;
        let tx = self.to_nbr(i, &self.ifs[&i].nbrs[id], OspfBody::Dbd(d));
        out.push(tx);
    }

    /// The neighbour's headers: ask for what is missing or older here. False if one is not an
    /// LSA type we know.
    fn take_headers(
        &mut self,
        now: SimTime,
        i: usize,
        id: &Ipv4Addr,
        headers: &[LsaHeader],
    ) -> bool {
        let area = self.ifs[&i].area;
        for h in headers {
            if !(1..=5).contains(&h.ls_type) {
                return false;
            }
            let ours = self
                .db
                .get(&akey(area, key_of(h)))
                .map(|e| e.header_at(now));
            if ours.is_none_or(|o| compare(h, &o) == Ordering::Greater) {
                let n = self.nbr_mut(i, id);
                match n.request.iter().position(|r| key_of(r) == key_of(h)) {
                    Some(p) => {
                        if compare(h, &n.request[p]) == Ordering::Greater {
                            n.request[p] = *h;
                        }
                    }
                    None => n.request.push(*h),
                }
            }
        }
        true
    }

    fn exchange_done(&mut self, now: SimTime, i: usize, id: &Ipv4Addr, out: &mut Vec<Tx>) {
        let n = self.nbr_mut(i, id);
        if n.request.is_empty() {
            n.state = NbrState::Full;
            self.reoriginate = true;
        } else {
            n.state = NbrState::Loading;
            n.next_rxmt = now + RXMT_INTERVAL;
            self.send_requests(i, id, out);
        }
    }

    fn check_loading(&mut self, i: usize, id: &Ipv4Addr) {
        let n = self.nbr_mut(i, id);
        if n.state == NbrState::Loading && n.request.is_empty() {
            n.state = NbrState::Full;
            self.reoriginate = true;
        }
    }

    fn send_requests(&mut self, i: usize, id: &Ipv4Addr, out: &mut Vec<Tx>) {
        let n = &self.ifs[&i].nbrs[id];
        for chunk in n.request.chunks(PER_PACKET) {
            let reqs = chunk
                .iter()
                .map(|h| LsRequest {
                    ls_type: h.ls_type as u32,
                    id: h.id,
                    adv_router: h.adv_router,
                })
                .collect();
            let tx = self.to_nbr(i, n, OspfBody::Request(reqs));
            out.push(tx);
        }
    }

    /// Something went wrong in the exchange: start it again.
    fn seq_mismatch(&mut self, now: SimTime, i: usize, id: Ipv4Addr, out: &mut Vec<Tx>) {
        self.start_exstart(now, i, id, out);
    }

    fn on_dbd(&mut self, now: SimTime, i: usize, id: Ipv4Addr, d: Dbd, out: &mut Vec<Tx>) {
        let rid = self.rid.expect("router id");
        let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&id)) else {
            return;
        };
        // A neighbour whose interface MTU is larger than ours cannot be exchanged with: it is
        // ignored and the exchange never gets past ExStart (10.6).
        let cfg = self.ifs[&i].cfg;
        if d.mtu > cfg.mtu() && !cfg.mtu_ignore {
            return;
        }
        match n.state {
            NbrState::Down | NbrState::Init | NbrState::TwoWay => {}
            NbrState::ExStart => {
                let claims_master = d.flags & (DBD_INIT | DBD_MORE | DBD_MASTER)
                    == (DBD_INIT | DBD_MORE | DBD_MASTER)
                    && d.headers.is_empty()
                    && id > rid;
                let answers_us =
                    d.flags & (DBD_INIT | DBD_MASTER) == 0 && d.seq == n.dd_seq && id < rid;
                if claims_master {
                    // The neighbour is master; we echo its sequence number.
                    let n = self.nbr_mut(i, &id);
                    n.master = false;
                    n.dd_seq = d.seq;
                    n.state = NbrState::Exchange;
                    n.last_rcvd = Some((d.flags, d.options, d.seq));
                    self.summarise(now, i, &id);
                    self.send_dbd(now, i, &id, out);
                } else if answers_us {
                    // We are master and this is the slave's first answer.
                    self.nbr_mut(i, &id).state = NbrState::Exchange;
                    self.summarise(now, i, &id);
                    self.exchange_packet(now, i, id, d, out);
                }
            }
            NbrState::Exchange => self.exchange_packet(now, i, id, d, out),
            NbrState::Loading | NbrState::Full => {
                // Only a repeat of the last packet is expected; a slave answers it again.
                if n.last_rcvd == Some((d.flags, d.options, d.seq)) {
                    if !n.master {
                        if let Some(last) = n.last_sent.clone() {
                            let tx = self.to_nbr(i, n, OspfBody::Dbd(last));
                            out.push(tx);
                        }
                    }
                } else {
                    self.seq_mismatch(now, i, id, out);
                }
            }
        }
    }

    /// A Database Description packet for a neighbour in Exchange (10.6).
    fn exchange_packet(&mut self, now: SimTime, i: usize, id: Ipv4Addr, d: Dbd, out: &mut Vec<Tx>) {
        let n = &self.ifs[&i].nbrs[&id];
        let key = (d.flags, d.options, d.seq);
        if n.last_rcvd == Some(key) {
            if !n.master {
                if let Some(last) = n.last_sent.clone() {
                    let tx = self.to_nbr(i, n, OspfBody::Dbd(last));
                    out.push(tx);
                }
            }
            return;
        }
        let sender_is_master = d.flags & DBD_MASTER != 0;
        let expected = if n.master {
            n.dd_seq
        } else {
            n.dd_seq.wrapping_add(1)
        };
        if sender_is_master == n.master || d.flags & DBD_INIT != 0 || d.seq != expected {
            self.seq_mismatch(now, i, id, out);
            return;
        }
        if !self.take_headers(now, i, &id, &d.headers) {
            self.seq_mismatch(now, i, id, out);
            return;
        }
        let n = self.nbr_mut(i, &id);
        n.last_rcvd = Some(key);
        let they_more = d.flags & DBD_MORE != 0;
        if n.master {
            let we_more = n
                .last_sent
                .as_ref()
                .is_some_and(|l| l.flags & DBD_MORE != 0);
            if !they_more && !we_more {
                self.exchange_done(now, i, &id, out);
            } else {
                n.dd_seq = n.dd_seq.wrapping_add(1);
                self.send_dbd(now, i, &id, out);
            }
        } else {
            n.dd_seq = d.seq;
            self.send_dbd(now, i, &id, out);
            let n = &self.ifs[&i].nbrs[&id];
            let we_more = n
                .last_sent
                .as_ref()
                .is_some_and(|l| l.flags & DBD_MORE != 0);
            if !they_more && !we_more {
                self.exchange_done(now, i, &id, out);
            }
        }
    }

    fn on_request(
        &mut self,
        now: SimTime,
        i: usize,
        id: Ipv4Addr,
        reqs: Vec<LsRequest>,
        out: &mut Vec<Tx>,
    ) {
        let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&id)) else {
            return;
        };
        if n.state < NbrState::Exchange {
            return;
        }
        let area = self.ifs[&i].area;
        let mut keys = Vec::new();
        for r in &reqs {
            let k = (r.ls_type as u8, r.id, r.adv_router);
            if r.ls_type > 255 || !self.db.contains_key(&akey(area, k)) {
                // We do not have what it asked for: the exchange went wrong.
                self.seq_mismatch(now, i, id, out);
                return;
            }
            keys.push(k);
        }
        self.send_lsas(now, i, &id, &keys, out);
    }

    /// A copy of an LSA as it is sent: aged by the time it takes to cross the link.
    fn lsa_for_send(&self, now: SimTime, k: &AKey) -> Option<Lsa> {
        let e = self.db.get(k)?;
        let mut l = e.lsa.clone();
        l.age = (e.age(now) as u32 + INF_TRANS_DELAY).min(MAX_AGE as u32) as u16;
        Some(l)
    }

    /// Send the LSAs to one neighbour, as many Updates as it takes.
    fn send_lsas(&self, now: SimTime, i: usize, id: &Ipv4Addr, keys: &[Key], out: &mut Vec<Tx>) {
        let n = &self.ifs[&i].nbrs[id];
        let area = self.ifs[&i].area;
        let mut batch: Vec<Lsa> = Vec::new();
        let mut size = 0;
        for k in keys {
            let Some(l) = self.lsa_for_send(now, &akey(area, *k)) else {
                continue;
            };
            let len = l.emit().len();
            if !batch.is_empty() && size + len > LSU_BUDGET {
                out.push(self.to_nbr(i, n, OspfBody::Update(std::mem::take(&mut batch))));
                size = 0;
            }
            size += len;
            batch.push(l);
        }
        if !batch.is_empty() {
            out.push(self.to_nbr(i, n, OspfBody::Update(batch)));
        }
    }

    fn retransmit_lsas(&mut self, now: SimTime, i: usize, id: &Ipv4Addr, out: &mut Vec<Tx>) {
        let area = self.ifs[&i].area;
        let n = self.nbr_mut(i, id);
        // Anything that has left the database needs no more sending.
        let keys: Vec<Key> = n.retrans.iter().copied().collect();
        let gone: Vec<Key> = keys
            .iter()
            .filter(|k| !self.db.contains_key(&akey(area, **k)))
            .copied()
            .collect();
        let n = self.nbr_mut(i, id);
        for k in &gone {
            n.retrans.remove(k);
        }
        let keys: Vec<Key> = keys.into_iter().filter(|k| !gone.contains(k)).collect();
        self.send_lsas(now, i, id, &keys, out);
    }

    // ---- flooding ----

    fn on_update(
        &mut self,
        now: SimTime,
        i: usize,
        id: Ipv4Addr,
        lsas: Vec<Lsa>,
        out: &mut Vec<Tx>,
    ) {
        let rid = self.rid.expect("router id");
        let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&id)) else {
            return;
        };
        if n.state < NbrState::Exchange {
            return;
        }
        let area = self.ifs[&i].area;
        let my_addrs: Vec<Ipv4Addr> = self.ifs.values().map(|f| f.addr).collect();
        let mut acks: Vec<LsaHeader> = Vec::new();
        for lsa in lsas {
            let h = lsa.header();
            if !(1..=5).contains(&h.ls_type) {
                continue;
            }
            let k = key_of(&h);
            let ak = akey(area, k);
            let ours = self.db.get(&ak).map(|e| e.header_at(now));
            let exchanging = self.ifs.values().any(|f| {
                f.nbrs
                    .values()
                    .any(|n| matches!(n.state, NbrState::Exchange | NbrState::Loading))
            });
            if h.age == MAX_AGE && ours.is_none() && !exchanging {
                acks.push(h);
                continue;
            }
            match ours.map(|o| compare(&h, &o)) {
                None | Some(Ordering::Greater) => {
                    let mine = h.adv_router == rid
                        || (h.ls_type == LSA_NETWORK && my_addrs.contains(&h.id));
                    if mine {
                        // Someone holds a newer copy of our own LSA (13.4).
                        let wanted = self.desired().contains_key(&ak);
                        if wanted {
                            let floor = self.seq_floor.entry(ak).or_insert(h.seq);
                            *floor = (*floor).max(h.seq);
                            self.reoriginate = true;
                            acks.push(h);
                            continue;
                        }
                    }
                    for f in self.ifs.values_mut() {
                        for n in f.nbrs.values_mut() {
                            n.retrans.remove(&k);
                        }
                    }
                    self.install(now, area, lsa);
                    self.flood(now, ak, Some((i, id)), out);
                    // Whoever sent it no longer needs to be asked for it.
                    for f in self.ifs.values_mut() {
                        for n in f.nbrs.values_mut() {
                            n.request
                                .retain(|r| key_of(r) != k || compare(&h, r) == Ordering::Less);
                        }
                    }
                    acks.push(h);
                    if mine {
                        // Not wanted any more: take it out of the network.
                        self.premature_age(now, ak, out);
                    }
                }
                Some(ord) => {
                    let n = &self.ifs[&i].nbrs[&id];
                    if n.request.iter().any(|r| key_of(r) == k) {
                        // We asked for something newer than this: the exchange is confused.
                        self.seq_mismatch(now, i, id, out);
                        return;
                    }
                    if ord == Ordering::Equal {
                        let n = self.nbr_mut(i, &id);
                        if !n.retrans.remove(&k) {
                            acks.push(h);
                        }
                    } else if let Some(l) = self.lsa_for_send(now, &ak) {
                        // Ours is newer: tell the sender.
                        let n = &self.ifs[&i].nbrs[&id];
                        out.push(self.to_nbr(i, n, OspfBody::Update(vec![l])));
                    }
                }
            }
        }
        if !acks.is_empty() {
            let n = &self.ifs[&i].nbrs[&id];
            for chunk in acks.chunks(PER_PACKET) {
                out.push(self.to_nbr(i, n, OspfBody::Ack(chunk.to_vec())));
            }
        }
        self.check_loading(i, &id);
    }

    fn on_ack(&mut self, now: SimTime, i: usize, id: Ipv4Addr, headers: Vec<LsaHeader>) {
        let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&id)) else {
            return;
        };
        if n.state < NbrState::Exchange {
            return;
        }
        let area = self.ifs[&i].area;
        for h in headers {
            let k = key_of(&h);
            let same = self
                .db
                .get(&akey(area, k))
                .is_some_and(|e| e.header_at(now).seq == h.seq && e.header.checksum == h.checksum);
            if same {
                self.nbr_mut(i, &id).retrans.remove(&k);
            }
        }
    }

    /// Put an LSA in the database and schedule a new shortest path calculation.
    fn install(&mut self, now: SimTime, area: u32, lsa: Lsa) {
        let e = DbEntry::new(lsa, now);
        self.db.insert(akey(area, key_of(&e.header)), e);
        self.spf_at.get_or_insert(now + SPF_DELAY);
    }

    /// Flood the database copy of `k` out of the interfaces it should go out of (13.3). `from`
    /// is where it came from, if it was received. Returns whether it went back out of the
    /// interface it arrived on.
    fn flood(
        &mut self,
        now: SimTime,
        ak: AKey,
        from: Option<(usize, Ipv4Addr)>,
        out: &mut Vec<Tx>,
    ) -> bool {
        let Some(e) = self.db.get(&ak) else {
            return false;
        };
        let k = plain(&ak);
        let new_h = e.header_at(now);
        let mut back = false;
        let ifaces: Vec<usize> = self.ifs.keys().copied().collect();
        for i in ifaces {
            // An area's LSAs stay in the area; AS-external ones go everywhere.
            if self.ifs[&i].cfg.passive || (ak.0 != AS_SCOPE && self.ifs[&i].area != ak.0) {
                continue;
            }
            let mut added = false;
            let nbr_ids: Vec<Ipv4Addr> = self.ifs[&i].nbrs.keys().copied().collect();
            for nid in nbr_ids {
                let n = self.nbr_mut(i, &nid);
                if n.state < NbrState::Exchange {
                    continue;
                }
                if n.state < NbrState::Full {
                    if let Some(p) = n.request.iter().position(|r| key_of(r) == k) {
                        match compare(&new_h, &n.request[p]) {
                            Ordering::Less => continue,
                            Ordering::Equal => {
                                n.request.remove(p);
                                continue;
                            }
                            Ordering::Greater => {
                                n.request.remove(p);
                            }
                        }
                    }
                }
                if from == Some((i, nid)) {
                    continue;
                }
                n.retrans.insert(k);
                added = true;
            }
            if !added {
                continue;
            }
            let f = &self.ifs[&i];
            if let Some((fi, fid)) = from {
                if fi == i {
                    let sender = f.nbrs[&fid].ip;
                    if sender == f.dr || sender == f.bdr || f.state == IfState::Backup {
                        continue;
                    }
                    back = true;
                }
            }
            let dst = if f.cfg.point_to_point || matches!(f.state, IfState::Dr | IfState::Backup) {
                ALL_SPF_ROUTERS
            } else {
                ALL_DR_ROUTERS
            };
            if let Some(l) = self.lsa_for_send(now, &ak) {
                out.push(self.to_group(i, dst, OspfBody::Update(vec![l])));
            }
        }
        back
    }

    // ---- origination ----

    /// The LSAs this router should have in the network right now.
    fn desired(&self) -> BTreeMap<AKey, LsaBody> {
        let rid = self.rid.expect("router id");
        let mut want: BTreeMap<AKey, LsaBody> = BTreeMap::new();
        // One router LSA per area, listing the links of the interfaces in that area.
        let mut links: BTreeMap<u32, Vec<RouterLink>> = BTreeMap::new();
        for f in self.ifs.values() {
            let ls = links.entry(f.area).or_default();
            let cost = self.cost_of(&f.cfg);
            let stub = RouterLink {
                id: f.net(),
                data: Ipv4Addr::from_u32(prefix_mask(f.len)),
                kind: LINK_STUB,
                metric: cost.min(0xffff) as u16,
            };
            if f.cfg.passive {
                ls.push(stub);
                continue;
            }
            let full: Vec<&Nbr> = f
                .nbrs
                .values()
                .filter(|n| n.state == NbrState::Full)
                .collect();
            if f.cfg.point_to_point {
                for n in &full {
                    ls.push(RouterLink {
                        id: n.id,
                        data: f.addr,
                        kind: LINK_P2P,
                        metric: cost.min(0xffff) as u16,
                    });
                }
                ls.push(stub);
                continue;
            }
            let transit = f.state != IfState::Waiting
                && f.dr != Ipv4Addr::UNSPECIFIED
                && ((f.state == IfState::Dr && !full.is_empty())
                    || full.iter().any(|n| n.ip == f.dr));
            if transit {
                ls.push(RouterLink {
                    id: f.dr,
                    data: f.addr,
                    kind: LINK_TRANSIT,
                    metric: cost.min(0xffff) as u16,
                });
            } else {
                ls.push(stub);
            }
            if f.state == IfState::Dr && !full.is_empty() {
                let mut routers: Vec<Ipv4Addr> = full.iter().map(|n| n.id).collect();
                routers.push(rid);
                routers.sort();
                want.insert(
                    (f.area, LSA_NETWORK, f.addr, rid),
                    LsaBody::Network {
                        mask: Ipv4Addr::from_u32(prefix_mask(f.len)),
                        routers,
                    },
                );
            }
        }
        let flags = if self.is_abr() { ROUTER_B } else { 0 }
            | if self.default_active() { ROUTER_E } else { 0 };
        for (area, links) in links {
            want.insert(
                (area, LSA_ROUTER, rid, rid),
                LsaBody::Router { flags, links },
            );
        }
        // What an area border router tells each area about the others.
        for (k, b) in &self.summaries {
            want.insert(*k, b.clone());
        }
        if let (Some(d), true) = (self.default_info, self.default_active()) {
            want.insert(
                (AS_SCOPE, LSA_EXTERNAL, Ipv4Addr::UNSPECIFIED, rid),
                LsaBody::External {
                    mask: Ipv4Addr::UNSPECIFIED,
                    e2: d.e2,
                    metric: d.metric,
                    forwarding: Ipv4Addr::UNSPECIFIED,
                    tag: 0,
                },
            );
        }
        want
    }

    /// Originate what changed and flush what is no longer wanted.
    fn originate(&mut self, now: SimTime, out: &mut Vec<Tx>) {
        let rid = self.rid.expect("router id");
        let want = self.desired();
        for (k, body) in &want {
            let cur = self.db.get(k);
            if cur.is_some_and(|e| e.age(now) < MAX_AGE && &e.lsa.body == body) {
                continue;
            }
            self.new_instance(now, *k, body.clone(), out);
        }
        let stale: Vec<AKey> = self
            .db
            .iter()
            .filter(|(k, e)| k.3 == rid && !want.contains_key(k) && e.age(now) < MAX_AGE)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            self.premature_age(now, k, out);
        }
    }

    /// Install and flood a new instance of one of our LSAs.
    fn new_instance(&mut self, now: SimTime, k: AKey, body: LsaBody, out: &mut Vec<Tx>) {
        let prev = self.db.get(&k).map(|e| e.lsa.seq);
        let floor = self.seq_floor.remove(&k);
        let seq = match (prev, floor) {
            (None, None) => INITIAL_SEQ,
            (a, b) => a.into_iter().chain(b).max().expect("one").saturating_add(1),
        };
        let lsa = Lsa {
            age: 0,
            options: OPT_E,
            id: k.2,
            adv_router: k.3,
            seq,
            body,
        };
        self.install(now, k.0, lsa);
        self.flood(now, k, None, out);
    }

    fn force_refresh(&mut self, now: SimTime, k: AKey, out: &mut Vec<Tx>) {
        if let Some(body) = self.db.get(&k).map(|e| e.lsa.body.clone()) {
            self.new_instance(now, k, body, out);
        }
    }

    /// Age an LSA to MaxAge and flood it, to withdraw it from the network (14.1).
    fn premature_age(&mut self, now: SimTime, k: AKey, out: &mut Vec<Tx>) {
        let Some(e) = self.db.get_mut(&k) else { return };
        e.lsa.age = MAX_AGE;
        e.installed = now;
        e.flushed = true;
        self.spf_at.get_or_insert(now + SPF_DELAY);
        self.flood(now, k, None, out);
    }

    fn housekeeping(&mut self, now: SimTime, out: &mut Vec<Tx>) {
        if self.reoriginate {
            self.reoriginate = false;
            self.originate(now, out);
        }
        if self.spf_at.is_some_and(|t| now >= t) {
            self.spf_at = None;
            self.run_spf(now);
            // The new routes may change what a border router summarises.
            if self.reoriginate {
                self.reoriginate = false;
                self.originate(now, out);
            }
        }
    }
}

// ---- shortest path first ----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Vertex {
    Router(Ipv4Addr),
    Network(Ipv4Addr),
}

fn links_of(l: &Lsa) -> &[RouterLink] {
    match &l.body {
        LsaBody::Router { links, .. } => links,
        _ => &[],
    }
}

/// Where traffic to a vertex leaves: our interface and the next router (none if the vertex is
/// on a directly attached network).
type Hop = (usize, Option<Ipv4Addr>);

/// One destination in the routing table. `hop` is `None` for our own stub networks, which are
/// connected but still summarised to other areas.
#[derive(Clone, Copy, Debug)]
struct Path {
    net: Ipv4Addr,
    len: u8,
    cost: u32,
    /// Distance to the boundary router, for type 2 externals.
    tie: u32,
    kind: PathKind,
    area: u32,
    hop: Option<Hop>,
}

impl Path {
    fn rank(&self) -> (PathKind, u32, u32, u32) {
        (self.kind, self.cost, self.tie, self.area)
    }
}

/// How to reach an AS boundary router.
#[derive(Clone, Copy, Debug)]
struct Asbr {
    cost: u32,
    kind: PathKind,
    area: u32,
    hop: Hop,
}

/// What the calculation over one area found.
struct AreaTree {
    /// Routers reached: distance and first hop.
    reach: BTreeMap<Ipv4Addr, (u32, Hop)>,
    paths: Vec<Path>,
    asbrs: BTreeMap<Ipv4Addr, Asbr>,
}

impl Ospf {
    fn iface_with_addr(&self, a: Ipv4Addr) -> Option<usize> {
        self.ifs.values().find(|f| f.addr == a).map(|f| f.index)
    }

    /// The calculation of 16.1 over the router and network LSAs of one area: the routers
    /// reached, the routes to the stub and transit networks, and the boundary routers.
    fn spf_area(&self, now: SimTime, area: u32) -> AreaTree {
        let rid = self.rid.expect("router id");
        let live: BTreeMap<Key, &Lsa> = self
            .db
            .iter()
            .filter(|(k, e)| k.0 == area && e.age(now) < MAX_AGE)
            .map(|(k, e)| (plain(k), &e.lsa))
            .collect();
        let router = |id: Ipv4Addr| live.get(&(LSA_ROUTER, id, id)).copied();
        let nets: BTreeMap<Ipv4Addr, &Lsa> = live
            .iter()
            .filter(|(k, _)| k.0 == LSA_NETWORK)
            .map(|(k, l)| (k.1, *l))
            .collect();
        let attached = |n: Ipv4Addr| -> &[Ipv4Addr] {
            match nets.get(&n).map(|l| &l.body) {
                Some(LsaBody::Network { routers, .. }) => routers,
                _ => &[],
            }
        };

        let root = Vertex::Router(rid);
        let mut dist: BTreeMap<Vertex, (u32, Option<Hop>)> = BTreeMap::new();
        let mut done: BTreeSet<Vertex> = BTreeSet::new();
        let mut heap = BinaryHeap::new();
        dist.insert(root, (0, None));
        heap.push(Reverse((0u32, root)));
        while let Some(Reverse((d, v))) = heap.pop() {
            if done.contains(&v) || dist[&v].0 != d {
                continue;
            }
            done.insert(v);
            let hop_v = dist[&v].1;
            // (vertex reached, its distance, how to get there)
            let mut found: Vec<(Vertex, u32, Hop)> = Vec::new();
            match v {
                Vertex::Router(r) => {
                    let Some(l) = router(r) else { continue };
                    for link in links_of(l) {
                        let (w, hop) = match link.kind {
                            LINK_P2P => {
                                let Some(wl) = router(link.id) else { continue };
                                let back = links_of(wl)
                                    .iter()
                                    .find(|b| b.kind == LINK_P2P && b.id == r);
                                let Some(back) = back else { continue };
                                let hop = if r == rid {
                                    let Some(i) = self.iface_with_addr(link.data) else {
                                        continue;
                                    };
                                    (i, Some(back.data))
                                } else {
                                    let Some(h) = hop_v else { continue };
                                    h
                                };
                                (Vertex::Router(link.id), hop)
                            }
                            LINK_TRANSIT => {
                                if !attached(link.id).contains(&r) {
                                    continue;
                                }
                                let hop = if r == rid {
                                    let Some(i) = self.iface_with_addr(link.data) else {
                                        continue;
                                    };
                                    (i, None)
                                } else {
                                    let Some(h) = hop_v else { continue };
                                    h
                                };
                                (Vertex::Network(link.id), hop)
                            }
                            _ => continue,
                        };
                        found.push((w, d + link.metric as u32, hop));
                    }
                }
                Vertex::Network(n) => {
                    for &x in attached(n) {
                        if x == rid {
                            continue;
                        }
                        let Some(xl) = router(x) else { continue };
                        let back = links_of(xl)
                            .iter()
                            .find(|b| b.kind == LINK_TRANSIT && b.id == n);
                        let Some(back) = back else { continue };
                        let Some(h) = hop_v else { continue };
                        // Directly attached network: the router's own address is the next hop.
                        let hop = if h.1.is_none() {
                            (h.0, Some(back.data))
                        } else {
                            h
                        };
                        found.push((Vertex::Router(x), d, hop));
                    }
                }
            }
            for (w, nd, hop) in found {
                if done.contains(&w) {
                    continue;
                }
                if dist.get(&w).is_none_or(|(od, _)| nd < *od) {
                    dist.insert(w, (nd, Some(hop)));
                    heap.push(Reverse((nd, w)));
                }
            }
        }

        let mut tree = AreaTree {
            reach: BTreeMap::new(),
            paths: Vec::new(),
            asbrs: BTreeMap::new(),
        };
        let mut push = |net: Ipv4Addr, mask: Ipv4Addr, cost: u32, hop: Option<Hop>| {
            let len = mask.to_u32().leading_ones() as u8;
            tree.paths.push(Path {
                net: Ipv4Addr::from_u32(net.to_u32() & prefix_mask(len)),
                len,
                cost,
                tie: 0,
                kind: PathKind::Intra,
                area,
                hop,
            });
        };
        // Our own stub networks are connected: no next hop, but still part of the area.
        for link in router(rid).map(links_of).unwrap_or(&[]) {
            if link.kind == LINK_STUB {
                push(link.id, link.data, link.metric as u32, None);
            }
        }
        for (v, (d, hop)) in &dist {
            let Some(hop) = *hop else { continue };
            match v {
                Vertex::Router(r) => {
                    let Some(l) = router(*r) else { continue };
                    for link in links_of(l) {
                        if link.kind == LINK_STUB {
                            push(link.id, link.data, d + link.metric as u32, Some(hop));
                        }
                    }
                    tree.reach.insert(*r, (*d, hop));
                    if matches!(&l.body, LsaBody::Router { flags, .. } if flags & ROUTER_E != 0) {
                        tree.asbrs.insert(
                            *r,
                            Asbr {
                                cost: *d,
                                kind: PathKind::Intra,
                                area,
                                hop,
                            },
                        );
                    }
                }
                Vertex::Network(n) => {
                    if let Some(LsaBody::Network { mask, .. }) = nets.get(n).map(|l| &l.body) {
                        push(*n, *mask, *d, Some(hop));
                    }
                }
            }
        }
        tree
    }

    /// All the routing table calculations of section 16: intra-area paths for every area we are
    /// in, then inter-area paths from summary LSAs, then external routes. Also decides what an
    /// area border router should summarise to its areas.
    fn run_spf(&mut self, now: SimTime) {
        let Some(rid) = self.rid else { return };
        let areas = self.areas();
        let abr = self.is_abr();
        let trees: Vec<(u32, AreaTree)> =
            areas.iter().map(|&a| (a, self.spf_area(now, a))).collect();

        let mut table: BTreeMap<(u32, u8), Path> = BTreeMap::new();
        let mut asbrs: BTreeMap<Ipv4Addr, Asbr> = BTreeMap::new();
        let offer = |table: &mut BTreeMap<(u32, u8), Path>, p: Path| {
            let slot = table.entry((p.net.to_u32(), p.len)).or_insert(p);
            if p.rank() < slot.rank() {
                *slot = p;
            }
        };
        let offer_asbr = |asbrs: &mut BTreeMap<Ipv4Addr, Asbr>, id: Ipv4Addr, a: Asbr| {
            let slot = asbrs.entry(id).or_insert(a);
            if (a.kind, a.cost, a.area) < (slot.kind, slot.cost, slot.area) {
                *slot = a;
            }
        };
        for (_, t) in &trees {
            for p in &t.paths {
                offer(&mut table, *p);
            }
            for (id, a) in &t.asbrs {
                offer_asbr(&mut asbrs, *id, *a);
            }
        }
        // 16.2: summaries from the area border routers we can reach. A border router looks at
        // the backbone's only.
        for (a, t) in &trees {
            if abr && *a != 0 {
                continue;
            }
            for (k, e) in &self.db {
                let LsaBody::Summary {
                    ls_type,
                    mask,
                    metric,
                } = &e.lsa.body
                else {
                    continue;
                };
                if k.0 != *a || k.3 == rid || *metric >= LS_INFINITY || e.age(now) >= MAX_AGE {
                    continue;
                }
                let Some((d, hop)) = t.reach.get(&k.3) else {
                    continue;
                };
                if *ls_type == LSA_SUMMARY {
                    let len = mask.to_u32().leading_ones() as u8;
                    offer(
                        &mut table,
                        Path {
                            net: Ipv4Addr::from_u32(k.2.to_u32() & prefix_mask(len)),
                            len,
                            cost: d + metric,
                            tie: 0,
                            kind: PathKind::Inter,
                            area: *a,
                            hop: Some(*hop),
                        },
                    );
                } else {
                    offer_asbr(
                        &mut asbrs,
                        k.2,
                        Asbr {
                            cost: d + metric,
                            kind: PathKind::Inter,
                            area: *a,
                            hop: *hop,
                        },
                    );
                }
            }
        }
        // 16.4: routes to what other autonomous systems advertise.
        let internal: Vec<Path> = table.values().copied().collect();
        for (k, e) in &self.db {
            let LsaBody::External {
                mask,
                e2,
                metric,
                forwarding,
                ..
            } = &e.lsa.body
            else {
                continue;
            };
            if k.3 == rid || *metric >= LS_INFINITY || e.age(now) >= MAX_AGE {
                continue;
            }
            let (dist, hop, area) = if *forwarding == Ipv4Addr::UNSPECIFIED {
                match asbrs.get(&k.3) {
                    Some(a) => (a.cost, a.hop, a.area),
                    None => continue,
                }
            } else {
                // Traffic goes to the forwarding address, which we must be able to reach.
                let f = forwarding.to_u32();
                let best = internal
                    .iter()
                    .filter(|p| f & prefix_mask(p.len) == p.net.to_u32())
                    .max_by_key(|p| p.len);
                match best {
                    Some(p) => match p.hop {
                        Some((i, None)) => (p.cost, (i, Some(*forwarding)), p.area),
                        Some(h) => (p.cost, h, p.area),
                        None => continue,
                    },
                    None => continue,
                }
            };
            let len = mask.to_u32().leading_ones() as u8;
            offer(
                &mut table,
                Path {
                    net: Ipv4Addr::from_u32(k.2.to_u32() & prefix_mask(len)),
                    len,
                    cost: if *e2 { *metric } else { dist + metric },
                    tie: dist,
                    kind: if *e2 {
                        PathKind::External2
                    } else {
                        PathKind::External1
                    },
                    area,
                    hop: Some(hop),
                },
            );
        }

        // What an area border router tells each area about the rest (12.4.3).
        let mut summaries: BTreeMap<AKey, LsaBody> = BTreeMap::new();
        if abr {
            for &target in &areas {
                // A path goes into an area unless it was learned there. Intra-area paths go
                // everywhere else; inter-area ones only if they came through the backbone, and
                // not back into it.
                let advertise = |kind: PathKind, area: u32| match kind {
                    PathKind::Intra => area != target,
                    PathKind::Inter => area == 0 && target != 0,
                    _ => false,
                };
                for p in table.values() {
                    if advertise(p.kind, p.area) && p.cost < LS_INFINITY {
                        summaries.insert(
                            (target, LSA_SUMMARY, p.net, rid),
                            LsaBody::Summary {
                                ls_type: LSA_SUMMARY,
                                mask: Ipv4Addr::from_u32(prefix_mask(p.len)),
                                metric: p.cost,
                            },
                        );
                    }
                }
                for (id, a) in &asbrs {
                    if advertise(a.kind, a.area) && a.cost < LS_INFINITY {
                        summaries.insert(
                            (target, LSA_ASBR_SUMMARY, *id, rid),
                            LsaBody::Summary {
                                ls_type: LSA_ASBR_SUMMARY,
                                mask: Ipv4Addr::UNSPECIFIED,
                                metric: a.cost,
                            },
                        );
                    }
                }
            }
        }
        if summaries != self.summaries {
            self.summaries = summaries;
            self.reoriginate = true;
        }

        let routes: Vec<OspfRoute> = table
            .values()
            .filter_map(|p| match p.hop {
                Some((iface, Some(via))) => Some(OspfRoute {
                    net: p.net,
                    len: p.len,
                    cost: p.cost,
                    via,
                    iface,
                    kind: p.kind,
                    area: p.area,
                }),
                _ => None,
            })
            .collect();
        if routes != self.routes {
            self.routes = routes;
            self.routes_changed = true;
        }
    }
}
