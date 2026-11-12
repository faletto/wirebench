//! Device logging: the message log buffer, the syslog hosts to send to and the trap level. The
//! device says what happened ([`Logging::log`]) and sends what comes back; the line states it
//! watches for `%LINK` and `%LINEPROTO` messages are kept here too.

use std::collections::{BTreeMap, VecDeque};
use wire::{Ipv4Addr, SyslogMessage};

/// Level numbers, emergencies (0) to debugging (7).
pub const ERRORS: u8 = 3;
pub const WARNINGS: u8 = 4;
pub const NOTIFICATIONS: u8 = 5;
pub const INFORMATIONAL: u8 = 6;
pub const DEBUGGING: u8 = 7;
/// local7, where Cisco devices log by default.
pub const DEFAULT_FACILITY: u8 = 23;
/// `logging buffered` without a size.
pub const BUFFER_DEFAULT: usize = 4096;

/// A level by name (`informational`, or the old `info`) or number.
pub fn parse_level(s: &str) -> Option<u8> {
    let s = s.to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (n <= 7).then_some(n);
    }
    let alias = match s.as_str() {
        "alert" => "alerts",
        "critical" | "crit" => "critical",
        "emergency" | "emerg" => "emergencies",
        "error" | "err" => "errors",
        "warning" | "warn" => "warnings",
        "notification" | "notice" => "notifications",
        "info" => "informational",
        "debug" => "debugging",
        other => other,
    };
    wire::SYSLOG_SEVERITIES
        .iter()
        .position(|n| *n == alias)
        .map(|i| i as u8)
}

pub fn level_name(level: u8) -> &'static str {
    wire::SYSLOG_SEVERITIES[level.min(7) as usize]
}

/// What was last seen of one interface, to notice changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineSeen {
    pub admin_down: bool,
    pub up: bool,
}

#[derive(Clone, Debug)]
pub struct Logging {
    /// `logging host`: each gets every message at the trap level or more severe.
    pub hosts: Vec<Ipv4Addr>,
    /// `logging trap`: the least severe level sent to hosts.
    pub trap: u8,
    /// `logging buffered`: size in bytes and level.
    pub buffered: Option<(usize, u8)>,
    /// `service timestamps log datetime msec`, on by default.
    pub timestamps: bool,
    pub facility: u8,
    buffer: VecDeque<String>,
    buffer_bytes: usize,
    /// Messages logged since the last reload, including ones nobody kept.
    pub total: u32,
    /// Messages sent to each host.
    pub sent: BTreeMap<Ipv4Addr, u32>,
    /// Interface states last seen by the watcher, by interface name.
    pub seen: BTreeMap<String, LineSeen>,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            hosts: Vec::new(),
            trap: INFORMATIONAL,
            buffered: None,
            timestamps: true,
            facility: DEFAULT_FACILITY,
            buffer: VecDeque::new(),
            buffer_bytes: 0,
            total: 0,
            sent: BTreeMap::new(),
            seen: BTreeMap::new(),
        }
    }
}

impl Logging {
    /// Whether the interface watcher needs to run: somebody is going to read what it finds.
    pub fn wants_tick(&self) -> bool {
        !self.hosts.is_empty() || self.buffered.is_some()
    }

    pub fn add_host(&mut self, addr: Ipv4Addr) {
        if !self.hosts.contains(&addr) {
            self.hosts.push(addr);
        }
    }

    pub fn remove_host(&mut self, addr: Ipv4Addr) -> bool {
        let n = self.hosts.len();
        self.hosts.retain(|h| *h != addr);
        self.hosts.len() != n
    }

