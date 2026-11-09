//! A small TCP (RFC 9293) and UDP socket table shared by hosts, routers and switches.
//!
//! This is the part of TCP the simulated services need: the three-way handshake, a byte stream in
//! each direction with acknowledgements and go-back-N retransmission, an orderly close, and
//! resets. It is a state machine over [`TcpSegment`]s and knows nothing about devices or timers:
//! the owner feeds it segments and the time, and sends what comes back in `out`.
//!
//! Left out on purpose: options (so the segment size is the 536 bytes of RFC 9293 section 3.7.1),
//! congestion control, out-of-order buffering, window scaling and urgent data. The receive window is
//! fixed. Segments that arrive ahead of the expected one are dropped and re-acknowledged, which the
//! sender repairs by retransmitting from the first unacknowledged byte.

use kernel::SimTime;
use std::collections::{BTreeMap, VecDeque};
use wire::{Ipv4Addr, TcpSegment};

/// Largest payload in one segment.
pub const MSS: usize = 536;
/// The receive window we advertise.
pub const WINDOW: usize = 8192;
/// First retransmission timeout; doubles each time.
pub const RTO: SimTime = SimTime::from_secs(1);
/// Retransmissions before the connection is given up. With the doubling this is 31 seconds,
/// close to the half minute a Cisco `telnet` waits.
pub const MAX_RETRIES: u32 = 4;
/// How long a closed connection lingers to answer a repeated FIN (2 MSL, shortened).
pub const TIME_WAIT: SimTime = SimTime::from_secs(4);
/// First port handed to outgoing connections.
pub const EPHEMERAL_START: u16 = 49_152;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    SynSent,
    SynRcvd,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    Closed,
}

impl State {
    /// The name `show tcp` uses.
    pub fn name(self) -> &'static str {
        match self {
            State::SynSent => "SYNSENT",
            State::SynRcvd => "SYNRCVD",
            State::Established => "ESTAB",
            State::FinWait1 => "FINWAIT1",
            State::FinWait2 => "FINWAIT2",
            State::CloseWait => "CLOSEWAIT",
            State::Closing => "CLOSING",
            State::LastAck => "LASTACK",
            State::TimeWait => "TIMEWAIT",
            State::Closed => "CLOSED",
        }
    }
}

/// How a connection ended, when it did not end in an orderly way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The other end answered the SYN with a reset: nothing listens there.
    Refused,
    /// The other end reset an established connection.
    Reset,
    /// Nothing answered after [`MAX_RETRIES`] tries.
    TimedOut,
}

/// A segment to send, with the addresses for its pseudo header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub seg: TcpSegment,
}

#[derive(Clone, Debug)]
pub struct Conn {
    pub id: u32,
    pub local: (Ipv4Addr, u16),
    pub remote: (Ipv4Addr, u16),
    pub state: State,
    pub failure: Option<Failure>,
    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    snd_wnd: u32,
    rcv_nxt: u32,
    /// Sequence number of the first byte in `send_q`.
    q_seq: u32,
    /// Bytes the application wrote that are not yet acknowledged.
    send_q: VecDeque<u8>,
    /// Bytes received and not yet read.
    rx: VecDeque<u8>,
    fin_queued: bool,
    fin_sent: bool,
    /// The other end's FIN has arrived and all its data has been read into `rx`.
    pub peer_closed: bool,
    ack_needed: bool,
    deadline: Option<SimTime>,
    rto: SimTime,
    retries: u32,
    /// The application closed or released it: drop it once it reaches CLOSED.
    orphan: bool,
    /// Opened by a listener (waiting to be accepted, or accepted).
    passive: bool,
    time_wait_until: SimTime,
}

fn lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
fn le(a: u32, b: u32) -> bool {
    a == b || lt(a, b)
}

impl Conn {
    fn new(id: u32, local: (Ipv4Addr, u16), remote: (Ipv4Addr, u16), iss: u32) -> Self {
        Conn {
            id,
            local,
            remote,
            state: State::Closed,
            failure: None,
            iss,
            snd_una: iss,
            snd_nxt: iss,
            snd_wnd: 0,
            rcv_nxt: 0,
            q_seq: iss.wrapping_add(1),
            send_q: VecDeque::new(),
            rx: VecDeque::new(),
            fin_queued: false,
            fin_sent: false,
            peer_closed: false,
            ack_needed: false,
            deadline: None,
            rto: RTO,
            retries: 0,
            orphan: false,
            passive: false,
            time_wait_until: SimTime(0),
        }
    }

