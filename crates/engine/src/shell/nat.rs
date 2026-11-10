//! `ip nat` commands and the `show ip nat` displays, for routers.

use super::*;
use netdev::{NatPool, NatRule, NatSide, NatStatic, NatTarget};

// Handler ids 480..509.
pub(super) const NAT_INSIDE: u32 = 480;
pub(super) const NAT_OUTSIDE: u32 = 481;
pub(super) const NO_NAT_SIDE: u32 = 482;
pub(super) const NAT_POOL_MASK: u32 = 483;
pub(super) const NAT_POOL_PREFIX: u32 = 484;
pub(super) const NO_NAT_POOL: u32 = 485;
pub(super) const NAT_RULE_POOL: u32 = 486;
pub(super) const NAT_RULE_POOL_OVERLOAD: u32 = 487;
pub(super) const NAT_RULE_IF: u32 = 488;
pub(super) const NO_NAT_RULE_POOL: u32 = 489;
pub(super) const NO_NAT_RULE_IF: u32 = 490;
pub(super) const NAT_STATIC: u32 = 491;
pub(super) const NAT_STATIC_TCP: u32 = 492;
pub(super) const NAT_STATIC_UDP: u32 = 493;
pub(super) const NO_NAT_STATIC: u32 = 494;
pub(super) const NO_NAT_STATIC_TCP: u32 = 495;
pub(super) const NO_NAT_STATIC_UDP: u32 = 496;
pub(super) const SHOW_NAT_TRANSLATIONS: u32 = 497;
pub(super) const SHOW_NAT_STATISTICS: u32 = 498;
pub(super) const CLEAR_NAT: u32 = 499;
pub(super) const NAT_TIMEOUT_SIMPLE: u32 = 500;
pub(super) const NAT_TIMEOUT_TCP: u32 = 501;
pub(super) const NAT_TIMEOUT_UDP: u32 = 502;
pub(super) const NAT_TIMEOUT_ICMP: u32 = 503;
pub(super) const NAT_TIMEOUT_FINRST: u32 = 504;
/// The `no` forms are these plus five, in the same order.
pub(super) const NO_NAT_TIMEOUT_SIMPLE: u32 = 505;
pub(super) const NO_NAT_TIMEOUT_FINRST: u32 = 509;
pub(super) const NAT_OUTSIDE_STATIC: u32 = 510;
pub(super) const NO_NAT_OUTSIDE_STATIC: u32 = 511;
pub(super) const NAT_POOL_MASK_ROTARY: u32 = 512;
pub(super) const NAT_POOL_PREFIX_ROTARY: u32 = 513;
pub(super) const NAT_DEST_RULE: u32 = 514;
pub(super) const NO_NAT_DEST_RULE: u32 = 515;
pub(super) const NAT_MAX_ENTRIES: u32 = 516;
pub(super) const NO_NAT_MAX_ENTRIES: u32 = 517;
pub(super) const NAT_TIMEOUT_SYN: u32 = 518;
pub(super) const NO_NAT_TIMEOUT_SYN: u32 = 519;
pub(super) const NAT_STATIC_EXT: u32 = 520;
pub(super) const NAT_STATIC_TCP_EXT: u32 = 521;
pub(super) const NAT_STATIC_UDP_EXT: u32 = 522;

const TCP: u8 = 6;
const UDP: u8 = 17;

const POOL_IN_USE: &str = "% That pool is used by a translation rule; remove the rule first.";
const POOL_ORDER: &str = "% The last address is below the first.";
const POOL_MASK: &str = "% The addresses are not all in one network of that size.";
const NO_POOL: &str = "% No such NAT pool.";
const NO_RULE: &str = "% That translation rule is not configured.";
const NO_STATIC: &str = "% That static translation is not configured.";
const NOT_ROUTER_IF: &str = "% No such interface:";
const SIMILAR_STATIC: &str =
    "% A similar static entry already exists; add 'extendable' to keep both.";
const NOT_ROTARY: &str = "% That pool is not of type rotary.";
const NOT_ROUTED: &str = "% NAT applies to routed interfaces (on a switch, VLAN interfaces).";
const NO_ACL_NAT: &str = "% Note: that access list does not exist yet, so nothing is translated.";

