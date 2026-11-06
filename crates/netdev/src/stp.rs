//! Spanning tree for one VLAN: classic STP (802.1D-1998) and rapid STP (802.1D-2004).
//!
//! This is a pure state machine. It is told when a BPDU arrives, when a second passes and when a
//! port gains or loses its link, and answers with [`Action`]s (BPDUs to send, MAC entries to
//! forget). It knows nothing about frames, VLAN tags or the clock, so the switch carries it and
//! tests can wire bridges together directly.
//!
//! What is modelled: root election, root/designated/alternate/backup roles from priority vectors,
//! the STP timers (hello 2 s, max age 20 s, forward delay 15 s), the RSTP proposal/agreement
//! handshake with its "sync" step, edge ports, topology change handling in both flavours, and
//! fallback to STP on a port whose neighbour speaks it. What is not: shared media (every link is
//! point to point), configurable timers, root guard, BPDU guard, loop guard, UplinkFast and
//! BackboneFast (RSTP makes the latter two unnecessary).

use wire::{Bpdu, BpduKind, BpduRole, BridgeId};

pub const HELLO: u32 = 2;
pub const MAX_AGE: u32 = 20;
pub const FORWARD_DELAY: u32 = 15;

/// Seconds a topology change flag is kept on in rapid mode (two hellos and a bit, as a real switch
/// uses) and the root keeps it on in classic mode (max age plus forward delay).
const RAPID_TC: u32 = HELLO + 1;
const CLASSIC_TC: u32 = MAX_AGE + FORWARD_DELAY;
/// Hellos that may go missing before a rapid bridge discards what a neighbour told it.
const MISSED_HELLOS: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// 802.1D: ports wait out listening and learning timers.
    Stp,
    /// 802.1D-2004 (Cisco's rapid-pvst): handshakes where it can, timers where it cannot.
    Rapid,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Disabled,
    Root,
    Designated,
    Alternate,
    Backup,
}

/// Forwarding state. Rapid mode uses `Discarding` where classic mode has blocking and listening;
/// a classic port shows `Listening` for its first forward-delay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Discarding,
    Listening,
    Learning,
    Forwarding,
}

/// What the machine wants done.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Send {
        port: usize,
        bpdu: Bpdu,
    },
    /// Forget learned addresses in this VLAN, except those learned on `except`.
    Flush {
        except: Option<usize>,
    },
}

/// A priority vector: lower is better, compared field by field in this order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Vector {
    pub root: BridgeId,
    pub cost: u32,
    pub bridge: BridgeId,
    pub port: u16,
}

#[derive(Clone, Debug)]
struct Info {
    vector: Vector,
    /// Message age as received, in wire units, so it can be passed on plus one second.
    message_age: u16,
    /// Seconds left before this is forgotten.
    age_left: u32,
}

#[derive(Clone, Debug)]
struct Port {
    enabled: bool,
    /// Configured portfast, and whether it still holds (a BPDU arriving ends it).
    edge: bool,
    edge_oper: bool,
    cost: u32,
    priority: u8,
    id: u16,
    role: Role,
    state: State,
    info: Option<Info>,
    /// Seconds until the next forward-delay step.
    fd_while: u32,
    /// Offering to become forwarding with a proposal.
    proposing: bool,
    /// The neighbour speaks classic STP, so no handshakes on this port.
    compat: bool,
    tc_while: u32,
    /// The last BPDU from this neighbour carried the topology change flag.
    rx_tc: bool,
    /// Classic: acknowledge a received TCN in the next configuration BPDU.
    tc_ack: bool,
}

/// The state of a port as `show spanning-tree` reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PortView {
    pub role: Role,
    pub state: State,
    pub cost: u32,
    pub priority: u8,
    pub id: u16,
    pub edge: bool,
    pub compat: bool,
}

#[derive(Clone, Debug)]
pub struct Stp {
    mode: Mode,
    bridge: BridgeId,
    ports: Vec<Port>,
    /// The best vector known: ours when we are the root.
    root: Vector,
    root_port: Option<usize>,
    hello_count: u32,
    /// Classic: the root's topology change flag period left, and whether a TCN is awaiting an ack.
    root_tc_while: u32,
    tcn_pending: bool,
    /// Classic non-root: the root's TC flag as last seen.
    root_tc: bool,
}

