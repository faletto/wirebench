//! Binds `clish` command grammars to devices. Each [`Shell`] is one terminal attached to
//! one device; the grammar is shared per device kind.

use crate::World;
use clish::{render_error, render_help, Action, Args, Exec, Grammar, ModeId, Session};
use kernel::{DeviceId, PortId, SimTime};
use netdev::{
    all_vlans, format_vlan_list, mask, parse_vlan_list, Host, L2Switch, PortMode, RouteKind,
    Router, StaticRoute, DEFAULT_VLAN,
};
use std::sync::Arc;
use wire::{IcmpErrorKind, Ipv4Addr};

// ---- message catalog (shell-level strings) ----
const UNKNOWN_INTERFACE: &str = "% No such interface:";
const NO_ROUTE_ENTRY: &str = "% That static route is not configured.";
const SAVED: &str = "Configuration saved.";
const TIMED_OUT: &str = "No reply (timed out).";
const DEFAULT_VLAN_KEPT: &str = "% Default VLAN 1 may not be deleted.";
const NO_SUCH_VLAN: &str = "% VLAN does not exist.";
const BAD_VLAN_LIST: &str = "% Invalid VLAN list:";
const NAME_TOO_LONG: &str = "% VLAN name may be at most 32 characters.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Router,
    Host,
    Switch,
}

#[derive(Clone, Copy, Debug)]
struct Modes {
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
    /// Switch only: `vlan N` configuration. Other kinds point it at `config`.
    config_vlan: ModeId,
}

// Handler ids.
const ENABLE: u32 = 1;
const DISABLE: u32 = 2;
const CONF_T: u32 = 3;
const HOSTNAME: u32 = 4;
const INTERFACE: u32 = 5;
const IP_ADDRESS: u32 = 6;
const NO_IP_ADDRESS: u32 = 7;
const SHUTDOWN: u32 = 8;
const NO_SHUTDOWN: u32 = 9;
const IP_ROUTE: u32 = 10;
const NO_IP_ROUTE: u32 = 11;
const SHOW_RUN: u32 = 12;
const SHOW_ROUTE: u32 = 13;
const SHOW_BRIEF: u32 = 14;
const COPY_RUN_START: u32 = 15;
const IPCONFIG: u32 = 20;
const IPCONFIG_SET: u32 = 21;
const IPCONFIG_GW: u32 = 22;
const PING: u32 = 23;
const SHOW_MACS: u32 = 30;
const SW_INTERFACE: u32 = 40;
const VLAN_ENTER: u32 = 41;
const NO_VLAN: u32 = 42;
const VLAN_NAME: u32 = 43;
const SW_MODE_ACCESS: u32 = 44;
const SW_MODE_TRUNK: u32 = 45;
const NO_SW_MODE: u32 = 46;
const SW_ACCESS_VLAN: u32 = 47;
const NO_SW_ACCESS_VLAN: u32 = 48;
const SW_NATIVE: u32 = 49;
const NO_SW_NATIVE: u32 = 50;
const SW_ALLOWED: u32 = 51;
const SW_ALLOWED_ADD: u32 = 52;
const SW_ALLOWED_REMOVE: u32 = 53;
const SW_ALLOWED_ALL: u32 = 54;
const SW_ALLOWED_NONE: u32 = 55;
const SHOW_VLAN: u32 = 56;
const SHOW_TRUNK: u32 = 57;

