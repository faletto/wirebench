//! `standby` (HSRP) and `vrrp` commands and their `show` output, for routers and switch VLAN
//! interfaces.

use super::*;
use netdev::{FhrpEntry, FhrpGroup, FhrpProto, FhrpState, Router as Rt, FHRP_DEFAULT_PRIORITY};

pub(super) const SB_IP: u32 = 400;
pub(super) const NO_SB_IP: u32 = 401;
pub(super) const NO_SB: u32 = 402;
pub(super) const SB_PRIO: u32 = 403;
pub(super) const NO_SB_PRIO: u32 = 404;
pub(super) const SB_PREEMPT: u32 = 405;
pub(super) const NO_SB_PREEMPT: u32 = 406;
pub(super) const SB_TIMERS: u32 = 407;
pub(super) const NO_SB_TIMERS: u32 = 408;
pub(super) const VR_IP: u32 = 410;
pub(super) const NO_VR: u32 = 411;
pub(super) const VR_PRIO: u32 = 412;
pub(super) const NO_VR_PRIO: u32 = 413;
pub(super) const VR_PREEMPT: u32 = 414;
pub(super) const NO_VR_PREEMPT: u32 = 415;
pub(super) const VR_TIMERS: u32 = 416;
pub(super) const NO_VR_TIMERS: u32 = 417;
pub(super) const SB_TRACK: u32 = 409;
pub(super) const SB_TRACK_N: u32 = 418;
pub(super) const NO_SB_TRACK: u32 = 419;
pub(super) const SHOW_SB: u32 = 420;
pub(super) const SHOW_SB_BRIEF: u32 = 421;
pub(super) const SHOW_VR: u32 = 422;
pub(super) const SHOW_VR_BRIEF: u32 = 423;

const ROUTED_ONLY: &str =
    "% First hop redundancy applies to routed interfaces (or VLAN interfaces).";
const BAD_VIP: &str = "% The virtual address must be a unicast address.";
const BAD_TIMERS: &str = "% The hold time must be longer than the hello time.";
const NO_SUCH_GROUP: &str = "% No such group on this interface.";