fn own_vector(bridge: BridgeId) -> Vector {
    Vector {
        root: bridge,
        cost: 0,
        bridge,
        port: 0,
    }
}

impl Stp {
    pub fn new(mode: Mode, bridge: BridgeId, n_ports: usize) -> Self {
        let ports = (0..n_ports)
            .map(|i| Port {
                enabled: false,
                edge: false,
                edge_oper: false,
                cost: 19,
                priority: 128,
                id: port_id(128, i),
                role: Role::Disabled,
                state: State::Discarding,
                info: None,
                fd_while: 0,
                proposing: false,
                compat: false,
                tc_while: 0,
                rx_tc: false,
                tc_ack: false,
            })
            .collect();
        Stp {
            mode,
            bridge,
            ports,
            root: own_vector(bridge),
            root_port: None,
            hello_count: 0,
            root_tc_while: 0,
            tcn_pending: false,
            root_tc: false,
        }
    }

    // ---- queries ----

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn bridge(&self) -> BridgeId {
        self.bridge
    }

    pub fn root(&self) -> Vector {
        self.root
    }

    pub fn is_root(&self) -> bool {
        self.root_port.is_none()
    }

    pub fn root_port(&self) -> Option<usize> {
        self.root_port
    }

    pub fn n_ports(&self) -> usize {
        self.ports.len()
    }

    pub fn port(&self, i: usize) -> PortView {
        let p = &self.ports[i];
        PortView {
            role: p.role,
            state: p.state,
            cost: p.cost,
            priority: p.priority,
            id: p.id,
            edge: p.edge,
            compat: p.compat,
        }
    }

    /// Whether the port passes data frames.
    pub fn forwards(&self, i: usize) -> bool {
        self.ports[i].state == State::Forwarding
    }

    /// Whether the port learns source addresses (learning and forwarding).
    pub fn learns(&self, i: usize) -> bool {
        matches!(self.ports[i].state, State::Learning | State::Forwarding)
    }

    // ---- configuration ----

    pub fn set_bridge(&mut self, bridge: BridgeId) -> Vec<Action> {
        self.bridge = bridge;
        self.reelect()
    }

    pub fn set_mode(&mut self, mode: Mode) -> Vec<Action> {
        if mode == self.mode {
            return vec![];
        }
        self.mode = mode;
        // Anything learned or in progress was under the other rules: start over.
        self.restart()
    }

    pub fn set_edge(&mut self, i: usize, edge: bool) -> Vec<Action> {
        let p = &mut self.ports[i];
        p.edge = edge;
        p.edge_oper = edge;
        // A port already waiting to forward stops waiting.
        if edge && p.role == Role::Designated {
            p.state = State::Forwarding;
            p.fd_while = 0;
            p.proposing = false;
        }
        self.reelect()
    }

    pub fn set_cost(&mut self, i: usize, cost: u32) -> Vec<Action> {
        self.ports[i].cost = cost.max(1);
        self.reelect()
    }

    pub fn set_port_priority(&mut self, i: usize, priority: u8) -> Vec<Action> {
        let p = &mut self.ports[i];
        p.priority = priority;
        p.id = port_id(priority, i);
        self.reelect()
    }

    /// The link and VLAN membership of port `i` changed: it takes part in this tree or not.
    pub fn set_enabled(&mut self, i: usize, enabled: bool) -> Vec<Action> {
        if self.ports[i].enabled == enabled {
            return vec![];
        }
        let was_forwarding = self.ports[i].state == State::Forwarding && !self.ports[i].edge_oper;
        let p = &mut self.ports[i];
        p.enabled = enabled;
        p.info = None;
        p.role = Role::Disabled;
        p.state = State::Discarding;
        p.fd_while = 0;
        p.proposing = false;
        p.compat = false;
        p.rx_tc = false;
        p.tc_while = 0;
        p.edge_oper = p.edge;
        let mut out = self.reelect();
        if was_forwarding {
            self.topology_change(None, &mut out);
        }
        out
    }