fn router_grammar() -> (Grammar, Modes) {
    let mut g = Grammar::new();
    let user = g.mode("user", "{host}>", None, None);
    let privileged = g.mode("privileged", "{host}#", None, None);
    let config = g.mode("config", "{host}(config)#", Some(privileged), None);
    let config_if = g.mode(
        "config-if",
        "{host}(config-if)#",
        Some(config),
        Some(config),
    );
    g.do_mode = Some(privileged);
    let h = Action::Handler;
    g.cmd(&[user], "enable", &["Become privileged"], h(ENABLE));
    g.cmd(
        &[user, privileged],
        "exit",
        &["Leave the session"],
        Action::Exit,
    );
    g.cmd(&[privileged], "disable", &["Drop privileges"], h(DISABLE));
    g.cmd(
        &[privileged],
        "configure terminal",
        &["Configure", "From this terminal"],
        h(CONF_T),
    );
    g.cmd(
        &[privileged],
        "show running-config",
        &["Show", "Current configuration"],
        h(SHOW_RUN),
    );
    g.cmd(
        &[privileged],
        "show ip route",
        &["Show", "IP information", "Routing table"],
        h(SHOW_ROUTE),
    );
    g.cmd(
        &[privileged],
        "show ip interface brief",
        &[
            "Show",
            "IP information",
            "Interface information",
            "One line per interface",
        ],
        h(SHOW_BRIEF),
    );
    g.cmd(
        &[privileged],
        "copy running-config startup-config",
        &["Copy", "Current configuration", "Saved configuration"],
        h(COPY_RUN_START),
    );
    g.cmd(
        &[config],
        "hostname <name:word>",
        &["Set the device name", "New name"],
        h(HOSTNAME),
    );
    g.cmd(
        &[config],
        "interface <name:word>",
        &["Configure an interface", "Interface, e.g. g0/0"],
        h(INTERFACE),
    );
    let route_help = [
        "IP",
        "Static route",
        "Destination network",
        "Network mask",
        "Next hop",
    ];
    g.cmd(
        &[config],
        "ip route <net:ipv4> <mask:mask> <via:ipv4>",
        &route_help,
        h(IP_ROUTE),
    );
    g.cmd(
        &[config],
        "no ip route <net:ipv4> <mask:mask> <via:ipv4>",
        &[
            "Negate",
            "IP",
            "Static route",
            "Destination network",
            "Network mask",
            "Next hop",
        ],
        h(NO_IP_ROUTE),
    );
    g.cmd(
        &[config, config_if],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    g.cmd(
        &[config, config_if],
        "exit",
        &["Up one level"],
        Action::Exit,
    );
    g.cmd(
        &[config_if],
        "ip address <addr:ipv4> <mask:mask>",
        &["IP", "Set address", "Address", "Network mask"],
        h(IP_ADDRESS),
    );
    g.cmd(
        &[config_if],
        "no ip address",
        &["Negate", "IP", "Remove the address"],
        h(NO_IP_ADDRESS),
    );
    g.cmd(
        &[config_if],
        "shutdown",
        &["Disable the interface"],
        h(SHUTDOWN),
    );
    g.cmd(
        &[config_if],
        "no shutdown",
        &["Negate", "Enable the interface"],
        h(NO_SHUTDOWN),
    );
    (
        g,
        Modes {
            user,
            privileged,
            config,
            config_if,
            config_vlan: config,
        },
    )
}

fn host_grammar() -> (Grammar, Modes) {
    let mut g = Grammar::new();
    let m = g.mode("host", "{host}>", None, None);
    let h = Action::Handler;
    g.cmd(&[m], "exit", &["Close the terminal"], Action::Exit);
    g.cmd(&[m], "ipconfig", &["Show addressing"], h(IPCONFIG));
    g.cmd(
        &[m],
        "ipconfig set <addr:ipv4> <mask:mask>",
        &["Addressing", "Set the address", "Address", "Network mask"],
        h(IPCONFIG_SET),
    );
    g.cmd(
        &[m],
        "ipconfig gateway <gw:ipv4>",
        &["Addressing", "Default gateway", "Gateway address"],
        h(IPCONFIG_GW),
    );
    g.cmd(
        &[m],
        "ping <dst:ipv4>",
        &["Send echo requests", "Destination"],
        h(PING),
    );
    (
        g,
        Modes {
            user: m,
            privileged: m,
            config: m,
            config_if: m,
            config_vlan: m,
        },
    )
}

fn switch_grammar() -> (Grammar, Modes) {
    let mut g = Grammar::new();
    let user = g.mode("user", "{host}>", None, None);
    let privileged = g.mode("privileged", "{host}#", None, None);
    let config = g.mode("config", "{host}(config)#", Some(privileged), None);
    let config_if = g.mode(
        "config-if",
        "{host}(config-if)#",
        Some(config),
        Some(config),
    );
    let config_vlan = g.mode(
        "config-vlan",
        "{host}(config-vlan)#",
        Some(config),
        Some(config),
    );
    g.do_mode = Some(privileged);
    let h = Action::Handler;
    g.cmd(&[user], "enable", &["Become privileged"], h(ENABLE));
    g.cmd(
        &[user, privileged],
        "exit",
        &["Leave the session"],
        Action::Exit,
    );
    g.cmd(&[privileged], "disable", &["Drop privileges"], h(DISABLE));
    g.cmd(
        &[privileged],
        "configure terminal",
        &["Configure", "From this terminal"],
        h(CONF_T),
    );
    g.cmd(
        &[user, privileged],
        "show running-config",
        &["Show", "Current configuration"],
        h(SHOW_RUN),
    );
    g.cmd(
        &[user, privileged],
        "show mac address-table",
        &["Show", "Hardware addresses", "Learned addresses by port"],
        h(SHOW_MACS),
    );
    g.cmd(
        &[user, privileged],
        "show vlan brief",
        &["Show", "VLAN information", "One line per VLAN"],
        h(SHOW_VLAN),
    );
    g.cmd(
        &[user, privileged],
        "show interfaces trunk",
        &["Show", "Interface information", "Trunk ports"],
        h(SHOW_TRUNK),
    );
    g.cmd(
        &[config],
        "hostname <name:word>",
        &["Set the device name", "New name"],
        h(HOSTNAME),
    );
    g.cmd(
        &[config],
        "interface <name:word>",
        &["Configure an interface", "Interface, e.g. fa0/1"],
        h(SW_INTERFACE),
    );
    g.cmd(
        &[config],
        "vlan <id:int(1..4094)>",
        &["Configure a VLAN", "VLAN id"],
        h(VLAN_ENTER),
    );
    g.cmd(
        &[config],
        "no vlan <id:int(1..4094)>",
        &["Negate", "Delete a VLAN", "VLAN id"],
        h(NO_VLAN),
    );
    g.cmd(
        &[config_vlan],
        "name <name:word>",
        &["Name the VLAN", "New name"],
        h(VLAN_NAME),
    );
    g.cmd(
        &[config_if],
        "switchport mode access",
        &["Switch port settings", "Port mode", "Untagged, one VLAN"],
        h(SW_MODE_ACCESS),
    );
    g.cmd(
        &[config_if],
        "switchport mode trunk",
        &["Switch port settings", "Port mode", "Tagged, many VLANs"],
        h(SW_MODE_TRUNK),
    );
    g.cmd(
        &[config_if],
        "no switchport mode",
        &["Negate", "Switch port settings", "Back to the default mode"],
        h(NO_SW_MODE),
    );
    g.cmd(
        &[config_if],
        "switchport access vlan <id:int(1..4094)>",
        &["Switch port settings", "Access port", "VLAN", "VLAN id"],
        h(SW_ACCESS_VLAN),
    );
    g.cmd(
        &[config_if],
        "no switchport access vlan",
        &[
            "Negate",
            "Switch port settings",
            "Access port",
            "Back to VLAN 1",
        ],
        h(NO_SW_ACCESS_VLAN),
    );
    g.cmd(
        &[config_if],
        "switchport trunk native vlan <id:int(1..4094)>",
        &[
            "Switch port settings",
            "Trunk port",
            "Untagged traffic",
            "VLAN",
            "VLAN id",
        ],
        h(SW_NATIVE),
    );
    g.cmd(
        &[config_if],
        "no switchport trunk native vlan",
        &[
            "Negate",
            "Switch port settings",
            "Trunk port",
            "Untagged traffic",
            "Back to VLAN 1",
        ],
        h(NO_SW_NATIVE),
    );
    let allowed = |tail: &str| format!("switchport trunk allowed vlan {tail}");
    let head = [
        "Switch port settings",
        "Trunk port",
        "Carried VLANs",
        "VLAN",
    ];
    for (tail, extra, id) in [
        ("<list:word>", "VLAN list, e.g. 10,20-30", SW_ALLOWED),
        ("add <list:word>", "Add VLANs", SW_ALLOWED_ADD),
        ("remove <list:word>", "Remove VLANs", SW_ALLOWED_REMOVE),
        ("all", "Every VLAN", SW_ALLOWED_ALL),
        ("none", "No VLANs", SW_ALLOWED_NONE),
    ] {
        let mut help: Vec<&str> = head.to_vec();
        if tail.contains(' ') {
            help.push(extra);
            help.push("VLAN list, e.g. 10,20-30");
        } else {
            help.push(extra);
        }
        g.cmd(&[config_if], &allowed(tail), &help, h(id));
    }
    g.cmd(
        &[config_if],
        "no switchport trunk allowed vlan",
        &[
            "Negate",
            "Switch port settings",
            "Trunk port",
            "Carried VLANs",
            "Back to every VLAN",
        ],
        h(SW_ALLOWED_ALL),
    );
    g.cmd(
        &[config, config_if, config_vlan],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    g.cmd(
        &[config, config_if, config_vlan],
        "exit",
        &["Up one level"],
        Action::Exit,
    );
    (
        g,
        Modes {
            user,
            privileged,
            config,
            config_if,
            config_vlan,
        },
    )
}

fn dotted(len: u8) -> Ipv4Addr {
    Ipv4Addr::from_u32(mask(len))
}

fn if_name(port: usize) -> String {
    format!("GigabitEthernet0/{port}")
}

/// Accepts `g0/1`, `gi0/1`, `gigabitethernet0/1` (any prefix of the full name, case-insensitive).
pub fn parse_ifname(s: &str, n_ports: usize) -> Option<usize> {
    let s = s.to_ascii_lowercase();
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, rest) = s.split_at(split);
    if letters.is_empty() || !"gigabitethernet".starts_with(letters) {
        return None;
    }
    let port: usize = rest.strip_prefix("0/")?.parse().ok()?;
    (port < n_ports).then_some(port)
}

/// One terminal on one device.
#[derive(Clone)]
pub struct Shell {
    grammar: Arc<Grammar>,
    modes: Modes,
    kind: Kind,
    session: Session,
    pub device: DeviceId,
    startup: Option<String>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("device", &self.device)
            .field("kind", &self.kind)
            .finish()
    }
}

