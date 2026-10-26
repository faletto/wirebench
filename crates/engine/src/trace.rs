//! `tracert`, `traceroute` and a router's `ping`: send probes one at a time, run the simulation
//! until each is answered or times out, and print what came back.

use crate::World;
use kernel::{DeviceId, SimTime};
use netdev::{Host, L2Switch, ProbeBody, ProbeReply, Router};
use wire::{IcmpErrorKind, Ipv4Addr};

/// What kind of device the console is on; they send and log probes in different places.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Origin {
    Host,
    Router,
    /// A layer 3 switch: probes leave through its SVIs.
    Switch,
}

/// ICMP identifiers, so a traceroute's and a ping's echoes are told apart.
const TRACE_ID: u16 = 0x7472;
const PING_ID: u16 = 0x7061;
/// First UDP port a traceroute probes; each probe uses the next one.
const BASE_PORT: u16 = 33_434;
const PROBES_PER_HOP: u8 = 3;
const TRACE_TIMEOUT: SimTime = SimTime::from_secs(3);
const PING_TIMEOUT: SimTime = SimTime::from_secs(2);
const PING_COUNT: u8 = 5;

struct Prober {
    origin: Origin,
    id: DeviceId,
}

/// How one probe came out.
enum Answer {
    /// Nothing within the timeout.
    Silence,
    /// The first reply: who sent it and how long it took.
    Reply {
        from: Ipv4Addr,
        took: SimTime,
        kind: Kind,
    },
    /// The device had no route for it.
    NoRoute,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An intermediate router said the TTL ran out.
    Passed,
    /// The destination answered (an echo reply, or port unreachable for UDP).
    Arrived,
    /// A router said it cannot deliver (the code is ICMP's).
    Unreachable(u8),
}

impl Prober {
    fn log<'a>(&self, w: &'a World) -> &'a [ProbeReply] {
        match self.origin {
            Origin::Host => &w.device::<Host>(self.id).probes,
            Origin::Router => &w.device::<Router>(self.id).probes,
            Origin::Switch => &w.device::<L2Switch>(self.id).l3().probes,
        }
    }

    fn send_echo(&self, w: &mut World, dst: Ipv4Addr, ttl: u8, ident: u16, seq: u16) -> bool {
        match self.origin {
            Origin::Host => {
                w.with_device::<Host, _>(self.id, |h, c| {
                    h.send_echo_probe(c, dst, ttl, ident, seq)
                });
                true
            }
            Origin::Router => w.with_device::<Router, _>(self.id, |r, c| {
                let ok = r.send_echo_probe(c, dst, ttl, ident, seq);
                r.flush_outbox(c);
                ok
            }),
            Origin::Switch => w.with_device::<L2Switch, _>(self.id, |s, c| {
                s.originate(c, |r, c| r.send_echo_probe(c, dst, ttl, ident, seq))
            }),
        }
    }

    fn send_udp(&self, w: &mut World, dst: Ipv4Addr, ttl: u8, dport: u16) -> bool {
        match self.origin {
            Origin::Host => false,
            Origin::Router => w.with_device::<Router, _>(self.id, |r, c| {
                let ok = r.send_udp_probe(c, dst, ttl, dport);
                r.flush_outbox(c);
                ok
            }),
            Origin::Switch => w.with_device::<L2Switch, _>(self.id, |s, c| {
                s.originate(c, |r, c| r.send_udp_probe(c, dst, ttl, dport))
            }),
        }
    }

    /// Send one probe with `send` and wait for the reply that `is_mine` recognises.
    fn probe(
        &self,
        w: &mut World,
        timeout: SimTime,
        send: impl FnOnce(&Prober, &mut World) -> bool,
        is_mine: impl Fn(&ProbeReply) -> bool,
    ) -> Answer {
        let before = self.log(w).len();
        let sent = w.kernel.now();
        if !send(self, w) {
            return Answer::NoRoute;
        }
        let me = |w: &World| {
            self.log(w)[before.min(self.log(w).len())..]
                .iter()
                .any(&is_mine)
        };
        w.run_until_or(sent + timeout, me);
        let log = self.log(w);
        match log[before.min(log.len())..].iter().find(|r| is_mine(r)) {
            None => Answer::Silence,
            Some(r) => Answer::Reply {
                from: r.from,
                took: SimTime(r.at.0.saturating_sub(sent.0)),
                kind: match &r.body {
                    ProbeBody::EchoReply { .. } => Kind::Arrived,
                    ProbeBody::Error {
                        kind: IcmpErrorKind::TimeExceeded,
                        ..
                    } => Kind::Passed,
                    ProbeBody::Error {
                        kind: IcmpErrorKind::DestUnreachable(3),
                        ..
                    } => Kind::Arrived,
                    ProbeBody::Error {
                        kind: IcmpErrorKind::DestUnreachable(c),
                        ..
                    } => Kind::Unreachable(*c),
                },
            },
        }
    }
}

/// A time as a person reads it here: microseconds below a millisecond, milliseconds above.
fn took(t: SimTime, micro: &str) -> String {
    let us = t.0 / 1_000;
    if us < 1_000 {
        format!("{us} {micro}")
    } else {
        format!("{} ms", us / 1_000)
    }
}

/// `!N`, `!H` and friends: what a router that cannot deliver said.
fn unreachable_mark(code: u8) -> String {
    match code {
        0 => "!N".into(),
        1 => "!H".into(),
        2 => "!P".into(),
        9 | 10 | 13 => "!A".into(),
        c => format!("!{c}"),
    }
}

