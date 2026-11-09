//! CDP and LLDP commands, for routers and switches alike.

use super::*;
use netdev::{Discovery, Neighbour};
use wire::cdp_cap;

pub(super) const CDP_RUN: u32 = 350;
pub(super) const NO_CDP_RUN: u32 = 351;
pub(super) const CDP_TIMER: u32 = 352;
pub(super) const CDP_HOLD: u32 = 353;
pub(super) const NO_CDP_TIMER: u32 = 354;
pub(super) const NO_CDP_HOLD: u32 = 355;
pub(super) const LLDP_RUN: u32 = 356;
pub(super) const NO_LLDP_RUN: u32 = 357;
pub(super) const LLDP_TIMER: u32 = 358;
pub(super) const LLDP_HOLD: u32 = 359;
pub(super) const NO_LLDP_TIMER: u32 = 360;
pub(super) const NO_LLDP_HOLD: u32 = 361;
pub(super) const CDP_ENABLE: u32 = 362;
pub(super) const NO_CDP_ENABLE: u32 = 363;
pub(super) const LLDP_TX: u32 = 364;
pub(super) const NO_LLDP_TX: u32 = 365;
pub(super) const LLDP_RX: u32 = 366;
pub(super) const NO_LLDP_RX: u32 = 367;
pub(super) const SHOW_CDP: u32 = 368;
pub(super) const SHOW_CDP_NBR: u32 = 369;
pub(super) const SHOW_CDP_DETAIL: u32 = 370;
pub(super) const SHOW_CDP_ENTRY: u32 = 371;
pub(super) const SHOW_CDP_IF: u32 = 372;
pub(super) const SHOW_LLDP: u32 = 373;
pub(super) const SHOW_LLDP_NBR: u32 = 374;
pub(super) const SHOW_LLDP_DETAIL: u32 = 375;
pub(super) const SHOW_LLDP_IF: u32 = 376;
pub(super) const CLEAR_CDP: u32 = 377;
pub(super) const CLEAR_LLDP: u32 = 378;

const CDP_OFF: &str = "% CDP is not enabled";
const LLDP_OFF: &str = "% LLDP is not enabled";
const PHYSICAL_ONLY: &str = "% Neighbour discovery applies to physical interfaces.";