impl Shell {
    pub fn router(device: DeviceId) -> Self {
        let (g, modes) = router_grammar();
        Shell {
            grammar: Arc::new(g),
            modes,
            kind: Kind::Router,
            session: Session::new(modes.user),
            device,
            startup: None,
        }
    }

    pub fn switch(device: DeviceId) -> Self {
        let (g, modes) = switch_grammar();
        Shell {
            grammar: Arc::new(g),
            modes,
            kind: Kind::Switch,
            session: Session::new(modes.user),
            device,
            startup: None,
        }
    }

    pub fn host(device: DeviceId) -> Self {
        let (g, modes) = host_grammar();
        Shell {
            grammar: Arc::new(g),
            modes,
            kind: Kind::Host,
            session: Session::new(modes.user),
            device,
            startup: None,
        }
    }

    pub fn prompt(&self, w: &World) -> String {
        let name = match self.kind {
            Kind::Router => w.device::<Router>(self.device).name.clone(),
            Kind::Host => w.device::<Host>(self.device).name.clone(),
            Kind::Switch => w.device::<L2Switch>(self.device).name.clone(),
        };
        self.session.prompt(&self.grammar, &name)
    }

    /// Saved configuration from `copy running-config startup-config`.
    pub fn startup_config(&self) -> Option<&str> {
        self.startup.as_deref()
    }

