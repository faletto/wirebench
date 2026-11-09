//! M3b commands: port security, EtherChannel, CDP/LLDP, NTP, remote login and first-hop
//! redundancy. Kept apart from the older commands so the big dispatch in the parent stays small.

use super::*;
use netdev::{parse_mac, PortSecurity, PsecViolation};

// Handler ids 300..399.
pub(super) const PSEC_ON: u32 = 300;
pub(super) const PSEC_OFF: u32 = 301;
pub(super) const PSEC_MAX: u32 = 302;
pub(super) const PSEC_NO_MAX: u32 = 303;
pub(super) const PSEC_PROTECT: u32 = 304;
pub(super) const PSEC_RESTRICT: u32 = 305;
pub(super) const PSEC_SHUTDOWN: u32 = 306;
pub(super) const PSEC_NO_VIOLATION: u32 = 307;
pub(super) const PSEC_STICKY: u32 = 308;
pub(super) const PSEC_NO_STICKY: u32 = 309;
pub(super) const PSEC_MAC: u32 = 310;
pub(super) const PSEC_NO_MAC: u32 = 311;
pub(super) const PSEC_STICKY_MAC: u32 = 312;
pub(super) const PSEC_NO_STICKY_MAC: u32 = 313;
pub(super) const SHOW_PSEC: u32 = 314;
pub(super) const SHOW_PSEC_IF: u32 = 315;
pub(super) const SHOW_PSEC_ADDR: u32 = 316;
pub(super) const CLEAR_PSEC: u32 = 317;
pub(super) const ERRDIS_CAUSE: u32 = 318;
pub(super) const NO_ERRDIS_CAUSE: u32 = 319;
pub(super) const ERRDIS_INTERVAL: u32 = 320;
pub(super) const SHOW_IF_STATUS: u32 = 321;

const PSEC_OFF_MSG: &str = "% Port security is not enabled on this port.";
const BAD_MAC: &str = "% Invalid MAC address: use the form aaaa.bbbb.cccc.";
const PSEC_FULL: &str = "% Maximum number of secure addresses reached on this port.";
const PSEC_BELOW: &str = "% The maximum is lower than the number of secure addresses in use.";
const PSEC_NO_SUCH: &str = "% That secure address is not configured.";
const PSEC_TRUNK_ONLY: &str = "% Port security applies to switch ports.";