pub(super) fn fhrp_commands(g: &mut Grammar, user: ModeId, privileged: ModeId, config_if: ModeId) {
    let h = Action::Handler;
    let sb = "HSRP interface configuration";
    let vr = "VRRP interface configuration";
    let grp = "Group number";
    let neg = "Negate";
    let cmds: &[(&[ModeId], &str, &[&str], u32)] = &[
        (
            &[config_if],
            "standby <g:int(0..255)> ip <vip:ipv4>",
            &[sb, grp, "Virtual address", "The address"],
            SB_IP,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)> ip",
            &[neg, sb, grp, "Virtual address"],
            NO_SB_IP,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)>",
            &[neg, sb, grp],
            NO_SB,
        ),
        (
            &[config_if],
            "standby <g:int(0..255)> priority <p:int(0..255)>",
            &[sb, grp, "Priority (default 100)", "Priority"],
            SB_PRIO,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)> priority",
            &[neg, sb, grp, "Priority"],
            NO_SB_PRIO,
        ),
        (
            &[config_if],
            "standby <g:int(0..255)> preempt",
            &[sb, grp, "Take over from a lower-priority active router"],
            SB_PREEMPT,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)> preempt",
            &[neg, sb, grp, "Preemption"],
            NO_SB_PREEMPT,
        ),
        (
            &[config_if],
            "standby <g:int(0..255)> timers <hello:int(1..254)> <hold:int(2..255)>",
            &[
                sb,
                grp,
                "Hello and hold times",
                "Hello, seconds",
                "Hold, seconds",
            ],
            SB_TIMERS,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)> timers",
            &[neg, sb, grp, "Timers"],
            NO_SB_TIMERS,
        ),
        (
            &[config_if],
            "standby <g:int(0..255)> track <name:word>",
            &[
                sb,
                grp,
                "Track an interface",
                "Interface (priority drops by 10 when it is down)",
            ],
            SB_TRACK,
        ),
        (
            &[config_if],
            "standby <g:int(0..255)> track <name:word> <dec:int(1..255)>",
            &[
                sb,
                grp,
                "Track an interface",
                "Interface",
                "How much the priority drops",
            ],
            SB_TRACK_N,
        ),
        (
            &[config_if],
            "no standby <g:int(0..255)> track <name:word>",
            &[neg, sb, grp, "Track an interface", "Interface"],
            NO_SB_TRACK,
        ),
        (
            &[config_if],
            "vrrp <g:int(1..255)> ip <vip:ipv4>",
            &[vr, grp, "Virtual address", "The address"],
            VR_IP,
        ),
        (
            &[config_if],
            "no vrrp <g:int(1..255)>",
            &[neg, vr, grp],
            NO_VR,
        ),
        (
            &[config_if],
            "vrrp <g:int(1..255)> priority <p:int(1..254)>",
            &[vr, grp, "Priority (default 100)", "Priority"],
            VR_PRIO,
        ),
        (
            &[config_if],
            "no vrrp <g:int(1..255)> priority",
            &[neg, vr, grp, "Priority"],
            NO_VR_PRIO,
        ),
        (
            &[config_if],
            "vrrp <g:int(1..255)> preempt",
            &[vr, grp, "Preemption (on by default)"],
            VR_PREEMPT,
        ),
        (
            &[config_if],
            "no vrrp <g:int(1..255)> preempt",
            &[neg, vr, grp, "Preemption"],
            NO_VR_PREEMPT,
        ),
        (
            &[config_if],
            "vrrp <g:int(1..255)> timers advertise <secs:int(1..255)>",
            &[vr, grp, "Timers", "Advertisement interval", "Seconds"],
            VR_TIMERS,
        ),
        (
            &[config_if],
            "no vrrp <g:int(1..255)> timers advertise",
            &[neg, vr, grp, "Timers", "Advertisement interval"],
            NO_VR_TIMERS,
        ),
        (
            &[user, privileged],
            "show standby",
            &["Show", "HSRP groups"],
            SHOW_SB,
        ),
        (
            &[user, privileged],
            "show standby brief",
            &["Show", "HSRP groups", "One line per group"],
            SHOW_SB_BRIEF,
        ),
        (
            &[user, privileged],
            "show vrrp",
            &["Show", "VRRP groups"],
            SHOW_VR,
        ),
        (
            &[user, privileged],
            "show vrrp brief",
            &["Show", "VRRP groups", "One line per group"],
            SHOW_VR_BRIEF,
        ),
    ];
    for (modes, line, help, id) in cmds {
        g.cmd(modes, line, help, h(*id));
    }
}

/// The `standby` / `vrrp` lines of a running config for logical interface `i`.
pub(super) fn fhrp_if_lines(r: &Rt, i: usize) -> String {
    let mut o = String::new();
    for FhrpEntry { iface, g } in &r.fhrp {
        if *iface != i {
            continue;
        }
        let (cmd, default_preempt, default_t) = match g.proto {
            FhrpProto::Hsrp => ("standby", false, g.hello == 3 && g.hold == 10),
            FhrpProto::Vrrp => ("vrrp", true, g.hello == 1),
        };
        if g.vip != Ipv4Addr::UNSPECIFIED {
            o += &format!(" {cmd} {} ip {}\n", g.group, g.vip);
        }
        for (t, d) in &g.tracks {
            let name = iface_name(r, *t);
            if *d == 10 {
                o += &format!(" standby {} track {name}\n", g.group);
            } else {
                o += &format!(" standby {} track {name} {d}\n", g.group);
            }
        }
        if g.priority != FHRP_DEFAULT_PRIORITY {
            o += &format!(" {cmd} {} priority {}\n", g.group, g.priority);
        }
        match (g.preempt, default_preempt) {
            (true, false) => o += &format!(" {cmd} {} preempt\n", g.group),
            (false, true) => o += &format!(" no {cmd} {} preempt\n", g.group),
            _ => {}
        }
        if !default_t {
            match g.proto {
                FhrpProto::Hsrp => {
                    o += &format!(" standby {} timers {} {}\n", g.group, g.hello, g.hold)
                }
                FhrpProto::Vrrp => {
                    o += &format!(" vrrp {} timers advertise {}\n", g.group, g.hello)
                }
            }
        }
    }
    o
}

