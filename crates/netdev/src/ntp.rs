//! A device's clock and NTP (RFC 5905, client and server, no authentication).
//!
//! The simulated clock is the simulation time plus an offset the device keeps, so two devices
//! only disagree about the time when somebody set one by hand.

use kernel::SimTime;
use wire::{
    ntp_from_timestamp, ntp_to_timestamp, Ipv4Addr, NtpPacket, Timestamp, NTP_MODE_CLIENT,
    NTP_MODE_SERVER,
};

/// Replies a client needs from a server before it will trust it. (A real client needs several
/// polls over minutes as well; the number is a simplification.)
pub const SAMPLES_NEEDED: u32 = 3;
/// Seconds between polls.
pub const POLL_SECS: u64 = 64;
pub const DEFAULT_MASTER_STRATUM: u8 = 8;

const NS: i64 = 1_000_000_000;

/// 1993-03-01 00:00:00 UTC, what a device reads before anyone sets it.
fn epoch_1993() -> i64 {
    unix_from_civil(1993, 3, 1, 0, 0, 0)
}

#[derive(Clone, Debug)]
pub struct Clock {
    /// Unix seconds when the simulation clock read zero.
    base: i64,
    /// What NTP (or `clock set`) has added since.
    adjust_ns: i64,
    /// `clock timezone`: name and minutes east of UTC.
    pub zone: Option<(String, i32)>,
    pub set_by_hand: bool,
    pub ever_synced: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Clock {
            base: epoch_1993(),
            adjust_ns: 0,
            zone: None,
            set_by_hand: false,
            ever_synced: false,
        }
    }
}

impl Clock {
    /// Unix nanoseconds now.
    pub fn now_ns(&self, sim: SimTime) -> i64 {
        self.base * NS + sim.0 as i64 + self.adjust_ns
    }

    /// `clock set`: make it `unix_secs` now.
    pub fn set(&mut self, sim: SimTime, unix_secs: i64) {
        self.adjust_ns = unix_secs * NS - self.base * NS - sim.0 as i64;
        self.set_by_hand = true;
    }

    /// Move the clock by `delta_ns`.
    pub fn step(&mut self, delta_ns: i64) {
        self.adjust_ns += delta_ns;
    }