    fn window(&self) -> u16 {
        WINDOW.saturating_sub(self.rx.len()).min(u16::MAX as usize) as u16
    }

    fn seg(&self, seq: u32, flags: u8, payload: Vec<u8>) -> Tx {
        let ack = if flags & TcpSegment::ACK != 0 {
            self.rcv_nxt
        } else {
            0
        };
        Tx {
            src: self.local.0,
            dst: self.remote.0,
            seg: TcpSegment {
                src_port: self.local.1,
                dst_port: self.remote.1,
                seq,
                ack,
                flags,
                window: self.window(),
                payload,
            },
        }
    }

    fn arm(&mut self, now: SimTime) {
        self.deadline = Some(now + self.rto);
    }

    fn outstanding(&self) -> bool {
        match self.state {
            State::SynSent | State::SynRcvd => true,
            State::Closed | State::TimeWait => false,
            _ => self.snd_una != self.snd_nxt,
        }
    }

    /// Send what the window allows: new data, then the FIN once the data is out.
    fn output(&mut self, now: SimTime, out: &mut Vec<Tx>) {
        if matches!(
            self.state,
            State::SynSent | State::SynRcvd | State::Closed | State::TimeWait
        ) {
            return;
        }
        loop {
            let sent = self.snd_nxt.wrapping_sub(self.q_seq) as usize;
            let sent = sent.min(self.send_q.len());
            let unsent = self.send_q.len() - sent;
            let in_flight = self.snd_nxt.wrapping_sub(self.snd_una) as usize;
            let room = (self.snd_wnd as usize).saturating_sub(in_flight);
            if unsent > 0 && room > 0 && !self.fin_sent {
                let n = unsent.min(MSS).min(room);
                let data: Vec<u8> = self.send_q.iter().skip(sent).take(n).copied().collect();
                let tx = self.seg(
                    self.snd_nxt,
                    TcpSegment::ACK | TcpSegment::PSH,
                    data,
                );
                self.snd_nxt = self.snd_nxt.wrapping_add(n as u32);
                self.ack_needed = false;
                out.push(tx);
                continue;
            }
            if unsent == 0 && self.fin_queued && !self.fin_sent {
                let tx = self.seg(self.snd_nxt, TcpSegment::FIN | TcpSegment::ACK, vec![]);
                self.snd_nxt = self.snd_nxt.wrapping_add(1);
                self.fin_sent = true;
                self.ack_needed = false;
                match self.state {
                    State::Established => self.state = State::FinWait1,
                    State::CloseWait => self.state = State::LastAck,
                    _ => {}
                }
                out.push(tx);
            }
            break;
        }
        if self.outstanding() && self.deadline.is_none() {
            self.arm(now);
        }
    }

    fn finish(&mut self, failure: Option<Failure>) {
        self.state = State::Closed;
        self.failure = failure;
        self.deadline = None;
    }

    /// Process an ACK number. Returns true if our FIN is now acknowledged.
    fn on_ack(&mut self, now: SimTime, ack: u32) -> bool {
        let mut fin_acked = false;
        if lt(self.snd_una, ack) && le(ack, self.snd_nxt) {
            let data_end = self.q_seq.wrapping_add(self.send_q.len() as u32);
            let upto = if lt(data_end, ack) { data_end } else { ack };
            let n = upto.wrapping_sub(self.q_seq) as usize;
            if lt(self.q_seq, upto) {
                self.send_q.drain(..n.min(self.send_q.len()));
                self.q_seq = upto;
            }
            if self.fin_sent && lt(data_end, ack) {
                fin_acked = true;
            }
            self.snd_una = ack;
            self.rto = RTO;
            self.retries = 0;
            self.deadline = None;
            if self.outstanding() {
                self.arm(now);
            }
        }
        fin_acked
    }
}

/// A bound UDP port and what has arrived on it.
#[derive(Clone, Debug, Default)]
pub struct UdpSocket {
    pub inbox: VecDeque<(Ipv4Addr, u16, Vec<u8>)>,
}