pub(super) fn nat_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
) {
    let h = Action::Handler;
    let nat = "Network address translation";
    let src = "Translate the source of packets from the inside";
    let cfg_if: &[(&str, &[&str], u32)] = &[
        (
            "ip nat inside",
            &["IP", nat, "This interface faces the inside network"],
            NAT_INSIDE,
        ),
        (
            "ip nat outside",
            &["IP", nat, "This interface faces the outside network"],
            NAT_OUTSIDE,
        ),
        (
            "no ip nat inside",
            &["Negate", "IP", nat, "Not an inside interface"],
            NO_NAT_SIDE,
        ),
        (
            "no ip nat outside",
            &["Negate", "IP", nat, "Not an outside interface"],
            NO_NAT_SIDE,
        ),
    ];
    for (pattern, help, id) in cfg_if {
        g.cmd(&[config_if], pattern, help, h(*id));
    }
    let pool_help: &[&str] = &[
        "IP",
        nat,
        "Define a pool of global addresses",
        "Pool name",
        "First address",
        "Last address",
    ];
    let cfg: Vec<(&str, Vec<&str>, u32)> = vec![
        (
            "ip nat pool <name:word> <start:ipv4> <end:ipv4> netmask <mask:mask>",
            [pool_help, &["Network mask", "Mask"]].concat(),
            NAT_POOL_MASK,
        ),
        (
            "ip nat pool <name:word> <start:ipv4> <end:ipv4> prefix-length <len:int(1..32)>",
            [pool_help, &["Prefix length", "Length"]].concat(),
            NAT_POOL_PREFIX,
        ),
        (
            "no ip nat pool <name:word>",
            vec!["Negate", "IP", nat, "Pool of global addresses", "Pool name"],
            NO_NAT_POOL,
        ),
        (
            "ip nat inside source list <acl:word> pool <pool:word>",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "Translate packets this access list permits",
                "List name or number",
                "Use addresses from a pool, one per host",
                "Pool name",
            ],
            NAT_RULE_POOL,
        ),
        (
            "ip nat inside source list <acl:word> pool <pool:word> overload",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "Translate packets this access list permits",
                "List name or number",
                "Use addresses from a pool",
                "Pool name",
                "Share addresses by changing port numbers (PAT)",
            ],
            NAT_RULE_POOL_OVERLOAD,
        ),
        (
            "ip nat inside source list <acl:word> interface <ifname:word> overload",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "Translate packets this access list permits",
                "List name or number",
                "Use the address of an interface",
                "Interface",
                "Share the address by changing port numbers (PAT)",
            ],
            NAT_RULE_IF,
        ),
        (
            "no ip nat inside source list <acl:word> pool <pool:word>",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Access list",
                "List name or number",
                "Pool",
                "Pool name",
            ],
            NO_NAT_RULE_POOL,
        ),
        (
            "no ip nat inside source list <acl:word> pool <pool:word> overload",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Access list",
                "List name or number",
                "Pool",
                "Pool name",
                "PAT",
            ],
            NO_NAT_RULE_POOL,
        ),
        (
            "no ip nat inside source list <acl:word> interface <ifname:word> overload",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Access list",
                "List name or number",
                "Interface",
                "Interface name",
                "PAT",
            ],
            NO_NAT_RULE_IF,
        ),
        (
            "ip nat inside source static <local:ipv4> <global:ipv4>",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "One fixed address for another",
                "Inside local address",
                "Inside global address",
            ],
            NAT_STATIC,
        ),
        (
            "ip nat inside source static tcp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)>",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "One fixed address and port for another",
                "TCP",
                "Inside local address",
                "Inside local port",
                "Inside global address",
                "Inside global port",
            ],
            NAT_STATIC_TCP,
        ),
        (
            "ip nat inside source static udp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)>",
            vec![
                "IP",
                nat,
                "Inside addresses",
                src,
                "One fixed address and port for another",
                "UDP",
                "Inside local address",
                "Inside local port",
                "Inside global address",
                "Inside global port",
            ],
            NAT_STATIC_UDP,
        ),
        (
            "no ip nat inside source static <local:ipv4> <global:ipv4>",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Fixed translation",
                "Inside local address",
                "Inside global address",
            ],
            NO_NAT_STATIC,
        ),
        (
            "no ip nat inside source static tcp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)>",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Fixed translation",
                "TCP",
                "Inside local address",
                "Inside local port",
                "Inside global address",
                "Inside global port",
            ],
            NO_NAT_STATIC_TCP,
        ),
        (
            "no ip nat inside source static udp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)>",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                src,
                "Fixed translation",
                "UDP",
                "Inside local address",
                "Inside local port",
                "Inside global address",
                "Inside global port",
            ],
            NO_NAT_STATIC_UDP,
        ),
    ];
    for (pattern, help, id) in &cfg {
        g.cmd(&[config], pattern, help, h(*id));
    }
    let timeouts: &[(&str, &str, u32)] = &[
        (
            "timeout",
            "Idle time of one-to-one dynamic translations",
            NAT_TIMEOUT_SIMPLE,
        ),
        (
            "tcp-timeout",
            "Idle time of TCP translations",
            NAT_TIMEOUT_TCP,
        ),
        (
            "udp-timeout",
            "Idle time of UDP translations",
            NAT_TIMEOUT_UDP,
        ),
        (
            "icmp-timeout",
            "Idle time of ICMP translations",
            NAT_TIMEOUT_ICMP,
        ),
        (
            "finrst-timeout",
            "How long a TCP translation lasts after FIN or RST",
            NAT_TIMEOUT_FINRST,
        ),
    ];
    for (word, what, id) in timeouts {
        g.cmd(
            &[config],
            &format!("ip nat translation {word} <secs:int(1..2147483)>"),
            &["IP", nat, "Translation timers", what, "Seconds"],
            h(*id),
        );
        g.cmd(
            &[config],
            &format!("no ip nat translation {word}"),
            &["Negate", "IP", nat, "Translation timers", what],
            h(*id + 5),
        );
    }
    g.cmd(
        &[config],
        "ip nat outside source static <global:ipv4> <local:ipv4>",
        &[
            "IP",
            nat,
            "Outside addresses",
            "Translate the source of packets from the outside",
            "One fixed address for another",
            "Outside global address (the real one)",
            "Outside local address (as the inside sees it)",
        ],
        h(NAT_OUTSIDE_STATIC),
    );
    g.cmd(
        &[config],
        "no ip nat outside source static <global:ipv4> <local:ipv4>",
        &[
            "Negate",
            "IP",
            nat,
            "Outside addresses",
            "Translate the source of packets from the outside",
            "Fixed translation",
            "Outside global address",
            "Outside local address",
        ],
        h(NO_NAT_OUTSIDE_STATIC),
    );
    // Pools handed out in turn, inside destination NAT, limits, and static entries that may share
    // an address.
    let pool_head: &[&str] = &[
        "IP",
        nat,
        "Define a pool of global addresses",
        "Pool name",
        "First address",
        "Last address",
    ];
    let rotary: Vec<(&str, Vec<&str>, u32)> = vec![
        (
            "ip nat pool <name:word> <start:ipv4> <end:ipv4> netmask <mask:mask> type rotary",
            [pool_head, &["Network mask", "Mask", "Pool type", "Hand out addresses in turn"]].concat(),
            NAT_POOL_MASK_ROTARY,
        ),
        (
            "ip nat pool <name:word> <start:ipv4> <end:ipv4> prefix-length <len:int(1..32)> type rotary",
            [pool_head, &["Prefix length", "Length", "Pool type", "Hand out addresses in turn"]].concat(),
            NAT_POOL_PREFIX_ROTARY,
        ),
        (
            "ip nat inside destination list <acl:word> pool <pool:word>",
            vec![
                "IP",
                nat,
                "Inside addresses",
                "Translate the destination of connections from the outside",
                "Translate calls to the addresses this list permits",
                "List name or number",
                "Send them to the hosts of a pool, in turn",
                "Pool name (type rotary)",
            ],
            NAT_DEST_RULE,
        ),
        (
            "no ip nat inside destination list <acl:word> pool <pool:word>",
            vec![
                "Negate",
                "IP",
                nat,
                "Inside addresses",
                "Destination translation",
                "Access list",
                "List name or number",
                "Pool",
                "Pool name",
            ],
            NO_NAT_DEST_RULE,
        ),
        (
            "ip nat translation max-entries <n:int(1..2147483647)>",
            vec!["IP", nat, "Translation limits", "Most dynamic translations at once", "How many"],
            NAT_MAX_ENTRIES,
        ),
        (
            "no ip nat translation max-entries",
            vec!["Negate", "IP", nat, "Translation limits", "Most dynamic translations at once"],
            NO_NAT_MAX_ENTRIES,
        ),
        (
            "ip nat translation syn-timeout <secs:int(1..2147483)>",
            vec![
                "IP",
                nat,
                "Translation timers",
                "Idle time of a TCP translation a SYN made that nothing has answered",
                "Seconds",
            ],
            NAT_TIMEOUT_SYN,
        ),
        (
            "no ip nat translation syn-timeout",
            vec![
                "Negate",
                "IP",
                nat,
                "Translation timers",
                "Idle time of a TCP translation a SYN made that nothing has answered",
            ],
            NO_NAT_TIMEOUT_SYN,
        ),
    ];
    for (pattern, help, id) in &rotary {
        g.cmd(&[config], pattern, help, h(*id));
    }
    let ext_help = |what: &'static str, extra: &[&'static str]| -> Vec<&'static str> {
        // The `no` forms say "Fixed translation" and take a leading "no".
        let mut v = if what == "Fixed translation" {
            vec!["Negate", "IP", nat, "Inside addresses", src, what]
        } else {
            vec!["IP", nat, "Inside addresses", src, what]
        };
        v.extend_from_slice(extra);
        v.push("Keep other static entries that share an address");
        v
    };
    let ext: Vec<(&str, Vec<&str>, u32)> = vec![
        (
            "ip nat inside source static <local:ipv4> <global:ipv4> extendable",
            ext_help("One fixed address for another", &["Inside local address", "Inside global address"]),
            NAT_STATIC_EXT,
        ),
        (
            "ip nat inside source static tcp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)> extendable",
            ext_help(
                "One fixed address and port for another",
                &["TCP", "Inside local address", "Inside local port", "Inside global address", "Inside global port"],
            ),
            NAT_STATIC_TCP_EXT,
        ),
        (
            "ip nat inside source static udp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)> extendable",
            ext_help(
                "One fixed address and port for another",
                &["UDP", "Inside local address", "Inside local port", "Inside global address", "Inside global port"],
            ),
            NAT_STATIC_UDP_EXT,
        ),
        (
            "no ip nat inside source static <local:ipv4> <global:ipv4> extendable",
            ext_help("Fixed translation", &["Inside local address", "Inside global address"]),
            NO_NAT_STATIC,
        ),
        (
            "no ip nat inside source static tcp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)> extendable",
            ext_help(
                "Fixed translation",
                &["TCP", "Inside local address", "Inside local port", "Inside global address", "Inside global port"],
            ),
            NO_NAT_STATIC_TCP,
        ),
        (
            "no ip nat inside source static udp <local:ipv4> <lport:int(1..65535)> <global:ipv4> <gport:int(1..65535)> extendable",
            ext_help(
                "Fixed translation",
                &["UDP", "Inside local address", "Inside local port", "Inside global address", "Inside global port"],
            ),
            NO_NAT_STATIC_UDP,
        ),
    ];
    for (pattern, help, id) in &ext {
        g.cmd(&[config], pattern, help, h(*id));
    }
    g.cmd(
        &[user, privileged],
        "show ip nat translations",
        &["Show", "IP", nat, "The translation table"],
        h(SHOW_NAT_TRANSLATIONS),
    );
    g.cmd(
        &[user, privileged],
        "show ip nat statistics",
        &["Show", "IP", nat, "Counters and configuration"],
        h(SHOW_NAT_STATISTICS),
    );
    g.cmd(
        &[privileged],
        "clear ip nat translation *",
        &["Clear", "IP", nat, "Translations", "All the dynamic ones"],
        h(CLEAR_NAT),
    );
}

