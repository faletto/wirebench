//! `logging` and `service timestamps` commands, `show logging` and `clear logging`, for routers
//! and switches. Handler ids 530..549.

use super::*;
use netdev::{log_level_name, parse_log_level, Logging, LOG_DEBUGGING, LOG_INFORMATIONAL};

pub(super) const LOGGING_HOST: u32 = 530;
pub(super) const NO_LOGGING_HOST: u32 = 531;
pub(super) const LOGGING_TRAP: u32 = 532;
pub(super) const NO_LOGGING_TRAP: u32 = 533;
pub(super) const LOGGING_BUFFERED: u32 = 534;
pub(super) const LOGGING_BUFFERED_SIZE: u32 = 535;
pub(super) const LOGGING_BUFFERED_LEVEL: u32 = 536;
pub(super) const NO_LOGGING_BUFFERED: u32 = 537;
pub(super) const TIMESTAMPS_ON: u32 = 538;
pub(super) const TIMESTAMPS_OFF: u32 = 539;
pub(super) const SHOW_LOGGING: u32 = 540;
pub(super) const CLEAR_LOGGING: u32 = 541;

const BAD_LEVEL: &str = "% Invalid input detected: the level is a name (emergencies, alerts, critical, errors, warnings, notifications, informational, debugging) or 0 to 7.";
const NO_SUCH_LOG_HOST: &str = "% That logging host is not configured.";

pub(super) fn logging_commands(g: &mut Grammar, user: ModeId, privileged: ModeId, config: ModeId) {
    let h = Action::Handler;
    let lg = "Modify message logging facilities";
    let cmds: &[(&[ModeId], &str, &[&str], u32)] = &[
        (
            &[config],
            "logging host <addr:ipv4>",
            &[lg, "Log to a syslog server", "Its address"],
            LOGGING_HOST,
        ),
        (
            &[config],
            "no logging host <addr:ipv4>",
            &["Negate", lg, "Log to a syslog server", "Its address"],
            NO_LOGGING_HOST,
        ),
        (
            &[config],
            "logging trap <level:word>",
            &[
                lg,
                "Set the level of messages sent to syslog servers",
                "Level name or 0 to 7",
            ],
            LOGGING_TRAP,
        ),
        (
            &[config],
            "no logging trap",
            &["Negate", lg, "Back to informational"],
            NO_LOGGING_TRAP,
        ),
        (
            &[config],
            "logging buffered",
            &[lg, "Keep messages in a buffer (4096 bytes)"],
            LOGGING_BUFFERED,
        ),
        (
            &[config],
            "logging buffered <size:int(4096..2147483647)>",
            &[lg, "Keep messages in a buffer", "Buffer size in bytes"],
            LOGGING_BUFFERED_SIZE,
        ),
        (
            &[config],
            "logging buffered <size:int(4096..2147483647)> <level:word>",
            &[
                lg,
                "Keep messages in a buffer",
                "Buffer size in bytes",
                "Level name or 0 to 7",
            ],
            LOGGING_BUFFERED_LEVEL,
        ),
        (
            &[config],
            "no logging buffered",
            &["Negate", lg, "Stop keeping messages"],
            NO_LOGGING_BUFFERED,
        ),
        (
            &[config],
            "service timestamps log datetime msec",
            &[
                "Modify use of network based services",
                "Timestamp messages",
                "Timestamp log messages",
                "Timestamp with date and time",
                "Include milliseconds",
            ],
            TIMESTAMPS_ON,
        ),
        (
            &[config],
            "no service timestamps log datetime msec",
            &[
                "Negate",
                "Modify use of network based services",
                "Timestamp messages",
                "Timestamp log messages",
                "Timestamp with date and time",
                "Include milliseconds",
            ],
            TIMESTAMPS_OFF,
        ),
        (
            &[user, privileged],
            "show logging",
            &["Show", "Show the contents of the logging buffer"],
            SHOW_LOGGING,
        ),
        (
            &[privileged],
            "clear logging",
            &["Reset functions", "Clear the logging buffer"],
            CLEAR_LOGGING,
        ),
    ];
    for (modes, line, help, id) in cmds {
        g.cmd(modes, line, help, h(*id));
    }
}

