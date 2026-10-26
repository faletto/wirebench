use crate::vlan::{all_vlans, DEFAULT_VLAN};
use crate::{Ctx, Device, DeviceEvent};
use kernel::{PortId, SimTime};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use wire::MacAddr;

const DOT1Q: [u8; 2] = [0x81, 0x00];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortMode {
    Access,
    Trunk,
}

/// What a port does with VLANs. Access and trunk settings are kept side by side, as on a real
/// switch: `mode` picks which set is in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortConfig {
    pub mode: PortMode,
    pub access_vlan: u16,
    pub native_vlan: u16,
    pub allowed: BTreeSet<u16>,
}

impl Default for PortConfig {
    fn default() -> Self {
        PortConfig {
            mode: PortMode::Access,
            access_vlan: DEFAULT_VLAN,
            native_vlan: DEFAULT_VLAN,
            allowed: all_vlans(),
        }
    }
}

/// One learned address, in the VLAN it was learned in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacEntry {
    pub vlan: u16,
    pub mac: MacAddr,
    pub port: PortId,
}

/// Learning bridge with VLANs: access and trunk ports, 802.1Q tagging, a MAC table per VLAN.
/// No STP, DTP or VTP yet.
#[derive(Debug)]
pub struct L2Switch {
    /// Hostname shown in prompts.
    pub name: String,
    ports: Vec<PortConfig>,
    /// VLAN id to name. VLAN 1 always exists.
    vlans: BTreeMap<u16, String>,
    table: BTreeMap<(u16, MacAddr), (PortId, SimTime)>,
    /// Entries unseen for this long are ignored (and replaced on next sight).
    pub aging: SimTime,
}

impl L2Switch {
    pub fn new(n_ports: u16) -> Self {
        L2Switch {
            name: "switch".to_string(),
            ports: vec![PortConfig::default(); n_ports as usize],
            vlans: BTreeMap::from([(DEFAULT_VLAN, "default".to_string())]),
            table: BTreeMap::new(),
            aging: SimTime::from_secs(300),
        }
    }

    pub fn n_ports(&self) -> u16 {
        self.ports.len() as u16
    }

    /// Live MAC table entries at `now`, in VLAN then address order.
    pub fn mac_entries(&self, now: SimTime) -> Vec<MacEntry> {
        self.table
            .iter()
            .filter(|(_, (_, seen))| *seen + self.aging > now)
            .map(|((vlan, mac), (port, _))| MacEntry {
                vlan: *vlan,
                mac: *mac,
                port: *port,
            })
            .collect()
    }

    /// Live entries without the VLAN column, for callers that only care where an address is.
    pub fn mac_table(&self, now: SimTime) -> Vec<(MacAddr, PortId)> {
        self.mac_entries(now)
            .into_iter()
            .map(|e| (e.mac, e.port))
            .collect()
    }

    // ---- VLAN database ----

    pub fn vlans(&self) -> impl Iterator<Item = (u16, &str)> {
        self.vlans.iter().map(|(id, n)| (*id, n.as_str()))
    }

    pub fn vlan_exists(&self, id: u16) -> bool {
        self.vlans.contains_key(&id)
    }

    /// Create a VLAN, or keep its name if it exists. A new VLAN is named `VLANnnnn`.
    pub fn add_vlan(&mut self, id: u16) {
        self.vlans
            .entry(id)
            .or_insert_with(|| format!("VLAN{id:04}"));
    }

    /// Rename a VLAN, creating it first if need be.
    pub fn set_vlan_name(&mut self, id: u16, name: &str) {
        self.vlans.insert(id, name.to_string());
    }

    /// Delete a VLAN. VLAN 1 cannot be deleted; returns whether anything was removed. Ports that
    /// were in it stay assigned to it but carry nothing until it exists again.
    pub fn remove_vlan(&mut self, id: u16) -> bool {
        if id == DEFAULT_VLAN {
            return false;
        }
        let gone = self.vlans.remove(&id).is_some();
        if gone {
            self.table.retain(|(v, _), _| *v != id);
        }
        gone
    }

    // ---- ports ----

