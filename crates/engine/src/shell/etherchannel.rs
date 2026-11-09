//! EtherChannel commands (`channel-group`, `interface port-channel`, `show etherchannel`,
//! `show lacp`), and `interface range`, which makes them practical.

use super::*;
use netdev::{ChannelMode, JoinError, LoadBalance, PortConfig};

pub(super) const CH_ON: u32 = 330;
pub(super) const CH_ACTIVE: u32 = 331;
pub(super) const CH_PASSIVE: u32 = 332;
pub(super) const CH_LEAVE: u32 = 333;
pub(super) const PO_ENTER: u32 = 334;
pub(super) const NO_PO: u32 = 335;
pub(super) const LB_SET: u32 = 336;
pub(super) const NO_LB: u32 = 337;
pub(super) const SHOW_EC: u32 = 338;
pub(super) const SHOW_EC_LB: u32 = 339;
pub(super) const SHOW_LACP_NBR: u32 = 340;
pub(super) const SHOW_LACP_INT: u32 = 341;
pub(super) const IF_RANGE: u32 = 342;
pub(super) const IF_RANGE_TO: u32 = 343;

const NO_SUCH_GROUP: &str = "% No such port-channel.";
const BAD_RANGE_SPEC: &str = "% Invalid interface range.";
const BAD_BALANCE: &str =
    "% Invalid method: use src-mac, dst-mac, src-dst-mac, src-ip, dst-ip or src-dst-ip.";

/// What an interface-mode command applies to: one switch port, or a whole port-channel.
#[derive(Clone, Copy, Debug)]
pub(super) enum Target {
    Port(PortId),
    Group(u8),
}

impl Target {
    pub(super) fn config(self, s: &L2Switch) -> Option<&PortConfig> {
        match self {
            Target::Port(p) => s.port(p),
            Target::Group(g) => s.group(g),
        }
    }

    pub(super) fn apply(self, s: &mut L2Switch, f: impl FnOnce(&mut PortConfig)) {
        match self {
            Target::Port(p) => s.configure_port(p, f),
            Target::Group(g) => s.configure_group(g, f),
        }
    }
}

/// `port-channel1`, `po1`, `Port-channel1`: the one-word spelling.
pub(super) fn parse_po_name(s: &str) -> Option<u8> {
    let l = s.to_ascii_lowercase();
    let rest = l
        .strip_prefix("port-channel")
        .or_else(|| l.strip_prefix("po"))?;
    let n: u8 = rest.parse().ok()?;
    (1..=netdev::MAX_GROUP).contains(&n).then_some(n)
}

/// `fa0/1-4`, `fa0/1,fa0/3`, `fa0/1-fa0/4`: ports named by a list of ranges.
fn parse_range(spec: &str, n: usize) -> Option<Vec<usize>> {
    let mut out = vec![];
    for piece in spec.split(',').filter(|p| !p.is_empty()) {
        match piece.split_once('-') {
            None => out.push(parse_switch_ifname(piece, n)?),
            Some((a, b)) => {
                let first = parse_switch_ifname(a, n)?;
                let last: usize = b.rsplit('/').next()?.parse().ok()?;
                if last == 0 || last > n || last - 1 < first {
                    return None;
                }
                out.extend(first..last);
            }
        }
    }
    out.dedup();
    (!out.is_empty()).then_some(out)
}