    pub fn help(&self, line: &str) -> String {
        render_help(&self.session.help(&self.grammar, line))
    }

    pub fn tab(&self, line: &str) -> String {
        self.session.tab(&self.grammar, line)
    }

    /// Run one line; returns the text to show (possibly empty). `None` means the session ended.
    pub fn exec(&mut self, w: &mut World, line: &str) -> Option<String> {
        let prompt = self.prompt(w);
        match self.session.exec(&self.grammar, line) {
            Exec::Empty | Exec::Moved => Some(String::new()),
            Exec::Logout => None,
            Exec::Error(e) => Some(render_error(&prompt, &e)),
            Exec::Run { handler, args } => Some(self.run(w, handler, &args)),
        }
    }

    fn run(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        let m = self.modes;
        match handler {
            ENABLE => self.session.enter(m.privileged, None),
            DISABLE => self.session.enter(m.user, None),
            CONF_T => self.session.enter(m.config, None),
            HOSTNAME => {
                let n = a.word("name").unwrap().to_string();
                if self.kind == Kind::Switch {
                    w.with_device::<L2Switch, _>(id, |s, _| s.name = n);
                } else {
                    w.with_device::<Router, _>(id, |r, _| r.name = n);
                }
            }
            INTERFACE => {
                let name = a.word("name").unwrap();
                let n = w.device::<Router>(id).interfaces().len();
                match parse_ifname(name, n) {
                    Some(p) => self.session.enter(m.config_if, Some(p.to_string())),
                    None => return format!("{UNKNOWN_INTERFACE} {name}"),
                }
            }
            IP_ADDRESS | NO_IP_ADDRESS | SHUTDOWN | NO_SHUTDOWN => {
                let port = PortId(
                    self.session
                        .context()
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(0),
                );
                w.with_device::<Router, _>(id, |r, _| match handler {
                    IP_ADDRESS => r.set_address(
                        port,
                        Some((Ipv4Addr(a.ipv4("addr").unwrap()), a.mask("mask").unwrap())),
                    ),
                    NO_IP_ADDRESS => r.set_address(port, None),
                    SHUTDOWN => r.set_shutdown(port, true),
                    _ => r.set_shutdown(port, false),
                });
            }
            IP_ROUTE | NO_IP_ROUTE => {
                let len = a.mask("mask").unwrap();
                let route = StaticRoute {
                    net: Ipv4Addr::from_u32(Ipv4Addr(a.ipv4("net").unwrap()).to_u32() & mask(len)),
                    len,
                    via: Ipv4Addr(a.ipv4("via").unwrap()),
                };
                if handler == IP_ROUTE {
                    w.with_device::<Router, _>(id, |r, _| r.add_static(route));
                } else if w.device::<Router>(id).statics().contains(&route) {
                    w.with_device::<Router, _>(id, |r, _| r.remove_static(route));
                } else {
                    return NO_ROUTE_ENTRY.to_string();
                }
            }
            SW_INTERFACE => {
                let name = a.word("name").unwrap();
                let n = w.device::<L2Switch>(id).n_ports() as usize;
                match parse_switch_ifname(name, n) {
                    Some(p) => self.session.enter(m.config_if, Some(p.to_string())),
                    None => return format!("{UNKNOWN_INTERFACE} {name}"),
                }
            }
            VLAN_ENTER => {
                let vid = a.int("id").unwrap() as u16;
                w.with_device::<L2Switch, _>(id, |s, _| s.add_vlan(vid));
                self.session.enter(m.config_vlan, Some(vid.to_string()));
            }
            NO_VLAN => {
                let vid = a.int("id").unwrap() as u16;
                if vid == DEFAULT_VLAN {
                    return DEFAULT_VLAN_KEPT.to_string();
                }
                if !w.with_device::<L2Switch, _>(id, |s, _| s.remove_vlan(vid)) {
                    return NO_SUCH_VLAN.to_string();
                }
            }
            VLAN_NAME => {
                let name = a.word("name").unwrap();
                if name.chars().count() > 32 {
                    return NAME_TOO_LONG.to_string();
                }
                if let Some(vid) = self.session.context().and_then(|c| c.parse::<u16>().ok()) {
                    w.with_device::<L2Switch, _>(id, |s, _| s.set_vlan_name(vid, name));
                }
            }
            SW_MODE_ACCESS | SW_MODE_TRUNK | NO_SW_MODE | SW_ACCESS_VLAN | NO_SW_ACCESS_VLAN
            | SW_NATIVE | NO_SW_NATIVE | SW_ALLOWED | SW_ALLOWED_ADD | SW_ALLOWED_REMOVE
            | SW_ALLOWED_ALL | SW_ALLOWED_NONE => return self.switchport(w, handler, a),
            SHOW_RUN if self.kind == Kind::Switch => {
                return switch_config(w.device::<L2Switch>(id));
            }
            SHOW_MACS => return show_macs(w.device::<L2Switch>(id), w.kernel.now()),
            SHOW_VLAN => return show_vlan_brief(w.device::<L2Switch>(id)),
            SHOW_TRUNK => return show_trunks(w.device::<L2Switch>(id)),
            SHOW_RUN => return running_config(w.device::<Router>(id)),
            SHOW_ROUTE => return show_routes(w.device::<Router>(id)),
            SHOW_BRIEF => return show_brief(w.device::<Router>(id)),
            COPY_RUN_START => {
                self.startup = Some(running_config(w.device::<Router>(id)));
                return SAVED.to_string();
            }
            IPCONFIG => return show_ipconfig(w.device::<Host>(id)),
            IPCONFIG_SET => {
                let (ip, len) = (Ipv4Addr(a.ipv4("addr").unwrap()), a.mask("mask").unwrap());
                w.with_device::<Host, _>(id, |h, _| {
                    h.ip = ip;
                    h.prefix_len = len;
                });
            }
            IPCONFIG_GW => {
                let gw = Ipv4Addr(a.ipv4("gw").unwrap());
                w.with_device::<Host, _>(id, |h, _| h.gateway = Some(gw));
            }
            PING => return ping(w, id, Ipv4Addr(a.ipv4("dst").unwrap())),
            _ => {}
        }
        String::new()
    }
}