#[derive(Clone, Debug)]
pub struct Tcp {
    conns: BTreeMap<u32, Conn>,
    next_id: u32,
    next_port: u16,
    listeners: Vec<u16>,
    accept_q: VecDeque<u32>,
    /// Bound UDP ports (the "minimal UDP sockets").
    pub udp: BTreeMap<u16, UdpSocket>,
    /// Added to initial sequence numbers so two devices do not pick the same ones.
    salt: u32,
}

impl Default for Tcp {
    fn default() -> Self {
        Tcp::new(0)
    }
}

impl Tcp {
    pub fn new(salt: u32) -> Self {
        Tcp {
            conns: BTreeMap::new(),
            next_id: 1,
            next_port: EPHEMERAL_START,
            listeners: Vec::new(),
            accept_q: VecDeque::new(),
            udp: BTreeMap::new(),
            salt,
        }
    }

    // ---- UDP ----

    pub fn udp_bind(&mut self, port: u16) -> bool {
        if self.udp.contains_key(&port) {
            return false;
        }
        self.udp.insert(port, UdpSocket::default());
        true
    }

    pub fn udp_close(&mut self, port: u16) {
        self.udp.remove(&port);
    }

    /// A datagram for `port`: queued if a socket is bound there. False if nobody is.
    pub fn udp_deliver(&mut self, src: Ipv4Addr, sport: u16, port: u16, data: &[u8]) -> bool {
        match self.udp.get_mut(&port) {
            Some(s) => {
                s.inbox.push_back((src, sport, data.to_vec()));
                true
            }
            None => false,
        }
    }

    pub fn udp_recv(&mut self, port: u16) -> Option<(Ipv4Addr, u16, Vec<u8>)> {
        self.udp.get_mut(&port)?.inbox.pop_front()
    }

    // ---- TCP: application side ----

    /// Accept SYNs on exactly these ports.
    pub fn set_listeners(&mut self, ports: &[u16]) {
        self.listeners = ports.to_vec();
    }

    pub fn listening(&self, port: u16) -> bool {
        self.listeners.contains(&port)
    }

    fn iss(&mut self, now: SimTime) -> u32 {
        // RFC 9293 3.4.1: a clock that ticks every 4 microseconds.
        ((now.0 / 4_000) as u32).wrapping_add(self.salt).wrapping_add(self.next_id.wrapping_mul(64_000))
    }

    fn ephemeral(&mut self, remote: (Ipv4Addr, u16)) -> u16 {
        loop {
            let p = self.next_port;
            self.next_port = if p == u16::MAX { EPHEMERAL_START } else { p + 1 };
            if !self.conns.values().any(|c| {
                c.local.1 == p && c.remote == remote && c.state != State::Closed
            }) {
                return p;
            }
        }
    }

    /// Open a connection from `src` to `dst`:`dport`. The SYN is in `out`.
    pub fn connect(
        &mut self,
        now: SimTime,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        dport: u16,
        out: &mut Vec<Tx>,
    ) -> u32 {
        let id = self.next_id;
        let iss = self.iss(now);
        self.next_id += 1;
        let sport = self.ephemeral((dst, dport));
        let mut c = Conn::new(id, (src, sport), (dst, dport), iss);
        c.state = State::SynSent;
        c.snd_nxt = iss.wrapping_add(1);
        let tx = c.seg(iss, TcpSegment::SYN, vec![]);
        c.arm(now);
        out.push(tx);
        self.conns.insert(id, c);
        id
    }

    pub fn conn(&self, id: u32) -> Option<&Conn> {
        self.conns.get(&id)
    }

    pub fn conns(&self) -> impl Iterator<Item = &Conn> {
        self.conns.values()
    }

    /// The next connection a listener has completed the handshake for.
    pub fn accept(&mut self) -> Option<u32> {
        while let Some(id) = self.accept_q.pop_front() {
            if self.conns.contains_key(&id) {
                return Some(id);
            }
        }
        None
    }

    /// Queue bytes to send. False if the connection cannot send (closed, or closing).
    pub fn send(&mut self, now: SimTime, id: u32, data: &[u8], out: &mut Vec<Tx>) -> bool {
        let Some(c) = self.conns.get_mut(&id) else {
            return false;
        };
        if !matches!(c.state, State::Established | State::CloseWait) || c.fin_queued {
            return false;
        }
        c.send_q.extend(data.iter().copied());
        c.output(now, out);
        true
    }