fn proto_of(handler: u32) -> u8 {
    match handler {
        NAT_STATIC_TCP | NO_NAT_STATIC_TCP | NAT_STATIC_TCP_EXT => TCP,
        NAT_STATIC_UDP | NO_NAT_STATIC_UDP | NAT_STATIC_UDP_EXT => UDP,
        _ => 0,
    }
}

impl Shell {
    pub(super) fn nat_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        match handler {
            NAT_INSIDE | NAT_OUTSIDE | NO_NAT_SIDE => {
                let Some(i) = self.current_iface(w) else {
                    return NOT_ROUTED.to_string();
                };
                let side = match handler {
                    NAT_INSIDE => Some(NatSide::Inside),
                    NAT_OUTSIDE => Some(NatSide::Outside),
                    _ => None,
                };
                self.with_l3(w, |r| r.set_nat_side(i, side));
            }
            NAT_POOL_MASK | NAT_POOL_PREFIX | NAT_POOL_MASK_ROTARY | NAT_POOL_PREFIX_ROTARY => {
                let rotary = handler >= NAT_POOL_MASK_ROTARY;
                let name = a.word("name").unwrap().to_string();
                let start = Ipv4Addr(a.ipv4("start").unwrap());
                let end = Ipv4Addr(a.ipv4("end").unwrap());
                let len = a
                    .mask("mask")
                    .or_else(|| a.int("len").map(|l| l as u8))
                    .unwrap();
                if end.to_u32() < start.to_u32() {
                    return POOL_ORDER.to_string();
                }
                if start.to_u32() & mask(len) != end.to_u32() & mask(len) {
                    return POOL_MASK.to_string();
                }
                let r = self.with_l3(w, |r| {
                    if r.nat.pool_in_use(&name) {
                        return Err(POOL_IN_USE);
                    }
                    r.nat.pools.retain(|p| p.name != name);
                    r.nat.pools.push(NatPool {
                        name,
                        start,
                        end,
                        prefix_len: len,
                        rotary,
                    });
                    Ok(())
                });
                if let Err(e) = r {
                    return e.to_string();
                }
            }
            NO_NAT_POOL => {
                let name = a.word("name").unwrap().to_string();
                let r = self.with_l3(w, |r| {
                    if r.nat.pool(&name).is_none() {
                        return Err(NO_POOL);
                    }
                    if r.nat.pool_in_use(&name) {
                        return Err(POOL_IN_USE);
                    }
                    r.nat.pools.retain(|p| p.name != name);
                    Ok(())
                });
                if let Err(e) = r {
                    return e.to_string();
                }
            }
            NAT_RULE_POOL | NAT_RULE_POOL_OVERLOAD | NAT_RULE_IF => {
                let acl = a.word("acl").unwrap().to_string();
                let overload = handler != NAT_RULE_POOL;
                let target = if handler == NAT_RULE_IF {
                    let name = a.word("ifname").unwrap();
                    match self.router_iface_index(w, name) {
                        Some(i) => NatTarget::Interface(i),
                        None => return format!("{NOT_ROUTER_IF} {name}"),
                    }
                } else {
                    let pool = a.word("pool").unwrap().to_string();
                    if self.l3(w).nat.pool(&pool).is_none() {
                        return NO_POOL.to_string();
                    }
                    NatTarget::Pool(pool)
                };
                let known = self.l3(w).acls.get(&acl).is_some();
                self.with_l3(w, |r| {
                    // One rule per access list: a new one replaces the old.
                    r.nat.rules.retain(|x| x.acl != acl);
                    r.nat.clear_dynamic(Some(&acl));
                    r.nat.rules.push(NatRule {
                        acl,
                        target,
                        overload,
                    });
                });
                if !known {
                    return NO_ACL_NAT.to_string();
                }
            }
            NO_NAT_RULE_POOL | NO_NAT_RULE_IF => {
                let acl = a.word("acl").unwrap().to_string();
                let found = self.with_l3(w, |r| {
                    let n = r.nat.rules.len();
                    r.nat.rules.retain(|x| x.acl != acl);
                    r.nat.clear_dynamic(Some(&acl));
                    r.nat.rules.len() != n
                });
                if !found {
                    return NO_RULE.to_string();
                }
            }
            NAT_STATIC | NAT_STATIC_TCP | NAT_STATIC_UDP | NAT_STATIC_EXT | NAT_STATIC_TCP_EXT
            | NAT_STATIC_UDP_EXT => {
                let st = NatStatic {
                    proto: proto_of(handler),
                    local: (
                        Ipv4Addr(a.ipv4("local").unwrap()),
                        a.int("lport").map_or(0, |p| p as u16),
                    ),
                    global: (
                        Ipv4Addr(a.ipv4("global").unwrap()),
                        a.int("gport").map_or(0, |p| p as u16),
                    ),
                    extendable: handler >= NAT_STATIC_EXT,
                };
                let added = self.with_l3(w, |r| {
                    let same = |s: &NatStatic| {
                        s.proto == st.proto && s.local == st.local && s.global == st.global
                    };
                    // Without `extendable` an entry may not share an address (or a port on it)
                    // with another.
                    if !st.extendable && r.nat.statics.iter().any(|s| !same(s) && s.overlaps(&st)) {
                        return Err(SIMILAR_STATIC);
                    }
                    r.nat.statics.retain(|s| !same(s));
                    r.nat.statics.push(st);
                    Ok(())
                });
                if let Err(e) = added {
                    return e.to_string();
                }
            }
            NO_NAT_STATIC | NO_NAT_STATIC_TCP | NO_NAT_STATIC_UDP => {
                let (proto, local) = (
                    proto_of(handler),
                    (
                        Ipv4Addr(a.ipv4("local").unwrap()),
                        a.int("lport").map_or(0, |p| p as u16),
                    ),
                );
                let found = self.with_l3(w, |r| {
                    let n = r.nat.statics.len();
                    r.nat
                        .statics
                        .retain(|s| !(s.proto == proto && s.local == local));
                    r.nat.statics.len() != n
                });
                if !found {
                    return NO_STATIC.to_string();
                }
            }
            NAT_TIMEOUT_SIMPLE..=NAT_TIMEOUT_FINRST
            | NO_NAT_TIMEOUT_SIMPLE..=NO_NAT_TIMEOUT_FINRST => {
                let reset = handler >= NO_NAT_TIMEOUT_SIMPLE;
                let kind = if reset { handler - 5 } else { handler };
                let secs = a.int("secs").map(|s| s as u64);
                self.with_l3(w, |r| {
                    let d = netdev::NatTimeouts::default();
                    let t = &mut r.nat.timeouts;
                    let (slot, default) = match kind {
                        NAT_TIMEOUT_SIMPLE => (&mut t.simple, d.simple),
                        NAT_TIMEOUT_TCP => (&mut t.tcp, d.tcp),
                        NAT_TIMEOUT_UDP => (&mut t.udp, d.udp),
                        NAT_TIMEOUT_ICMP => (&mut t.icmp, d.icmp),
                        _ => (&mut t.finrst, d.finrst),
                    };
                    *slot = secs.map_or(default, SimTime::from_secs);
                });
            }
            NAT_OUTSIDE_STATIC => {
                let pair = (
                    Ipv4Addr(a.ipv4("global").unwrap()),
                    Ipv4Addr(a.ipv4("local").unwrap()),
                );
                self.with_l3(w, |r| {
                    r.nat.outside.retain(|(g, _)| *g != pair.0);
                    r.nat.outside.push(pair);
                });
            }
            NO_NAT_OUTSIDE_STATIC => {
                let pair = (
                    Ipv4Addr(a.ipv4("global").unwrap()),
                    Ipv4Addr(a.ipv4("local").unwrap()),
                );
                let found = self.with_l3(w, |r| {
                    let n = r.nat.outside.len();
                    r.nat.outside.retain(|p| *p != pair);
                    r.nat.outside.len() != n
                });
                if !found {
                    return NO_STATIC.to_string();
                }
            }
            NAT_DEST_RULE => {
                let acl = a.word("acl").unwrap().to_string();
                let pool = a.word("pool").unwrap().to_string();
                let r = self.with_l3(w, |r| match r.nat.pool(&pool) {
                    None => Err(NO_POOL),
                    Some(p) if !p.rotary => Err(NOT_ROTARY),
                    Some(_) => {
                        r.nat.dest_rules.retain(|x| x.acl != acl);
                        r.nat.clear_dynamic(Some(&acl));
                        r.nat.dest_rules.push(netdev::NatDestRule::new(&acl, &pool));
                        Ok(())
                    }
                });
                if let Err(e) = r {
                    return e.to_string();
                }
            }
            NO_NAT_DEST_RULE => {
                let acl = a.word("acl").unwrap().to_string();
                let found = self.with_l3(w, |r| {
                    let n = r.nat.dest_rules.len();
                    r.nat.dest_rules.retain(|x| x.acl != acl);
                    r.nat.clear_dynamic(Some(&acl));
                    r.nat.dest_rules.len() != n
                });
                if !found {
                    return NO_RULE.to_string();
                }
            }
            NAT_MAX_ENTRIES | NO_NAT_MAX_ENTRIES => {
                let n = a.int("n").map(|n| n as usize);
                self.with_l3(w, |r| r.nat.max_entries = n);
            }
            NAT_TIMEOUT_SYN | NO_NAT_TIMEOUT_SYN => {
                let secs = a.int("secs").map(|s| s as u64);
                self.with_l3(w, |r| {
                    r.nat.timeouts.syn =
                        secs.map_or(netdev::NatTimeouts::default().syn, SimTime::from_secs)
                });
            }
            SHOW_NAT_TRANSLATIONS => return self.show_nat_translations(w),
            SHOW_NAT_STATISTICS => return self.show_nat_statistics(w),
            CLEAR_NAT => {
                self.with_l3(w, |r| r.nat.clear_dynamic(None));
            }
            _ => {}
        }
        String::new()
    }

    /// The logical interface `name` stands for, if it exists: `g0/1`, a subinterface, or on a
    /// switch `Vlan10`.
    fn router_iface_index(&self, w: &World, name: &str) -> Option<usize> {
        self.l3_iface_by_name(w, name)
    }

    fn show_nat_translations(&self, w: &mut World) -> String {
        let now = w.kernel.now();
        self.with_l3(w, |r| r.nat.expire(now));
        let r = self.l3(w);
        let tuple = |ip: Ipv4Addr, port: u16| {
            if port == 0 {
                ip.to_string()
            } else {
                format!("{ip}:{port}")
            }
        };
        let name = |proto: u8| match proto {
            TCP => "tcp",
            UDP => "udp",
            1 => "icmp",
            _ => "---",
        };
        let mut o = format!(
            "{:<5}{:<23}{:<23}{:<23}{}",
            "Pro", "Inside global", "Inside local", "Outside local", "Outside global"
        );
        for s in &r.nat.statics {
            o += &format!(
                "\n{:<5}{:<23}{:<23}{:<23}{}",
                name(s.proto),
                tuple(s.global.0, s.global.1),
                tuple(s.local.0, s.local.1),
                "---",
                "---"
            );
        }
        for (global, local) in &r.nat.outside {
            o += &format!(
                "\n{:<5}{:<23}{:<23}{:<23}{}",
                "---", "---", "---", local, global
            );
        }
        for e in r.nat.entries() {
            let far = match (e.remote, e.proto) {
                (Some((ip, port)), p) if p != 0 => tuple(ip, port),
                _ => "---".to_string(),
            };
            o += &format!(
                "\n{:<5}{:<23}{:<23}{:<23}{}",
                name(e.proto),
                tuple(e.global.0, e.global.1),
                tuple(e.local.0, e.local.1),
                far,
                far
            );
        }
        o
    }

    fn show_nat_statistics(&self, w: &mut World) -> String {
        let now = w.kernel.now();
        self.with_l3(w, |r| r.nat.expire(now));
        let r = self.l3(w);
        let statics = r.nat.statics.len();
        let dynamic = r.nat.entries().len();
        let extended = r.nat.entries().iter().filter(|e| e.proto != 0).count()
            + r.nat.statics.iter().filter(|s| s.proto != 0).count();
        let mut o = format!(
            "Total active translations: {} ({statics} static, {dynamic} dynamic; {extended} extended)",
            statics + dynamic
        );
        for (title, side) in [
            ("Outside interfaces:", NatSide::Outside),
            ("Inside interfaces:", NatSide::Inside),
        ] {
            o += &format!("\n{title}");
            let names: Vec<String> = (0..r.interfaces().len())
                .filter(|i| r.interfaces()[*i].nat == Some(side))
                .map(|i| iface_name(r, i))
                .collect();
            if !names.is_empty() {
                o += &format!("\n{}", names.join(", "));
            }
        }
        let s = &r.nat.stats;
        o += &format!(
            "\nHits: {}  Misses: {}\nExpired translations: {}\nDynamic mappings:",
            s.hits, s.misses, s.expired
        );
        for (n, rule) in r.nat.rules.iter().enumerate() {
            o += &format!(
                "\n-- Inside Source\n[Id: {}] access-list {} ",
                n + 1,
                rule.acl
            );
            match &rule.target {
                NatTarget::Interface(i) => {
                    o += &format!(
                        "interface {} refcount {}",
                        iface_name(r, *i),
                        self.refs(r, rule)
                    );
                }
                NatTarget::Pool(p) => {
                    o += &format!("pool {p} refcount {}", self.refs(r, rule));
                    if let Some(pool) = r.nat.pool(p) {
                        let total = pool.addrs().len();
                        let used = r
                            .nat
                            .entries()
                            .iter()
                            .filter(|e| e.acl == rule.acl)
                            .map(|e| e.global.0)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len();
                        o += &format!(
                            "\n pool {p}: netmask {}\n\tstart {} end {}\n\ttype generic, total addresses {total}, allocated {used} ({}%)",
                            dotted(pool.prefix_len),
                            pool.start,
                            pool.end,
                            used * 100 / total.max(1)
                        );
                    }
                }
            }
        }
        for (n, d) in r.nat.dest_rules.iter().enumerate() {
            let conns = r.nat.entries().iter().filter(|e| e.acl == d.acl).count();
            o += &format!(
                "\n-- Inside Destination\n[Id: {}] access-list {} pool {} refcount {conns}",
                n + 1,
                d.acl,
                d.pool
            );
            if let Some(pool) = r.nat.pool(&d.pool) {
                let total = pool.addrs().len();
                o += &format!(
                    "\n pool {}: netmask {}\n\tstart {} end {}\n\ttype rotary, total addresses {total}",
                    pool.name,
                    dotted(pool.prefix_len),
                    pool.start,
                    pool.end
                );
            }
        }
        if let Some(m) = r.nat.max_entries {
            o += &format!("\nMaximum translations: {m}");
        }
        o
    }

    /// Translations a rule has made, for `refcount`.
    fn refs(&self, r: &Router, rule: &NatRule) -> usize {
        r.nat.entries().iter().filter(|e| e.acl == rule.acl).count()
    }
}