impl Shell {
    /// The `switchport ...` family, for the interface the session is in.
    fn switchport(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        let port = PortId(
            self.session
                .context()
                .and_then(|c| c.parse().ok())
                .unwrap_or(0),
        );
        let mut notice = String::new();
        // Parse the VLAN list first so a bad one changes nothing.
        let list = match handler {
            SW_ALLOWED | SW_ALLOWED_ADD | SW_ALLOWED_REMOVE => {
                let text = a.word("list").unwrap_or_default();
                match parse_vlan_list(text) {
                    Some(l) => l,
                    None => return format!("{BAD_VLAN_LIST} {text}"),
                }
            }
            _ => Default::default(),
        };
        let vid = a.int("id").map_or(DEFAULT_VLAN, |v| v as u16);
        if handler == SW_ACCESS_VLAN && !w.device::<L2Switch>(id).vlan_exists(vid) {
            w.with_device::<L2Switch, _>(id, |s, _| s.add_vlan(vid));
            notice = format!("% Access VLAN does not exist. Creating vlan {vid}");
        }
        w.with_device::<L2Switch, _>(id, |s, _| {
            s.configure_port(port, |c| match handler {
                SW_MODE_ACCESS | NO_SW_MODE => c.mode = PortMode::Access,
                SW_MODE_TRUNK => c.mode = PortMode::Trunk,
                SW_ACCESS_VLAN | NO_SW_ACCESS_VLAN => c.access_vlan = vid,
                SW_NATIVE | NO_SW_NATIVE => c.native_vlan = vid,
                SW_ALLOWED => c.allowed = list,
                SW_ALLOWED_ADD => c.allowed.extend(list),
                SW_ALLOWED_REMOVE => c.allowed.retain(|v| !list.contains(v)),
                SW_ALLOWED_ALL => c.allowed = all_vlans(),
                _ => c.allowed.clear(),
            })
        });
        notice
    }
}