    /// Take the bytes that have arrived. Reading reopens the window.
    pub fn read(&mut self, now: SimTime, id: u32, out: &mut Vec<Tx>) -> Vec<u8> {
        let Some(c) = self.conns.get_mut(&id) else {
            return vec![];
        };
        let was_closed = c.window() == 0;
        let data: Vec<u8> = c.rx.drain(..).collect();
        if was_closed && !data.is_empty() && c.state != State::Closed {
            c.ack_needed = true;
            Self::flush_ack(c, now, out);
        }
        data
    }

    fn flush_ack(c: &mut Conn, now: SimTime, out: &mut Vec<Tx>) {
        c.output(now, out);
        if c.ack_needed {
            c.ack_needed = false;
            out.push(c.seg(c.snd_nxt, TcpSegment::ACK, vec![]));
        }
    }

    /// Start an orderly close: the FIN follows the data already written.
    pub fn close(&mut self, now: SimTime, id: u32, out: &mut Vec<Tx>) {
        let Some(c) = self.conns.get_mut(&id) else {
            return;
        };
        c.orphan = true;
        match c.state {
            State::SynSent => c.finish(None),
            State::SynRcvd | State::Established | State::CloseWait => {
                c.fin_queued = true;
                if c.state == State::SynRcvd {
                    // Not yet open: the handshake finishes first, then the FIN goes.
                } else {
                    c.output(now, out);
                }
            }
            _ => {}
        }
        self.purge();
    }

    /// Abort: send a reset and forget the connection.
    pub fn abort(&mut self, id: u32, out: &mut Vec<Tx>) {
        if let Some(c) = self.conns.get_mut(&id) {
            if !matches!(c.state, State::Closed | State::SynSent | State::TimeWait) {
                out.push(c.seg(c.snd_nxt, TcpSegment::RST | TcpSegment::ACK, vec![]));
            }
            c.orphan = true;
            c.finish(None);
        }
        self.purge();
    }

    /// The application is done with a connection that has ended (reset, refused, timed out).
    pub fn release(&mut self, id: u32) {
        if let Some(c) = self.conns.get_mut(&id) {
            c.orphan = true;
        }
        self.purge();
    }

    fn purge(&mut self) {
        self.conns
            .retain(|_, c| !(c.orphan && c.state == State::Closed));
    }

    // ---- TCP: network side ----

    /// The soonest time something needs doing (a retransmission or a lingering close ending).
    pub fn next_deadline(&self) -> Option<SimTime> {
        self.conns
            .values()
            .filter_map(|c| match c.state {
                State::TimeWait => Some(c.time_wait_until),
                _ => c.deadline,
            })
            .min()
    }

    /// Retransmit what is overdue and expire what has waited long enough.
    pub fn tick(&mut self, now: SimTime, out: &mut Vec<Tx>) {
        for c in self.conns.values_mut() {
            if c.state == State::TimeWait && now >= c.time_wait_until {
                c.finish(None);
                continue;
            }
            let Some(d) = c.deadline else { continue };
            if now < d {
                continue;
            }
            if c.retries >= MAX_RETRIES {
                c.finish(Some(Failure::TimedOut));
                continue;
            }
            c.retries += 1;
            c.rto = SimTime(c.rto.0 * 2);
            match c.state {
                State::SynSent => out.push(c.seg(c.iss, TcpSegment::SYN, vec![])),
                State::SynRcvd => {
                    out.push(c.seg(c.iss, TcpSegment::SYN | TcpSegment::ACK, vec![]))
                }
                _ => {
                    // Go back to the first unacknowledged byte and send again.
                    c.snd_nxt = c.snd_una;
                    if c.fin_sent {
                        c.fin_sent = false;
                        match c.state {
                            State::FinWait1 => c.state = State::Established,
                            State::LastAck => c.state = State::CloseWait,
                            _ => {}
                        }
                    }
                    c.deadline = None;
                    c.output(now, out);
                }
            }
            if c.state != State::Closed {
                c.arm(now);
            }
        }
        self.purge();
    }

    /// A TCP segment from `src` to `dst` (both ours). `raw` is the IP payload; segments with a bad
    /// checksum are dropped.
    pub fn on_ip(&mut self, now: SimTime, src: Ipv4Addr, dst: Ipv4Addr, raw: &[u8], out: &mut Vec<Tx>) {
        if !TcpSegment::checksum_ok(raw, src, dst) {
            return;
        }
        let Ok(seg) = TcpSegment::parse(raw) else {
            return;
        };
        self.on_segment(now, src, dst, seg, out);
    }