pub(super) fn discovery_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
) {
    let h = Action::Handler;
    let cdp = "CDP global configuration";
    let lldp = "LLDP global configuration";
    let cmds: &[(&[ModeId], &str, &[&str], u32)] = &[
        (&[config], "cdp run", &[cdp, "Enable CDP"], CDP_RUN),
        (
            &[config],
            "no cdp run",
            &["Negate", cdp, "Disable CDP"],
            NO_CDP_RUN,
        ),
        (
            &[config],
            "cdp timer <secs:int(5..254)>",
            &[cdp, "Seconds between advertisements", "Seconds"],
            CDP_TIMER,
        ),
        (
            &[config],
            "cdp holdtime <secs:int(10..255)>",
            &[cdp, "Seconds neighbours keep our entry", "Seconds"],
            CDP_HOLD,
        ),
        (
            &[config],
            "no cdp timer",
            &["Negate", cdp, "Back to 60"],
            NO_CDP_TIMER,
        ),
        (
            &[config],
            "no cdp holdtime",
            &["Negate", cdp, "Back to 180"],
            NO_CDP_HOLD,
        ),
        (&[config], "lldp run", &[lldp, "Enable LLDP"], LLDP_RUN),
        (
            &[config],
            "no lldp run",
            &["Negate", lldp, "Disable LLDP"],
            NO_LLDP_RUN,
        ),
        (
            &[config],
            "lldp timer <secs:int(5..65534)>",
            &[lldp, "Seconds between advertisements", "Seconds"],
            LLDP_TIMER,
        ),
        (
            &[config],
            "lldp holdtime <secs:int(0..65535)>",
            &[lldp, "Seconds neighbours keep our entry", "Seconds"],
            LLDP_HOLD,
        ),
        (
            &[config],
            "no lldp timer",
            &["Negate", lldp, "Back to 30"],
            NO_LLDP_TIMER,
        ),
        (
            &[config],
            "no lldp holdtime",
            &["Negate", lldp, "Back to 120"],
            NO_LLDP_HOLD,
        ),
        (
            &[config_if],
            "cdp enable",
            &[
                "CDP interface configuration",
                "Enable CDP on this interface",
            ],
            CDP_ENABLE,
        ),
        (
            &[config_if],
            "no cdp enable",
            &[
                "Negate",
                "CDP interface configuration",
                "Disable CDP on this interface",
            ],
            NO_CDP_ENABLE,
        ),
        (
            &[config_if],
            "lldp transmit",
            &["LLDP interface configuration", "Send LLDP"],
            LLDP_TX,
        ),
        (
            &[config_if],
            "no lldp transmit",
            &[
                "Negate",
                "LLDP interface configuration",
                "Stop sending LLDP",
            ],
            NO_LLDP_TX,
        ),
        (
            &[config_if],
            "lldp receive",
            &["LLDP interface configuration", "Listen for LLDP"],
            LLDP_RX,
        ),
        (
            &[config_if],
            "no lldp receive",
            &["Negate", "LLDP interface configuration", "Ignore LLDP"],
            NO_LLDP_RX,
        ),
        (
            &[user, privileged],
            "show cdp",
            &["Show", "CDP global information"],
            SHOW_CDP,
        ),
        (
            &[user, privileged],
            "show cdp neighbors",
            &["Show", "CDP", "Neighbours heard"],
            SHOW_CDP_NBR,
        ),
        (
            &[user, privileged],
            "show cdp neighbors detail",
            &["Show", "CDP", "Neighbours heard", "All that was said"],
            SHOW_CDP_DETAIL,
        ),
        (
            &[user, privileged],
            "show cdp entry <name:word>",
            &["Show", "CDP", "One neighbour", "Its device id"],
            SHOW_CDP_ENTRY,
        ),
        (
            &[user, privileged],
            "show cdp interface",
            &["Show", "CDP", "Per-interface settings"],
            SHOW_CDP_IF,
        ),
        (
            &[user, privileged],
            "show lldp",
            &["Show", "LLDP global information"],
            SHOW_LLDP,
        ),
        (
            &[user, privileged],
            "show lldp neighbors",
            &["Show", "LLDP", "Neighbours heard"],
            SHOW_LLDP_NBR,
        ),
        (
            &[user, privileged],
            "show lldp neighbors detail",
            &["Show", "LLDP", "Neighbours heard", "All that was said"],
            SHOW_LLDP_DETAIL,
        ),
        (
            &[user, privileged],
            "show lldp interface",
            &["Show", "LLDP", "Per-interface settings"],
            SHOW_LLDP_IF,
        ),
        (
            &[privileged],
            "clear cdp table",
            &["Reset functions", "CDP", "Forget the neighbours"],
            CLEAR_CDP,
        ),
        (
            &[privileged],
            "clear lldp table",
            &["Reset functions", "LLDP", "Forget the neighbours"],
            CLEAR_LLDP,
        ),
    ];
    for (modes, line, help, id) in cmds {
        g.cmd(modes, line, help, h(*id));
    }
}