/// Switch ports are named FastEthernet0/1 up; accepts `fa0/1`, `fastethernet0/1` and any prefix
/// of the full name. Returns the zero-based port.
pub fn parse_switch_ifname(s: &str, n_ports: usize) -> Option<usize> {
    let s = s.to_ascii_lowercase();
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, rest) = s.split_at(split);
    if letters.is_empty() || !"fastethernet".starts_with(letters) {
        return None;
    }
    let n: usize = rest.strip_prefix("0/")?.parse().ok()?;
    (1..=n_ports).contains(&n).then(|| n - 1)
}

fn fa(port: usize) -> String {
    format!("Fa0/{}", port + 1)
}

pub fn running_config(r: &Router) -> String {
    let mut o = format!("hostname {}\n!\n", r.name);
    for (i, ifc) in r.interfaces().iter().enumerate() {
        o += &format!("interface {}\n", if_name(i));
        match ifc.addr {
            Some((ip, len)) => o += &format!(" ip address {ip} {}\n", dotted(len)),
            None => o += " no ip address\n",
        }
        o += if ifc.shutdown {
            " shutdown\n"
        } else {
            " no shutdown\n"
        };
        o += "!\n";
    }
    for s in r.statics() {
        o += &format!("ip route {} {} {}\n", s.net, dotted(s.len), s.via);
    }
    o + "end"
}

