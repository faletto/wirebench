use crate::stp::{Action as StpAction, Mode as StpMode, Stp};
use crate::vlan::{all_vlans, DEFAULT_VLAN};
use crate::{Ctx, Device, DeviceEvent, Mgmt, Router};
use kernel::{PortId, SimTime, TimerId};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use wire::{Bpdu, BridgeId, EtherType, EthernetFrame, MacAddr, PVST_MULTICAST};

const DOT1Q: [u8; 2] = [0x81, 0x00];

/// Timer token for the one-second spanning tree tick (the layer 3 half uses addresses as tokens,
/// which never reach this high).
const STP_TIMER: u64 = 1 << 61;
/// The class of service BPDUs are sent with, as network control traffic.
const BPDU_PRIO: u8 = 0xc0;
/// The priority a bridge has unless told otherwise.
pub const DEFAULT_BRIDGE_PRIORITY: u16 = 32768;

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
    /// Administratively down: carries nothing and takes no part in spanning tree.
    pub shutdown: bool,
    /// Spanning tree: forward at once, as a port facing a host does (`spanning-tree portfast`).
    pub portfast: bool,
    /// Spanning tree path cost; `None` is the default for the port.
    pub stp_cost: Option<u32>,
    /// Spanning tree port priority, in steps of 16; `None` is 128.
    pub stp_priority: Option<u8>,
}

impl Default for PortConfig {
    fn default() -> Self {
        PortConfig {
            mode: PortMode::Access,
            access_vlan: DEFAULT_VLAN,
            native_vlan: DEFAULT_VLAN,
            allowed: all_vlans(),
            shutdown: false,
            portfast: false,
            stp_cost: None,
            stp_priority: None,
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

/// Spanning tree settings for the whole switch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanningConfig {
    /// `None` leaves spanning tree off, which is how a new switch starts (unlike IOS, which starts
    /// in `pvst`), so a lab with no redundant links forwards at once.
    pub mode: Option<StpMode>,
    /// Bridge priority by VLAN; a VLAN not listed has [`DEFAULT_BRIDGE_PRIORITY`].
    pub priority: BTreeMap<u16, u16>,
    /// VLANs with spanning tree turned off (`no spanning-tree vlan N`).
    pub disabled: BTreeSet<u16>,
}

/// Learning bridge with VLANs: access and trunk ports, 802.1Q tagging, a MAC table per VLAN, and
/// optionally one spanning tree per VLAN. No DTP or VTP yet.
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
    /// The switch's layer 3 half: its interfaces are the SVIs. Routing between them is off until
    /// `ip routing`; until then it only answers for its own addresses.
    l3: Router,
    /// Makes SVI MAC addresses unique across switches in a lab.
    unit: u8,
    /// Passwords, banner and lines (see [`Mgmt`]).
    pub mgmt: Mgmt,
    /// The saved configuration: what `copy running-config startup-config` stored.
    pub startup: Option<String>,
    spanning: SpanningConfig,
    /// One tree per VLAN while spanning tree is on.
    trees: BTreeMap<u16, Stp>,
    /// Which ports had a link at the last look (polled every second while spanning tree runs).
    link_up: Vec<bool>,
    /// BPDUs and flushes decided without a context to send them in, delivered at the next event.
    stp_pending: Vec<(u16, StpAction)>,
    stp_timer: Option<TimerId>,
}

impl L2Switch {
    pub fn new(n_ports: u16) -> Self {
        L2Switch {
            name: "Switch".to_string(),
            ports: vec![PortConfig::default(); n_ports as usize],
            vlans: BTreeMap::from([(DEFAULT_VLAN, "default".to_string())]),
            table: BTreeMap::new(),
            aging: SimTime::from_secs(300),
            l3: {
                let mut r = Router::new("switch", 0, 0);
                r.set_routing(false);
                r
            },
            unit: 0,
            mgmt: Mgmt::default(),
            startup: None,
            spanning: SpanningConfig::default(),
            trees: BTreeMap::new(),
            link_up: vec![false; n_ports as usize],
            stp_pending: Vec::new(),
            stp_timer: None,
        }
    }