    /// Forget everything and elect again, as after a reload.
    pub fn restart(&mut self) -> Vec<Action> {
        for p in &mut self.ports {
            p.info = None;
            p.role = Role::Disabled;
            p.state = State::Discarding;
            p.fd_while = 0;
            p.proposing = false;
            p.compat = false;
            p.rx_tc = false;
            p.tc_while = 0;
            p.tc_ack = false;
            p.edge_oper = p.edge;
        }
        self.root_tc_while = 0;
        self.tcn_pending = false;
        self.root_tc = false;
        self.hello_count = 0;
        self.reelect()
    }

    // ---- events ----

    /// A BPDU arrived on port `i`.
    pub fn receive(&mut self, i: usize, bpdu: &Bpdu) -> Vec<Action> {
        let mut out = vec![];
        if !self.ports[i].enabled {
            return out;
        }
        self.ports[i].edge_oper = false;
        if bpdu.kind == BpduKind::Tcn {
            if self.ports[i].role == Role::Designated {
                self.ports[i].tc_ack = true;
                self.topology_change(Some(i), &mut out);
                let b = self.bpdu_for(i, false);
                out.push(Action::Send { port: i, bpdu: b });
                self.ports[i].tc_ack = false;
            }
            return out;
        }
        if u32::from(bpdu.message_age) >= MAX_AGE * 256 {
            return out;
        }
        if bpdu.kind == BpduKind::Config && self.mode == Mode::Rapid {
            self.ports[i].compat = true;
        } else if bpdu.kind == BpduKind::Rapid {
            self.ports[i].compat = false;
        }
        let vector = Vector {
            root: bpdu.root,
            cost: bpdu.root_cost,
            bridge: bpdu.bridge,
            port: bpdu.port,
        };
        // An agreement is the answer to our proposal, from the neighbour's root port or from an
        // alternate port that is already blocking. It describes that port, which is inferior to
        // ours by definition, so it is read before the information is compared.
        if bpdu.kind == BpduKind::Rapid
            && bpdu.agreement
            && matches!(bpdu.role, BpduRole::Root | BpduRole::AlternateOrBackup)
            && self.ports[i].role == Role::Designated
            && self.ports[i].proposing
            && bpdu.root == self.root.root
        {
            self.agreed(i, &mut out);
            return out;
        }
        let ours = self.designated_vector(i);
        let p = &self.ports[i];
        // The same sender is the same bridge address and port number, whatever their priorities:
        // a neighbour that changed its bridge or port priority is still the one we heard before,
        // and its word replaces what it said, even if worse (802.1Q-2005 13.10).
        let same_sender = p.info.as_ref().is_some_and(|k| {
            k.vector.bridge.mac == vector.bridge.mac
                && k.vector.port & 0x0fff == vector.port & 0x0fff
        });
        let better = match &p.info {
            None => vector <= ours,
            Some(k) => vector <= k.vector || same_sender,
        };
        // An inferior BPDU from a neighbour that should be listening to us gets our answer at
        // once, which is what moves a stale neighbour on.
        // (A root port's own BPDUs, which is what an agreement is, are never worth an answer.)
        let from_designated = bpdu.kind == BpduKind::Config || bpdu.role == BpduRole::Designated;
        if !better && p.role == Role::Designated && from_designated {
            let b = self.bpdu_for(i, false);
            out.push(Action::Send { port: i, bpdu: b });
            return out;
        }
        if !better {
            return out;
        }
        let age_left = if bpdu.kind == BpduKind::Rapid {
            MISSED_HELLOS * HELLO
        } else {
            let used = u32::from(bpdu.message_age).div_ceil(256);
            (MAX_AGE - used).max(1)
        };
        // A bridge has one idea of the root at a time. Whatever else we were told by this same
        // bridge, on another port, about a different root or cost is out of date now.
        for (j, other) in self.ports.iter_mut().enumerate() {
            if j != i
                && other.info.as_ref().is_some_and(|k| {
                    k.vector.bridge.mac == vector.bridge.mac
                        && (k.vector.root, k.vector.cost, k.vector.bridge)
                            != (vector.root, vector.cost, vector.bridge)
                })
            {
                other.info = None;
                other.rx_tc = false;
            }
        }
        let old_root = self.root;
        let old_roles: Vec<Role> = self.ports.iter().map(|p| p.role).collect();
        let tc_now = bpdu.topology_change && !self.ports[i].rx_tc;
        self.ports[i].rx_tc = bpdu.topology_change;
        self.ports[i].info = Some(Info {
            vector,
            message_age: bpdu.message_age,
            age_left,
        });
        self.reelect_into(&mut out);
        if self.root_port == Some(i) {
            if self.mode == Mode::Stp || self.ports[i].compat {
                self.root_tc = bpdu.topology_change;
                if bpdu.topology_change_ack {
                    self.tcn_pending = false;
                }
            }
            // Classic bridges relay what the root port hears.
            if self.mode == Mode::Stp {
                self.send_designated(&mut out);
            }
        }
        if self.root != old_root || old_roles.iter().zip(&self.ports).any(|(r, p)| *r != p.role) {
            self.send_designated(&mut out);
        }
        if bpdu.kind == BpduKind::Rapid && bpdu.role == BpduRole::Designated && bpdu.proposal {
            self.on_proposal(i, &mut out);
        }
        if tc_now && self.ports[i].role != Role::Disabled {
            self.topology_change(Some(i), &mut out);
        }
        out
    }