    fn find(&self, local: (Ipv4Addr, u16), remote: (Ipv4Addr, u16)) -> Option<u32> {
        self.conns
            .values()
            .find(|c| c.local == local && c.remote == remote && c.state != State::Closed)
            .map(|c| c.id)
    }

    pub fn on_segment(
        &mut self,
        now: SimTime,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        seg: TcpSegment,
        out: &mut Vec<Tx>,
    ) {
        let local = (dst, seg.dst_port);
        let remote = (src, seg.src_port);
        let Some(id) = self.find(local, remote) else {
            self.no_connection(now, local, remote, &seg, out);
            return;
        };
        let c = self.conns.get_mut(&id).unwrap();
        Self::segment(c, now, &seg, &mut self.accept_q, out);
        self.purge();
    }

    /// A segment for a connection that does not exist: open one if it is a SYN to a listener,
    /// else reset the sender (RFC 9293 section 3.10.7.1).
    fn no_connection(
        &mut self,
        now: SimTime,
        local: (Ipv4Addr, u16),
        remote: (Ipv4Addr, u16),
        seg: &TcpSegment,
        out: &mut Vec<Tx>,
    ) {
        if seg.flags & TcpSegment::RST != 0 {
            return;
        }
        if seg.flags & (TcpSegment::SYN | TcpSegment::ACK) == TcpSegment::SYN
            && self.listeners.contains(&local.1)
        {
            let id = self.next_id;
            let iss = self.iss(now);
            self.next_id += 1;
            let mut c = Conn::new(id, local, remote, iss);
            c.state = State::SynRcvd;
            c.passive = true;
            c.rcv_nxt = seg.seq.wrapping_add(1);
            c.snd_wnd = u32::from(seg.window);
            c.snd_nxt = iss.wrapping_add(1);
            out.push(c.seg(iss, TcpSegment::SYN | TcpSegment::ACK, vec![]));
            c.arm(now);
            self.conns.insert(id, c);
            return;
        }
        let rst = if seg.flags & TcpSegment::ACK != 0 {
            TcpSegment {
                src_port: local.1,
                dst_port: remote.1,
                seq: seg.ack,
                ack: 0,
                flags: TcpSegment::RST,
                window: 0,
                payload: vec![],
            }
        } else {
            let mut len = seg.payload.len() as u32;
            if seg.flags & TcpSegment::SYN != 0 {
                len += 1;
            }
            if seg.flags & TcpSegment::FIN != 0 {
                len += 1;
            }
            TcpSegment {
                src_port: local.1,
                dst_port: remote.1,
                seq: 0,
                ack: seg.seq.wrapping_add(len),
                flags: TcpSegment::RST | TcpSegment::ACK,
                window: 0,
                payload: vec![],
            }
        };
        out.push(Tx {
            src: local.0,
            dst: remote.0,
            seg: rst,
        });
    }