fn short(name: &str) -> String {
    name.replace("GigabitEthernet", "Gi").replace("Vlan", "Vl")
}

fn hms(secs: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

fn local_or(ip: Option<(Ipv4Addr, u8)>) -> String {
    ip.map_or("unknown".to_string(), |(ip, _)| ip.to_string())
}

fn show_standby_brief(r: &Rt, now: SimTime) -> String {
    let mut o = String::from(
        "                     P indicates configured to preempt.\n                     |\n\
Interface   Grp  Pri P State   Active          Standby         Virtual IP",
    );
    for e in r.fhrp.iter().filter(|e| e.g.proto == FhrpProto::Hsrp) {
        let g = &e.g;
        let me = r.live(e.iface).map(|a| a.0);
        let active = if g.state() == FhrpState::Active {
            "local".to_string()
        } else {
            local_or(g.active_peer(now))
        };
        let standby = if g.state() == FhrpState::Standby {
            "local".to_string()
        } else {
            local_or(g.standby_peer(now))
        };
        let _ = me;
        o += &format!(
            "\n{:<12}{:<5}{:<4}{} {:<7} {:<15} {:<15} {}",
            short(&iface_name(r, e.iface)),
            g.group,
            g.priority,
            if g.preempt { "P" } else { " " },
            g.state().name(),
            active,
            standby,
            if g.vip == Ipv4Addr::UNSPECIFIED {
                "unknown".to_string()
            } else {
                g.vip.to_string()
            },
        );
    }
    o
}

fn show_standby(r: &Rt, now: SimTime) -> String {
    r.fhrp
        .iter()
        .filter(|e| e.g.proto == FhrpProto::Hsrp)
        .map(|e| {
            let g = &e.g;
            let ago = now.0.saturating_sub(g.last_change.0) / 1_000_000_000;
            let active = if g.state() == FhrpState::Active {
                "local".to_string()
            } else {
                g.active_peer(now).map_or("unknown".to_string(), |(ip, p)| {
                    format!(
                        "{ip}, priority {p} (expires in {} sec)",
                        g.active_expires_in(now).unwrap_or(0)
                    )
                })
            };
            let standby = if g.state() == FhrpState::Standby {
                "local".to_string()
            } else {
                g.standby_peer(now)
                    .map_or("unknown".to_string(), |(ip, p)| format!("{ip}, priority {p}"))
            };
            let tracks: String = g
                .tracks
                .iter()
                .map(|(t, d)| {
                    format!(
                        "\n  Track interface {} state {} decrement {d}",
                        short(&iface_name(r, *t)),
                        if r.is_up(*t) { "Up" } else { "Down" }
                    )
                })
                .collect();
            format!(
                "{} - Group {}\n  State is {}\n    {} state changes, last state change {}\n  Virtual IP address is {}\n  Active virtual MAC address is {}\n    Local virtual MAC address is {} (v1 default)\n  Hello time {} sec, hold time {} sec\n  Preemption {}\n  Active router is {active}\n  Standby router is {standby}\n  Priority {} (configured {}){tracks}\n  Group name is \"hsrp-{}-{}\" (default)",
                iface_name(r, e.iface),
                g.group,
                g.state().name(),
                g.transitions,
                hms(ago),
                if g.vip == Ipv4Addr::UNSPECIFIED { "unknown".to_string() } else { g.vip.to_string() },
                if g.is_active() { g.vmac().to_string() } else { "unknown".to_string() },
                g.vmac(),
                g.hello,
                g.hold,
                if g.preempt { "enabled" } else { "disabled" },
                g.priority.saturating_sub(g.penalty),
                g.priority,
                short(&iface_name(r, e.iface)),
                g.group,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn show_vrrp_brief(r: &Rt, now: SimTime) -> String {
    let mut o =
        String::from("Interface          Grp Pri Time  Own Pre State   Master addr     Group addr");
    for e in r.fhrp.iter().filter(|e| e.g.proto == FhrpProto::Vrrp) {
        let g = &e.g;
        let me = r.live(e.iface).map(|a| a.0);
        let owner = me == Some(g.vip);
        let master = if g.state() == FhrpState::Master {
            me.map_or("unknown".to_string(), |m| m.to_string())
        } else {
            local_or(g.active_peer(now))
        };
        let skew = (256 - u64::from(g.effective_priority(owner))) * 1000 / 256;
        o += &format!(
            "\n{:<18} {:>3} {:>3} {:>4}  {:<3} {:<3} {:<7} {:<15} {}",
            short(&iface_name(r, e.iface)),
            g.group,
            g.effective_priority(owner),
            3 * g.hello * 1000 + skew as u32,
            if owner { "Y" } else { "" },
            if g.preempt { "Y" } else { "" },
            g.state().name(),
            master,
            g.vip
        );
    }
    o
}

fn show_vrrp(r: &Rt, now: SimTime) -> String {
    r.fhrp
        .iter()
        .filter(|e| e.g.proto == FhrpProto::Vrrp)
        .map(|e| {
            let g = &e.g;
            let me = r.live(e.iface).map(|a| a.0);
            let owner = me == Some(g.vip);
            let prio = g.effective_priority(owner);
            let skew = (256 - u64::from(prio)) as f64 / 256.0;
            let master = if g.state() == FhrpState::Master {
                format!("{} (local), priority is {prio}", me.map_or(String::new(), |m| m.to_string()))
            } else {
                g.active_peer(now).map_or("unknown".to_string(), |(ip, p)| {
                    format!("{ip}, priority is {p}")
                })
            };
            format!(
                "{} - Group {}\n  State is {}\n  Virtual IP address is {}\n  Virtual MAC address is {}\n  Advertisement interval is {}.000 sec\n  Preemption {}\n  Priority is {prio}\n  Master Router is {master}\n  Master Advertisement interval is {}.000 sec\n  Master Down interval is {:.3} sec",
                iface_name(r, e.iface),
                g.group,
                g.state().name(),
                g.vip,
                g.vmac(),
                g.hello,
                if g.preempt { "enabled" } else { "disabled" },
                g.hello,
                f64::from(3 * g.hello) + skew,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Shell {
    /// Run `f` on the layer 3 half (the router, or the switch's VLAN interfaces) with a context.
    fn l3_ctx<R>(&self, w: &mut World, f: impl FnOnce(&mut Rt, &mut netdev::Ctx) -> R) -> R {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, ctx| s.originate(ctx, f)),
            _ => w.with_device::<Rt, _>(self.device, |r, ctx| f(r, ctx)),
        }
    }

    /// The logical interface a `track` names: a router port, or a switch's VLAN interface.
    fn track_target(&self, w: &World, name: &str) -> Option<usize> {
        match self.kind {
            Kind::Switch => parse_vlan_name(name).and_then(|v| self.l3(w).svi_index(v)),
            _ => parse_ifname(name, self.l3(w).n_ports() as usize),
        }
    }

    pub(super) fn fhrp_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let now = w.kernel.now();
        if matches!(handler, SHOW_SB..=SHOW_VR_BRIEF) {
            let r = self.l3(w);
            return match handler {
                SHOW_SB => show_standby(r, now),
                SHOW_SB_BRIEF => show_standby_brief(r, now),
                SHOW_VR => show_vrrp(r, now),
                _ => show_vrrp_brief(r, now),
            };
        }
        let Some(i) = self.current_iface(w) else {
            return ROUTED_ONLY.to_string();
        };
        let group = a.int("g").unwrap() as u8;
        let proto = if (VR_IP..=NO_VR_TIMERS).contains(&handler) {
            FhrpProto::Vrrp
        } else {
            FhrpProto::Hsrp
        };
        let exists = self
            .l3(w)
            .fhrp
            .iter()
            .any(|e| e.iface == i && e.g.proto == proto && e.g.group == group);
        if matches!(
            handler,
            NO_SB
                | NO_SB_IP
                | NO_SB_PRIO
                | NO_SB_PREEMPT
                | NO_SB_TIMERS
                | NO_SB_TRACK
                | NO_VR
                | NO_VR_PRIO
                | NO_VR_PREEMPT
                | NO_VR_TIMERS
        ) && !exists
        {
            return NO_SUCH_GROUP.to_string();
        }
        match handler {
            NO_SB | NO_VR => {
                self.l3_ctx(w, |r, ctx| r.fhrp_remove(ctx, i, proto, group));
                return String::new();
            }
            SB_IP | VR_IP => {
                let vip = Ipv4Addr(a.ipv4("vip").unwrap());
                if vip.0[0] == 0 || vip.0[0] == 127 || vip.0[0] >= 224 {
                    return BAD_VIP.to_string();
                }
            }
            SB_TIMERS => {
                if a.int("hold").unwrap() <= a.int("hello").unwrap() {
                    return BAD_TIMERS.to_string();
                }
            }
            _ => {}
        }
        let (hello, hold) = (
            a.int("hello").map(|h| h as u32),
            a.int("hold").map(|h| h as u32),
        );
        let secs = a.int("secs").map(|s| s as u32);
        let track = if matches!(handler, SB_TRACK | SB_TRACK_N | NO_SB_TRACK) {
            let name = a.word("name").unwrap_or_default();
            match self.track_target(w, name) {
                Some(t) => Some((t, a.int("dec").map_or(10, |d| d as u8))),
                None => return format!("{UNKNOWN_INTERFACE} {name}"),
            }
        } else {
            None
        };
        let prio = a.int("p").map(|p| p as u8);
        let vip = a.ipv4("vip").map(Ipv4Addr);
        self.l3_ctx(w, |r, ctx| {
            r.fhrp_configure(i, proto, group, |g: &mut FhrpGroup| match handler {
                SB_IP | VR_IP => g.vip = vip.unwrap(),
                NO_SB_IP => g.vip = Ipv4Addr::UNSPECIFIED,
                SB_PRIO | VR_PRIO => g.priority = prio.unwrap(),
                NO_SB_PRIO | NO_VR_PRIO => g.priority = FHRP_DEFAULT_PRIORITY,
                SB_PREEMPT | VR_PREEMPT => g.preempt = true,
                NO_SB_PREEMPT | NO_VR_PREEMPT => g.preempt = false,
                SB_TIMERS => {
                    g.hello = hello.unwrap();
                    g.hold = hold.unwrap();
                }
                NO_SB_TIMERS => {
                    g.hello = 3;
                    g.hold = 10;
                }
                SB_TRACK | SB_TRACK_N | NO_SB_TRACK => {
                    let (t, d) = track.unwrap();
                    g.tracks.retain(|(x, _)| *x != t);
                    if handler != NO_SB_TRACK {
                        g.tracks.push((t, d));
                    }
                }
                VR_TIMERS => g.hello = secs.unwrap(),
                _ => g.hello = 1,
            });
            r.fhrp_kick(ctx);
        });
        String::new()
    }
}