/// Global lines of a running config: only what differs from a new device.
pub(super) fn disc_global_lines(d: &Discovery) -> String {
    let mut o = String::new();
    if d.cdp_run {
        o += "cdp run\n";
    }
    if d.cdp_timer != netdev::DEFAULT_CDP_TIMER {
        o += &format!("cdp timer {}\n", d.cdp_timer);
    }
    if d.cdp_hold != netdev::DEFAULT_CDP_HOLD {
        o += &format!("cdp holdtime {}\n", d.cdp_hold);
    }
    if d.lldp_run {
        o += "lldp run\n";
    }
    if d.lldp_timer != netdev::DEFAULT_LLDP_TIMER {
        o += &format!("lldp timer {}\n", d.lldp_timer);
    }
    if d.lldp_hold != netdev::DEFAULT_LLDP_HOLD {
        o += &format!("lldp holdtime {}\n", d.lldp_hold);
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

/// Interface lines of a running config.
pub(super) fn disc_port_lines(d: &Discovery, p: usize) -> String {
    let mut o = String::new();
    if !d.cdp_port.get(p).copied().unwrap_or(true) {
        o += " no cdp enable\n";
    }
    if !d.lldp_tx.get(p).copied().unwrap_or(true) {
        o += " no lldp transmit\n";
    }
    if !d.lldp_rx.get(p).copied().unwrap_or(true) {
        o += " no lldp receive\n";
    }
    o
}

/// Names a device uses for its own interfaces in these displays.
struct Names {
    /// `Fas 0/1`.
    cdp_short: fn(usize) -> String,
    /// `Fa0/1`.
    lldp_short: fn(usize) -> String,
    /// `FastEthernet0/1`.
    long: fn(usize) -> String,
}

const SWITCH_NAMES: Names = Names {
    cdp_short: |p| format!("Fas 0/{}", p + 1),
    lldp_short: |p| format!("Fa0/{}", p + 1),
    long: |p| format!("FastEthernet0/{}", p + 1),
};

const ROUTER_NAMES: Names = Names {
    cdp_short: |p| format!("Gig 0/{p}"),
    lldp_short: |p| format!("Gi0/{p}"),
    long: |p| format!("GigabitEthernet0/{p}"),
};

/// `Fa0/1` becomes `Fas 0/1`, `GigabitEthernet0/0` becomes `Gig 0/0`: how CDP shows a neighbour's port.
fn short_remote(long: &str) -> String {
    let split = long
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(long.len());
    let (name, num) = long.split_at(split);
    let abbr: String = name.chars().take(3).collect();
    format!("{abbr} {num}")
}

fn caps_letters(c: u32) -> String {
    let mut v = vec![];
    if c & cdp_cap::ROUTER != 0 {
        v.push("R");
    }
    if c & cdp_cap::SWITCH != 0 {
        v.push("S");
    }
    if c & cdp_cap::HOST != 0 {
        v.push("H");
    }
    v.join(" ")
}

fn caps_words(c: u32) -> String {
    let mut v = vec![];
    if c & cdp_cap::ROUTER != 0 {
        v.push("Router");
    }
    if c & cdp_cap::SWITCH != 0 {
        v.push("Switch");
    }
    if c & cdp_cap::HOST != 0 {
        v.push("Host");
    }
    v.join(" ")
}

fn left(n: &Neighbour, now: SimTime) -> u64 {
    n.expires.0.saturating_sub(now.0) / 1_000_000_000
}

fn show_cdp_neighbours(d: &Discovery, names: &Names, now: SimTime) -> String {
    if !d.cdp_run {
        return CDP_OFF.to_string();
    }
    let mut o = String::from(
        "Capability Codes: R - Router, S - Switch, H - Host\n\n\
Device ID        Local Intrfce     Holdtme    Capability  Platform            Port ID",
    );
    for n in d.cdp_neighbours() {
        o += &format!(
            "\n{:<17}{:<18}{:<11}{:<12}{:<20}{}",
            n.device_id,
            (names.cdp_short)(n.local_port),
            left(n, now),
            caps_letters(n.caps),
            n.platform,
            short_remote(&n.port_id)
        );
    }
    o
}

fn cdp_detail(n: &Neighbour, names: &Names, now: SimTime) -> String {
    let mut o = format!(
        "-------------------------\nDevice ID: {}\nEntry address(es):",
        n.device_id
    );
    for a in &n.addresses {
        o += &format!("\n  IP address: {a}");
    }
    o += &format!(
        "\nPlatform: {},  Capabilities: {}\nInterface: {},  Port ID (outgoing port): {}\nHoldtime : {} sec\n\nVersion :\n{}\n\nadvertisement version: 2",
        n.platform,
        caps_words(n.caps),
        (names.long)(n.local_port),
        n.port_id,
        left(n, now),
        n.software
    );
    o
}

fn show_cdp_detail(d: &Discovery, names: &Names, now: SimTime, only: Option<&str>) -> String {
    if !d.cdp_run {
        return CDP_OFF.to_string();
    }
    let all: Vec<String> = d
        .cdp_neighbours()
        .iter()
        .filter(|n| only.is_none_or(|o| o.eq_ignore_ascii_case(&n.device_id)))
        .map(|n| cdp_detail(n, names, now))
        .collect();
    if all.is_empty() && only.is_some() {
        return "% No such CDP entry.".to_string();
    }
    all.join("\n\n")
}

fn show_cdp_global(d: &Discovery) -> String {
    if !d.cdp_run {
        return CDP_OFF.to_string();
    }
    format!(
        "Global CDP information:\n\tSending CDP packets every {} seconds\n\tSending a holdtime value of {} seconds\n\tSending CDPv2 advertisements is  enabled",
        d.cdp_timer, d.cdp_hold
    )
}

fn show_cdp_interfaces(d: &Discovery, names: &Names, ports: &[(usize, bool)]) -> String {
    if !d.cdp_run {
        return CDP_OFF.to_string();
    }
    ports
        .iter()
        .filter(|(p, _)| d.cdp_port[*p])
        .map(|(p, up)| {
            let state = if *up { "up" } else { "down" };
            format!(
                "{} is {state}, line protocol is {state}\n  Encapsulation ARPA\n  Sending CDP packets every {} seconds\n  Holdtime is {} seconds",
                (names.long)(*p),
                d.cdp_timer,
                d.cdp_hold
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn lldp_letters(c: u32) -> String {
    let mut s = String::new();
    if c & cdp_cap::ROUTER != 0 {
        s.push('R');
    }
    if c & cdp_cap::SWITCH != 0 {
        s.push('B');
    }
    if c & cdp_cap::HOST != 0 {
        s.push('S');
    }
    s
}

fn show_lldp_neighbours(d: &Discovery, names: &Names, now: SimTime) -> String {
    if !d.lldp_run {
        return LLDP_OFF.to_string();
    }
    let mut o = String::from(
        "Capability codes:\n    (R) Router, (B) Bridge, (S) Station\n\n\
Device ID           Local Intf     Hold-time  Capability      Port ID",
    );
    for n in d.lldp_neighbours() {
        o += &format!(
            "\n{:<20}{:<15}{:<11}{:<16}{}",
            n.device_id,
            (names.lldp_short)(n.local_port),
            left(n, now),
            lldp_letters(n.caps),
            n.port_id
        );
    }
    o += &format!("\n\nTotal entries displayed: {}", d.lldp_neighbours().len());
    o
}

fn show_lldp_detail(d: &Discovery, names: &Names, now: SimTime) -> String {
    if !d.lldp_run {
        return LLDP_OFF.to_string();
    }
    let mut blocks: Vec<String> = d
        .lldp_neighbours()
        .iter()
        .map(|n| {
            let mut o = format!(
                "------------------------------------------------\nLocal Intf: {}\nChassis id: {}\nPort id: {}\nPort Description: {}\nSystem Name: {}\n\nSystem Description: \n{}\n\nTime remaining: {} seconds\nSystem Capabilities: {}\nEnabled Capabilities: {}\nManagement Addresses:",
                (names.lldp_short)(n.local_port),
                n.chassis.map_or(String::new(), |c| c.to_string()),
                n.port_id,
                n.port_description,
                n.device_id,
                n.platform,
                left(n, now),
                lldp_letters(n.caps),
                lldp_letters(n.caps)
            );
            for a in &n.addresses {
                o += &format!("\n    IP: {a}");
            }
            o
        })
        .collect();
    blocks.push(format!(
        "\nTotal entries displayed: {}",
        d.lldp_neighbours().len()
    ));
    blocks.join("\n")
}

fn show_lldp_global(d: &Discovery) -> String {
    if !d.lldp_run {
        return LLDP_OFF.to_string();
    }
    format!(
        "Global LLDP Information:\n    Status: ACTIVE\n    LLDP advertisements are sent every {} seconds\n    LLDP hold time advertised is {} seconds\n    LLDP interface reinitialisation delay is 2 seconds",
        d.lldp_timer, d.lldp_hold
    )
}

fn show_lldp_interfaces(d: &Discovery, names: &Names, ports: &[(usize, bool)]) -> String {
    if !d.lldp_run {
        return LLDP_OFF.to_string();
    }
    let on = |b: bool| if b { "enabled" } else { "disabled" };
    ports
        .iter()
        .map(|(p, _)| {
            format!(
                "{}:\n    Tx: {}\n    Rx: {}\n    Tx state: {}\n    Rx state: {}",
                (names.lldp_short)(*p),
                on(d.lldp_tx[*p]),
                on(d.lldp_rx[*p]),
                if d.lldp_tx[*p] { "ACTIVE" } else { "IDLE" },
                if d.lldp_rx[*p] {
                    "WAIT FOR FRAME"
                } else {
                    "IDLE"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Shell {
    fn with_disc<R>(&self, w: &mut World, f: impl FnOnce(&mut Discovery) -> R) -> R {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, _| f(&mut s.disc)),
            _ => w.with_device::<Router, _>(self.device, |r, _| f(&mut r.disc)),
        }
    }

    fn disc<'a>(&self, w: &'a World) -> &'a Discovery {
        match self.kind {
            Kind::Switch => &w.device::<L2Switch>(self.device).disc,
            _ => &w.device::<Router>(self.device).disc,
        }
    }

    /// `(port, link up)` for every physical port.
    fn disc_ports(&self, w: &World) -> Vec<(usize, bool)> {
        let id = self.device;
        let n = match self.kind {
            Kind::Switch => w.device::<L2Switch>(id).n_ports() as usize,
            _ => w.device::<Router>(id).n_ports() as usize,
        };
        (0..n)
            .map(|p| (p, w.kernel.port_up(id, PortId(p as u16))))
            .collect()
    }

    pub(super) fn disc_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let names = if self.kind == Kind::Switch {
            SWITCH_NAMES
        } else {
            ROUTER_NAMES
        };
        let now = w.kernel.now();
        let secs = a.int("secs").map(|s| s as u32);
        match handler {
            CDP_RUN | NO_CDP_RUN => self.with_disc(w, |d| d.set_cdp(handler == CDP_RUN)),
            LLDP_RUN | NO_LLDP_RUN => self.with_disc(w, |d| d.set_lldp(handler == LLDP_RUN)),
            CDP_TIMER | NO_CDP_TIMER => self.with_disc(w, |d| {
                d.cdp_timer = secs.unwrap_or(netdev::DEFAULT_CDP_TIMER)
            }),
            CDP_HOLD | NO_CDP_HOLD => {
                self.with_disc(w, |d| d.cdp_hold = secs.unwrap_or(netdev::DEFAULT_CDP_HOLD))
            }
            LLDP_TIMER | NO_LLDP_TIMER => self.with_disc(w, |d| {
                d.lldp_timer = secs.unwrap_or(netdev::DEFAULT_LLDP_TIMER)
            }),
            LLDP_HOLD | NO_LLDP_HOLD => self.with_disc(w, |d| {
                d.lldp_hold = secs.unwrap_or(netdev::DEFAULT_LLDP_HOLD)
            }),
            CDP_ENABLE..=NO_LLDP_RX => {
                let ports = self.disc_ports(w).len();
                let Some(p) = self
                    .session
                    .context()
                    .and_then(|c| c.parse::<usize>().ok())
                    .filter(|p| *p < ports)
                else {
                    return PHYSICAL_ONLY.to_string();
                };
                self.with_disc(w, |d| match handler {
                    CDP_ENABLE => d.cdp_port[p] = true,
                    NO_CDP_ENABLE => d.cdp_port[p] = false,
                    LLDP_TX => d.lldp_tx[p] = true,
                    NO_LLDP_TX => d.lldp_tx[p] = false,
                    LLDP_RX => d.lldp_rx[p] = true,
                    _ => d.lldp_rx[p] = false,
                });
            }
            SHOW_CDP => return show_cdp_global(self.disc(w)),
            SHOW_CDP_NBR => return show_cdp_neighbours(self.disc(w), &names, now),
            SHOW_CDP_DETAIL => return show_cdp_detail(self.disc(w), &names, now, None),
            SHOW_CDP_ENTRY => return show_cdp_detail(self.disc(w), &names, now, a.word("name")),
            SHOW_CDP_IF => {
                let ports = self.disc_ports(w);
                return show_cdp_interfaces(self.disc(w), &names, &ports);
            }
            SHOW_LLDP => return show_lldp_global(self.disc(w)),
            SHOW_LLDP_NBR => return show_lldp_neighbours(self.disc(w), &names, now),
            SHOW_LLDP_DETAIL => return show_lldp_detail(self.disc(w), &names, now),
            SHOW_LLDP_IF => {
                let ports = self.disc_ports(w);
                return show_lldp_interfaces(self.disc(w), &names, &ports);
            }
            CLEAR_CDP => self.with_disc(w, |d| d.clear_cdp()),
            _ => self.with_disc(w, |d| d.clear_lldp()),
        }
        String::new()
    }
}