    /// One second has passed.
    pub fn tick(&mut self) -> Vec<Action> {
        let mut out = vec![];
        let mut expired = false;
        for p in &mut self.ports {
            if p.tc_while > 0 {
                p.tc_while -= 1;
            }
            if let Some(info) = &mut p.info {
                info.age_left -= 1;
                if info.age_left == 0 {
                    p.info = None;
                    p.rx_tc = false;
                    expired = true;
                }
            }
        }
        if self.root_tc_while > 0 {
            self.root_tc_while -= 1;
        }
        if expired {
            self.reelect_into(&mut out);
            self.send_designated(&mut out);
        }
        // Forward-delay steps.
        for i in 0..self.ports.len() {
            let p = &mut self.ports[i];
            if p.fd_while == 0 || !matches!(p.role, Role::Root | Role::Designated) {
                continue;
            }
            p.fd_while -= 1;
            if p.fd_while > 0 {
                continue;
            }
            match p.state {
                State::Discarding | State::Listening => {
                    p.state = State::Learning;
                    p.fd_while = FORWARD_DELAY;
                }
                State::Learning => {
                    p.state = State::Forwarding;
                    p.proposing = false;
                    if !p.edge_oper {
                        self.topology_change(Some(i), &mut out);
                    }
                }
                State::Forwarding => {}
            }
        }
        // Hello.
        self.hello_count += 1;
        if self.hello_count >= HELLO {
            self.hello_count = 0;
            match self.mode {
                Mode::Rapid => self.send_designated(&mut out),
                Mode::Stp if self.is_root() => self.send_designated(&mut out),
                Mode::Stp => {}
            }
            if self.tcn_pending {
                if let Some(rp) = self.root_port {
                    out.push(Action::Send {
                        port: rp,
                        bpdu: tcn(),
                    });
                }
            }
        }
        out
    }

    // ---- the machine ----

    fn designated_vector(&self, i: usize) -> Vector {
        Vector {
            root: self.root.root,
            cost: self.root.cost,
            bridge: self.bridge,
            port: self.ports[i].id,
        }
    }

    fn reelect(&mut self) -> Vec<Action> {
        let mut out = vec![];
        self.reelect_into(&mut out);
        self.send_designated(&mut out);
        out
    }