/// The switch's configuration as commands the shell accepts back: the hostname, VLANs other than
/// VLAN 1, and every port that differs from the defaults (access, VLAN 1, all VLANs allowed).
pub fn switch_config(s: &L2Switch) -> String {
    let mut o = format!("hostname {}\n!\n", s.name);
    for (id, name) in s.vlans().filter(|(id, _)| *id != DEFAULT_VLAN) {
        o += &format!("vlan {id}\n");
        if name != format!("VLAN{id:04}") {
            o += &format!(" name {name}\n");
        }
        o += "!\n";
    }
    let default = netdev::PortConfig::default();
    for p in 0..s.n_ports() {
        let c = s.port(kernel::PortId(p)).expect("port in range");
        if *c == default {
            continue;
        }
        o += &format!("interface FastEthernet0/{}\n", p + 1);
        if c.mode == PortMode::Trunk {
            o += " switchport mode trunk\n";
        }
        if c.access_vlan != DEFAULT_VLAN {
            o += &format!(" switchport access vlan {}\n", c.access_vlan);
        }
        if c.native_vlan != DEFAULT_VLAN {
            o += &format!(" switchport trunk native vlan {}\n", c.native_vlan);
        }
        if c.allowed != all_vlans() {
            o += &format!(
                " switchport trunk allowed vlan {}\n",
                format_vlan_list(&c.allowed)
            );
        }
        o += "!\n";
    }
    o + "end"
}

/// Switch ports are numbered from 1 in names (Fa0/1 is port 0).
fn show_macs(s: &L2Switch, now: SimTime) -> String {
    let mut o = format!("{:<6}{:<16}{}", "Vlan", "Mac Address", "Port");
    let table = s.mac_entries(now);
    for e in &table {
        o += &format!(
            "\n{:<6}{:<16}{}",
            e.vlan,
            e.mac.to_string(),
            fa(e.port.0 as usize)
        );
    }
    if table.is_empty() {
        o += "\n(no addresses learned)";
    }
    o
}

fn show_vlan_brief(s: &L2Switch) -> String {
    let mut o = format!("{:<5}{:<33}{:<10}{}", "VLAN", "Name", "Status", "Ports");
    o += &format!("\n{:-<4} {:-<32} {:-<9} {:-<31}", "", "", "", "");
    for (id, name) in s.vlans() {
        let ports: Vec<String> = (0..s.n_ports())
            .filter(|p| {
                let c = s.port(kernel::PortId(*p)).expect("port in range");
                c.mode == PortMode::Access && c.access_vlan == id
            })
            .map(|p| fa(p as usize))
            .collect();
        o += &format!(
            "\n{:<5}{:<33}{:<10}{}",
            id,
            name,
            "active",
            ports.join(", ")
        );
    }
    o
}