/// The lines of a running config that belong to logging.
pub(super) fn logging_lines(l: &Logging) -> String {
    let mut o = String::new();
    if !l.timestamps {
        o += "no service timestamps log datetime msec\n";
    }
    if let Some((size, level)) = l.buffered {
        if level == LOG_DEBUGGING {
            o += &format!("logging buffered {size}\n");
        } else {
            o += &format!("logging buffered {size} {}\n", log_level_name(level));
        }
    }
    if l.trap != LOG_INFORMATIONAL {
        o += &format!("logging trap {}\n", log_level_name(l.trap));
    }
    for h in &l.hosts {
        o += &format!("logging host {h}\n");
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

fn show_logging(l: &Logging) -> String {
    let level_lines = |what: &str, level: &str, n: u32| {
        format!("    {what} logging: level {level}, {n} messages logged, xml disabled,\n                     filtering disabled\n")
    };
    let mut o = String::from("Syslog logging: enabled (0 messages dropped, 0 messages rate-limited, 0 flushed, 0 overruns, xml disabled, filtering disabled)\n\n");
    o += "No Active Message Discriminator.\n\nNo Inactive Message Discriminator.\n\n";
    o += &level_lines("Console", "debugging", l.total);
    o += &level_lines("Monitor", "debugging", 0);
    match l.buffered {
        Some((_, level)) => {
            o += &level_lines("Buffer", log_level_name(level), l.lines().count() as u32).replacen(
                "Buffer logging: ",
                "Buffer logging:  ",
                1,
            )
        }
        None => o += "    Buffer logging:  disabled\n",
    }
    o += "    Exception Logging: size (4096 bytes)\n    Count and timestamp logging messages: disabled\n";
    o += &format!(
        "    Trap logging: level {}, {} message lines logged",
        log_level_name(l.trap),
        l.sent.values().sum::<u32>()
    );
    for h in &l.hosts {
        o += &format!(
            "\n        Logging to {h}  (udp port 514, audit disabled, link up), {} message lines logged, xml disabled,\n                filtering disabled",
            l.sent.get(h).copied().unwrap_or(0)
        );
    }
    if let Some((size, _)) = l.buffered {
        o += &format!("\n\nLog Buffer ({size} bytes):\n");
        for line in l.lines() {
            o += &format!("\n{line}");
        }
    }
    o
}

impl Shell {
    fn log_start(&self, w: &mut World) {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, ctx| {
                s.originate(ctx, |r, ctx| r.log_kick(ctx))
            }),
            _ => w.with_device::<Router, _>(self.device, |r, ctx| r.log_kick(ctx)),
        }
    }

    /// Log a message on this device.
    pub(super) fn log_event(&self, w: &mut World, sev: u8, tag: &str, text: &str) {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, ctx| {
                s.originate(ctx, |r, ctx| r.syslog(ctx, sev, tag, text))
            }),
            _ => w.with_device::<Router, _>(self.device, |r, ctx| r.syslog(ctx, sev, tag, text)),
        }
    }

    pub(super) fn logging_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        match handler {
            LOGGING_HOST => {
                let addr = Ipv4Addr(a.ipv4("addr").unwrap());
                self.with_l3(w, |r| r.log.add_host(addr));
                self.log_start(w);
            }
            NO_LOGGING_HOST => {
                let addr = Ipv4Addr(a.ipv4("addr").unwrap());
                if !self.with_l3(w, |r| r.log.remove_host(addr)) {
                    return NO_SUCH_LOG_HOST.to_string();
                }
            }
            LOGGING_TRAP => {
                let Some(level) = parse_log_level(a.word("level").unwrap_or_default()) else {
                    return BAD_LEVEL.to_string();
                };
                self.with_l3(w, |r| r.log.trap = level);
            }
            NO_LOGGING_TRAP => self.with_l3(w, |r| r.log.trap = LOG_INFORMATIONAL),
            LOGGING_BUFFERED | LOGGING_BUFFERED_SIZE | LOGGING_BUFFERED_LEVEL => {
                let size = a.int("size").map_or(netdev::BUFFER_DEFAULT, |s| s as usize);
                let level = match a.word("level") {
                    Some(l) => match parse_log_level(l) {
                        Some(l) => l,
                        None => return BAD_LEVEL.to_string(),
                    },
                    None => LOG_DEBUGGING,
                };
                self.with_l3(w, |r| r.log.set_buffered(Some((size, level))));
                self.log_start(w);
            }
            NO_LOGGING_BUFFERED => self.with_l3(w, |r| r.log.set_buffered(None)),
            TIMESTAMPS_ON => self.with_l3(w, |r| r.log.timestamps = true),
            TIMESTAMPS_OFF => self.with_l3(w, |r| r.log.timestamps = false),
            CLEAR_LOGGING => self.with_l3(w, |r| r.log.clear_buffer()),
            _ => return show_logging(&self.l3(w).log),
        }
        String::new()
    }
}