    /// Choose root, root port and roles, then move ports whose role changed. Returns whether the
    /// root vector or any role changed.
    fn reelect_into(&mut self, out: &mut Vec<Action>) -> bool {
        let own = own_vector(self.bridge);
        let mut best: Option<(Vector, u16, usize)> = None;
        for (i, p) in self.ports.iter().enumerate() {
            let (true, Some(info)) = (p.enabled, &p.info) else {
                continue;
            };
            if info.vector.bridge == self.bridge {
                continue;
            }
            let cand = Vector {
                cost: info.vector.cost.saturating_add(p.cost),
                ..info.vector
            };
            if best.is_none_or(|(v, id, _)| (cand, p.id) < (v, id)) {
                best = Some((cand, p.id, i));
            }
        }
        let (root, root_port) = match best {
            Some((v, _, i)) if v < own => (v, Some(i)),
            _ => (own, None),
        };
        let mut changed = root != self.root || root_port != self.root_port;
        self.root = root;
        self.root_port = root_port;
        let mut new_roles = Vec::with_capacity(self.ports.len());
        for (i, p) in self.ports.iter().enumerate() {
            let role = if !p.enabled {
                Role::Disabled
            } else if root_port == Some(i) {
                Role::Root
            } else {
                match &p.info {
                    Some(k) if k.vector <= self.designated_vector(i) => {
                        if k.vector.bridge == self.bridge {
                            Role::Backup
                        } else {
                            Role::Alternate
                        }
                    }
                    _ => Role::Designated,
                }
            };
            new_roles.push(role);
        }
        // Moving to the new roles; sync before a new root port forwards.
        let mut rapid_root_up = false;
        for (i, role) in new_roles.iter().copied().enumerate() {
            if self.ports[i].role == role {
                continue;
            }
            changed = true;
            let leaving_forwarding = self.ports[i].state == State::Forwarding
                && !self.ports[i].edge_oper
                && matches!(role, Role::Alternate | Role::Backup | Role::Disabled);
            self.enter_role(i, role);
            if role == Role::Root && self.rapid_port(i) {
                rapid_root_up = true;
            }
            if leaving_forwarding {
                self.topology_change(None, out);
            }
        }
        if rapid_root_up {
            self.sync_others(out);
            if let Some(rp) = self.root_port {
                self.ports[rp].state = State::Forwarding;
                self.ports[rp].fd_while = 0;
                self.topology_change(Some(rp), out);
            }
        }
        // A port that is no longer designated (or not yet) holds no stale offers.
        for p in &mut self.ports {
            if p.role != Role::Designated {
                p.proposing = false;
            }
        }
        // Designated ports hold their own vector as their information.
        for i in 0..self.ports.len() {
            if self.ports[i].role == Role::Designated {
                self.ports[i].info = None;
            }
        }
        changed
    }

    fn rapid_port(&self, i: usize) -> bool {
        self.mode == Mode::Rapid && !self.ports[i].compat
    }

    fn enter_role(&mut self, i: usize, role: Role) {
        let rapid = self.mode == Mode::Rapid;
        let p = &mut self.ports[i];
        let was = p.role;
        p.role = role;
        match role {
            Role::Disabled | Role::Alternate | Role::Backup => {
                p.state = State::Discarding;
                p.fd_while = 0;
                p.proposing = false;
            }
            Role::Root => {
                p.proposing = false;
                if p.state == State::Discarding {
                    p.state = if rapid {
                        State::Discarding
                    } else {
                        State::Listening
                    };
                    p.fd_while = FORWARD_DELAY;
                }
            }
            Role::Designated => {
                if p.edge_oper {
                    p.state = State::Forwarding;
                    p.fd_while = 0;
                } else if rapid {
                    if matches!(was, Role::Root) || p.state == State::Learning {
                        p.state = State::Discarding;
                    }
                    if p.state == State::Discarding {
                        p.proposing = !p.compat;
                        p.fd_while = FORWARD_DELAY;
                    }
                } else if p.state == State::Discarding {
                    p.state = State::Listening;
                    p.fd_while = FORWARD_DELAY;
                }
            }
        }
    }

    /// Put every non-edge designated port that is passing traffic back to discarding, offering to
    /// forward again once its neighbour agrees.
    fn sync_others(&mut self, out: &mut Vec<Action>) {
        for i in 0..self.ports.len() {
            let p = &mut self.ports[i];
            if p.role == Role::Designated
                && !p.edge_oper
                && matches!(p.state, State::Forwarding | State::Learning)
            {
                let was_forwarding = p.state == State::Forwarding;
                p.state = State::Discarding;
                p.proposing = !p.compat;
                p.fd_while = FORWARD_DELAY;
                if was_forwarding {
                    self.topology_change(Some(i), out);
                }
            }
        }
    }

    /// A proposal arrived on port `i` from a bridge whose information we now accept (or already
    /// had). Agree once the rest of the bridge is quiet.
    fn on_proposal(&mut self, i: usize, out: &mut Vec<Action>) {
        match self.ports[i].role {
            Role::Root if self.rapid_port(i) => {
                self.sync_others(out);
                self.ports[i].state = State::Forwarding;
                self.ports[i].fd_while = 0;
                let b = self.bpdu_for(i, true);
                out.push(Action::Send { port: i, bpdu: b });
                // The ports just discarded now offer themselves downstream.
                self.send_designated(out);
            }
            Role::Alternate | Role::Backup if self.rapid_port(i) => {
                let b = self.bpdu_for(i, true);
                out.push(Action::Send { port: i, bpdu: b });
            }
            _ => {}
        }
    }

