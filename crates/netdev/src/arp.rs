use kernel::SimTime;
use std::collections::BTreeMap;
use wire::{Ipv4Addr, Ipv4Packet, MacAddr};

/// ARP cache plus the queue of packets waiting for resolution, shared by hosts and routers.
/// Expiry is lazy (checked on lookup) so it generates no timer events.
#[derive(Debug, Clone, Default)]
pub struct Resolver {
    cache: BTreeMap<Ipv4Addr, (MacAddr, SimTime)>,
    pending: BTreeMap<Ipv4Addr, Vec<Ipv4Packet>>,
}

impl Resolver {
    pub fn lookup(&self, ip: Ipv4Addr, now: SimTime) -> Option<MacAddr> {
        self.cache
            .get(&ip)
            .filter(|(_, exp)| *exp > now)
            .map(|(m, _)| *m)
    }

    pub fn learn(&mut self, ip: Ipv4Addr, mac: MacAddr, expires: SimTime) {
        self.cache.insert(ip, (mac, expires));
    }

    /// Queue `pkt` behind `hop`. Returns true if this is the first waiter, i.e. the caller
    /// should send an ARP request and arm the timeout.
    pub fn queue(&mut self, hop: Ipv4Addr, pkt: Ipv4Packet) -> bool {
        let first = !self.pending.contains_key(&hop);
        self.pending.entry(hop).or_default().push(pkt);
        first
    }

    pub fn take_pending(&mut self, hop: Ipv4Addr) -> Vec<Ipv4Packet> {
        self.pending.remove(&hop).unwrap_or_default()
    }

    /// Drop waiters for `hop`; true if there were any.
    pub fn timeout(&mut self, hop: Ipv4Addr) -> bool {
        self.pending.remove(&hop).is_some()
    }

    pub fn entries(&self, now: SimTime) -> Vec<(Ipv4Addr, MacAddr)> {
        self.cache
            .iter()
            .filter(|(_, (_, e))| *e > now)
            .map(|(i, (m, _))| (*i, *m))
            .collect()
    }
}