    fn segment(
        c: &mut Conn,
        now: SimTime,
        seg: &TcpSegment,
        accept_q: &mut VecDeque<u32>,
        out: &mut Vec<Tx>,
    ) {
        let has = |f: u8| seg.flags & f != 0;
        // Reset.
        if has(TcpSegment::RST) {
            match c.state {
                State::SynSent => {
                    if has(TcpSegment::ACK) && seg.ack == c.iss.wrapping_add(1) {
                        c.finish(Some(Failure::Refused));
                    }
                }
                State::Closed | State::TimeWait => {}
                _ => {
                    if seg.seq == c.rcv_nxt {
                        let f = if c.state == State::SynRcvd {
                            Failure::Refused
                        } else {
                            Failure::Reset
                        };
                        c.finish(Some(f));
                    }
                }
            }
            return;
        }
        if c.state == State::SynSent {
            if has(TcpSegment::SYN) && has(TcpSegment::ACK) && seg.ack == c.iss.wrapping_add(1) {
                c.rcv_nxt = seg.seq.wrapping_add(1);
                c.snd_una = seg.ack;
                c.snd_wnd = u32::from(seg.window);
                c.state = State::Established;
                c.deadline = None;
                c.rto = RTO;
                c.retries = 0;
                out.push(c.seg(c.snd_nxt, TcpSegment::ACK, vec![]));
                c.output(now, out);
            } else if has(TcpSegment::ACK) {
                // An ACK for something we did not send: reset it.
                out.push(Tx {
                    src: c.local.0,
                    dst: c.remote.0,
                    seg: TcpSegment {
                        src_port: c.local.1,
                        dst_port: c.remote.1,
                        seq: seg.ack,
                        ack: 0,
                        flags: TcpSegment::RST,
                        window: 0,
                        payload: vec![],
                    },
                });
            }
            return;
        }
        if c.state == State::SynRcvd {
            if has(TcpSegment::SYN) && !has(TcpSegment::ACK) {
                // The client repeated its SYN: our SYN-ACK was lost.
                out.push(c.seg(c.iss, TcpSegment::SYN | TcpSegment::ACK, vec![]));
                return;
            }
            if has(TcpSegment::ACK) && seg.ack == c.iss.wrapping_add(1) {
                c.state = State::Established;
                c.snd_una = seg.ack;
                c.snd_wnd = u32::from(seg.window);
                c.deadline = None;
                c.rto = RTO;
                c.retries = 0;
                if c.fin_queued {
                    // Closed by the application before it finished opening.
                } else if c.passive {
                    accept_q.push_back(c.id);
                }
            } else {
                return;
            }
        }
        if c.state == State::TimeWait {
            // A repeated FIN: acknowledge it again.
            if has(TcpSegment::FIN) {
                out.push(c.seg(c.snd_nxt, TcpSegment::ACK, vec![]));
                c.time_wait_until = now + TIME_WAIT;
            }
            return;
        }
        if c.state == State::Closed {
            return;
        }
        if has(TcpSegment::ACK) {
            c.snd_wnd = u32::from(seg.window);
            let fin_acked = c.on_ack(now, seg.ack);
            if fin_acked {
                match c.state {
                    State::FinWait1 => c.state = State::FinWait2,
                    State::Closing => {
                        c.state = State::TimeWait;
                        c.time_wait_until = now + TIME_WAIT;
                        c.deadline = None;
                    }
                    State::LastAck => {
                        c.finish(None);
                        return;
                    }
                    _ => {}
                }
            }
        }
        // Data.
        let mut payload: &[u8] = &seg.payload;
        let mut seq = seg.seq;
        if lt(seq, c.rcv_nxt) {
            // Already have the front of it (a retransmission): keep only what is new.
            let skip = c.rcv_nxt.wrapping_sub(seq) as usize;
            if skip >= payload.len() {
                payload = &[];
            } else {
                payload = &payload[skip..];
            }
            seq = c.rcv_nxt;
        }
        let receiving = matches!(
            c.state,
            State::Established | State::FinWait1 | State::FinWait2
        );
        if !seg.payload.is_empty() || has(TcpSegment::FIN) {
            c.ack_needed = true;
        }
        if receiving && seq == c.rcv_nxt && !payload.is_empty() {
            let room = WINDOW.saturating_sub(c.rx.len());
            let take = payload.len().min(room);
            c.rx.extend(payload[..take].iter().copied());
            c.rcv_nxt = c.rcv_nxt.wrapping_add(take as u32);
            if take < payload.len() {
                // No room: the rest is ignored and will be sent again.
                Self::flush_ack(c, now, out);
                return;
            }
        }
        let fin_at = seg.seq.wrapping_add(seg.payload.len() as u32);
        if has(TcpSegment::FIN) && fin_at == c.rcv_nxt && !c.peer_closed {
            c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
            c.peer_closed = true;
            match c.state {
                State::Established => c.state = State::CloseWait,
                State::FinWait1 => c.state = State::Closing,
                State::FinWait2 => {
                    c.state = State::TimeWait;
                    c.time_wait_until = now + TIME_WAIT;
                    c.deadline = None;
                }
                _ => {}
            }
        }
        Self::flush_ack(c, now, out);
        // A close the application asked for while the handshake was still going.
        if c.fin_queued && c.state == State::Established {
            c.output(now, out);
        }
    }