    fn agreed(&mut self, i: usize, out: &mut Vec<Action>) {
        let p = &mut self.ports[i];
        p.proposing = false;
        if p.state != State::Forwarding {
            p.state = State::Forwarding;
            p.fd_while = 0;
            if !p.edge_oper {
                self.topology_change(Some(i), out);
            }
        }
    }

    /// Something changed in the active topology. `except` is the port it came from.
    fn topology_change(&mut self, except: Option<usize>, out: &mut Vec<Action>) {
        out.push(Action::Flush { except });
        match self.mode {
            Mode::Rapid => {
                for (i, p) in self.ports.iter_mut().enumerate() {
                    if Some(i) != except
                        && !p.edge_oper
                        && matches!(p.role, Role::Root | Role::Designated)
                    {
                        p.tc_while = RAPID_TC;
                    }
                }
            }
            Mode::Stp => {
                if self.is_root() {
                    self.root_tc_while = CLASSIC_TC;
                } else {
                    // Sent with the next hello, and again each hello until acknowledged: a TCN
                    // passed on at once could chase itself round a loop of stale information.
                    self.tcn_pending = true;
                }
            }
        }
    }

    // ---- sending ----

    /// Send a BPDU on every designated port.
    fn send_designated(&mut self, out: &mut Vec<Action>) {
        for i in 0..self.ports.len() {
            if self.ports[i].role != Role::Designated {
                continue;
            }
            let b = self.bpdu_for(i, false);
            out.push(Action::Send { port: i, bpdu: b });
        }
    }

    fn bpdu_for(&self, i: usize, agreement: bool) -> Bpdu {
        let p = &self.ports[i];
        let rapid = self.rapid_port(i);
        let message_age = match self.root_port {
            None => 0,
            Some(rp) => self.ports[rp]
                .info
                .as_ref()
                .map_or(0, |k| k.message_age + 256),
        };
        let tc = match self.mode {
            Mode::Rapid => p.tc_while > 0,
            Mode::Stp if self.is_root() => self.root_tc_while > 0,
            Mode::Stp => self.root_tc,
        };
        Bpdu {
            kind: if rapid {
                BpduKind::Rapid
            } else {
                BpduKind::Config
            },
            topology_change: tc,
            topology_change_ack: p.tc_ack,
            proposal: rapid && p.proposing && p.state != State::Forwarding,
            agreement,
            learning: matches!(p.state, State::Learning | State::Forwarding),
            forwarding: p.state == State::Forwarding,
            role: match p.role {
                Role::Root => BpduRole::Root,
                Role::Designated => BpduRole::Designated,
                Role::Alternate | Role::Backup => BpduRole::AlternateOrBackup,
                Role::Disabled => BpduRole::Unknown,
            },
            root: self.root.root,
            root_cost: self.root.cost,
            bridge: self.bridge,
            port: p.id,
            message_age,
            max_age: Bpdu::seconds(MAX_AGE as u16),
            hello_time: Bpdu::seconds(HELLO as u16),
            forward_delay: Bpdu::seconds(FORWARD_DELAY as u16),
        }
    }
}

fn tcn() -> Bpdu {
    Bpdu {
        kind: BpduKind::Tcn,
        topology_change: false,
        topology_change_ack: false,
        proposal: false,
        agreement: false,
        learning: false,
        forwarding: false,
        role: BpduRole::Unknown,
        root: BridgeId::from_bytes(&[0; 8]),
        root_cost: 0,
        bridge: BridgeId::from_bytes(&[0; 8]),
        port: 0,
        message_age: 0,
        max_age: 0,
        hello_time: 0,
        forward_delay: 0,
    }
}

/// Port identifier: priority in the top byte, the port's number (from 1) below it, which is how
/// `show spanning-tree` prints it (`128.3`).
pub fn port_id(priority: u8, index: usize) -> u16 {
    (u16::from(priority) << 8) | (index as u16 + 1)
}