    /// Back to an unconfigured switch: default name, every port an access port in VLAN 1, only
    /// VLAN 1, no SVIs, empty MAC table, no passwords or banner. The saved configuration
    /// survives (it is what a reload restores).
    pub fn factory_reset(&mut self) {
        self.name = "Switch".to_string();
        let n = self.ports.len();
        self.ports = vec![PortConfig::default(); n];
        self.vlans = BTreeMap::from([(DEFAULT_VLAN, "default".to_string())]);
        self.table.clear();
        self.l3 = {
            let mut r = Router::new("switch", 0, 0);
            r.set_routing(false);
            r
        };
        self.mgmt = Mgmt::default();
        self.spanning = SpanningConfig::default();
        self.trees.clear();
        self.stp_pending.clear();
    }

    /// Set before creating SVIs; their MACs are `02:bb:00:<unit>:<vlan>`.
    pub fn set_unit(&mut self, unit: u8) {
        self.unit = unit;
    }

    // ---- layer 3 (SVIs) ----

    pub fn l3(&self) -> &Router {
        &self.l3
    }

    /// For addresses, shutdown, routes and `ip routing` on the SVIs. Create and delete SVIs with
    /// [`L2Switch::add_svi`] and [`L2Switch::remove_svi`] so their line state stays right.
    pub fn l3_mut(&mut self) -> &mut Router {
        &mut self.l3
    }

    /// Run `f` on the layer 3 half with a context (to send a probe, say) and put whatever it
    /// queued into the VLANs.
    pub fn originate<R>(&mut self, ctx: &mut Ctx, f: impl FnOnce(&mut Router, &mut Ctx) -> R) -> R {
        let r = f(&mut self.l3, ctx);
        self.drain_l3(ctx);
        r
    }

    /// Find or create the SVI for `vlan` and return its interface index.
    pub fn add_svi(&mut self, vlan: u16) -> usize {
        let mac = MacAddr([2, 0xbb, 0, self.unit, (vlan >> 8) as u8, vlan as u8]);
        let i = self.l3.add_svi(vlan, mac);
        self.refresh_svis();
        i
    }

    pub fn remove_svi(&mut self, vlan: u16) -> bool {
        self.l3.remove_svi(vlan)
    }

    /// An SVI is up while its VLAN exists and some port carries it. (Cable and link state are not
    /// modelled at the switch.)
    fn vlan_active(&self, vlan: u16) -> bool {
        (0..self.n_ports()).any(|p| self.carries(PortId(p), vlan))
    }

    fn refresh_svis(&mut self) {
        let wanted: Vec<(usize, bool)> = self
            .l3
            .interfaces()
            .iter()
            .enumerate()
            .filter_map(|(i, ifc)| ifc.svi.map(|v| (i, !self.vlan_active(v))))
            .collect();
        for (i, down) in wanted {
            self.l3.set_line_down(i, down);
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
        self.refresh_svis();
        self.sync_stp();
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
            self.refresh_svis();
            self.sync_stp();
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
            self.refresh_svis();
            self.sync_stp();
        }
    }

