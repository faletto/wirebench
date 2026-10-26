//! The DHCP server's state and decisions (RFC 2131 section 4.3), independent of any wire or
//! device: a message and the time go in, the reply (if any) comes out.

use kernel::SimTime;
use std::collections::{BTreeMap, BTreeSet};
use wire::{DhcpMessage, DhcpOp, DhcpType, Ipv4Addr};

/// How long an offered address is held while waiting for the client's request.
const OFFER_HOLD: SimTime = SimTime::from_secs(120);
const DAY: SimTime = SimTime::from_secs(24 * 3600);

/// One `ip dhcp pool`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub name: String,
    /// Addresses handed out and the mask given to clients (`network`).
    pub network: Option<(Ipv4Addr, u8)>,
    pub default_router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub lease: SimTime,
}

impl Pool {
    fn new(name: &str) -> Self {
        Pool {
            name: name.to_string(),
            network: None,
            default_router: None,
            dns: Vec::new(),
            lease: DAY,
        }
    }

    fn contains(&self, a: Ipv4Addr) -> bool {
        self.network.is_some_and(|(n, len)| {
            let m = crate::mask(len);
            a.to_u32() & m == n.to_u32() & m
        })
    }

    /// Candidate addresses in order: every host address of the network (not the network or
    /// broadcast address).
    fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> {
        let (lo, hi) = match self.network {
            Some((n, len)) if len < 31 => {
                let m = crate::mask(len);
                let base = n.to_u32() & m;
                (base + 1, (base | !m) - 1)
            }
            _ => (1, 0),
        };
        (lo..=hi).map(Ipv4Addr::from_u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingState {
    /// Offered, not yet requested.
    Offered,
    Bound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ip: Ipv4Addr,
    /// The client identifier (option 61), or `01` followed by the hardware address.
    pub client: Vec<u8>,
    pub expires: SimTime,
    pub state: BindingState,
}

#[derive(Clone, Debug)]
pub struct DhcpServer {
    /// `service dhcp` (on by default).
    pub enabled: bool,
    pools: Vec<Pool>,
    /// Inclusive ranges never handed out (`ip dhcp excluded-address`).
    excluded: Vec<(Ipv4Addr, Ipv4Addr)>,
    bindings: BTreeMap<Ipv4Addr, Binding>,
    /// Addresses a client reported as already in use. Not handed out again until cleared.
    conflicts: BTreeSet<Ipv4Addr>,
}

impl Default for DhcpServer {
    fn default() -> Self {
        DhcpServer {
            enabled: true,
            pools: Vec::new(),
            excluded: Vec::new(),
            bindings: BTreeMap::new(),
            conflicts: BTreeSet::new(),
        }
    }
}

/// The key a client is known by: option 61, or its hardware address.
pub fn client_key(m: &DhcpMessage) -> Vec<u8> {
    match &m.client_id {
        Some(c) => c.clone(),
        None => {
            let mut k = vec![1];
            k.extend_from_slice(&m.chaddr.0);
            k
        }
    }
}

impl DhcpServer {
    // ---- configuration ----

    pub fn pools(&self) -> &[Pool] {
        &self.pools
    }

    pub fn pool(&self, name: &str) -> Option<&Pool> {
        self.pools.iter().find(|p| p.name == name)
    }

    /// Find or create a pool.
    pub fn pool_mut(&mut self, name: &str) -> &mut Pool {
        match self.pools.iter().position(|p| p.name == name) {
            Some(i) => &mut self.pools[i],
            None => {
                self.pools.push(Pool::new(name));
                self.pools.last_mut().expect("just pushed")
            }
        }
    }

    pub fn remove_pool(&mut self, name: &str) -> bool {
        let before = self.pools.len();
        self.pools.retain(|p| p.name != name);
        self.pools.len() != before
    }

    pub fn excluded(&self) -> &[(Ipv4Addr, Ipv4Addr)] {
        &self.excluded
    }

    pub fn exclude(&mut self, low: Ipv4Addr, high: Ipv4Addr) {
        if !self.excluded.contains(&(low, high)) {
            self.excluded.push((low, high));
        }
    }

    pub fn include(&mut self, low: Ipv4Addr, high: Ipv4Addr) -> bool {
        let before = self.excluded.len();
        self.excluded.retain(|e| *e != (low, high));
        self.excluded.len() != before
    }

    fn is_excluded(&self, a: Ipv4Addr) -> bool {
        self.excluded
            .iter()
            .any(|(lo, hi)| lo.to_u32() <= a.to_u32() && a.to_u32() <= hi.to_u32())
    }

    // ---- state ----

    /// Live bindings, by address. Expired ones are dropped first.
    pub fn bindings(&mut self, now: SimTime) -> Vec<Binding> {
        self.purge(now);
        self.bindings.values().cloned().collect()
    }

    pub fn conflicts(&self) -> Vec<Ipv4Addr> {
        self.conflicts.iter().copied().collect()
    }

    /// `clear ip dhcp binding`: one address, or all of them with `None`. Returns how many went.
    pub fn clear(&mut self, ip: Option<Ipv4Addr>) -> usize {
        let before = self.bindings.len();
        match ip {
            Some(a) => {
                self.bindings.remove(&a);
            }
            None => {
                self.bindings.clear();
                self.conflicts.clear();
            }
        }
        before - self.bindings.len()
    }

    /// Forget the lot (a reload); the configuration is kept by the caller.
    pub fn reset(&mut self) {
        *self = DhcpServer::default();
    }

    fn purge(&mut self, now: SimTime) {
        self.bindings.retain(|_, b| b.expires > now);
    }

    fn pool_for(&self, link: Ipv4Addr) -> Option<&Pool> {
        self.pools.iter().find(|p| p.contains(link))
    }

    /// Whether `ip` may be given to `client`: inside the pool, not excluded, not in conflict, not
    /// one of the server's own addresses, and not bound to someone else.
    fn available(&self, pool: &Pool, ip: Ipv4Addr, client: &[u8], reserved: &[Ipv4Addr]) -> bool {
        pool.hosts().any(|h| h == ip)
            && !self.is_excluded(ip)
            && !self.conflicts.contains(&ip)
            && !reserved.contains(&ip)
            && self.bindings.get(&ip).is_none_or(|b| b.client == client)
    }

    /// Handle one client message. `server_ip` is the address this server answers from (the
    /// receiving interface); `link` picks the pool: the relay's address (`giaddr`) or, for a
    /// directly attached client, the receiving interface's address. `reserved` lists addresses the
    /// device itself owns. Returns the reply, if any.
    pub fn serve(
        &mut self,
        now: SimTime,
        msg: &DhcpMessage,
        server_ip: Ipv4Addr,
        link: Ipv4Addr,
        reserved: &[Ipv4Addr],
    ) -> Option<DhcpMessage> {
        if !self.enabled || msg.op != DhcpOp::Request {
            return None;
        }
        self.purge(now);
        let client = client_key(msg);
        let t = msg.msg_type?;
        match t {
            DhcpType::Release => {
                if self
                    .bindings
                    .get(&msg.ciaddr)
                    .is_some_and(|b| b.client == client)
                {
                    self.bindings.remove(&msg.ciaddr);
                }
                None
            }
            DhcpType::Decline => {
                if let Some(ip) = msg.requested_ip {
                    if self.bindings.get(&ip).is_some_and(|b| b.client == client) {
                        self.bindings.remove(&ip);
                        self.conflicts.insert(ip);
                    }
                }
                None
            }
            DhcpType::Discover => {
                let pool = self.pool_for(link)?.clone();
                let ip = self.choose(&pool, msg, &client, reserved)?;
                self.bindings.insert(
                    ip,
                    Binding {
                        ip,
                        client,
                        expires: now + OFFER_HOLD,
                        state: BindingState::Offered,
                    },
                );
                Some(self.reply(
                    DhcpType::Offer,
                    msg,
                    &pool,
                    ip,
                    server_ip,
                    Ipv4Addr::UNSPECIFIED,
                ))
            }
            DhcpType::Request => self.on_request(now, msg, &client, server_ip, link, reserved),
            _ => None,
        }
    }

    /// Pick the address to offer: the client's existing binding, the one it asked for, or the
    /// lowest free one.
    fn choose(
        &self,
        pool: &Pool,
        msg: &DhcpMessage,
        client: &[u8],
        reserved: &[Ipv4Addr],
    ) -> Option<Ipv4Addr> {
        if let Some(b) = self
            .bindings
            .values()
            .find(|b| b.client == client && pool.contains(b.ip))
        {
            return Some(b.ip);
        }
        if let Some(r) = msg.requested_ip {
            if self.available(pool, r, client, reserved) {
                return Some(r);
            }
        }
        pool.hosts()
            .find(|h| self.available(pool, *h, client, reserved))
    }

    fn on_request(
        &mut self,
        now: SimTime,
        msg: &DhcpMessage,
        client: &[u8],
        server_ip: Ipv4Addr,
        link: Ipv4Addr,
        reserved: &[Ipv4Addr],
    ) -> Option<DhcpMessage> {
        let pool = self.pool_for(link)?.clone();
        let nak = |s: &Self| {
            s.reply(
                DhcpType::Nak,
                msg,
                &pool,
                Ipv4Addr::UNSPECIFIED,
                server_ip,
                Ipv4Addr::UNSPECIFIED,
            )
        };
        let unspecified = Ipv4Addr::UNSPECIFIED;
        let (wanted, selecting) = if let Some(sid) = msg.server_id {
            // SELECTING: the client picked a server. If it was not us, our offer is withdrawn.
            if sid != server_ip {
                self.bindings
                    .retain(|_, b| !(b.client == client && b.state == BindingState::Offered));
                return None;
            }
            (msg.requested_ip?, true)
        } else if msg.ciaddr != unspecified {
            // RENEWING or REBINDING.
            (msg.ciaddr, false)
        } else {
            // INIT-REBOOT.
            (msg.requested_ip?, false)
        };
        if !pool.contains(wanted) {
            return Some(nak(self));
        }
        let owned = self.bindings.get(&wanted).map(|b| b.client.as_slice());
        let ok = match owned {
            Some(c) if c == client => true,
            Some(_) => false,
            // Nobody holds it: only a client in SELECTING is told, and only if it is free to take.
            None if selecting => self.available(&pool, wanted, client, reserved),
            None => return None,
        };
        if !ok {
            return Some(nak(self));
        }
        self.bindings.insert(
            wanted,
            Binding {
                ip: wanted,
                client: client.to_vec(),
                expires: now + pool.lease,
                state: BindingState::Bound,
            },
        );
        Some(self.reply(DhcpType::Ack, msg, &pool, wanted, server_ip, msg.ciaddr))
    }

    fn reply(
        &self,
        kind: DhcpType,
        req: &DhcpMessage,
        pool: &Pool,
        yiaddr: Ipv4Addr,
        server_ip: Ipv4Addr,
        ciaddr: Ipv4Addr,
    ) -> DhcpMessage {
        let mut m = DhcpMessage {
            op: DhcpOp::Reply,
            hops: 0,
            xid: req.xid,
            secs: 0,
            flags: req.flags,
            ciaddr,
            yiaddr,
            siaddr: server_ip,
            giaddr: req.giaddr,
            chaddr: req.chaddr,
            msg_type: Some(kind),
            server_id: Some(server_ip),
            client_id: req.client_id.clone(),
            ..DhcpMessage::default()
        };
        if kind == DhcpType::Nak {
            m.message = Some(b"requested address not available".to_vec());
            return m;
        }
        let lease = pool.lease.0 / 1_000_000_000;
        let lease = u32::try_from(lease).unwrap_or(u32::MAX);
        m.lease_secs = Some(lease);
        m.t1_secs = Some(lease / 2);
        m.t2_secs = Some((u64::from(lease) * 7 / 8) as u32);
        m.subnet_mask = pool
            .network
            .map(|(_, len)| Ipv4Addr::from_u32(crate::mask(len)));
        m.routers = pool.default_router.into_iter().collect();
        m.dns = pool.dns.clone();
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::MacAddr;

    const SERVER: Ipv4Addr = Ipv4Addr([10, 0, 0, 1]);

    fn server() -> DhcpServer {
        let mut s = DhcpServer::default();
        let p = s.pool_mut("lan");
        p.network = Some((Ipv4Addr::new(10, 0, 0, 0), 24));
        p.default_router = Some(SERVER);
        p.dns = vec![Ipv4Addr::new(8, 8, 8, 8)];
        s.exclude(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 9));
        s
    }

    fn msg(kind: DhcpType, mac: u8) -> DhcpMessage {
        DhcpMessage {
            xid: u32::from(mac),
            chaddr: MacAddr([2, 0, 0, 0, 0, mac]),
            msg_type: Some(kind),
            ..DhcpMessage::default()
        }
    }

    fn t(secs: u64) -> SimTime {
        SimTime::from_secs(secs)
    }

    fn serve(s: &mut DhcpServer, now: u64, m: &DhcpMessage) -> Option<DhcpMessage> {
        s.serve(t(now), m, SERVER, SERVER, &[SERVER])
    }

    fn lease(s: &mut DhcpServer, mac: u8) -> Ipv4Addr {
        let offer = serve(s, 0, &msg(DhcpType::Discover, mac)).expect("an offer");
        let mut req = msg(DhcpType::Request, mac);
        req.server_id = Some(SERVER);
        req.requested_ip = Some(offer.yiaddr);
        let ack = serve(s, 1, &req).expect("an ack");
        assert_eq!(ack.msg_type, Some(DhcpType::Ack));
        ack.yiaddr
    }

    #[test]
    fn offers_the_lowest_free_address_skipping_the_excluded_range() {
        let mut s = server();
        let offer = serve(&mut s, 0, &msg(DhcpType::Discover, 1)).unwrap();
        assert_eq!(offer.msg_type, Some(DhcpType::Offer));
        assert_eq!(offer.yiaddr, Ipv4Addr::new(10, 0, 0, 10));
        assert_eq!(offer.server_id, Some(SERVER));
        assert_eq!(offer.subnet_mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(offer.routers, [SERVER]);
        assert_eq!(offer.dns, [Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(offer.lease_secs, Some(86_400));
        assert_eq!((offer.t1_secs, offer.t2_secs), (Some(43_200), Some(75_600)));
        assert_eq!(offer.xid, 1);
        assert_eq!(offer.op, DhcpOp::Reply);
        assert_eq!(
            offer.ciaddr,
            Ipv4Addr::UNSPECIFIED,
            "an offer has no client address yet"
        );
    }

    #[test]
    fn different_clients_get_different_addresses_and_the_same_client_keeps_its_own() {
        let mut s = server();
        let a = lease(&mut s, 1);
        let b = lease(&mut s, 2);
        assert_ne!(a, b);
        assert_eq!(
            lease(&mut s, 1),
            a,
            "a returning client gets its address back"
        );
    }

    #[test]
    fn a_requested_address_is_honoured_when_free_and_ignored_when_not() {
        let mut s = server();
        let mut d = msg(DhcpType::Discover, 1);
        d.requested_ip = Some(Ipv4Addr::new(10, 0, 0, 77));
        assert_eq!(
            serve(&mut s, 0, &d).unwrap().yiaddr,
            Ipv4Addr::new(10, 0, 0, 77)
        );
        d.requested_ip = Some(Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(
            serve(&mut s, 0, &d).unwrap().yiaddr,
            Ipv4Addr::new(10, 0, 0, 77),
            "an excluded address is not given even if asked for; the held offer is kept"
        );
        let mut other = msg(DhcpType::Discover, 2);
        other.requested_ip = Some(Ipv4Addr::new(10, 0, 0, 77));
        assert_eq!(
            serve(&mut s, 0, &other).unwrap().yiaddr,
            Ipv4Addr::new(10, 0, 0, 10),
            "77 is held for client 1"
        );
    }

    #[test]
    fn the_pool_runs_out() {
        let mut s = server();
        s.exclude(Ipv4Addr::new(10, 0, 0, 11), Ipv4Addr::new(10, 0, 0, 254));
        assert!(serve(&mut s, 0, &msg(DhcpType::Discover, 1)).is_some());
        assert!(
            serve(&mut s, 0, &msg(DhcpType::Discover, 2)).is_none(),
            "only .10 was left"
        );
    }

    #[test]
    fn requests_for_the_wrong_server_or_address_are_refused_or_ignored() {
        let mut s = server();
        let offer = serve(&mut s, 0, &msg(DhcpType::Discover, 1)).unwrap();
        // The client chose a different server: ours is withdrawn and we stay silent.
        let mut other = msg(DhcpType::Request, 1);
        other.server_id = Some(Ipv4Addr::new(10, 0, 0, 99));
        other.requested_ip = Some(offer.yiaddr);
        assert!(serve(&mut s, 1, &other).is_none());
        assert!(s.bindings(t(1)).is_empty());
        // Selecting us but asking for an address that is not in the pool: NAK.
        let mut bad = msg(DhcpType::Request, 1);
        bad.server_id = Some(SERVER);
        bad.requested_ip = Some(Ipv4Addr::new(192, 168, 0, 5));
        let nak = serve(&mut s, 1, &bad).unwrap();
        assert_eq!(nak.msg_type, Some(DhcpType::Nak));
        assert_eq!(nak.yiaddr, Ipv4Addr::UNSPECIFIED);
        assert!(nak.message.is_some());
    }

    #[test]
    fn a_client_cannot_take_an_address_bound_to_another() {
        let mut s = server();
        let a = lease(&mut s, 1);
        let mut req = msg(DhcpType::Request, 2);
        req.server_id = Some(SERVER);
        req.requested_ip = Some(a);
        assert_eq!(
            serve(&mut s, 2, &req).unwrap().msg_type,
            Some(DhcpType::Nak)
        );
    }

    #[test]
    fn renewing_extends_the_lease_and_init_reboot_checks_the_record() {
        let mut s = server();
        let a = lease(&mut s, 1);
        let mut renew = msg(DhcpType::Request, 1);
        renew.ciaddr = a;
        let ack = serve(&mut s, 40_000, &renew).unwrap();
        assert_eq!(
            (ack.msg_type, ack.yiaddr, ack.ciaddr),
            (Some(DhcpType::Ack), a, a)
        );
        assert_eq!(s.bindings(t(40_000))[0].expires, t(40_000 + 86_400));
        // INIT-REBOOT: address in the right network and on record: ack; unknown: silence.
        let mut reboot = msg(DhcpType::Request, 1);
        reboot.requested_ip = Some(a);
        assert_eq!(
            serve(&mut s, 40_001, &reboot).unwrap().msg_type,
            Some(DhcpType::Ack)
        );
        reboot.requested_ip = Some(Ipv4Addr::new(10, 0, 0, 200));
        assert!(serve(&mut s, 40_001, &reboot).is_none());
        reboot.requested_ip = Some(Ipv4Addr::new(172, 16, 0, 1));
        assert_eq!(
            serve(&mut s, 40_001, &reboot).unwrap().msg_type,
            Some(DhcpType::Nak)
        );
    }

    #[test]
    fn leases_and_offers_expire() {
        let mut s = server();
        let a = lease(&mut s, 1);
        assert_eq!(s.bindings(t(86_000)).len(), 1);
        assert!(s.bindings(t(1 + 86_400)).is_empty(), "lease ran out");
        assert_eq!(lease(&mut s, 2), a, "the address is free again");
        // An offer nobody takes is released after the hold time.
        let mut s = server();
        serve(&mut s, 0, &msg(DhcpType::Discover, 1)).unwrap();
        assert_eq!(s.bindings(t(119)).len(), 1);
        assert!(s.bindings(t(121)).is_empty());
    }

    #[test]
    fn release_and_decline() {
        let mut s = server();
        let a = lease(&mut s, 1);
        let mut rel = msg(DhcpType::Release, 2);
        rel.ciaddr = a;
        serve(&mut s, 5, &rel);
        assert_eq!(
            s.bindings(t(5)).len(),
            1,
            "someone else's release does nothing"
        );
        rel = msg(DhcpType::Release, 1);
        rel.ciaddr = a;
        assert!(serve(&mut s, 5, &rel).is_none());
        assert!(s.bindings(t(5)).is_empty());
        // A decline marks the address as in conflict and it is not offered again.
        let b = lease(&mut s, 1);
        let mut dec = msg(DhcpType::Decline, 1);
        dec.requested_ip = Some(b);
        serve(&mut s, 6, &dec);
        assert_eq!(s.conflicts(), [b]);
        assert_ne!(lease(&mut s, 1), b);
        s.clear(None);
        assert!(s.conflicts().is_empty());
    }

    #[test]
    fn the_servers_own_addresses_are_never_given_out_and_disabled_means_silent() {
        let mut s = server();
        s.include(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 9));
        let offer = s
            .serve(t(0), &msg(DhcpType::Discover, 1), SERVER, SERVER, &[SERVER])
            .unwrap();
        assert_eq!(
            offer.yiaddr,
            Ipv4Addr::new(10, 0, 0, 2),
            ".1 is the server itself"
        );
        s.enabled = false;
        assert!(serve(&mut s, 0, &msg(DhcpType::Discover, 2)).is_none());
    }

    #[test]
    fn a_relayed_request_is_matched_to_a_pool_by_giaddr() {
        let mut s = server();
        let relay = Ipv4Addr::new(10, 0, 0, 254);
        let mut d = msg(DhcpType::Discover, 1);
        d.giaddr = relay;
        let offer = s
            .serve(t(0), &d, Ipv4Addr::new(172, 16, 0, 1), relay, &[])
            .unwrap();
        assert_eq!(
            offer.giaddr, relay,
            "giaddr is echoed so the relay can deliver it"
        );
        assert_eq!(offer.yiaddr, Ipv4Addr::new(10, 0, 0, 10));
        // A link no pool covers gets nothing.
        assert!(s
            .serve(t(0), &d, SERVER, Ipv4Addr::new(192, 168, 9, 1), &[])
            .is_none());
    }

    #[test]
    fn client_identifiers_beat_hardware_addresses() {
        let mut s = server();
        let mut a = msg(DhcpType::Discover, 1);
        a.client_id = Some(vec![9, 9]);
        let mut b = msg(DhcpType::Discover, 2);
        b.client_id = Some(vec![9, 9]);
        let first = serve(&mut s, 0, &a).unwrap().yiaddr;
        assert_eq!(
            serve(&mut s, 0, &b).unwrap().yiaddr,
            first,
            "same identifier, same client"
        );
    }
}
