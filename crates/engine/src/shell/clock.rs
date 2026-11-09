//! `clock` and `ntp` commands, for routers and switches.

use super::*;
use netdev::{civil_from_unix, unix_from_civil, Clock, Ntp, MONTHS};

pub(super) const CLOCK_SET: u32 = 380;
pub(super) const CLOCK_TZ: u32 = 381;
pub(super) const NO_CLOCK_TZ: u32 = 382;
pub(super) const NTP_SERVER: u32 = 383;
pub(super) const NO_NTP_SERVER: u32 = 384;
pub(super) const NTP_MASTER: u32 = 385;
pub(super) const NTP_MASTER_N: u32 = 386;
pub(super) const NO_NTP_MASTER: u32 = 387;
pub(super) const SHOW_CLOCK: u32 = 388;
pub(super) const SHOW_NTP_STATUS: u32 = 389;
pub(super) const SHOW_NTP_ASSOC: u32 = 390;

const BAD_TIME: &str = "% Invalid input detected: the time is hh:mm:ss.";
const BAD_MONTH: &str = "% Invalid input detected: the month is Jan to Dec.";
const BAD_DATE: &str = "% Invalid input detected: no such day in that month.";
const NO_SUCH_NTP_SERVER: &str = "% That NTP server is not configured.";

pub(super) fn clock_commands(g: &mut Grammar, user: ModeId, privileged: ModeId, config: ModeId) {
    let h = Action::Handler;
    let ntp = "Configure NTP";
    let cmds: &[(&[ModeId], &str, &[&str], u32)] = &[
        (
            &[privileged],
            "clock set <time:word> <day:int(1..31)> <month:word> <year:int(1993..2035)>",
            &[
                "Manage the system clock",
                "Set the time and date",
                "Time, hh:mm:ss",
                "Day of the month",
                "Month, e.g. Nov",
                "Year",
            ],
            CLOCK_SET,
        ),
        (
            &[config],
            "clock timezone <name:word> <hours:int(-23..23)>",
            &[
                "Configure the clock",
                "Time zone",
                "Name shown with the time",
                "Hours east of UTC",
            ],
            CLOCK_TZ,
        ),
        (
            &[config],
            "no clock timezone",
            &["Negate", "Configure the clock", "Back to UTC"],
            NO_CLOCK_TZ,
        ),
        (
            &[config],
            "ntp server <addr:ipv4>",
            &[ntp, "Poll a time server", "Its address"],
            NTP_SERVER,
        ),
        (
            &[config],
            "no ntp server <addr:ipv4>",
            &["Negate", ntp, "Poll a time server", "Its address"],
            NO_NTP_SERVER,
        ),
        (
            &[config],
            "ntp master",
            &[ntp, "Serve this clock as a time source (stratum 8)"],
            NTP_MASTER,
        ),
        (
            &[config],
            "ntp master <stratum:int(1..15)>",
            &[ntp, "Serve this clock as a time source", "Stratum to claim"],
            NTP_MASTER_N,
        ),
        (
            &[config],
            "no ntp master",
            &["Negate", ntp, "Stop serving this clock"],
            NO_NTP_MASTER,
        ),
        (
            &[user, privileged],
            "show clock",
            &["Show", "The system clock"],
            SHOW_CLOCK,
        ),
        (
            &[user, privileged],
            "show ntp status",
            &["Show", "NTP", "Synchronisation state"],
            SHOW_NTP_STATUS,
        ),
        (
            &[user, privileged],
            "show ntp associations",
            &["Show", "NTP", "Servers polled"],
            SHOW_NTP_ASSOC,
        ),
    ];
    for (modes, line, help, id) in cmds {
        g.cmd(modes, line, help, h(*id));
    }
}