    /// Whether VLAN `v` is carried by port `p` (the VLAN exists and the port is in it).
    fn carries(&self, p: PortId, v: u16) -> bool {
        let Some(cfg) = self.ports.get(p.0 as usize) else {
            return false;
        };
        !cfg.shutdown
            && self.vlans.contains_key(&v)
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

impl L2Switch {
    /// Send a frame into VLAN `vlan`: to its learned port, else to every port that carries the
    /// VLAN. `ingress` is where it came from (never sent back out of), or `None` when the switch
    /// itself originated it.
    fn bridge(
        &mut self,
        ctx: &mut Ctx,
        ingress: Option<PortId>,
        vlan: u16,
        prio: u8,
        plain: &[u8],
    ) {
        let dst = MacAddr(plain[0..6].try_into().unwrap());
        let known = self
            .table
            .get(&(vlan, dst))
            .filter(|(_, seen)| *seen + self.aging > ctx.now())
            .map(|(p, _)| *p)
            .filter(|p| self.passes(*p, vlan));
        match known {
            Some(out) if !dst.is_multicast() => {
                // Destination is behind the ingress port: filter.
                if Some(out) != ingress {
                    ctx.transmit(out, self.egress(out, vlan, prio, plain));
                }
            }
            _ => {
                for p in (0..self.n_ports())
                    .map(PortId)
                    .filter(|p| Some(*p) != ingress && self.passes(*p, vlan))
                {
                    ctx.transmit(p, self.egress(p, vlan, prio, plain));
                }
            }
        }
    }

    /// Send what the layer 3 half queued into the VLANs of the SVIs that produced it.
    fn drain_l3(&mut self, ctx: &mut Ctx) {
        for o in self.l3.take_outbox() {
            let ifc = &self.l3.interfaces()[o.iface];
            let Some(vlan) = ifc.svi else { continue };
            let bytes = EthernetFrame {
                dst: o.dst,
                src: ifc.mac,
                vlan: None,
                ethertype: o.ethertype,
                payload: o.payload,
            }
            .emit();
            self.bridge(ctx, None, vlan, 0, &bytes);
        }
    }
}

// ---- spanning tree ----

impl L2Switch {
    pub fn spanning(&self) -> &SpanningConfig {
        &self.spanning
    }

    /// Change the spanning tree settings; trees are created, removed and updated to match.
    pub fn configure_spanning(&mut self, f: impl FnOnce(&mut SpanningConfig)) {
        f(&mut self.spanning);
        self.sync_stp();
    }

    /// The tree for `vlan`, while spanning tree runs for it.
    pub fn tree(&self, vlan: u16) -> Option<&Stp> {
        self.trees.get(&vlan)
    }

    pub fn trees(&self) -> impl Iterator<Item = (u16, &Stp)> {
        self.trees.iter().map(|(v, t)| (*v, t))
    }

    /// This switch's address in bridge identifiers and BPDU source addresses.
    pub fn bridge_mac(&self) -> MacAddr {
        MacAddr([2, 0xbb, 0xff, self.unit, 0, 0])
    }

    pub fn bridge_priority(&self, vlan: u16) -> u16 {
        self.spanning
            .priority
            .get(&vlan)
            .copied()
            .unwrap_or(DEFAULT_BRIDGE_PRIORITY)
    }

    /// The identifier in `vlan`'s tree: priority with the VLAN added in (the extended system id).
    fn bridge_id(&self, vlan: u16) -> BridgeId {
        BridgeId {
            priority: self.bridge_priority(vlan) + vlan,
            mac: self.bridge_mac(),
        }
    }

    /// Whether data for `vlan` may leave through `p` right now.
    fn passes(&self, p: PortId, vlan: u16) -> bool {
        self.carries(p, vlan)
            && self
                .trees
                .get(&vlan)
                .is_none_or(|t| t.forwards(p.0 as usize))
    }

    /// Bring the trees in line with the configuration, the VLANs, the ports and the links.
    /// Whatever the trees want sent is held until a context is at hand.
    fn sync_stp(&mut self) {
        let Some(mode) = self.spanning.mode else {
            self.trees.clear();
            self.stp_pending.clear();
            return;
        };
        let wanted: BTreeSet<u16> = self
            .vlans
            .keys()
            .copied()
            .filter(|v| !self.spanning.disabled.contains(v))
            .collect();
        self.trees.retain(|v, _| wanted.contains(v));
        for v in wanted {
            let id = self.bridge_id(v);
            let n = self.ports.len();
            let on: Vec<bool> = (0..n)
                .map(|p| self.link_up[p] && self.carries(PortId(p as u16), v))
                .collect();
            let tree = self.trees.entry(v).or_insert_with(|| Stp::new(mode, id, n));
            let mut acts = vec![];
            if tree.mode() != mode {
                acts.extend(tree.set_mode(mode));
            }
            if tree.bridge() != id {
                acts.extend(tree.set_bridge(id));
            }
            for (p, cfg) in self.ports.iter().enumerate() {
                let view = tree.port(p);
                let cost = cfg.stp_cost.unwrap_or(19);
                let prio = cfg.stp_priority.unwrap_or(128);
                if view.cost != cost {
                    acts.extend(tree.set_cost(p, cost));
                }
                if view.priority != prio {
                    acts.extend(tree.set_port_priority(p, prio));
                }
                if view.edge != cfg.portfast {
                    acts.extend(tree.set_edge(p, cfg.portfast));
                }
                acts.extend(tree.set_enabled(p, on[p]));
            }
            self.stp_pending.extend(acts.into_iter().map(|a| (v, a)));
        }
    }

    /// Look at the cables, settle the trees and send what is waiting. Called after
    /// configuration changes, which is what starts spanning tree running, and on every tick.
    pub fn stp_refresh(&mut self, ctx: &mut Ctx) {
        if self.spanning.mode.is_none() {
            self.trees.clear();
            return;
        }
        for p in 0..self.ports.len() {
            self.link_up[p] = ctx.port_up(PortId(p as u16));
        }
        self.sync_stp();
        self.flush_stp(ctx);
        if self.stp_timer.is_none() {
            self.stp_timer = Some(ctx.schedule_background(SimTime::from_secs(1), STP_TIMER));
        }
    }

    fn stp_tick(&mut self, ctx: &mut Ctx) {
        if self.spanning.mode.is_none() {
            return;
        }
        self.stp_refresh(ctx);
        let mut acts = vec![];
        for (v, t) in &mut self.trees {
            acts.extend(t.tick().into_iter().map(|a| (*v, a)));
        }
        self.stp_pending.extend(acts);
        self.flush_stp(ctx);
    }

    fn flush_stp(&mut self, ctx: &mut Ctx) {
        for (vlan, a) in std::mem::take(&mut self.stp_pending) {
            self.do_stp(ctx, vlan, a);
        }
    }

    fn do_stp(&mut self, ctx: &mut Ctx, vlan: u16, a: StpAction) {
        match a {
            StpAction::Flush { except } => {
                self.table.retain(|(v, _), (p, _)| {
                    *v != vlan || except.is_some_and(|e| e == p.0 as usize)
                });
            }
            StpAction::Send { port, bpdu } => {
                let payload = bpdu.emit();
                let frame = EthernetFrame {
                    dst: PVST_MULTICAST,
                    src: MacAddr([2, 0xbb, 0xff, self.unit, 0, port as u8 + 1]),
                    vlan: None,
                    ethertype: EtherType(payload.len() as u16),
                    payload,
                }
                .emit();
                let p = PortId(port as u16);
                if self.carries(p, vlan) {
                    ctx.transmit(p, self.egress(p, vlan, BPDU_PRIO, &frame));
                }
            }
        }
    }

    fn on_bpdu(&mut self, ctx: &mut Ctx, port: PortId, vlan: u16, plain: &[u8]) {
        let Ok(f) = EthernetFrame::parse(plain) else {
            return;
        };
        let Ok(b) = Bpdu::parse(&f.payload) else {
            return;
        };
        let acts = match self.trees.get_mut(&vlan) {
            Some(t) => t.receive(port.0 as usize, &b),
            None => return,
        };
        self.stp_pending.extend(acts.into_iter().map(|a| (vlan, a)));
        self.flush_stp(ctx);
    }
}

impl Device for L2Switch {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        let (port, frame) = match ev {
            DeviceEvent::FrameRx { port, frame } => (port, frame),
            DeviceEvent::Timer { token, .. } if token == STP_TIMER => {
                self.stp_timer = None;
                self.stp_tick(ctx);
                return;
            }
            DeviceEvent::Timer { token, .. } => {
                self.l3.on_timer(ctx, token);
                self.drain_l3(ctx);
                return;
            }
        };
        if frame.len() < 14 {
            return;
        }
        let Some((vlan, prio, plain)) = self.ingress(port, &frame) else {
            return;
        };
        let dst = MacAddr(plain[0..6].try_into().unwrap());
        let src = MacAddr(plain[6..12].try_into().unwrap());
        if dst == PVST_MULTICAST && self.trees.contains_key(&vlan) {
            self.on_bpdu(ctx, port, vlan, &plain);
            return;
        }
        let (learns, forwards) = match self.trees.get(&vlan) {
            Some(t) => (t.learns(port.0 as usize), t.forwards(port.0 as usize)),
            None => (true, true),
        };
        if !learns {
            return;
        }
        if !src.is_multicast() {
            self.table.insert((vlan, src), (port, ctx.now()));
        }
        if !forwards {
            return;
        }
        // The SVI for this VLAN gets frames sent to it and broadcasts. Frames sent to it are not
        // bridged on: its address belongs to the switch, not to a port.
        if let Some(i) = self.l3.svi_index(vlan) {
            let mac = self.l3.interfaces()[i].mac;
            if dst == mac || dst.is_broadcast() {
                if let Ok(f) = EthernetFrame::parse(&plain) {
                    self.l3.receive(ctx, i, f);
                    self.drain_l3(ctx);
                }
                if dst == mac {
                    return;
                }
            }
        }
        self.bridge(ctx, Some(port), vlan, prio, &plain);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