    pub fn port(&self, p: PortId) -> Option<&PortConfig> {
        self.ports.get(p.0 as usize)
    }

    /// Change a port's VLAN settings. Addresses learned on it are forgotten, as a real switch
    /// does when the port's role changes.
    pub fn configure_port(&mut self, p: PortId, f: impl FnOnce(&mut PortConfig)) {
        if let Some(cfg) = self.ports.get_mut(p.0 as usize) {
            f(cfg);
            self.table.retain(|_, (port, _)| *port != p);
        }
    }

    /// Whether VLAN `v` is carried by port `p` (the VLAN exists and the port is in it).
    fn carries(&self, p: PortId, v: u16) -> bool {
        let Some(cfg) = self.ports.get(p.0 as usize) else {
            return false;
        };
        self.vlans.contains_key(&v)
            && match cfg.mode {
                PortMode::Access => cfg.access_vlan == v,
                PortMode::Trunk => cfg.allowed.contains(&v),
            }
    }

    /// Classify an incoming frame: the VLAN it belongs to, its priority bits (the top nibble of
    /// the tag's first byte, zero when untagged) and the frame with any tag removed. `None` means
    /// drop it.
    fn ingress(&self, port: PortId, frame: &[u8]) -> Option<(u16, u8, Vec<u8>)> {
        let cfg = self.ports.get(port.0 as usize)?;
        let tagged = frame[12..14] == DOT1Q && frame.len() >= 18;
        let (vlan, prio, plain) = if tagged {
            let vid = u16::from_be_bytes([frame[14] & 0x0f, frame[15]]);
            let mut plain = frame[..12].to_vec();
            plain.extend_from_slice(&frame[16..]);
            (vid, frame[14] & 0xf0, plain)
        } else {
            (0, 0, frame.to_vec())
        };
        let vlan = match cfg.mode {
            PortMode::Access => match (tagged, vlan) {
                (false, _) => cfg.access_vlan,
                (true, v) if v == cfg.access_vlan || v == 0 => cfg.access_vlan,
                _ => return None,
            },
            // A priority tag (id 0) belongs to the native VLAN, like an untagged frame.
            PortMode::Trunk if !tagged || vlan == 0 => cfg.native_vlan,
            PortMode::Trunk => vlan,
        };
        self.carries(port, vlan).then_some((vlan, prio, plain))
    }

    /// The frame as port `p` sends it for VLAN `v`: tagged on a trunk unless it is the native VLAN.
    fn egress(&self, p: PortId, v: u16, prio: u8, plain: &[u8]) -> Vec<u8> {
        let cfg = &self.ports[p.0 as usize];
        if cfg.mode == PortMode::Access || cfg.native_vlan == v {
            return plain.to_vec();
        }
        let mut out = plain[..12].to_vec();
        out.extend_from_slice(&DOT1Q);
        out.push(prio | (v >> 8) as u8);
        out.push(v as u8);
        out.extend_from_slice(&plain[12..]);
        out
    }
}

impl Device for L2Switch {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        let DeviceEvent::FrameRx { port, frame } = ev else {
            return;
        };
        if frame.len() < 14 {
            return;
        }
        let Some((vlan, prio, plain)) = self.ingress(port, &frame) else {
            return;
        };
        let dst = MacAddr(plain[0..6].try_into().unwrap());
        let src = MacAddr(plain[6..12].try_into().unwrap());
        let now = ctx.now();
        if !src.is_multicast() {
            self.table.insert((vlan, src), (port, now));
        }
        let known = self
            .table
            .get(&(vlan, dst))
            .filter(|(_, seen)| *seen + self.aging > now)
            .map(|(p, _)| *p)
            .filter(|p| self.carries(*p, vlan));
        match known {
            Some(out) if !dst.is_multicast() => {
                // Destination is behind the ingress port: filter.
                if out != port {
                    ctx.transmit(out, self.egress(out, vlan, prio, &plain));
                }
            }
            _ => {
                for p in (0..self.n_ports())
                    .map(PortId)
                    .filter(|p| *p != port && self.carries(*p, vlan))
                {
                    ctx.transmit(p, self.egress(p, vlan, prio, &plain));
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
