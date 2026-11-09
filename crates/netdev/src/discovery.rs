//! CDP and LLDP: each device tells its neighbours who it is and listens for theirs. Shared by
//! switches and routers; the device supplies its identity and the cables it has at each tick.

use kernel::SimTime;
use wire::{cdp_cap, lldp_cap, Cdp, Ipv4Addr, Lldp, MacAddr};

pub const DEFAULT_CDP_TIMER: u32 = 60;
pub const DEFAULT_CDP_HOLD: u32 = 180;
pub const DEFAULT_LLDP_TIMER: u32 = 30;
pub const DEFAULT_LLDP_HOLD: u32 = 120;

/// One port as the discovery protocols see it.
#[derive(Clone, Debug)]
pub struct PortInfo {
    /// `FastEthernet0/1`.
    pub long: String,
    /// `Fa0/1`.
    pub short: String,
    pub mac: MacAddr,
    /// Cabled, up and not shut: worth sending on.
    pub live: bool,
    /// The address to announce on this port, if it has one.
    pub addr: Option<Ipv4Addr>,
}

/// What a device says about itself.
#[derive(Clone, Debug)]
pub struct Identity {
    pub name: String,
    pub platform: String,
    pub software: String,
    /// CDP capability bits.
    pub caps: u32,
    pub ports: Vec<PortInfo>,
}

/// A device heard on a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neighbour {
    pub local_port: usize,
    pub device_id: String,
    pub port_id: String,
    pub platform: String,
    pub software: String,
    /// CDP capability bits (LLDP's are mapped onto them).
    pub caps: u32,
    pub addresses: Vec<Ipv4Addr>,
    /// For LLDP: the sender's chassis address.
    pub chassis: Option<MacAddr>,
    /// For LLDP: the port description.
    pub port_description: String,
    pub heard: SimTime,
    pub expires: SimTime,
}

#[derive(Clone, Debug)]
pub struct Discovery {
    pub cdp_run: bool,
    pub lldp_run: bool,
    pub cdp_timer: u32,
    pub cdp_hold: u32,
    pub lldp_timer: u32,
    pub lldp_hold: u32,
    /// Per port: `cdp enable`, `lldp transmit`, `lldp receive`.
    pub cdp_port: Vec<bool>,
    pub lldp_tx: Vec<bool>,
    pub lldp_rx: Vec<bool>,
    cdp: Vec<Neighbour>,
    lldp: Vec<Neighbour>,
    next_cdp: SimTime,
    next_lldp: SimTime,
    was_live: Vec<bool>,
    /// Frames sent and received, for `show cdp traffic`-style counters and tests.
    pub sent: [u64; 2],
    pub received: [u64; 2],
}

impl Discovery {
    pub fn new(n_ports: usize) -> Self {
        Discovery {
            // Off, unlike a real switch: a device with CDP running never lets the simulation go
            // idle, and every lab would send neighbour frames the learner did not ask for.
            cdp_run: false,
            lldp_run: false,
            cdp_timer: DEFAULT_CDP_TIMER,
            cdp_hold: DEFAULT_CDP_HOLD,
            lldp_timer: DEFAULT_LLDP_TIMER,
            lldp_hold: DEFAULT_LLDP_HOLD,
            cdp_port: vec![true; n_ports],
            lldp_tx: vec![true; n_ports],
            lldp_rx: vec![true; n_ports],
            cdp: vec![],
            lldp: vec![],
            next_cdp: SimTime::ZERO,
            next_lldp: SimTime::ZERO,
            was_live: vec![false; n_ports],
            sent: [0; 2],
            received: [0; 2],
        }
    }

    /// Whether anything needs the periodic tick.
    pub fn wants_tick(&self) -> bool {
        self.cdp_run || self.lldp_run
    }

    pub fn cdp_neighbours(&self) -> &[Neighbour] {
        &self.cdp
    }

    pub fn lldp_neighbours(&self) -> &[Neighbour] {
        &self.lldp
    }

    pub fn clear_cdp(&mut self) {
        self.cdp.clear();
    }

    pub fn clear_lldp(&mut self) {
        self.lldp.clear();
    }

    /// Turn the protocols on or off. Turning one off forgets its neighbours.
    pub fn set_cdp(&mut self, on: bool) {
        if on && !self.cdp_run {
            self.next_cdp = SimTime::ZERO;
        }
        self.cdp_run = on;
        if !on {
            self.cdp.clear();
        }
    }

    pub fn set_lldp(&mut self, on: bool) {
        if on && !self.lldp_run {
            self.next_lldp = SimTime::ZERO;
        }
        self.lldp_run = on;
        if !on {
            self.lldp.clear();
        }
    }

    /// True if any setting differs from a new device's, for the running config.
    pub fn port_changed(&self, p: usize) -> bool {
        !self.cdp_port[p] || !self.lldp_tx[p] || !self.lldp_rx[p]
    }