pub(super) fn channel_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
) {
    let h = Action::Handler;
    let cg = "Add this interface to a channel group";
    let n = "Channel group number";
    g.cmd(
        &[config_if],
        "channel-group <n:int(1..6)> mode on",
        &[cg, n, "Mode", "Bundle without negotiation"],
        h(CH_ON),
    );
    g.cmd(
        &[config_if],
        "channel-group <n:int(1..6)> mode active",
        &[cg, n, "Mode", "Start LACP negotiation"],
        h(CH_ACTIVE),
    );
    g.cmd(
        &[config_if],
        "channel-group <n:int(1..6)> mode passive",
        &[cg, n, "Mode", "Answer LACP but do not start it"],
        h(CH_PASSIVE),
    );
    g.cmd(
        &[config_if],
        "no channel-group",
        &["Negate", cg],
        h(CH_LEAVE),
    );
    g.cmd(
        &[config],
        "interface port-channel <n:int(1..6)>",
        &["Configure an interface", "Ethernet channel", n],
        h(PO_ENTER),
    );
    g.cmd(
        &[config],
        "no interface port-channel <n:int(1..6)>",
        &["Negate", "Delete an interface", "Ethernet channel", n],
        h(NO_PO),
    );
    g.cmd(
        &[config],
        "port-channel load-balance <method:word>",
        &[
            "Ethernet channel",
            "How frames pick a link",
            "src-mac, dst-mac, src-dst-mac, src-ip, dst-ip, src-dst-ip",
        ],
        h(LB_SET),
    );
    g.cmd(
        &[config],
        "no port-channel load-balance",
        &[
            "Negate",
            "Ethernet channel",
            "Back to the default (src-mac)",
        ],
        h(NO_LB),
    );
    g.cmd(
        &[config],
        "interface range <ports:word>",
        &[
            "Configure an interface",
            "Several interfaces",
            "Ports, e.g. fa0/1-4 or fa0/1,fa0/3",
        ],
        h(IF_RANGE),
    );
    g.cmd(
        &[config],
        "interface range <first:word> - <last:word>",
        &[
            "Configure an interface",
            "Several interfaces",
            "First port",
            "To",
            "Last port or number",
        ],
        h(IF_RANGE_TO),
    );
    g.cmd(
        &[user, privileged],
        "show etherchannel summary",
        &["Show", "Ethernet channels", "One line per group"],
        h(SHOW_EC),
    );
    g.cmd(
        &[user, privileged],
        "show etherchannel load-balance",
        &["Show", "Ethernet channels", "How frames pick a link"],
        h(SHOW_EC_LB),
    );
    g.cmd(
        &[user, privileged],
        "show lacp neighbor",
        &["Show", "LACP", "What neighbours said"],
        h(SHOW_LACP_NBR),
    );
    g.cmd(
        &[user, privileged],
        "show lacp internal",
        &["Show", "LACP", "This switch's side"],
        h(SHOW_LACP_INT),
    );
}

impl Shell {
    /// What `switchport`, `shutdown` and `spanning-tree` apply to in the current interface mode.
    pub(super) fn cfg_target(&self) -> Option<Target> {
        let c = self.session.context()?;
        if let Some(g) = c.strip_prefix("po:") {
            return g.parse().ok().map(Target::Group);
        }
        c.parse().ok().map(|p| Target::Port(PortId(p)))
    }

    /// The ports of the current `interface range`, if that is where the session is.
    pub(super) fn range_ports(&self) -> Option<Vec<usize>> {
        let list = self.session.context()?.strip_prefix("range:")?;
        Some(list.split(',').filter_map(|p| p.parse().ok()).collect())
    }