    /// Whole seconds since the Unix epoch, shifted into the configured time zone.
    pub fn local_secs(&self, sim: SimTime) -> i64 {
        let zone = self.zone.as_ref().map_or(0, |z| z.1);
        self.now_ns(sim).div_euclid(NS) + i64::from(zone) * 60
    }
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month, day)` for days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn unix_from_civil(y: i64, m: i64, d: i64, hh: i64, mm: i64, ss: i64) -> i64 {
    days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss
}

pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// `(year, month 1..12, day, hour, minute, second, weekday name)` for Unix seconds.
pub fn civil_from_unix(secs: i64) -> (i64, usize, i64, i64, i64, i64, &'static str) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m as usize,
        d,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        DAYS[days.rem_euclid(7) as usize],
    )
}

/// A configured server and what is known about it.
#[derive(Clone, Debug)]
pub struct Assoc {
    pub addr: Ipv4Addr,
    /// One bit per poll, newest lowest; set when an answer came.
    pub reach: u8,
    pub stratum: u8,
    pub ref_id: [u8; 4],
    pub delay_ns: i64,
    pub offset_ns: i64,
    pub last_rx: Option<SimTime>,
    next_poll: SimTime,
    pending: Option<Timestamp>,
    good: u32,
    root_delay_ns: i64,
}

/// What the clock is synchronised to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncState {
    pub stratum: u8,
    pub ref_id: [u8; 4],
    /// The server it follows; `None` when it is its own source (`ntp master`).
    pub peer: Option<Ipv4Addr>,
    pub reference_ns: i64,
    pub offset_ns: i64,
    pub root_delay_ns: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Ntp {
    /// `ntp master [stratum]`.
    pub master: Option<u8>,
    pub assocs: Vec<Assoc>,
    sync: Option<SyncState>,
    pub sent: u64,
    pub received: u64,
}

fn fixed(ns: i64) -> u32 {
    ((ns.max(0) as i128 * 65_536) / i128::from(NS)) as u32
}

fn unfixed(v: u32) -> i64 {
    ((i128::from(v) * i128::from(NS)) / 65_536) as i64
}

impl Ntp {
    /// Servers to poll are the only thing that needs the timer.
    pub fn wants_tick(&self) -> bool {
        !self.assocs.is_empty()
    }

    pub fn add_server(&mut self, addr: Ipv4Addr, now: SimTime) {
        if self.assocs.iter().any(|a| a.addr == addr) {
            return;
        }
        self.assocs.push(Assoc {
            addr,
            reach: 0,
            stratum: 16,
            ref_id: [0; 4],
            delay_ns: 0,
            offset_ns: 0,
            last_rx: None,
            next_poll: now,
            pending: None,
            good: 0,
            root_delay_ns: 0,
        });
    }

    pub fn remove_server(&mut self, addr: Ipv4Addr) -> bool {
        let n = self.assocs.len();
        self.assocs.retain(|a| a.addr != addr);
        if self.sync.as_ref().and_then(|s| s.peer) == Some(addr) {
            self.sync = None;
        }
        self.assocs.len() != n
    }

    /// The state to report: its own if it is a master, else what it learned.
    pub fn state(&self) -> Option<SyncState> {
        match self.master {
            Some(st) => Some(SyncState {
                stratum: st,
                ref_id: *b"LOCL",
                peer: None,
                reference_ns: self.sync.as_ref().map_or(0, |s| s.reference_ns),
                offset_ns: 0,
                root_delay_ns: 0,
            }),
            None => self.sync.clone(),
        }
    }

    /// Stratum this device answers clients with, if it is synchronised at all.
    fn serving(&self) -> Option<SyncState> {
        self.state()
    }

    /// Drop everything, as after a reload.
    pub fn reset(&mut self) {
        *self = Ntp::default();
    }

    /// Poll the servers that are due. `out` gets `(server, UDP payload)`.
    pub fn tick(&mut self, now: SimTime, clock: &Clock, out: &mut Vec<(Ipv4Addr, Vec<u8>)>) {
        let t1 = ntp_to_timestamp(clock.now_ns(now));
        let mut lost = None;
        for a in &mut self.assocs {
            if now < a.next_poll {
                continue;
            }
            a.reach <<= 1;
            if a.reach == 0 && a.good > 0 {
                a.good = 0;
                lost = Some(a.addr);
            }
            a.pending = Some(t1);
            a.next_poll = now + SimTime::from_secs(POLL_SECS);
            self.sent += 1;
            out.push((
                a.addr,
                NtpPacket {
                    leap: 3,
                    version: 4,
                    mode: NTP_MODE_CLIENT,
                    stratum: 0,
                    poll: 6,
                    precision: -18,
                    root_delay: 0,
                    root_dispersion: 0,
                    ref_id: [0; 4],
                    reference: 0,
                    origin: 0,
                    receive: 0,
                    transmit: t1,
                }
                .emit(),
            ));
        }
        if lost.is_some() && self.sync.as_ref().and_then(|s| s.peer) == lost {
            self.sync = None;
        }
    }

    /// A packet arrived from `src`. Returns a reply to send back, for a client's request.
    pub fn on_packet(
        &mut self,
        now: SimTime,
        clock: &mut Clock,
        src: Ipv4Addr,
        pkt: &NtpPacket,
    ) -> Option<Vec<u8>> {
        let t_now = clock.now_ns(now);
        match pkt.mode {
            NTP_MODE_CLIENT => {
                let st = self.serving()?;
                self.received += 1;
                let ts = ntp_to_timestamp(t_now);
                Some(
                    NtpPacket {
                        leap: 0,
                        version: pkt.version.min(4),
                        mode: NTP_MODE_SERVER,
                        stratum: st.stratum,
                        poll: pkt.poll,
                        precision: -18,
                        root_delay: fixed(st.root_delay_ns),
                        root_dispersion: fixed(8 * 1_000_000),
                        ref_id: st.ref_id,
                        reference: ntp_to_timestamp(st.reference_ns),
                        origin: pkt.transmit,
                        receive: ts,
                        transmit: ts,
                    }
                    .emit(),
                )
            }
            NTP_MODE_SERVER => {
                let a = self.assocs.iter_mut().find(|a| a.addr == src)?;
                // Only the answer to our latest question counts, and only from a usable server.
                if a.pending != Some(pkt.origin)
                    || pkt.leap == 3
                    || !(1..=15).contains(&pkt.stratum)
                {
                    return None;
                }
                a.pending = None;
                self.received += 1;
                let (t1, t2, t3, t4) = (
                    ntp_from_timestamp(pkt.origin),
                    ntp_from_timestamp(pkt.receive),
                    ntp_from_timestamp(pkt.transmit),
                    t_now,
                );
                a.offset_ns = ((t2 - t1) + (t3 - t4)) / 2;
                a.delay_ns = ((t4 - t1) - (t3 - t2)).max(0);
                a.reach |= 1;
                a.stratum = pkt.stratum;
                a.ref_id = pkt.ref_id;
                a.last_rx = Some(now);
                a.good += 1;
                a.root_delay_ns = unfixed(pkt.root_delay) + a.delay_ns;
                let a = a.clone();
                self.discipline(now, clock, &a);
                None
            }
            _ => None,
        }
    }

    /// Use the answer from `a` to set the clock, if it is the best source and trusted.
    fn discipline(&mut self, now: SimTime, clock: &mut Clock, a: &Assoc) {
        if self.master.is_some() || a.good < SAMPLES_NEEDED {
            return;
        }
        let best = self
            .assocs
            .iter()
            .filter(|o| o.good >= SAMPLES_NEEDED && o.reach & 1 != 0)
            .min_by_key(|o| (o.stratum, o.delay_ns, o.addr.to_u32()))
            .map(|o| o.addr);
        if best != Some(a.addr) {
            return;
        }
        clock.step(a.offset_ns);
        clock.ever_synced = true;
        self.sync = Some(SyncState {
            stratum: a.stratum + 1,
            ref_id: a.addr.0,
            peer: Some(a.addr),
            reference_ns: clock.now_ns(now),
            // What is left once the clock has been moved.
            offset_ns: 0,
            root_delay_ns: a.root_delay_ns,
        });
        if let Some(o) = self.assocs.iter_mut().find(|o| o.addr == a.addr) {
            o.offset_ns = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: u64) -> SimTime {
        SimTime::from_secs(s)
    }

    /// One round trip between a client and a server with `one_way` ns each way.
    fn exchange(
        client: &mut Ntp,
        cc: &mut Clock,
        server: &mut Ntp,
        sc: &mut Clock,
        at: u64,
        one_way: u64,
    ) {
        let mut out = vec![];
        client.tick(t(at), cc, &mut out);
        for (_, req) in out {
            let pkt = NtpPacket::parse(&req).unwrap();
            let arrive = SimTime(t(at).0 + one_way);
            if let Some(reply) = server.on_packet(arrive, sc, Ipv4Addr([10, 0, 0, 2]), &pkt) {
                let back = SimTime(arrive.0 + one_way);
                let pkt = NtpPacket::parse(&reply).unwrap();
                client.on_packet(back, cc, Ipv4Addr([10, 0, 0, 1]), &pkt);
            }
        }
    }

    #[test]
    fn civil_dates_round_trip_and_weekdays_are_right() {
        assert_eq!(unix_from_civil(1993, 3, 1, 0, 0, 0), 731_030_400 - 86_400);
        assert_eq!(unix_from_civil(1970, 1, 1, 0, 0, 0), 0);
        let (y, m, d, h, mi, s, wd) = civil_from_unix(epoch_1993());
        assert_eq!((y, m, d, h, mi, s, wd), (1993, 3, 1, 0, 0, 0, "Mon"));
        let (y, m, d, _, _, _, wd) = civil_from_unix(unix_from_civil(2024, 2, 29, 12, 0, 0));
        assert_eq!((y, m, d, wd), (2024, 2, 29, "Thu"));
        for secs in [-86_400_i64, 0, 951_782_400, 1_700_000_000, 2_000_000_000] {
            let (y, m, d, h, mi, s, _) = civil_from_unix(secs);
            assert_eq!(unix_from_civil(y, m as i64, d, h, mi, s), secs);
        }
    }

    #[test]
    fn a_client_adopts_a_masters_time_after_enough_polls() {
        let (mut client, mut server) = (Ntp::default(), Ntp::default());
        let (mut cc, mut sc) = (Clock::default(), Clock::default());
        sc.set(t(0), unix_from_civil(2026, 11, 8, 12, 0, 0));
        server.master = Some(5);
        client.add_server(Ipv4Addr([10, 0, 0, 1]), t(0));
        let want = sc.now_ns(t(1000));
        for i in 0..SAMPLES_NEEDED - 1 {
            exchange(
                &mut client,
                &mut cc,
                &mut server,
                &mut sc,
                1 + u64::from(i) * POLL_SECS,
                20_000,
            );
            assert!(
                client.state().is_none(),
                "not yet trusted after {} polls",
                i + 1
            );
        }
        exchange(
            &mut client,
            &mut cc,
            &mut server,
            &mut sc,
            1 + 2 * POLL_SECS,
            20_000,
        );
        let st = client.state().expect("synchronised");
        assert_eq!((st.stratum, st.peer), (6, Some(Ipv4Addr([10, 0, 0, 1]))));
        // Within a few microseconds of the server's clock (the symmetric path makes it exact).
        let off = (cc.now_ns(t(1000)) - want).abs();
        assert!(off < 1000, "off by {off} ns");
        assert!(cc.ever_synced);
    }

    #[test]
    fn a_server_that_is_not_synchronised_does_not_answer_and_a_master_does() {
        let (mut server, mut sc) = (Ntp::default(), Clock::default());
        let req = NtpPacket::parse(
            &NtpPacket {
                leap: 3,
                version: 4,
                mode: NTP_MODE_CLIENT,
                stratum: 0,
                poll: 6,
                precision: 0,
                root_delay: 0,
                root_dispersion: 0,
                ref_id: [0; 4],
                reference: 0,
                origin: 0,
                receive: 0,
                transmit: 77,
            }
            .emit(),
        )
        .unwrap();
        assert!(server
            .on_packet(t(1), &mut sc, Ipv4Addr([10, 0, 0, 9]), &req)
            .is_none());
        server.master = Some(3);
        let r = server
            .on_packet(t(1), &mut sc, Ipv4Addr([10, 0, 0, 9]), &req)
            .unwrap();
        let r = NtpPacket::parse(&r).unwrap();
        assert_eq!(
            (r.mode, r.stratum, r.origin, r.ref_id),
            (NTP_MODE_SERVER, 3, 77, *b"LOCL")
        );
    }

    #[test]
    fn a_stale_or_unsolicited_answer_is_ignored_and_silence_loses_sync() {
        let (mut client, mut server) = (Ntp::default(), Ntp::default());
        let (mut cc, mut sc) = (Clock::default(), Clock::default());
        server.master = Some(2);
        client.add_server(Ipv4Addr([10, 0, 0, 1]), t(0));
        for i in 0..SAMPLES_NEEDED {
            exchange(
                &mut client,
                &mut cc,
                &mut server,
                &mut sc,
                1 + u64::from(i) * POLL_SECS,
                1000,
            );
        }
        assert!(client.state().is_some());
        // A reply nobody asked for.
        let before = client.received;
        let bogus = NtpPacket {
            leap: 0,
            version: 4,
            mode: NTP_MODE_SERVER,
            stratum: 1,
            poll: 6,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            ref_id: [0; 4],
            reference: 0,
            origin: 12345,
            receive: 1,
            transmit: 2,
        };
        client.on_packet(t(500), &mut cc, Ipv4Addr([10, 0, 0, 1]), &bogus);
        assert_eq!(client.received, before);
        // The server goes quiet: eight polls later the register is empty and the clock is free.
        for i in 0..9 {
            let mut out = vec![];
            client.tick(t(300 + i * POLL_SECS), &cc, &mut out);
        }
        assert!(client.state().is_none());
    }
}