pub(super) fn port_security_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
) {
    let h = Action::Handler;
    let sw = "Switch port settings";
    let ps = "Security related command";
    let cmds: &[(&str, &[&str], u32)] = &[
        ("switchport port-security", &[sw, ps], PSEC_ON),
        ("no switchport port-security", &["Negate", sw, ps], PSEC_OFF),
        (
            "switchport port-security maximum <n:int(1..132)>",
            &[sw, ps, "Secure addresses allowed", "How many"],
            PSEC_MAX,
        ),
        (
            "no switchport port-security maximum",
            &["Negate", sw, ps, "Secure addresses allowed"],
            PSEC_NO_MAX,
        ),
        (
            "switchport port-security violation protect",
            &[sw, ps, "What a violation does", "Drop, count nothing"],
            PSEC_PROTECT,
        ),
        (
            "switchport port-security violation restrict",
            &[sw, ps, "What a violation does", "Drop and count"],
            PSEC_RESTRICT,
        ),
        (
            "switchport port-security violation shutdown",
            &[sw, ps, "What a violation does", "Error-disable the port"],
            PSEC_SHUTDOWN,
        ),
        (
            "no switchport port-security violation",
            &["Negate", sw, ps, "What a violation does"],
            PSEC_NO_VIOLATION,
        ),
        (
            "switchport port-security mac-address sticky",
            &[
                sw,
                ps,
                "Secure address",
                "Learn addresses and keep them in the config",
            ],
            PSEC_STICKY,
        ),
        (
            "no switchport port-security mac-address sticky",
            &[
                "Negate",
                sw,
                ps,
                "Secure address",
                "Stop keeping learned addresses",
            ],
            PSEC_NO_STICKY,
        ),
        (
            "switchport port-security mac-address sticky <mac:word>",
            &[
                sw,
                ps,
                "Secure address",
                "Sticky",
                "Address, aaaa.bbbb.cccc",
            ],
            PSEC_STICKY_MAC,
        ),
        (
            "no switchport port-security mac-address sticky <mac:word>",
            &["Negate", sw, ps, "Secure address", "Sticky", "Address"],
            PSEC_NO_STICKY_MAC,
        ),
        (
            "switchport port-security mac-address <mac:word>",
            &[sw, ps, "Secure address", "Address, aaaa.bbbb.cccc"],
            PSEC_MAC,
        ),
        (
            "no switchport port-security mac-address <mac:word>",
            &["Negate", sw, ps, "Secure address", "Address"],
            PSEC_NO_MAC,
        ),
    ];
    for (line, help, id) in cmds {
        g.cmd(&[config_if], line, help, h(*id));
    }
    g.cmd(
        &[user, privileged],
        "show port-security",
        &["Show", "Port security status"],
        h(SHOW_PSEC),
    );
    g.cmd(
        &[user, privileged],
        "show port-security interface <name:word>",
        &[
            "Show",
            "Port security status",
            "Interface",
            "Interface, e.g. fa0/1",
        ],
        h(SHOW_PSEC_IF),
    );
    g.cmd(
        &[user, privileged],
        "show port-security address",
        &["Show", "Port security status", "Secure addresses"],
        h(SHOW_PSEC_ADDR),
    );
    g.cmd(
        &[privileged],
        "clear port-security dynamic",
        &[
            "Reset functions",
            "Port security",
            "Forget dynamically learned addresses",
        ],
        h(CLEAR_PSEC),
    );
    g.cmd(
        &[config],
        "errdisable recovery cause psecure-violation",
        &[
            "Error-disable settings",
            "Recovery",
            "Cause",
            "Port security violation",
        ],
        h(ERRDIS_CAUSE),
    );
    g.cmd(
        &[config],
        "no errdisable recovery cause psecure-violation",
        &[
            "Negate",
            "Error-disable settings",
            "Recovery",
            "Cause",
            "Port security violation",
        ],
        h(NO_ERRDIS_CAUSE),
    );
    g.cmd(
        &[config],
        "errdisable recovery interval <secs:int(30..86400)>",
        &[
            "Error-disable settings",
            "Recovery",
            "Wait before recovery",
            "Seconds",
        ],
        h(ERRDIS_INTERVAL),
    );
    g.cmd(
        &[user, privileged],
        "show interfaces status",
        &["Show", "Interface information", "One line per port"],
        h(SHOW_IF_STATUS),
    );
}

impl Shell {
    fn switch_port(&self) -> Option<PortId> {
        self.session
            .context()
            .and_then(|c| c.parse().ok())
            .map(PortId)
    }

