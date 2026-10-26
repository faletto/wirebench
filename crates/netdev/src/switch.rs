use crate::stp::{Action as StpAction, Mode as StpMode, Stp};
use crate::vlan::{all_vlans, DEFAULT_VLAN};
use crate::{Ctx, Device, DeviceEvent, Mgmt, Router};
use kernel::{PortId, SimTime, TimerId};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use wire::{
    Bpdu, BridgeId, Dtp, DtpMode, EtherType, EthernetFrame, MacAddr, DTP_MULTICAST, PVST_MULTICAST,
};

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
    /// `dynamic auto`: becomes a trunk if the neighbour asks. The default, as on the real thing.
    DynamicAuto,
    /// `dynamic desirable`: asks the neighbour to trunk.
    DynamicDesirable,
}

impl PortMode {
    /// The mode is settled by negotiation rather than by configuration.
    pub fn is_dynamic(self) -> bool {
        matches!(self, PortMode::DynamicAuto | PortMode::DynamicDesirable)
    }
}

/// What a port does with VLANs. Access and trunk settings are kept side by side, as on a real
/// switch: `mode` picks which set is in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortConfig {
    pub mode: PortMode,
    pub access_vlan: u16,
    pub native_vlan: u16,
    pub allowed: BTreeSet<u16>,
    /// Send no DTP and ignore any received (`switchport nonegotiate`). Only for static modes.
    pub nonegotiate: bool,
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
            mode: PortMode::DynamicAuto,
            access_vlan: DEFAULT_VLAN,
            native_vlan: DEFAULT_VLAN,
            allowed: all_vlans(),
            nonegotiate: false,
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
#[derive(Debug, Clone)]
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
    /// Dynamic ports: whether negotiation has made each one a trunk.
    trunking: Vec<bool>,
    /// What each neighbour last said in DTP, and when.
    dtp_peer: Vec<Option<(DtpMode, SimTime)>>,
    /// Something changed that the neighbours should hear about.
    dtp_dirty: bool,
    /// Ticks since DTP was last sent periodically.
    dtp_secs: u32,
}

