//! First hop redundancy: the HSRP (RFC 2281) and VRRP (RFC 3768) state machines for one group on
//! one interface. Pure: callers pass the time and the packets in and send what comes out.

use kernel::SimTime;
use std::collections::BTreeMap;
use wire::{hsrp_vmac, vrrp_vmac, Ipv4Addr, MacAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Hsrp,
    Vrrp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Init,
    Listen,
    Speak,
    Standby,
    Active,
    /// VRRP only.
    Backup,
    /// VRRP only.
    Master,
}

impl State {
    pub fn is_active(self) -> bool {
        matches!(self, State::Active | State::Master)
    }

    pub fn name(self) -> &'static str {
        match self {
            State::Init => "Init",
            State::Listen => "Listen",
            State::Speak => "Speak",
            State::Standby => "Standby",
            State::Active => "Active",
            State::Backup => "Backup",
            State::Master => "Master",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hello,
    /// HSRP: "I want to be active and I outrank you".
    Coup,
    /// HSRP: I am giving up the active role. VRRP: an advertisement with priority 0.
    Resign,
}

/// One protocol message, with the wire details taken away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg {
    pub kind: Kind,
    pub state: State,
    pub priority: u8,
    pub hello: u32,
    pub hold: u32,
}

#[derive(Clone, Copy, Debug)]
struct Peer {
    priority: u8,
    state: State,
    seen: SimTime,
    hold: u32,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub proto: Proto,
    pub group: u8,
    pub vip: Ipv4Addr,
    pub priority: u8,
    pub preempt: bool,
    /// Interfaces whose failure lowers the priority: `(interface, decrement)`.
    pub tracks: Vec<(usize, u8)>,
    /// How much the tracked interfaces that are down take off the priority right now.
    pub penalty: u8,
    /// HSRP hello / VRRP advertisement interval, seconds.
    pub hello: u32,
    /// HSRP hold time, seconds. (VRRP derives its own from the interval and priority.)
    pub hold: u32,
    state: State,
    entered: SimTime,
    peers: BTreeMap<Ipv4Addr, Peer>,
    next_hello: SimTime,
    master_down: SimTime,
    garp: bool,
    owner: bool,
    pub transitions: u32,
    pub last_change: SimTime,
}

pub const DEFAULT_PRIORITY: u8 = 100;