    /// Expire neighbours, forget those on dead links, and queue what is due: `(port, frame)`.
    pub fn tick(&mut self, now: SimTime, id: &Identity, out: &mut Vec<(usize, Vec<u8>)>) {
        let live: Vec<bool> = id.ports.iter().map(|p| p.live).collect();
        self.cdp
            .retain(|n| n.expires > now && live.get(n.local_port).copied().unwrap_or(false));
        self.lldp
            .retain(|n| n.expires > now && live.get(n.local_port).copied().unwrap_or(false));
        let cdp_due = now >= self.next_cdp;
        let lldp_due = now >= self.next_lldp;
        for (i, port) in id.ports.iter().enumerate() {
            let fresh = port.live && !self.was_live.get(i).copied().unwrap_or(false);
            if i < self.was_live.len() {
                self.was_live[i] = port.live;
            }
            if !port.live {
                continue;
            }
            if self.cdp_run && self.cdp_port.get(i).copied().unwrap_or(false) && (cdp_due || fresh)
            {
                let cdp = Cdp {
                    device_id: id.name.clone(),
                    port_id: port.long.clone(),
                    capabilities: id.caps,
                    platform: id.platform.clone(),
                    software: id.software.clone(),
                    addresses: port.addr.into_iter().collect(),
                    holdtime: self.cdp_hold.min(255) as u8,
                };
                out.push((i, cdp.emit(port.mac)));
                self.sent[0] += 1;
            }
            if self.lldp_run && self.lldp_tx.get(i).copied().unwrap_or(false) && (lldp_due || fresh)
            {
                let mut caps = 0;
                if id.caps & cdp_cap::SWITCH != 0 {
                    caps |= lldp_cap::BRIDGE;
                }
                if id.caps & cdp_cap::ROUTER != 0 {
                    caps |= lldp_cap::ROUTER;
                }
                let l = Lldp {
                    chassis: id.ports.first().map_or(MacAddr::default(), |p| p.mac),
                    port_id: port.short.clone(),
                    ttl: self.lldp_hold.min(65535) as u16,
                    port_description: port.long.clone(),
                    system_name: id.name.clone(),
                    system_description: id.platform.clone(),
                    capabilities: caps,
                    enabled: caps,
                    management: port.addr,
                };
                out.push((i, l.emit(port.mac)));
                self.sent[1] += 1;
            }
        }
        if cdp_due {
            self.next_cdp = now + SimTime::from_secs(u64::from(self.cdp_timer));
        }
        if lldp_due {
            self.next_lldp = now + SimTime::from_secs(u64::from(self.lldp_timer));
        }
    }

    /// Whether `frame` is one of ours to consume on this device (given the protocol is on).
    pub fn consumes(&self, frame: &[u8]) -> bool {
        (Cdp::is_cdp(frame) && self.cdp_run)
            || (frame.len() >= 14
                && frame[0..6] == wire::LLDP_MULTICAST.0
                && frame[12..14] == wire::LLDP_TYPE.to_be_bytes())
    }

    /// A frame heard on `port`.
    pub fn receive(&mut self, now: SimTime, port: usize, frame: &[u8]) {
        if Cdp::is_cdp(frame) {
            if !self.cdp_run || !self.cdp_port.get(port).copied().unwrap_or(false) {
                return;
            }
            let Ok((_, c)) = Cdp::parse(frame) else {
                return;
            };
            self.received[0] += 1;
            let n = Neighbour {
                local_port: port,
                device_id: c.device_id,
                port_id: c.port_id,
                platform: c.platform,
                software: c.software,
                caps: c.capabilities,
                addresses: c.addresses,
                chassis: None,
                port_description: String::new(),
                heard: now,
                expires: now + SimTime::from_secs(u64::from(c.holdtime)),
            };
            store(&mut self.cdp, n);
        } else if let Ok((_, l)) = Lldp::parse(frame) {
            if !self.lldp_run || !self.lldp_rx.get(port).copied().unwrap_or(false) {
                return;
            }
            self.received[1] += 1;
            // A TTL of zero is a goodbye: the neighbour is gone.
            if l.ttl == 0 {
                self.lldp
                    .retain(|n| !(n.local_port == port && n.chassis == Some(l.chassis)));
                return;
            }
            let mut caps = 0;
            if l.capabilities & lldp_cap::BRIDGE != 0 {
                caps |= cdp_cap::SWITCH;
            }
            if l.capabilities & lldp_cap::ROUTER != 0 {
                caps |= cdp_cap::ROUTER;
            }
            if l.capabilities & lldp_cap::STATION != 0 {
                caps |= cdp_cap::HOST;
            }
            let n = Neighbour {
                local_port: port,
                device_id: l.system_name,
                port_id: l.port_id,
                platform: l.system_description,
                software: String::new(),
                caps,
                addresses: l.management.into_iter().collect(),
                chassis: Some(l.chassis),
                port_description: l.port_description,
                heard: now,
                expires: now + SimTime::from_secs(u64::from(l.ttl)),
            };
            store(&mut self.lldp, n);
        }
    }
}