/// DTP is repeated this often, and a neighbour not heard from for three times as long is gone.
const DTP_INTERVAL: u32 = 30;

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
            trunking: vec![false; n_ports as usize],
            dtp_peer: vec![None; n_ports as usize],
            dtp_dirty: false,
            dtp_secs: 0,
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
        self.trunking = vec![false; n];
        self.dtp_peer = vec![None; n];
        self.dtp_dirty = false;
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
            self.dtp_dirty = true;
            self.dtp_decide(p);
            self.table.retain(|_, (port, _)| *port != p);
            self.refresh_svis();
            self.sync_stp();
        }
    }

    /// Whether port `p` is operating as a trunk: configured so, or negotiated.
    pub fn is_trunk(&self, p: PortId) -> bool {
        match self.ports.get(p.0 as usize).map(|c| c.mode) {
            Some(PortMode::Trunk) => true,
            Some(m) if m.is_dynamic() => self.trunking[p.0 as usize],
            _ => false,
        }
    }

    /// What the neighbour on `p` last announced, if it is still heard.
    pub fn dtp_neighbour(&self, p: PortId) -> Option<DtpMode> {
        self.dtp_peer.get(p.0 as usize)?.map(|(m, _)| m)
    }

    /// Settle whether a dynamic port trunks, from its own mode and its neighbour's.
    fn dtp_decide(&mut self, p: PortId) {
        let i = p.0 as usize;
        let want = matches!(
            (self.ports[i].mode, self.dtp_peer[i].map(|(m, _)| m)),
            (
                PortMode::DynamicAuto,
                Some(DtpMode::On | DtpMode::Desirable)
            ) | (
                PortMode::DynamicDesirable,
                Some(DtpMode::On | DtpMode::Desirable | DtpMode::Auto)
            )
        ) && !self.ports[i].nonegotiate;
        if self.trunking[i] != want {
            self.trunking[i] = want;
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
            && if self.is_trunk(p) {
                cfg.allowed.contains(&v)
            } else {
                cfg.access_vlan == v
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
        let vlan = if !self.is_trunk(port) {
            match (tagged, vlan) {
                (false, _) => cfg.access_vlan,
                (true, v) if v == cfg.access_vlan || v == 0 => cfg.access_vlan,
                _ => return None,
            }
        } else if !tagged || vlan == 0 {
            // A priority tag (id 0) belongs to the native VLAN, like an untagged frame.
            cfg.native_vlan
        } else {
            vlan
        };
        self.carries(port, vlan).then_some((vlan, prio, plain))
    }

    /// The frame as port `p` sends it for VLAN `v`: tagged on a trunk unless it is the native VLAN.
    fn egress(&self, p: PortId, v: u16, prio: u8, plain: &[u8]) -> Vec<u8> {
        let cfg = &self.ports[p.0 as usize];
        if !self.is_trunk(p) || cfg.native_vlan == v {
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
    pub fn refresh(&mut self, ctx: &mut Ctx) {
        let initiates = self.ports.iter().any(|c| {
            matches!(c.mode, PortMode::Trunk | PortMode::DynamicDesirable) && !c.nonegotiate
        });
        if !self.upkeep_needed() && !initiates {
            self.trees.clear();
            self.link_up = vec![false; self.ports.len()];
            return;
        }
        for p in 0..self.ports.len() {
            let up = ctx.port_up(PortId(p as u16));
            if self.link_up[p] != up {
                self.link_up[p] = up;
                // A cable gone takes its neighbour with it; one arriving should hear from us.
                self.dtp_peer[p] = None;
                self.dtp_decide(PortId(p as u16));
                self.dtp_dirty = true;
            }
        }
        self.sync_stp();
        self.flush_stp(ctx);
        if self.dtp_dirty {
            self.dtp_dirty = false;
            // Ports that want a trunk, or have just said they never will, speak up; dynamic auto ports
            // only answer (see `on_dtp`).
            for p in 0..self.ports.len() {
                if matches!(
                    self.ports[p].mode,
                    PortMode::Trunk | PortMode::DynamicDesirable | PortMode::Access
                ) {
                    self.dtp_send(ctx, PortId(p as u16));
                }
            }
        }
        let busy = self.spanning.mode.is_some() || self.dtp_peer.iter().any(Option::is_some);
        if busy && self.stp_timer.is_none() {
            self.stp_timer = Some(ctx.schedule_background(SimTime::from_secs(1), STP_TIMER));
        }
    }

    /// Whether the one-second tick has anything to do: spanning tree is running, or some
    /// neighbour is being listened to for DTP.
    fn upkeep_needed(&self) -> bool {
        self.spanning.mode.is_some() || self.dtp_peer.iter().any(Option::is_some) || self.dtp_dirty
    }

    fn stp_tick(&mut self, ctx: &mut Ctx) {
        if !self.upkeep_needed() {
            return;
        }
        // Neighbours not heard for three intervals are gone; the rest hear from us every interval.
        let now = ctx.now();
        for p in 0..self.ports.len() {
            if let Some((_, heard)) = self.dtp_peer[p] {
                if now > heard + SimTime::from_secs(u64::from(3 * DTP_INTERVAL)) {
                    self.dtp_peer[p] = None;
                    self.dtp_decide(PortId(p as u16));
                }
            }
        }
        self.dtp_secs += 1;
        if self.dtp_secs >= DTP_INTERVAL {
            self.dtp_secs = 0;
            self.dtp_dirty = true;
        }
        self.refresh(ctx);
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

    fn dtp_mac(&self, p: PortId) -> MacAddr {
        MacAddr([2, 0xbb, 0xff, self.unit, 0, p.0 as u8 + 1])
    }

    /// Tell the neighbour on `p` what this port is set to, if it takes part in DTP.
    fn dtp_send(&mut self, ctx: &mut Ctx, p: PortId) {
        let i = p.0 as usize;
        let cfg = &self.ports[i];
        if cfg.nonegotiate || cfg.shutdown || !self.link_up[i] {
            return;
        }
        let mode = match cfg.mode {
            PortMode::Trunk => DtpMode::On,
            PortMode::Access => DtpMode::Off,
            PortMode::DynamicAuto => DtpMode::Auto,
            PortMode::DynamicDesirable => DtpMode::Desirable,
        };
        let payload = Dtp {
            mode,
            trunking: self.is_trunk(p),
            domain: vec![],
            sender: self.bridge_mac(),
        }
        .emit();
        let frame = EthernetFrame {
            dst: DTP_MULTICAST,
            src: self.dtp_mac(p),
            vlan: None,
            ethertype: EtherType(payload.len() as u16),
            payload,
        }
        .emit();
        ctx.transmit(p, frame);
    }

    /// A DTP message from the neighbour on `port`. Answers at once when it is news, which is what
    /// lets a switch with no configuration at all be trunked by one that has some.
    fn on_dtp(&mut self, ctx: &mut Ctx, port: PortId, frame: &[u8]) {
        let i = port.0 as usize;
        let cfg = &self.ports[i];
        if cfg.nonegotiate || cfg.shutdown || cfg.mode == PortMode::Access {
            return;
        }
        let Ok(d) = EthernetFrame::parse(frame)
            .map_err(drop)
            .and_then(|f| Dtp::parse(&f.payload).map_err(drop))
        else {
            return;
        };
        let news = self.dtp_peer[i].map(|(m, _)| m) != Some(d.mode);
        self.dtp_peer[i] = Some((d.mode, ctx.now()));
        let was = self.is_trunk(port);
        self.dtp_decide(port);
        if news || was != self.is_trunk(port) {
            self.link_up[i] = true;
            self.dtp_send(ctx, port);
        }
        if self.stp_timer.is_none() {
            self.stp_timer = Some(ctx.schedule_background(SimTime::from_secs(1), STP_TIMER));
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
        if frame[0..6] == DTP_MULTICAST.0 && frame[12..14] != DOT1Q {
            self.on_dtp(ctx, port, &frame);
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