    /// The buffered lines, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &String> {
        self.buffer.iter()
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.buffer_bytes = 0;
    }

    /// Resize or switch off the buffer; lines that no longer fit go, oldest first.
    pub fn set_buffered(&mut self, b: Option<(usize, u8)>) {
        self.buffered = b;
        if b.is_none() {
            self.clear_buffer();
        } else {
            self.trim();
        }
    }

    fn trim(&mut self) {
        let size = self.buffered.map_or(0, |b| b.0);
        while self.buffer_bytes > size {
            match self.buffer.pop_front() {
                Some(l) => self.buffer_bytes -= l.len() + 1,
                None => break,
            }
        }
    }

    /// Log one message. `stamp` is the timestamp text (`*Mar  1 00:00:05.123`) and is left out
    /// when `service timestamps` is off. Returns the datagrams to send, one per host.
    pub fn log(&mut self, stamp: &str, sev: u8, tag: &str, text: &str) -> Vec<(Ipv4Addr, Vec<u8>)> {
        self.total += 1;
        let line = if self.timestamps {
            format!("{stamp}: %{tag}: {text}")
        } else {
            format!("%{tag}: {text}")
        };
        if let Some((_, level)) = self.buffered {
            if sev <= level {
                self.buffer_bytes += line.len() + 1;
                self.buffer.push_back(line.clone());
                self.trim();
            }
        }
        let mut out = Vec::new();
        if sev <= self.trap {
            let payload = SyslogMessage {
                facility: self.facility,
                severity: sev,
                text: line,
            }
            .emit();
            for h in &self.hosts {
                *self.sent.entry(*h).or_insert(0) += 1;
                out.push((*h, payload.clone()));
            }
        }
        out
    }

    /// Back to the factory state, as after `reload` without a saved configuration.
    pub fn reset(&mut self) {
        *self = Logging::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> Ipv4Addr {
        Ipv4Addr::new(10, 0, 0, n)
    }

    #[test]
    fn levels_by_name_alias_and_number() {
        assert_eq!(parse_level("informational"), Some(6));
        assert_eq!(parse_level("Warnings"), Some(4));
        assert_eq!(parse_level("debug"), Some(7));
        assert_eq!(parse_level("3"), Some(3));
        assert_eq!(parse_level("8"), None);
        assert_eq!(parse_level("loud"), None);
        for l in 0..8 {
            assert_eq!(parse_level(level_name(l)), Some(l));
        }
    }

    #[test]
    fn trap_level_filters_what_hosts_get_and_buffer_level_what_is_kept() {
        let mut l = Logging::default();
        l.add_host(ip(2));
        l.add_host(ip(2));
        l.add_host(ip(3));
        l.trap = WARNINGS;
        l.set_buffered(Some((4096, NOTIFICATIONS)));
        assert!(l
            .log("*Mar  1 00:00:01.000", INFORMATIONAL, "SYS-6-X", "quiet")
            .is_empty());
        let out = l.log("*Mar  1 00:00:02.000", ERRORS, "LINK-3-UPDOWN", "loud");
        assert_eq!(out.len(), 2);
        let m = SyslogMessage::parse(&out[0].1).unwrap();
        assert_eq!((m.facility, m.severity), (23, 3));
        assert_eq!(m.text, "*Mar  1 00:00:02.000: %LINK-3-UPDOWN: loud");
        // The informational message is below the buffer level too.
        assert_eq!(l.lines().count(), 1);
        assert_eq!(l.total, 2);
        assert_eq!(l.sent[&ip(3)], 1);
    }

    #[test]
    fn the_buffer_drops_its_oldest_lines_when_full_and_empties_when_switched_off() {
        let mut l = Logging::default();
        l.set_buffered(Some((60, DEBUGGING)));
        for i in 0..10 {
            l.log("s", DEBUGGING, "T", &format!("message {i}"));
        }
        let kept: Vec<_> = l.lines().cloned().collect();
        assert!(
            kept.len() < 10 && kept.last().unwrap().ends_with("message 9"),
            "{kept:?}"
        );
        l.set_buffered(None);
        assert_eq!(l.lines().count(), 0);
        assert!(!l.wants_tick());
    }

    #[test]
    fn without_timestamps_the_line_starts_at_the_tag() {
        let mut l = Logging {
            timestamps: false,
            ..Logging::default()
        };
        l.set_buffered(Some((4096, DEBUGGING)));
        l.log(
            "ignored",
            NOTIFICATIONS,
            "SYS-5-CONFIG_I",
            "Configured from console by console",
        );
        assert_eq!(
            l.lines().next().unwrap(),
            "%SYS-5-CONFIG_I: Configured from console by console"
        );
    }
}

/// One message a collector received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub at: kernel::SimTime,
    pub from: Ipv4Addr,
    pub msg: SyslogMessage,
}

/// A syslog server on a PC (`syslogd start`): keeps what arrives on UDP 514 while it runs.
#[derive(Clone, Debug, Default)]
pub struct Collector {
    pub running: bool,
    pub received: Vec<Received>,
    /// Datagrams that were not syslog.
    pub malformed: u32,
}

/// Most messages a collector keeps; the oldest go first.
pub const COLLECTOR_KEEPS: usize = 1000;

impl Collector {
    pub fn on_datagram(&mut self, now: kernel::SimTime, from: Ipv4Addr, data: &[u8]) {
        match SyslogMessage::parse(data) {
            Ok(msg) => {
                self.received.push(Received { at: now, from, msg });
                if self.received.len() > COLLECTOR_KEEPS {
                    self.received.remove(0);
                }
            }
            Err(_) => self.malformed += 1,
        }
    }
}