    pub(super) fn channel_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        match handler {
            CH_ON | CH_ACTIVE | CH_PASSIVE => {
                let Some(Target::Port(p)) = self.cfg_target() else {
                    return PORT_ONLY.to_string();
                };
                let g = a.int("n").unwrap() as u8;
                let mode = match handler {
                    CH_ON => ChannelMode::On,
                    CH_ACTIVE => ChannelMode::Active,
                    _ => ChannelMode::Passive,
                };
                let new = w.device::<L2Switch>(id).group(g).is_none();
                match w.with_device::<L2Switch, _>(id, |s, _| s.channel_join(p, g, mode)) {
                    Ok(()) if new => format!("Creating a port-channel interface Port-channel {g}"),
                    Ok(()) => String::new(),
                    Err(JoinError::ProtocolMismatch) => {
                        format!(
                            "Command rejected (Channel protocol mismatch for channel-group {g})."
                        )
                    }
                    Err(JoinError::TooMany) => {
                        format!(
                            "% At most {} interfaces fit in one channel group.",
                            netdev::MAX_MEMBERS
                        )
                    }
                }
            }
            CH_LEAVE => {
                if let Some(Target::Port(p)) = self.cfg_target() {
                    w.with_device::<L2Switch, _>(id, |s, _| s.channel_leave(p));
                }
                String::new()
            }
            PO_ENTER => {
                let g = a.int("n").unwrap() as u8;
                w.with_device::<L2Switch, _>(id, |s, _| s.ensure_group(g));
                self.session
                    .enter(self.modes.config_if, Some(format!("po:{g}")));
                String::new()
            }
            NO_PO => {
                let g = a.int("n").unwrap() as u8;
                if w.with_device::<L2Switch, _>(id, |s, _| s.remove_group(g)) {
                    String::new()
                } else {
                    NO_SUCH_GROUP.to_string()
                }
            }
            LB_SET | NO_LB => {
                let m = if handler == NO_LB {
                    Some(LoadBalance::default())
                } else {
                    let w = a.word("method").unwrap_or_default();
                    LoadBalance::ALL.into_iter().find(|l| l.word() == w)
                };
                match m {
                    Some(m) => {
                        w.with_device::<L2Switch, _>(id, |s, _| s.set_load_balance(m));
                        String::new()
                    }
                    None => BAD_BALANCE.to_string(),
                }
            }
            IF_RANGE | IF_RANGE_TO => {
                let n = w.device::<L2Switch>(id).n_ports() as usize;
                let spec = if handler == IF_RANGE {
                    a.word("ports").unwrap_or_default().to_string()
                } else {
                    format!(
                        "{}-{}",
                        a.word("first").unwrap_or_default(),
                        a.word("last").unwrap_or_default()
                    )
                };
                let Some(ports) = parse_range(&spec, n) else {
                    return BAD_RANGE_SPEC.to_string();
                };
                let list: Vec<String> = ports.iter().map(usize::to_string).collect();
                self.session.enter(
                    self.modes.config_if,
                    Some(format!("range:{}", list.join(","))),
                );
                String::new()
            }
            SHOW_EC => show_etherchannel(w.device::<L2Switch>(id)),
            SHOW_EC_LB => format!(
                "EtherChannel Load-Balancing Configuration:\n        {}\n\n\
EtherChannel Load-Balancing Addresses Used Per-Protocol:\n\
Non-IP: {}\nIPv4: {}",
                w.device::<L2Switch>(id).load_balance().word(),
                match w.device::<L2Switch>(id).load_balance() {
                    LoadBalance::SrcMac | LoadBalance::SrcIp => "Source MAC address",
                    LoadBalance::DstMac | LoadBalance::DstIp => "Destination MAC address",
                    _ => "Source XOR Destination MAC address",
                },
                match w.device::<L2Switch>(id).load_balance() {
                    LoadBalance::SrcMac => "Source MAC address",
                    LoadBalance::DstMac => "Destination MAC address",
                    LoadBalance::SrcDstMac => "Source XOR Destination MAC address",
                    LoadBalance::SrcIp => "Source IP address",
                    LoadBalance::DstIp => "Destination IP address",
                    LoadBalance::SrcDstIp => "Source XOR Destination IP address",
                }
            ),
            SHOW_LACP_NBR => show_lacp(w.device::<L2Switch>(id), w.kernel.now(), true),
            _ => show_lacp(w.device::<L2Switch>(id), w.kernel.now(), false),
        }
    }
}

/// `Po1` for a port in a bundle, `Fa0/1` otherwise.
pub(super) fn port_label(s: &L2Switch, p: usize) -> String {
    match s.bundle_of(PortId(p as u16)) {
        Some((g, _)) => format!("Po{g}"),
        None => fa(p),
    }
}

/// Whether a port should be listed as itself: not a bundled port other than the one standing
/// for the bundle.
pub(super) fn listed(s: &L2Switch, p: usize) -> bool {
    s.bundle_of(PortId(p as u16))
        .is_none_or(|(_, r)| r.0 as usize == p)
}