fn show_trunks(s: &L2Switch) -> String {
    let trunks: Vec<(usize, &netdev::PortConfig)> = (0..s.n_ports())
        .filter_map(|p| s.port(kernel::PortId(p)).map(|c| (p as usize, c)))
        .filter(|(_, c)| c.mode == PortMode::Trunk)
        .collect();
    if trunks.is_empty() {
        return String::new();
    }
    let mut o = format!(
        "{:<12}{:<13}{:<15}{:<14}{}",
        "Port", "Mode", "Encapsulation", "Status", "Native vlan"
    );
    for (p, c) in &trunks {
        o += &format!(
            "\n{:<12}{:<13}{:<15}{:<14}{}",
            fa(*p),
            "on",
            "802.1q",
            "trunking",
            c.native_vlan
        );
    }
    o += &format!("\n\n{:<12}{}", "Port", "Vlans allowed on trunk");
    for (p, c) in &trunks {
        o += &format!("\n{:<12}{}", fa(*p), format_vlan_list(&c.allowed));
    }
    o += &format!(
        "\n\n{:<12}{}",
        "Port", "Vlans allowed and active in management domain"
    );
    for (p, c) in &trunks {
        let active: std::collections::BTreeSet<u16> = c
            .allowed
            .iter()
            .copied()
            .filter(|v| s.vlan_exists(*v))
            .collect();
        o += &format!("\n{:<12}{}", fa(*p), format_vlan_list(&active));
    }
    o
}

fn show_routes(r: &Router) -> String {
    let mut o = String::from("Codes: C connected, S static\n");
    let mut any = false;
    for rt in r.fib().routes() {
        any = true;
        o += "\n";
        let net = format!("{}/{}", rt.net, rt.len);
        match (rt.kind, rt.via) {
            (RouteKind::Connected, _) => {
                o += &format!(
                    "C  {net}  directly connected, {}",
                    if_name(rt.port.0 as usize)
                )
            }
            (RouteKind::Static, Some(v)) => {
                o += &format!("S  {net}  via {v}, {}", if_name(rt.port.0 as usize))
            }
            (RouteKind::Static, None) => o += &format!("S  {net}"),
        }
    }
    if !any {
        o += "\n(no routes)";
    }
    o
}

fn show_brief(r: &Router) -> String {
    let mut o = format!("{:<22}{:<18}{}", "Interface", "Address", "Status");
    for (i, ifc) in r.interfaces().iter().enumerate() {
        let addr = ifc
            .addr
            .map_or("unassigned".to_string(), |(ip, _)| ip.to_string());
        let st = if ifc.shutdown { "admin down" } else { "up" };
        o += &format!("\n{:<22}{:<18}{}", if_name(i), addr, st);
    }
    o
}

fn show_ipconfig(h: &Host) -> String {
    let mut o = format!("Address : {}\nMask    : {}", h.ip, dotted(h.prefix_len));
    if let Some(g) = h.gateway {
        o += &format!("\nGateway : {g}");
    }
    o
}

/// Four echo requests, one per virtual second. Runs the simulation forward.
fn ping(w: &mut World, id: DeviceId, dst: Ipv4Addr) -> String {
    let mut lines = Vec::new();
    let mut received = 0;
    for _ in 0..4 {
        let (r0, e0) = {
            let h = w.device::<Host>(id);
            (h.ping_results.len(), h.icmp_errors.len())
        };
        w.with_device::<Host, _>(id, |h, c| h.ping(c, dst));
        let deadline = w.kernel.now() + SimTime::from_secs(1);
        w.run_until(deadline);
        let h = w.device::<Host>(id);
        if let Some(r) = h.ping_results.get(r0) {
            received += 1;
            lines.push(format!("Reply from {dst}: time={}us", r.rtt.0 / 1_000));
        } else if let Some(e) = h.icmp_errors.get(e0) {
            lines.push(match e {
                IcmpErrorKind::TimeExceeded => "TTL expired in transit.".to_string(),
                IcmpErrorKind::DestUnreachable(_) => "Destination unreachable.".to_string(),
            });
        } else {
            lines.push(TIMED_OUT.to_string());
        }
    }
    lines.push(format!("Sent 4, received {received}"));
    lines.join("\n")
}