fn unreachable_words(code: u8) -> &'static str {
    match code {
        0 => "Destination net unreachable.",
        1 => "Destination host unreachable.",
        2 => "Destination protocol unreachable.",
        9 | 10 | 13 => "Destination administratively prohibited.",
        _ => "Destination unreachable.",
    }
}

/// A PC's `tracert`: echo requests with rising TTL, three per hop, Windows style.
pub(crate) fn tracert(w: &mut World, id: DeviceId, dst: Ipv4Addr, max_hops: u8) -> String {
    let p = Prober {
        origin: Origin::Host,
        id,
    };
    let mut out = format!("Tracing route to {dst} over a maximum of {max_hops} hops\n\n");
    let mut seq: u16 = 0;
    'hops: for ttl in 1..=max_hops {
        let mut cells = Vec::new();
        let mut from: Option<Ipv4Addr> = None;
        let mut stop: Option<Kind> = None;
        let mut code = 0;
        for _ in 0..PROBES_PER_HOP {
            seq += 1;
            let s = seq;
            let a = p.probe(
                w,
                TRACE_TIMEOUT,
                |p, w| p.send_echo(w, dst, ttl, TRACE_ID, s),
                |r| r.echo_id() == Some((TRACE_ID, s)),
            );
            match a {
                Answer::Silence => cells.push("*".to_string()),
                Answer::NoRoute => return "General failure.".to_string(),
                Answer::Reply {
                    from: f,
                    took: t,
                    kind,
                } => {
                    cells.push(took(t, "us"));
                    from = Some(f);
                    if kind != Kind::Passed {
                        stop = Some(kind);
                    }
                    if let Kind::Unreachable(c) = kind {
                        // Nothing more to learn at this hop; do not wait out more probes.
                        code = c;
                        break;
                    }
                }
            }
        }
        let times: String = cells.iter().map(|c| format!("{c:>9} ")).collect();
        match (from, stop) {
            (None, _) => out += &format!("{ttl:>3} {times} Request timed out.\n"),
            (Some(f), Some(Kind::Unreachable(_))) => {
                out += &format!("{ttl:>3}  {f}  reports: {}\n", unreachable_words(code));
                break 'hops;
            }
            (Some(f), s) => {
                out += &format!("{ttl:>3} {times} {f}\n");
                if s == Some(Kind::Arrived) {
                    break 'hops;
                }
            }
        }
    }
    out + "\nTrace complete."
}

/// A router's or layer 3 switch's `traceroute`: UDP probes to successive high ports.
pub(crate) fn traceroute(
    w: &mut World,
    origin: Origin,
    id: DeviceId,
    dst: Ipv4Addr,
    first_ttl: u8,
    max_ttl: u8,
) -> String {
    let p = Prober { origin, id };
    let mut out = format!("Type escape sequence to abort.\nTracing the route to {dst}\n");
    let mut port = BASE_PORT;
    for ttl in first_ttl..=max_ttl {
        let mut line = format!("{ttl:>3}");
        let mut last: Option<Ipv4Addr> = None;
        let mut arrived = false;
        let mut stopped = false;
        for _ in 0..PROBES_PER_HOP {
            let dport = port;
            port += 1;
            let a = p.probe(
                w,
                TRACE_TIMEOUT,
                |p, w| p.send_udp(w, dst, ttl, dport),
                |r| r.quoted_udp_dport() == Some(dport),
            );
            match a {
                Answer::NoRoute => return format!("% No route to {dst}"),
                Answer::Silence => line += " *",
                Answer::Reply {
                    from,
                    took: t,
                    kind,
                } => {
                    if last != Some(from) {
                        line += &format!(" {from}");
                        last = Some(from);
                    }
                    match kind {
                        Kind::Unreachable(c) => {
                            line += &format!(" {}", unreachable_mark(c));
                            stopped = true;
                            break;
                        }
                        Kind::Arrived => {
                            line += &format!(" {}", took(t, "usec"));
                            arrived = true;
                        }
                        Kind::Passed => line += &format!(" {}", took(t, "usec")),
                    }
                }
            }
        }
        out += &format!("{line}\n");
        if arrived || stopped {
            break;
        }
    }
    out.trim_end().to_string()
}

/// A router's `ping`: five echo requests, one at a time, as `!` (reply), `.` (nothing) or `U`.
pub(crate) fn router_ping(w: &mut World, origin: Origin, id: DeviceId, dst: Ipv4Addr) -> String {
    let p = Prober { origin, id };
    let mut marks = String::new();
    let mut times = Vec::new();
    for seq in 1..=u16::from(PING_COUNT) {
        let a = p.probe(
            w,
            PING_TIMEOUT,
            |p, w| p.send_echo(w, dst, 255, PING_ID, seq),
            |r| r.echo_id() == Some((PING_ID, seq)),
        );
        match a {
            Answer::NoRoute => marks.push('.'),
            Answer::Silence => marks.push('.'),
            Answer::Reply {
                took,
                kind: Kind::Arrived,
                ..
            } => {
                marks.push('!');
                times.push(took.0 / 1_000);
            }
            Answer::Reply { .. } => marks.push('U'),
        }
    }
    let ok = times.len();
    let mut out = format!(
        "Type escape sequence to abort.\nSending {PING_COUNT}, 100-byte ICMP Echos to {dst}, timeout is 2 seconds:\n{marks}\nSuccess rate is {} percent ({ok}/{PING_COUNT})",
        ok * 100 / usize::from(PING_COUNT)
    );
    if let (Some(min), Some(max)) = (times.iter().min(), times.iter().max()) {
        let avg = times.iter().sum::<u64>() / times.len() as u64;
        out += &format!(", round-trip min/avg/max = {min}/{avg}/{max} usec");
    }
    out
}