    /// Dispatch for the M3b handler range.
    pub(super) fn m3b_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        match handler {
            PSEC_ON..=PSEC_NO_STICKY_MAC => self.psec_config(w, handler, a),
            SHOW_PSEC..=CLEAR_PSEC => self.psec_show(w, handler, a),
            ERRDIS_CAUSE | NO_ERRDIS_CAUSE | ERRDIS_INTERVAL => {
                let secs = a.int("secs").map(|s| s as u32);
                w.with_device::<L2Switch, _>(self.device, |s, _| {
                    s.set_errdisable_recovery(
                        match handler {
                            ERRDIS_CAUSE => Some(true),
                            NO_ERRDIS_CAUSE => Some(false),
                            _ => None,
                        },
                        secs,
                    )
                });
                String::new()
            }
            SHOW_IF_STATUS => show_interfaces_status(w, self.device),
            _ => String::new(),
        }
    }

    fn psec_config(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let Some(port) = self.switch_port() else {
            return PSEC_TRUNK_ONLY.to_string();
        };
        let sw = w.device::<L2Switch>(self.device);
        let cfg = sw.port(port).expect("port in range");
        if handler == PSEC_ON && cfg.mode.is_dynamic() {
            return format!(
                "Command rejected: FastEthernet0/{} is a dynamic port.",
                port.0 + 1
            );
        }
        if handler != PSEC_ON && handler != PSEC_OFF && cfg.psec.is_none() {
            return PSEC_OFF_MSG.to_string();
        }
        let mac = match a.word("mac") {
            Some(m) => match parse_mac(m) {
                Some(m) => Some(m),
                None => return BAD_MAC.to_string(),
            },
            None => None,
        };
        let in_use = cfg
            .psec
            .as_ref()
            .map_or(0, |s| sw.psec.count(port.0 as usize, s));
        let max = a.int("n").map(|n| n as u16);
        if handler == PSEC_MAX && usize::from(max.unwrap_or(1)) < in_use {
            return PSEC_BELOW.to_string();
        }
        if matches!(handler, PSEC_MAC | PSEC_STICKY_MAC)
            && cfg
                .psec
                .as_ref()
                .is_some_and(|s| !mac.is_some_and(|m| s.knows(&m)) && in_use >= usize::from(s.max))
        {
            return PSEC_FULL.to_string();
        }
        if matches!(handler, PSEC_NO_MAC | PSEC_NO_STICKY_MAC)
            && !cfg.psec.as_ref().is_some_and(|s| {
                let m = mac.unwrap();
                if handler == PSEC_NO_MAC {
                    s.statics.contains(&m)
                } else {
                    s.stuck.contains(&m)
                }
            })
        {
            return PSEC_NO_SUCH.to_string();
        }
        w.with_device::<L2Switch, _>(self.device, |s, _| {
            s.configure_port(port, |c| match handler {
                PSEC_ON => {
                    c.psec.get_or_insert_with(PortSecurity::default);
                }
                PSEC_OFF => c.psec = None,
                _ => {
                    let p = c.psec.as_mut().expect("checked above");
                    match handler {
                        PSEC_MAX => p.max = max.unwrap_or(1),
                        PSEC_NO_MAX => p.max = 1,
                        PSEC_PROTECT => p.violation = PsecViolation::Protect,
                        PSEC_RESTRICT => p.violation = PsecViolation::Restrict,
                        PSEC_SHUTDOWN | PSEC_NO_VIOLATION => p.violation = PsecViolation::Shutdown,
                        PSEC_STICKY => p.sticky = true,
                        PSEC_NO_STICKY => p.sticky = false,
                        PSEC_MAC => {
                            p.statics.insert(mac.unwrap());
                        }
                        PSEC_NO_MAC => {
                            p.statics.remove(&mac.unwrap());
                        }
                        PSEC_STICKY_MAC => {
                            p.stuck.insert(mac.unwrap());
                        }
                        _ => {
                            p.stuck.remove(&mac.unwrap());
                        }
                    }
                }
            })
        });
        String::new()
    }

    fn psec_show(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        if handler == CLEAR_PSEC {
            w.with_device::<L2Switch, _>(self.device, |s, _| s.clear_secure_dynamic());
            return String::new();
        }
        let sw = w.device::<L2Switch>(self.device);
        match handler {
            SHOW_PSEC => show_port_security(sw),
            SHOW_PSEC_ADDR => show_secure_addresses(sw),
            _ => {
                let name = a.word("name").unwrap_or_default();
                match parse_switch_ifname(name, sw.n_ports() as usize) {
                    Some(p) => show_port_security_if(w, self.device, p),
                    None => format!("{UNKNOWN_INTERFACE} {name}"),
                }
            }
        }
    }
}

pub(super) fn psec_lines(c: &netdev::PortConfig) -> String {
    let Some(p) = &c.psec else {
        return String::new();
    };
    let mut o = String::from(" switchport port-security\n");
    if p.max != 1 {
        o += &format!(" switchport port-security maximum {}\n", p.max);
    }
    if p.violation != PsecViolation::Shutdown {
        o += &format!(
            " switchport port-security violation {}\n",
            p.violation.word()
        );
    }
    if p.sticky {
        o += " switchport port-security mac-address sticky\n";
    }
    for m in &p.statics {
        o += &format!(" switchport port-security mac-address {m}\n");
    }
    for m in &p.stuck {
        o += &format!(" switchport port-security mac-address sticky {m}\n");
    }
    o
}

