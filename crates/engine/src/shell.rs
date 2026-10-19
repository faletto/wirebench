//! Binds `clish` command grammars to devices. Each [`Shell`] is one terminal attached to
//! one device; the grammar is shared per device kind.

use crate::World;
use clish::{render_error, render_help, Action, Args, Exec, Grammar, ModeId, Session};
use kernel::{DeviceId, PortId, SimTime};
use netdev::{mask, Host, RouteKind, Router, StaticRoute};
use std::sync::Arc;
use wire::{IcmpErrorKind, Ipv4Addr};

// ---- message catalog (shell-level strings) ----
const UNKNOWN_INTERFACE: &str = "% No such interface:";
const NO_ROUTE_ENTRY: &str = "% That static route is not configured.";
const SAVED: &str = "Configuration saved.";
const TIMED_OUT: &str = "No reply (timed out).";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Router,
    Host,
}

#[derive(Clone, Copy, Debug)]
struct Modes {
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
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

fn router_grammar() -> (Grammar, Modes) {
    let mut g = Grammar::new();
    let user = g.mode("user", "{host}>", None, None);
    let privileged = g.mode("privileged", "{host}#", None, None);
    let config = g.mode("config", "{host}(config)#", Some(privileged), None);
    let config_if = g.mode("config-if", "{host}(config-if)#", Some(config), Some(config));
    g.do_mode = Some(privileged);
    let h = Action::Handler;
    g.cmd(&[user], "enable", &["Become privileged"], h(ENABLE));
    g.cmd(&[user, privileged], "exit", &["Leave the session"], Action::Exit);
    g.cmd(&[privileged], "disable", &["Drop privileges"], h(DISABLE));
    g.cmd(&[privileged], "configure terminal", &["Configure", "From this terminal"], h(CONF_T));
    g.cmd(&[privileged], "show running-config", &["Show", "Current configuration"], h(SHOW_RUN));
    g.cmd(&[privileged], "show ip route", &["Show", "IP information", "Routing table"], h(SHOW_ROUTE));
    g.cmd(
        &[privileged],
        "show ip interface brief",
        &["Show", "IP information", "Interface information", "One line per interface"],
        h(SHOW_BRIEF),
    );
    g.cmd(
        &[privileged],
        "copy running-config startup-config",
        &["Copy", "Current configuration", "To", "Saved configuration"],
        h(COPY_RUN_START),
    );
    g.cmd(&[config], "hostname <name:word>", &["Set the device name", "New name"], h(HOSTNAME));
    g.cmd(&[config], "interface <name:word>", &["Configure an interface", "Interface, e.g. g0/0"], h(INTERFACE));
    let route_help = ["IP", "Static route", "Destination network", "Network mask", "Next hop"];
    g.cmd(&[config], "ip route <net:ipv4> <mask:mask> <via:ipv4>", &route_help, h(IP_ROUTE));
    g.cmd(
        &[config],
        "no ip route <net:ipv4> <mask:mask> <via:ipv4>",
        &["Negate", "IP", "Static route", "Destination network", "Network mask", "Next hop"],
        h(NO_IP_ROUTE),
    );
    g.cmd(&[config, config_if], "end", &["Back to privileged"], Action::End(privileged));
    g.cmd(&[config, config_if], "exit", &["Up one level"], Action::Exit);
    g.cmd(
        &[config_if],
        "ip address <addr:ipv4> <mask:mask>",
        &["IP", "Set address", "Address", "Network mask"],
        h(IP_ADDRESS),
    );
    g.cmd(&[config_if], "no ip address", &["Negate", "IP", "Remove the address"], h(NO_IP_ADDRESS));
    g.cmd(&[config_if], "shutdown", &["Disable the interface"], h(SHUTDOWN));
    g.cmd(&[config_if], "no shutdown", &["Negate", "Enable the interface"], h(NO_SHUTDOWN));
    (g, Modes { user, privileged, config, config_if })
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
    g.cmd(&[m], "ping <dst:ipv4>", &["Send echo requests", "Destination"], h(PING));
    (g, Modes { user: m, privileged: m, config: m, config_if: m })
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
        f.debug_struct("Shell").field("device", &self.device).field("kind", &self.kind).finish()
    }
}

impl Shell {
    pub fn router(device: DeviceId) -> Self {
        let (g, modes) = router_grammar();
        Shell { grammar: Arc::new(g), modes, kind: Kind::Router, session: Session::new(modes.user), device, startup: None }
    }

    pub fn host(device: DeviceId) -> Self {
        let (g, modes) = host_grammar();
        Shell { grammar: Arc::new(g), modes, kind: Kind::Host, session: Session::new(modes.user), device, startup: None }
    }

    pub fn prompt(&self, w: &World) -> String {
        let name = match self.kind {
            Kind::Router => w.device::<Router>(self.device).name.clone(),
            Kind::Host => w.device::<Host>(self.device).name.clone(),
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
                w.with_device::<Router, _>(id, |r, _| r.name = n);
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
                let port = PortId(self.session.context().and_then(|c| c.parse().ok()).unwrap_or(0));
                w.with_device::<Router, _>(id, |r, _| match handler {
                    IP_ADDRESS => r.set_address(port, Some((Ipv4Addr(a.ipv4("addr").unwrap()), a.mask("mask").unwrap()))),
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

pub fn running_config(r: &Router) -> String {
    let mut o = format!("hostname {}\n!\n", r.name);
    for (i, ifc) in r.interfaces().iter().enumerate() {
        o += &format!("interface {}\n", if_name(i));
        match ifc.addr {
            Some((ip, len)) => o += &format!(" ip address {ip} {}\n", dotted(len)),
            None => o += " no ip address\n",
        }
        o += if ifc.shutdown { " shutdown\n" } else { " no shutdown\n" };
        o += "!\n";
    }
    for s in r.statics() {
        o += &format!("ip route {} {} {}\n", s.net, dotted(s.len), s.via);
    }
    o + "end"
}

fn show_routes(r: &Router) -> String {
    let mut o = String::from("Codes: C connected, S static\n");
    let mut any = false;
    for rt in r.fib().routes() {
        any = true;
        o += "\n";
        let net = format!("{}/{}", rt.net, rt.len);
        match (rt.kind, rt.via) {
            (RouteKind::Connected, _) => o += &format!("C  {net}  directly connected, {}", if_name(rt.port.0 as usize)),
            (RouteKind::Static, Some(v)) => o += &format!("S  {net}  via {v}, {}", if_name(rt.port.0 as usize)),
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
        let addr = ifc.addr.map_or("unassigned".to_string(), |(ip, _)| ip.to_string());
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