fn show_etherchannel(s: &L2Switch) -> String {
    let n = s.groups().count();
    let mut o = String::from(
        "Flags:  D - down        P - bundled in port-channel\n\
\x20       I - stand-alone s - suspended\n\
\x20       H - Hot-standby (LACP only)\n\
\x20       R - Layer3      S - Layer2\n\
\x20       U - in use      f - failed to allocate aggregator\n\n\
\x20       M - not in use, minimum links not met\n\
\x20       u - unsuitable for bundling\n\
\x20       w - waiting to be aggregated\n\
\x20       d - default port\n\n\n",
    );
    o += &format!("Number of channel-groups in use: {n}\nNumber of aggregators:           {n}\n\n");
    o += "Group  Port-channel  Protocol    Ports\n";
    o += "------+-------------+-----------+-----------------------------------------------";
    for (g, _) in s.groups() {
        let members = s.members(g);
        let up = !s.bundled(g).is_empty();
        let proto = match members
            .first()
            .and_then(|p| s.port(*p))
            .and_then(|c| c.channel)
        {
            Some(m) if m.mode.is_lacp() => "LACP",
            _ => "-",
        };
        let ports: Vec<String> = members
            .iter()
            .map(|p| {
                let flag = if s.is_bundled(*p) {
                    'P'
                } else if s.link_is_up(*p) && !s.port(*p).is_some_and(|c| c.shutdown) {
                    'I'
                } else {
                    'D'
                };
                format!("{}({flag})", fa(p.0 as usize))
            })
            .collect();
        o += &format!(
            "\n{g:<7}{:<14}{proto:<12}{}",
            format!("Po{g}({})", if up { "SU" } else { "SD" }),
            ports.join("    ")
        );
    }
    o
}

fn show_lacp(s: &L2Switch, now: SimTime, neighbour: bool) -> String {
    let mut o = String::from(
        "Flags:  S - Device is requesting Slow LACPDUs\n\
\x20       F - Device is requesting Fast LACPDUs\n\
\x20       A - Device is in Active mode       P - Device is in Passive mode\n",
    );
    for (g, _) in s.groups() {
        let members = s.members(g);
        if members.is_empty() {
            continue;
        }
        o += &format!(
            "\nChannel group {g}{}\n",
            if neighbour { " neighbors" } else { "" }
        );
        if neighbour {
            o += "\nPartner's information:\n\n\
\x20                 LACP port                        Admin  Oper   Port    Port\n\
Port      Flags   Priority  Dev ID           Age    key    Key    Number  State\n";
        } else {
            o += "                            LACP port     Admin     Oper    Port        Port\n\
Port      Flags   State     Priority      Key       Key     Number      State\n";
        }
        for p in members {
            let Some(m) = s.port(p).and_then(|c| c.channel) else {
                continue;
            };
            if neighbour {
                let Some((i, at)) = s.lacp_neighbour(p) else {
                    continue;
                };
                let flags = format!(
                    "S{}",
                    if i.state & wire::STATE_ACTIVITY != 0 {
                        'A'
                    } else {
                        'P'
                    }
                );
                o += &format!(
                    "{:<10}{:<8}{:<10}{:<17}{:<7}{:<7}0x{:<5x}0x{:<7x}0x{:X}\n",
                    fa(p.0 as usize),
                    flags,
                    i.port_priority,
                    i.system.to_string(),
                    format!("{}s", (now.0.saturating_sub(at.0)) / 1_000_000_000),
                    "0x0",
                    i.key,
                    i.port,
                    i.state
                );
            } else {
                let flags = format!(
                    "S{}",
                    if m.mode == ChannelMode::Active {
                        'A'
                    } else {
                        'P'
                    }
                );
                let state = if s.is_bundled(p) {
                    "bndl"
                } else if s.link_is_up(p) {
                    "susp"
                } else {
                    "down"
                };
                let info = s.lacp_actor(p);
                o += &format!(
                    "{:<10}{:<8}{:<10}{:<14}{:<10}0x{:<6x}0x{:<10x}0x{:X}\n",
                    fa(p.0 as usize),
                    flags,
                    state,
                    info.map_or(0, |i| i.port_priority),
                    "0x0",
                    g,
                    p.0 + 1,
                    info.map_or(0, |i| i.state)
                );
            }
        }
    }
    o.trim_end().to_string()
}