/// `clock timezone`, `ntp master`, `ntp server`: the lines of a running config.
pub(super) fn clock_lines(clock: &Clock, ntp: &Ntp) -> String {
    let mut o = String::new();
    if let Some((name, mins)) = &clock.zone {
        o += &format!("clock timezone {name} {}\n", mins / 60);
    }
    if let Some(st) = ntp.master {
        if st == netdev::DEFAULT_MASTER_STRATUM {
            o += "ntp master\n";
        } else {
            o += &format!("ntp master {st}\n");
        }
    }
    for a in &ntp.assocs {
        o += &format!("ntp server {}\n", a.addr);
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

fn zone_name(clock: &Clock) -> String {
    clock
        .zone
        .as_ref()
        .map_or("UTC".to_string(), |z| z.0.clone())
}

fn show_clock(clock: &Clock, ntp: &Ntp, now: SimTime) -> String {
    let (y, mo, d, h, mi, s, wd) = civil_from_unix(clock.local_secs(now));
    let ms = clock.now_ns(now).rem_euclid(1_000_000_000) / 1_000_000;
    let trusted = clock.set_by_hand || clock.ever_synced || ntp.master.is_some();
    let flag = if !trusted {
        "*"
    } else if ntp.wants_tick() && ntp.state().is_none() {
        "."
    } else {
        ""
    };
    format!(
        "{flag}{h:02}:{mi:02}:{s:02}.{ms:03} {} {wd} {} {d} {y}",
        zone_name(clock),
        MONTHS[mo - 1]
    )
}

fn ref_text(id: [u8; 4]) -> String {
    if &id == b"LOCL" {
        "127.127.1.1".to_string()
    } else {
        format!("{}.{}.{}.{}", id[0], id[1], id[2], id[3])
    }
}

fn ms(ns: i64) -> f64 {
    ns as f64 / 1_000_000.0
}

fn show_ntp_status(clock: &Clock, ntp: &Ntp, now: SimTime) -> String {
    let Some(st) = ntp.state() else {
        return "Clock is unsynchronized, stratum 16, no reference clock\nnominal freq is 250.0000 Hz, actual freq is 250.0000 Hz, precision is 2**18\nntp uptime is 0 (1/100 of seconds), resolution is 4000\nreference time is 00000000.00000000 (00:00:00.000 UTC Mon Jan 1 1900)\nclock offset is 0.0000 msec, root delay is 0.00 msec\nroot dispersion is 0.00 msec, peer dispersion is 0.00 msec".to_string();
    };
    // A master is always in step with itself; a client last stepped at `reference_ns`.
    let reference = if st.peer.is_none() {
        clock.now_ns(now)
    } else {
        st.reference_ns
    };
    let ts = wire::ntp_to_timestamp(reference);
    let (y, mo, d, h, mi, s, wd) = civil_from_unix(reference.div_euclid(1_000_000_000));
    format!(
        "Clock is synchronized, stratum {}, reference is {}\nnominal freq is 250.0000 Hz, actual freq is 250.0000 Hz, precision is 2**18\nntp uptime is {} (1/100 of seconds), resolution is 4000\nreference time is {:08X}.{:08X} ({h:02}:{mi:02}:{s:02}.000 UTC {wd} {} {d} {y})\nclock offset is {:.4} msec, root delay is {:.2} msec\nroot dispersion is 8.00 msec, peer dispersion is 4.00 msec",
        st.stratum,
        ref_text(st.ref_id),
        now.0 / 10_000_000,
        ts >> 32,
        ts & 0xffff_ffff,
        MONTHS[mo - 1],
        ms(st.offset_ns),
        ms(st.root_delay_ns),
    )
}

fn show_ntp_associations(ntp: &Ntp, now: SimTime) -> String {
    let mut o = String::from(
        "  address         ref clock       st   when   poll reach  delay  offset   disp",
    );
    let peer = ntp.state().and_then(|s| s.peer);
    if let Some(st) = ntp.master {
        o += &format!(
            "\n*~127.127.1.1     .LOCL.          {:<4} {:<6} {:<4} {:>5}  {:.3}  {:.3}  {:.3}",
            st.saturating_sub(1),
            "-",
            16,
            377,
            0.0,
            0.0,
            0.0
        );
    }
    for a in &ntp.assocs {
        let when = a.last_rx.map_or("-".to_string(), |t| {
            (now.0.saturating_sub(t.0) / 1_000_000_000).to_string()
        });
        let mark = if peer == Some(a.addr) { "*~" } else { " ~" };
        o += &format!(
            "\n{mark}{:<15} {:<15} {:<4} {:<6} {:<4} {:>5}  {:.3}  {:.3}  {:.3}",
            a.addr.to_string(),
            if a.stratum > 15 {
                ".INIT.".to_string()
            } else {
                ref_text(a.ref_id)
            },
            a.stratum,
            when,
            netdev::NTP_POLL_SECS,
            format!("{:o}", a.reach),
            ms(a.delay_ns),
            ms(a.offset_ns),
            3.906
        );
    }
    o += "\n * sys.peer, # selected, + candidate, - outlyer, x falseticker, ~ configured";
    o
}

impl Shell {
    fn clock_ntp<'a>(&self, w: &'a World) -> (&'a Clock, &'a Ntp) {
        let r: &Router = match self.kind {
            Kind::Switch => w.device::<L2Switch>(self.device).l3(),
            _ => w.device::<Router>(self.device),
        };
        (&r.clock, &r.ntp)
    }

    /// Start NTP polling on this device now.
    fn ntp_start(&self, w: &mut World) {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, ctx| {
                s.originate(ctx, |r, ctx| r.ntp_kick(ctx))
            }),
            _ => w.with_device::<Router, _>(self.device, |r, ctx| r.ntp_kick(ctx)),
        }
    }

    pub(super) fn clock_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let now = w.kernel.now();
        match handler {
            CLOCK_SET => {
                let t: Vec<&str> = a.word("time").unwrap_or_default().split(':').collect();
                let nums: Vec<i64> = t.iter().filter_map(|p| p.parse().ok()).collect();
                if nums.len() != 3 || nums[0] > 23 || nums[1] > 59 || nums[2] > 59 {
                    return BAD_TIME.to_string();
                }
                let month = a.word("month").unwrap_or_default().to_ascii_lowercase();
                let Some(mo) = MONTHS.iter().position(|m| {
                    month.len() >= 3 && m.to_ascii_lowercase().starts_with(&month[..3])
                }) else {
                    return BAD_MONTH.to_string();
                };
                let (day, year) = (a.int("day").unwrap(), a.int("year").unwrap());
                let secs = unix_from_civil(year, mo as i64 + 1, day, nums[0], nums[1], nums[2]);
                // 31 Feb and the like would roll over into March: refuse them.
                if civil_from_unix(secs).2 != day {
                    return BAD_DATE.to_string();
                }
                // The zone is where the user is: the clock shows what was typed.
                let zone = self.clock_ntp(w).0.zone.as_ref().map_or(0, |z| z.1);
                let utc = secs - i64::from(zone) * 60;
                self.with_l3(w, |r| r.clock.set(now, utc));
            }
            CLOCK_TZ => {
                let name = a.word("name").unwrap_or_default().to_string();
                let hours = a.int("hours").unwrap() as i32;
                self.with_l3(w, |r| r.clock.zone = Some((name, hours * 60)));
            }
            NO_CLOCK_TZ => self.with_l3(w, |r| r.clock.zone = None),
            NTP_SERVER => {
                let addr = Ipv4Addr(a.ipv4("addr").unwrap());
                self.with_l3(w, |r| r.ntp.add_server(addr, now));
                self.ntp_start(w);
            }
            NO_NTP_SERVER => {
                let addr = Ipv4Addr(a.ipv4("addr").unwrap());
                if !self.with_l3(w, |r| r.ntp.remove_server(addr)) {
                    return NO_SUCH_NTP_SERVER.to_string();
                }
            }
            NTP_MASTER | NTP_MASTER_N => {
                let st = a
                    .int("stratum")
                    .map_or(netdev::DEFAULT_MASTER_STRATUM, |s| s as u8);
                self.with_l3(w, |r| r.ntp.master = Some(st));
            }
            NO_NTP_MASTER => self.with_l3(w, |r| r.ntp.master = None),
            SHOW_CLOCK => {
                let (c, n) = self.clock_ntp(w);
                return show_clock(c, n, now);
            }
            SHOW_NTP_STATUS => {
                let (c, n) = self.clock_ntp(w);
                return show_ntp_status(c, n, now);
            }
            _ => return show_ntp_associations(self.clock_ntp(w).1, now),
        }
        String::new()
    }
}