/// Keep one entry per (port, device, remote port); a refreshed advertisement replaces the old one
/// but keeps its place in the list.
fn store(list: &mut Vec<Neighbour>, n: Neighbour) {
    match list.iter_mut().find(|o| {
        o.local_port == n.local_port && o.device_id == n.device_id && o.port_id == n.port_id
    }) {
        Some(o) => *o = n,
        None => list.push(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str, live: bool) -> Identity {
        Identity {
            name: name.into(),
            platform: "wirebench test".into(),
            software: "0".into(),
            caps: cdp_cap::SWITCH,
            ports: vec![PortInfo {
                long: "FastEthernet0/1".into(),
                short: "Fa0/1".into(),
                mac: MacAddr([2, 0, 0, 0, 0, 1]),
                live,
                addr: Some(Ipv4Addr([10, 0, 0, 1])),
            }],
        }
    }

    #[test]
    fn a_fresh_link_is_announced_at_once_and_the_neighbour_is_learned() {
        let (mut a, mut b) = (Discovery::new(1), Discovery::new(1));
        a.set_cdp(true);
        b.set_cdp(true);
        let mut out = vec![];
        a.tick(SimTime::from_secs(5), &ident("a", true), &mut out);
        assert_eq!(out.len(), 1);
        b.receive(SimTime::from_secs(5), 0, &out[0].1);
        let n = &b.cdp_neighbours()[0];
        assert_eq!(
            (n.device_id.as_str(), n.port_id.as_str()),
            ("a", "FastEthernet0/1")
        );
        assert_eq!(n.expires, SimTime::from_secs(185));
        // Hearing it again refreshes, not duplicates.
        b.receive(SimTime::from_secs(65), 0, &out[0].1);
        assert_eq!(b.cdp_neighbours().len(), 1);
        assert_eq!(b.cdp_neighbours()[0].expires, SimTime::from_secs(245));
    }

    #[test]
    fn neighbours_expire_and_die_with_the_link() {
        let (mut a, mut b) = (Discovery::new(1), Discovery::new(1));
        a.set_cdp(true);
        b.set_cdp(true);
        let mut out = vec![];
        a.tick(SimTime::from_secs(5), &ident("a", true), &mut out);
        b.receive(SimTime::from_secs(5), 0, &out[0].1);
        let mut none = vec![];
        b.tick(SimTime::from_secs(100), &ident("b", true), &mut none);
        assert_eq!(b.cdp_neighbours().len(), 1);
        b.tick(SimTime::from_secs(101), &ident("b", false), &mut none);
        assert!(b.cdp_neighbours().is_empty(), "link down");
        b.receive(SimTime::from_secs(102), 0, &out[0].1);
        b.tick(SimTime::from_secs(300), &ident("b", true), &mut none);
        assert!(b.cdp_neighbours().is_empty(), "held 180 s, then gone");
    }

    #[test]
    fn lldp_is_off_until_asked_and_goodbye_removes() {
        let (mut a, mut b) = (Discovery::new(1), Discovery::new(1));
        a.set_cdp(true);
        let mut out = vec![];
        a.tick(SimTime::from_secs(1), &ident("a", true), &mut out);
        assert!(out.iter().all(|(_, f)| wire::Cdp::is_cdp(f)));
        let (mut a, mut b2) = (Discovery::new(1), Discovery::new(1));
        a.set_lldp(true);
        a.set_cdp(false);
        b2.set_lldp(true);
        let mut out = vec![];
        a.tick(SimTime::from_secs(1), &ident("a", true), &mut out);
        assert_eq!(out.len(), 1);
        b2.receive(SimTime::from_secs(1), 0, &out[0].1);
        assert_eq!(b2.lldp_neighbours().len(), 1);
        b.receive(SimTime::from_secs(1), 0, &out[0].1);
        assert!(b.lldp_neighbours().is_empty(), "b does not run LLDP");
        // A goodbye (ttl 0).
        let bye = wire::Lldp {
            chassis: MacAddr([2, 0, 0, 0, 0, 1]),
            port_id: "Fa0/1".into(),
            ttl: 0,
            ..Default::default()
        }
        .emit(MacAddr([2, 0, 0, 0, 0, 1]));
        b2.receive(SimTime::from_secs(2), 0, &bye);
        assert!(b2.lldp_neighbours().is_empty());
    }

    #[test]
    fn per_port_switches_are_respected() {
        let mut a = Discovery::new(1);
        a.set_cdp(true);
        a.cdp_port[0] = false;
        let mut out = vec![];
        a.tick(SimTime::from_secs(1), &ident("a", true), &mut out);
        assert!(out.is_empty());
    }
}