    pub fn state_of(&self, id: u32) -> Option<State> {
        self.conns.get(&id).map(|c| c.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a() -> Ipv4Addr {
        Ipv4Addr::new(10, 0, 0, 1)
    }
    fn b() -> Ipv4Addr {
        Ipv4Addr::new(10, 0, 0, 2)
    }

    /// Two stacks joined by a wire that can lose segments.
    struct Net {
        a: Tcp,
        b: Tcp,
        now: SimTime,
        drop_next: usize,
        log: Vec<(char, u8, usize)>,
    }

    impl Net {
        fn new() -> Self {
            let mut n = Net {
                a: Tcp::new(0),
                b: Tcp::new(7),
                now: SimTime::from_secs(1),
                drop_next: 0,
                log: vec![],
            };
            n.b.set_listeners(&[23]);
            n
        }

        /// Deliver segments back and forth until nothing is left.
        fn pump(&mut self, mut from_a: Vec<Tx>, mut from_b: Vec<Tx>) {
            for _ in 0..200 {
                if from_a.is_empty() && from_b.is_empty() {
                    return;
                }
                let mut next_a = vec![];
                let mut next_b = vec![];
                for tx in from_a.drain(..) {
                    self.log.push(('a', tx.seg.flags, tx.seg.payload.len()));
                    if self.drop_next > 0 {
                        self.drop_next -= 1;
                        continue;
                    }
                    let raw = tx.seg.emit(tx.src, tx.dst);
                    self.b.on_ip(self.now, tx.src, tx.dst, &raw, &mut next_b);
                }
                for tx in from_b.drain(..) {
                    self.log.push(('b', tx.seg.flags, tx.seg.payload.len()));
                    if self.drop_next > 0 {
                        self.drop_next -= 1;
                        continue;
                    }
                    let raw = tx.seg.emit(tx.src, tx.dst);
                    self.a.on_ip(self.now, tx.src, tx.dst, &raw, &mut next_a);
                }
                from_a = next_a;
                from_b = next_b;
            }
            panic!("segments never stopped");
        }

        /// Move time forward, running both stacks' timers.
        fn advance(&mut self, secs: u64) {
            self.now = self.now + SimTime::from_secs(secs);
            let (mut oa, mut ob) = (vec![], vec![]);
            self.a.tick(self.now, &mut oa);
            self.b.tick(self.now, &mut ob);
            self.pump(oa, ob);
        }

        fn open(&mut self) -> (u32, u32) {
            let mut out = vec![];
            let ca = self.a.connect(self.now, a(), b(), 23, &mut out);
            self.pump(out, vec![]);
            let cb = self.b.accept().expect("accepted");
            (ca, cb)
        }
    }

    #[test]
    fn handshake_opens_both_ends_and_the_server_accepts() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        assert_eq!(n.a.state_of(ca), Some(State::Established));
        assert_eq!(n.b.state_of(cb), Some(State::Established));
        assert_eq!(n.b.conn(cb).unwrap().remote, (a(), n.a.conn(ca).unwrap().local.1));
        assert_eq!(n.b.accept(), None);
        // SYN, SYN-ACK, ACK.
        let flags: Vec<u8> = n.log.iter().map(|l| l.1).collect();
        assert_eq!(
            flags,
            vec![
                TcpSegment::SYN,
                TcpSegment::SYN | TcpSegment::ACK,
                TcpSegment::ACK
            ]
        );
    }

    #[test]
    fn data_flows_both_ways_and_is_acknowledged() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        let mut out = vec![];
        assert!(n.a.send(n.now, ca, b"hello", &mut out));
        n.pump(out, vec![]);
        let mut out = vec![];
        assert_eq!(n.b.read(n.now, cb, &mut out), b"hello");
        let mut out = vec![];
        assert!(n.b.send(n.now, cb, b"world!", &mut out));
        n.pump(vec![], out);
        let mut out = vec![];
        assert_eq!(n.a.read(n.now, ca, &mut out), b"world!");
        // Everything acknowledged: nothing left to retransmit.
        assert_eq!(n.a.next_deadline(), None);
        assert_eq!(n.b.next_deadline(), None);
    }