impl Group {
    pub fn new(proto: Proto, group: u8, vip: Ipv4Addr) -> Group {
        let (hello, hold, preempt) = match proto {
            Proto::Hsrp => (3, 10, false),
            Proto::Vrrp => (1, 3, true),
        };
        Group {
            proto,
            group,
            vip,
            priority: DEFAULT_PRIORITY,
            preempt,
            tracks: Vec::new(),
            penalty: 0,
            hello,
            hold,
            state: State::Init,
            entered: SimTime::ZERO,
            peers: BTreeMap::new(),
            next_hello: SimTime::ZERO,
            master_down: SimTime::ZERO,
            garp: false,
            owner: false,
            transitions: 0,
            last_change: SimTime::ZERO,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn vmac(&self) -> MacAddr {
        match self.proto {
            Proto::Hsrp => hsrp_vmac(self.group),
            Proto::Vrrp => vrrp_vmac(self.group),
        }
    }

    /// This router currently owns the virtual address.
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Must a gratuitous ARP be sent? Clears the flag.
    pub fn take_garp(&mut self) -> bool {
        std::mem::take(&mut self.garp)
    }

    /// The priority in force: a VRRP owner of the address counts as 255.
    pub fn effective_priority(&self, owner: bool) -> u8 {
        if self.proto == Proto::Vrrp && owner {
            255
        } else {
            self.priority.saturating_sub(self.penalty)
        }
    }

    /// The active (master) router heard from, and its priority.
    pub fn active_peer(&self, now: SimTime) -> Option<(Ipv4Addr, u8)> {
        self.best_peer(now, |s| s.is_active())
    }

    pub fn standby_peer(&self, now: SimTime) -> Option<(Ipv4Addr, u8)> {
        self.best_peer(now, |s| s == State::Standby)
    }

    /// Seconds until the active router's last hello expires.
    pub fn active_expires_in(&self, now: SimTime) -> Option<u64> {
        let (ip, _) = self.active_peer(now)?;
        let p = self.peers.get(&ip)?;
        let end = p.seen + SimTime::from_secs(u64::from(p.hold));
        Some(end.0.saturating_sub(now.0) / 1_000_000_000)
    }

    fn live(&self, now: SimTime) -> impl Iterator<Item = (&Ipv4Addr, &Peer)> {
        self.peers
            .iter()
            .filter(move |(_, p)| p.seen + SimTime::from_secs(u64::from(p.hold)) > now)
    }

    fn best_peer(&self, now: SimTime, want: impl Fn(State) -> bool) -> Option<(Ipv4Addr, u8)> {
        self.live(now)
            .filter(|(_, p)| want(p.state))
            .map(|(ip, p)| (*ip, p.priority))
            .max_by_key(|(ip, prio)| (*prio, ip.to_u32()))
    }

    fn go(&mut self, now: SimTime, s: State, out: &mut Vec<Msg>, coup: bool) {
        if self.state == s {
            return;
        }
        self.state = s;
        self.entered = now;
        self.transitions += 1;
        self.last_change = now;
        if s.is_active() {
            self.garp = true;
            out.push(self.msg(if coup { Kind::Coup } else { Kind::Hello }));
            self.next_hello = now + SimTime::from_secs(u64::from(self.hello));
        }
        if matches!(s, State::Speak | State::Standby) {
            // Speak at once so the others hear about us.
            self.next_hello = now;
        }
    }

    fn msg(&self, kind: Kind) -> Msg {
        Msg {
            kind,
            state: self.state,
            priority: self.effective_priority(self.owner),
            hello: self.hello,
            hold: self.hold,
        }
    }

    fn msg_with(&self, kind: Kind, prio: u8) -> Msg {
        Msg {
            priority: prio,
            ..self.msg(kind)
        }
    }

    /// Whether `(prio, ip)` outranks `(other_prio, other)`.
    fn beats(prio: u8, ip: Ipv4Addr, other_prio: u8, other: Ipv4Addr) -> bool {
        (prio, ip.to_u32()) > (other_prio, other.to_u32())
    }

    fn skew(&self, prio: u8) -> SimTime {
        SimTime::from_millis(((256 - u64::from(prio)) * 1000) / 256)
    }

    fn master_down_interval(&self, prio: u8) -> SimTime {
        SimTime::from_secs(u64::from(3 * self.hello)) + self.skew(prio)
    }

    /// The interface came up or went down, or time passed.
    pub fn step(&mut self, now: SimTime, me: Ipv4Addr, up: bool, owner: bool, out: &mut Vec<Msg>) {
        self.owner = owner;
        let prio = self.effective_priority(owner);
        if !up {
            if self.state.is_active() {
                out.push(self.msg_with(
                    Kind::Resign,
                    if self.proto == Proto::Vrrp { 0 } else { prio },
                ));
            }
            if self.state != State::Init {
                self.go(now, State::Init, out, false);
            }
            self.peers.clear();
            return;
        }
        match self.proto {
            Proto::Hsrp => self.step_hsrp(now, me, prio, out),
            Proto::Vrrp => self.step_vrrp(now, prio, out),
        }
    }

    fn hello_due(&mut self, now: SimTime, prio: u8, out: &mut Vec<Msg>) {
        if now >= self.next_hello {
            out.push(self.msg_with(Kind::Hello, prio));
            self.next_hello = now + SimTime::from_secs(u64::from(self.hello));
        }
    }

    fn step_hsrp(&mut self, now: SimTime, me: Ipv4Addr, prio: u8, out: &mut Vec<Msg>) {
        let active = self.active_peer(now);
        let standby = self.standby_peer(now);
        match self.state {
            State::Init | State::Backup | State::Master => {
                self.go(now, State::Listen, out, false);
            }
            State::Listen => {
                let waited = now >= self.entered + SimTime::from_secs(u64::from(self.hold));
                if waited && (active.is_none() || standby.is_none()) {
                    self.go(now, State::Speak, out, false);
                }
            }
            State::Speak => {
                self.hello_due(now, prio, out);
                if active.is_none() {
                    // No active router: the best of those speaking takes over, unless a standby
                    // router already exists, which will.
                    let best = self
                        .live(now)
                        .filter(|(_, p)| p.state == State::Speak)
                        .all(|(ip, p)| Self::beats(prio, me, p.priority, *ip));
                    if standby.is_none() && best {
                        self.go(now, State::Active, out, false);
                    }
                } else if standby.is_none() {
                    let best = self
                        .live(now)
                        .filter(|(_, p)| p.state == State::Speak)
                        .all(|(ip, p)| Self::beats(prio, me, p.priority, *ip));
                    if best {
                        self.go(now, State::Standby, out, false);
                    }
                } else {
                    self.go(now, State::Listen, out, false);
                }
            }
            State::Standby => {
                self.hello_due(now, prio, out);
                match active {
                    None => self.go(now, State::Active, out, false),
                    Some((_, ap)) if self.preempt && prio > ap => {
                        self.go(now, State::Active, out, true)
                    }
                    _ => {
                        if let Some((sip, sp)) = standby {
                            if Self::beats(sp, sip, prio, me) {
                                self.go(now, State::Listen, out, false);
                            }
                        }
                    }
                }
            }
            State::Active => {
                self.hello_due(now, prio, out);
                if let Some((aip, ap)) = active {
                    if Self::beats(ap, aip, prio, me) {
                        self.go(now, State::Speak, out, false);
                    }
                }
            }
        }
    }

    fn step_vrrp(&mut self, now: SimTime, prio: u8, out: &mut Vec<Msg>) {
        match self.state {
            State::Init | State::Listen | State::Speak | State::Standby | State::Active => {
                if prio == 255 {
                    self.go(now, State::Master, out, false);
                } else {
                    self.master_down = now + self.master_down_interval(prio);
                    self.go(now, State::Backup, out, false);
                }
            }
            State::Backup => {
                if now >= self.master_down {
                    self.go(now, State::Master, out, false);
                }
            }
            State::Master => self.hello_due(now, prio, out),
        }
    }

    /// A message from the router at `from`.
    pub fn receive(
        &mut self,
        now: SimTime,
        me: Ipv4Addr,
        from: Ipv4Addr,
        owner: bool,
        msg: Msg,
        out: &mut Vec<Msg>,
    ) {
        self.owner = owner;
        let prio = self.effective_priority(owner);
        if msg.kind == Kind::Resign {
            self.peers.remove(&from);
            if self.proto == Proto::Vrrp && self.state == State::Backup {
                self.master_down = now + self.skew(prio);
            }
            if self.proto == Proto::Vrrp && self.state == State::Master {
                out.push(self.msg_with(Kind::Hello, prio));
            }
        } else {
            let hold = match self.proto {
                Proto::Hsrp => msg.hold.max(1),
                Proto::Vrrp => 3 * msg.hello.max(1) + 1,
            };
            self.peers.insert(
                from,
                Peer {
                    priority: msg.priority,
                    state: if self.proto == Proto::Vrrp {
                        State::Master
                    } else if msg.kind == Kind::Coup {
                        State::Active
                    } else {
                        msg.state
                    },
                    seen: now,
                    hold,
                },
            );
            if self.proto == Proto::Vrrp {
                let outranks = Self::beats(msg.priority, from, prio, me);
                match self.state {
                    State::Backup if !self.preempt || outranks => {
                        self.master_down = now + self.master_down_interval(prio);
                    }
                    State::Master if outranks => {
                        self.master_down = now + self.master_down_interval(prio);
                        self.go(now, State::Backup, out, false);
                    }
                    _ => {}
                }
                return;
            }
        }
        if self.proto == Proto::Hsrp {
            self.step_hsrp(now, me, prio, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Node {
        ip: Ipv4Addr,
        g: Group,
        up: bool,
    }

    /// Routers on one wire with no delay; time moves in whole seconds.
    struct Bus {
        nodes: Vec<Node>,
        now: u64,
    }

    impl Bus {
        fn new(proto: Proto, cfg: &[(u8, bool)]) -> Bus {
            Bus {
                nodes: cfg
                    .iter()
                    .enumerate()
                    .map(|(i, (prio, preempt))| {
                        let mut g = Group::new(proto, 1, Ipv4Addr([10, 0, 0, 254]));
                        g.priority = *prio;
                        g.preempt = *preempt;
                        Node {
                            ip: Ipv4Addr([10, 0, 0, i as u8 + 1]),
                            g,
                            up: true,
                        }
                    })
                    .collect(),
                now: 0,
            }
        }

        fn deliver(&mut self, from: usize, msgs: Vec<Msg>) {
            let mut queue: Vec<(usize, Msg)> = msgs.into_iter().map(|m| (from, m)).collect();
            while let Some((f, m)) = queue.pop() {
                let src = self.nodes[f].ip;
                for i in 0..self.nodes.len() {
                    if i == f || !self.nodes[i].up {
                        continue;
                    }
                    let mut out = vec![];
                    let n = &mut self.nodes[i];
                    n.g.receive(SimTime::from_secs(self.now), n.ip, src, false, m, &mut out);
                    queue.extend(out.into_iter().map(|m| (i, m)));
                }
            }
        }

        fn tick(&mut self) {
            self.now += 1;
            for i in 0..self.nodes.len() {
                let mut out = vec![];
                let n = &mut self.nodes[i];
                n.g.step(SimTime::from_secs(self.now), n.ip, n.up, false, &mut out);
                if n.up {
                    self.deliver(i, out);
                } else {
                    self.deliver(
                        i,
                        out.into_iter().filter(|m| m.kind == Kind::Resign).collect(),
                    );
                }
            }
        }

        fn run(&mut self, secs: u64) {
            for _ in 0..secs {
                self.tick();
            }
        }

        fn states(&self) -> Vec<State> {
            self.nodes.iter().map(|n| n.g.state()).collect()
        }
    }

    #[test]
    fn hsrp_elects_the_highest_priority_active_and_the_next_standby() {
        let mut b = Bus::new(Proto::Hsrp, &[(100, false), (120, false), (90, false)]);
        b.run(40);
        assert_eq!(
            b.states(),
            vec![State::Standby, State::Active, State::Listen]
        );
    }

    #[test]
    fn hsrp_equal_priorities_go_by_address() {
        let mut b = Bus::new(Proto::Hsrp, &[(100, false), (100, false)]);
        b.run(40);
        assert_eq!(
            b.states(),
            vec![State::Standby, State::Active],
            "higher address wins"
        );
    }

    #[test]
    fn hsrp_standby_takes_over_after_the_hold_time_and_not_before() {
        let mut b = Bus::new(Proto::Hsrp, &[(100, false), (120, false)]);
        b.run(40);
        b.nodes[1].up = false; // dies silently: unplugged, not resigned
        b.nodes[1].g = Group::new(Proto::Hsrp, 1, Ipv4Addr([10, 0, 0, 254]));
        b.run(8);
        assert_eq!(b.nodes[0].g.state(), State::Standby, "hold time is 10 s");
        b.run(5);
        assert_eq!(b.nodes[0].g.state(), State::Active);
        assert!(b.nodes[0].g.take_garp());
    }

    #[test]
    fn a_resigning_active_router_hands_over_at_once() {
        let mut b = Bus::new(Proto::Hsrp, &[(100, false), (120, false)]);
        b.run(40);
        let mut out = vec![];
        let ip = b.nodes[1].ip;
        b.nodes[1].up = false;
        b.nodes[1]
            .g
            .step(SimTime::from_secs(b.now), ip, false, false, &mut out);
        assert!(out.iter().any(|m| m.kind == Kind::Resign));
        b.deliver(1, out);
        b.run(1);
        assert_eq!(b.nodes[0].g.state(), State::Active);
    }

    #[test]
    fn preempt_takes_the_role_back_and_without_it_the_role_stays_put() {
        for preempt in [false, true] {
            let mut b = Bus::new(Proto::Hsrp, &[(100, false), (120, preempt)]);
            b.nodes[1].up = false;
            b.run(40);
            assert_eq!(b.nodes[0].g.state(), State::Active);
            b.nodes[1].up = true;
            b.run(60);
            let want = if preempt {
                vec![State::Standby, State::Active]
            } else {
                vec![State::Active, State::Standby]
            };
            assert_eq!(b.states(), want, "preempt = {preempt}");
        }
    }

    #[test]
    fn vrrp_master_is_the_highest_priority_and_preempts_by_default() {
        let mut b = Bus::new(Proto::Vrrp, &[(100, true), (150, true), (50, true)]);
        b.run(10);
        assert_eq!(
            b.states(),
            vec![State::Backup, State::Master, State::Backup]
        );
        b.nodes[1].up = false;
        b.nodes[1].g = Group::new(Proto::Vrrp, 1, Ipv4Addr([10, 0, 0, 254]));
        b.run(8);
        assert_eq!(
            b.nodes[0].g.state(),
            State::Master,
            "3 x advert + skew later"
        );
        assert_eq!(b.nodes[2].g.state(), State::Backup);
        b.nodes[1].up = true;
        b.nodes[1].g.priority = 150;
        b.run(8);
        assert_eq!(
            b.states(),
            vec![State::Backup, State::Master, State::Backup]
        );
    }

    #[test]
    fn a_vrrp_owner_is_master_straight_away_whatever_its_configured_priority() {
        let mut g = Group::new(Proto::Vrrp, 1, Ipv4Addr([10, 0, 0, 1]));
        g.priority = 10;
        let mut out = vec![];
        g.step(
            SimTime::from_secs(1),
            Ipv4Addr([10, 0, 0, 1]),
            true,
            true,
            &mut out,
        );
        assert_eq!(g.state(), State::Master);
        assert_eq!(g.effective_priority(true), 255);
    }

    #[test]
    fn an_interface_going_down_resets_the_group() {
        let mut g = Group::new(Proto::Vrrp, 1, Ipv4Addr([10, 0, 0, 254]));
        let me = Ipv4Addr([10, 0, 0, 1]);
        let mut out = vec![];
        g.step(SimTime::from_secs(1), me, true, true, &mut out);
        out.clear();
        g.step(SimTime::from_secs(2), me, false, true, &mut out);
        assert_eq!(g.state(), State::Init);
        assert!(out
            .iter()
            .any(|m| m.kind == Kind::Resign && m.priority == 0));
    }
}