/// `ip nat pool`, `ip nat inside source`: the global lines of a running config.
pub(super) fn nat_config(r: &Router) -> String {
    let mut o = String::new();
    for p in &r.nat.pools {
        o += &format!(
            "ip nat pool {} {} {} netmask {}{}\n",
            p.name,
            p.start,
            p.end,
            dotted(p.prefix_len),
            if p.rotary { " type rotary" } else { "" }
        );
    }
    for rule in &r.nat.rules {
        let over = if rule.overload { " overload" } else { "" };
        match &rule.target {
            NatTarget::Pool(p) => {
                o += &format!("ip nat inside source list {} pool {p}{over}\n", rule.acl)
            }
            NatTarget::Interface(i) => {
                o += &format!(
                    "ip nat inside source list {} interface {}{over}\n",
                    rule.acl,
                    iface_name(r, *i)
                )
            }
        }
    }
    for d in &r.nat.dest_rules {
        o += &format!("ip nat inside destination list {} pool {}\n", d.acl, d.pool);
    }
    for (global, local) in &r.nat.outside {
        o += &format!("ip nat outside source static {global} {local}\n");
    }
    if let Some(n) = r.nat.max_entries {
        o += &format!("ip nat translation max-entries {n}\n");
    }
    let (t, d) = (&r.nat.timeouts, netdev::NatTimeouts::default());
    for (word, now, default) in [
        ("timeout", t.simple, d.simple),
        ("tcp-timeout", t.tcp, d.tcp),
        ("udp-timeout", t.udp, d.udp),
        ("icmp-timeout", t.icmp, d.icmp),
        ("finrst-timeout", t.finrst, d.finrst),
        ("syn-timeout", t.syn, d.syn),
    ] {
        if now != default {
            o += &format!("ip nat translation {word} {}\n", now.0 / 1_000_000_000);
        }
    }
    for s in &r.nat.statics {
        let proto = match s.proto {
            TCP => " tcp",
            UDP => " udp",
            _ => "",
        };
        let ext = if s.extendable { " extendable" } else { "" };
        if s.proto == 0 {
            o += &format!(
                "ip nat inside source static {} {}{ext}\n",
                s.local.0, s.global.0
            );
        } else {
            o += &format!(
                "ip nat inside source static{proto} {} {} {} {}{ext}\n",
                s.local.0, s.local.1, s.global.0, s.global.1
            );
        }
    }
    o
}