/// Global `errdisable recovery` lines of a running config.
pub(super) fn errdisable_lines(s: &L2Switch) -> String {
    let mut o = String::new();
    if s.psec.recover {
        o += "errdisable recovery cause psecure-violation\n";
    }
    if s.psec.interval != netdev::DEFAULT_RECOVERY_SECS {
        o += &format!("errdisable recovery interval {}\n", s.psec.interval);
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

fn show_port_security(s: &L2Switch) -> String {
    let mut o = String::from(
        "Secure Port  MaxSecureAddr  CurrentAddr  SecurityViolation  Security Action\n\
\x20               (Count)       (Count)          (Count)\n",
    );
    let bar = "-".repeat(75);
    o += &bar;
    o += "\n";
    let mut extra = 0;
    for p in 0..s.n_ports() {
        let Some(ps) = s.port(PortId(p)).and_then(|c| c.psec.as_ref()) else {
            continue;
        };
        let n = s.psec.count(p as usize, ps);
        extra += n.saturating_sub(1);
        let action = match ps.violation {
            PsecViolation::Shutdown => "Shutdown",
            PsecViolation::Restrict => "Restrict",
            PsecViolation::Protect => "Protect",
        };
        o += &format!(
            "{:>10}  {:>13}  {:>11}  {:>17}  {:>15}\n",
            fa(p as usize),
            ps.max,
            n,
            s.psec.violations[p as usize],
            action
        );
    }
    o += &bar;
    o += &format!("\nTotal Addresses in System (excluding one mac per port)     : {extra}");
    o
}

fn show_port_security_if(w: &World, id: DeviceId, p: usize) -> String {
    let s = w.device::<L2Switch>(id);
    let port = PortId(p as u16);
    let Some(ps) = s.port(port).and_then(|c| c.psec.as_ref()) else {
        return "Port Security              : Disabled\nPort Status                : Secure-down\n\
Violation Mode             : Shutdown"
            .to_string();
    };
    let status = if s.err_disabled(port) {
        "Secure-shutdown"
    } else if w.kernel.port_up(id, port) {
        "Secure-up"
    } else {
        "Secure-down"
    };
    let (last, vlan) = s.psec.last[p].unwrap_or((wire::MacAddr([0; 6]), 0));
    format!(
        "Port Security              : Enabled\nPort Status                : {status}\n\
Violation Mode             : {}\nAging Time                 : 0 mins\n\
Aging Type                 : Absolute\nSecureStatic Address Aging : Disabled\n\
Maximum MAC Addresses      : {}\nTotal MAC Addresses        : {}\n\
Configured MAC Addresses   : {}\nSticky MAC Addresses       : {}\n\
Last Source Address:Vlan   : {last}:{vlan}\nSecurity Violation Count   : {}",
        match ps.violation {
            PsecViolation::Shutdown => "Shutdown",
            PsecViolation::Restrict => "Restrict",
            PsecViolation::Protect => "Protect",
        },
        ps.max,
        s.psec.count(p, ps),
        ps.statics.len(),
        ps.stuck.len(),
        s.psec.violations[p]
    )
}

fn show_secure_addresses(s: &L2Switch) -> String {
    let mut o = String::from(
        "          Secure Mac Address Table\n\
-------------------------------------------------------------------\n\
Vlan    Mac Address       Type                          Ports   Remaining Age\n\
\x20                                                                  (mins)\n\
----    -----------       ----                          -----   -------------",
    );
    for p in 0..s.n_ports() {
        let c = s.port(PortId(p)).expect("port in range");
        let Some(ps) = &c.psec else { continue };
        let vlan = if s.is_trunk(PortId(p)) {
            c.native_vlan
        } else {
            c.access_vlan
        };
        let rows = ps
            .statics
            .iter()
            .map(|m| (*m, vlan, "SecureConfigured"))
            .chain(ps.stuck.iter().map(|m| (*m, vlan, "SecureSticky")))
            .chain(
                s.psec.dynamic[p as usize]
                    .iter()
                    .map(|(m, v)| (*m, *v, "SecureDynamic")),
            );
        for (m, v, kind) in rows {
            o += &format!(
                "\n{v:>4}    {m}    {kind:<28}  {:<6}  {:>13}",
                fa(p as usize),
                "-"
            );
        }
    }
    o
}

fn show_interfaces_status(w: &World, id: DeviceId) -> String {
    let s = w.device::<L2Switch>(id);
    let mut o = format!(
        "{:<9} {:<18} {:<12} {:<10} {}",
        "Port", "Name", "Status", "Vlan", "Duplex Speed Type"
    );
    for p in 0..s.n_ports() {
        let port = PortId(p);
        let c = s.port(port).expect("port in range");
        let status = if s.err_disabled(port) {
            "err-disabled"
        } else if c.shutdown {
            "disabled"
        } else if w.kernel.port_up(id, port) {
            "connected"
        } else {
            "notconnect"
        };
        let vlan = if s.is_trunk(port) {
            "trunk".to_string()
        } else {
            c.access_vlan.to_string()
        };
        o += &format!(
            "\n{:<9} {:<18} {:<12} {:<10} a-full a-100 10/100BaseTX",
            fa(p as usize),
            "",
            status,
            vlan
        );
    }
    o
}