    #[test]
    fn a_long_write_is_split_into_segments_and_arrives_in_order() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let mut out = vec![];
        n.a.send(n.now, ca, &data, &mut out);
        assert!(out.iter().all(|t| t.seg.payload.len() <= MSS));
        n.pump(out, vec![]);
        let mut out = vec![];
        assert_eq!(n.b.read(n.now, cb, &mut out), data);
    }

    #[test]
    fn a_lost_segment_is_sent_again_after_the_timeout() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        n.drop_next = 1;
        let mut out = vec![];
        n.a.send(n.now, ca, b"ping", &mut out);
        n.pump(out, vec![]);
        let mut o = vec![];
        assert!(n.b.read(n.now, cb, &mut o).is_empty());
        assert!(n.a.next_deadline().is_some());
        n.advance(1);
        let mut o = vec![];
        assert_eq!(n.b.read(n.now, cb, &mut o), b"ping");
        assert_eq!(n.a.next_deadline(), None);
    }

    #[test]
    fn a_lost_syn_ack_is_repaired_by_the_clients_retry() {
        let mut n = Net::new();
        let mut out = vec![];
        let ca = n.a.connect(n.now, a(), b(), 23, &mut out);
        // Lose the SYN-ACK: the first segment b sends.
        let mut from_b = vec![];
        let raw = out[0].seg.emit(a(), b());
        n.b.on_ip(n.now, a(), b(), &raw, &mut from_b);
        assert_eq!(from_b.len(), 1);
        n.advance(1);
        assert_eq!(n.a.state_of(ca), Some(State::Established));
        assert!(n.b.accept().is_some());
    }

    #[test]
    fn orderly_close_from_either_end() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        let mut out = vec![];
        n.a.send(n.now, ca, b"bye", &mut out);
        n.a.close(n.now, ca, &mut out);
        n.pump(out, vec![]);
        // b has the data and then end-of-file.
        let mut o = vec![];
        assert_eq!(n.b.read(n.now, cb, &mut o), b"bye");
        assert!(n.b.conn(cb).unwrap().peer_closed);
        assert_eq!(n.b.state_of(cb), Some(State::CloseWait));
        let mut out = vec![];
        n.b.close(n.now, cb, &mut out);
        n.pump(vec![], out);
        assert_eq!(n.b.state_of(cb), None, "passive closer is gone once acked");
        assert_eq!(n.a.state_of(ca), Some(State::TimeWait));
        n.advance(5);
        assert_eq!(n.a.state_of(ca), None);
    }

    #[test]
    fn a_connection_to_a_closed_port_is_refused() {
        let mut n = Net::new();
        let mut out = vec![];
        let ca = n.a.connect(n.now, a(), b(), 80, &mut out);
        n.pump(out, vec![]);
        let c = n.a.conn(ca).unwrap();
        assert_eq!(c.state, State::Closed);
        assert_eq!(c.failure, Some(Failure::Refused));
    }

    #[test]
    fn a_silent_peer_times_out_after_the_retries() {
        let mut n = Net::new();
        let mut out = vec![];
        let ca = n.a.connect(n.now, a(), b(), 23, &mut out);
        n.drop_next = usize::MAX;
        n.pump(out, vec![]);
        for _ in 0..40 {
            n.advance(1);
        }
        let c = n.a.conn(ca).unwrap();
        assert_eq!(c.failure, Some(Failure::TimedOut));
    }

    #[test]
    fn abort_resets_the_peer() {
        let mut n = Net::new();
        let (ca, cb) = n.open();
        let mut out = vec![];
        n.a.abort(ca, &mut out);
        n.pump(out, vec![]);
        assert_eq!(n.b.conn(cb).unwrap().failure, Some(Failure::Reset));
    }

    #[test]
    fn a_corrupt_checksum_is_ignored() {
        let mut n = Net::new();
        let mut out = vec![];
        n.a.connect(n.now, a(), b(), 23, &mut out);
        let mut raw = out[0].seg.emit(a(), b());
        raw[4] ^= 1;
        let mut back = vec![];
        n.b.on_ip(n.now, a(), b(), &raw, &mut back);
        assert!(back.is_empty());
        assert!(n.b.accept().is_none());
    }

    #[test]
    fn udp_sockets_queue_datagrams_for_bound_ports_only() {
        let mut t = Tcp::new(0);
        assert!(t.udp_bind(5000));
        assert!(!t.udp_bind(5000));
        assert!(t.udp_deliver(a(), 4000, 5000, b"x"));
        assert!(!t.udp_deliver(a(), 4000, 5001, b"x"));
        assert_eq!(t.udp_recv(5000), Some((a(), 4000, b"x".to_vec())));
        assert_eq!(t.udp_recv(5000), None);
        t.udp_close(5000);
        assert!(!t.udp_deliver(a(), 4000, 5000, b"x"));
    }
}
