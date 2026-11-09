//! Binds `clish` command grammars to devices. Each [`Shell`] is one terminal attached to
//! one device; the grammar is shared per device kind.

use crate::trace::{self, Origin};
use crate::World;
use clish::{render_error, render_help, Action, Args, Exec, Grammar, ModeId, Session};
use kernel::{DeviceId, PortId, SimTime};
use netdev::{
    acl_config_text, acl_kind_of_number, acl_show, all_vlans, banner_delimiter, format_vlan_list,
    mask, parse_acl_rule, parse_vlan_list, password_text, plain_password, stored_secret,
    AclAddError, AclBody, AclKind, EncapError, Host, L2Switch, Mgmt, Ospf, OspfNbrState,
    OspfNetStmt, PortMode, RouteKind, Router, StaticRoute, StpMode, StpRole, StpState,
    DEFAULT_VLAN,
};
use std::sync::Arc;
use wire::{IcmpErrorKind, Ipv4Addr};

mod disc;
mod etherchannel;
mod m3b;

// ---- message catalog (shell-level strings) ----
const UNKNOWN_INTERFACE: &str = "% No such interface:";
const NO_ROUTE_ENTRY: &str = "% That static route is not configured.";
const SAVED: &str = "Configuration saved.";
const TIMED_OUT: &str = "No reply (timed out).";
const DEFAULT_VLAN_KEPT: &str = "% Default VLAN 1 may not be deleted.";
const NO_SUCH_VLAN: &str = "% VLAN does not exist.";
const BAD_VLAN_LIST: &str = "% Invalid VLAN list:";
const NEEDS_ENCAP: &str =
    "% Set an encapsulation (encapsulation dot1q <vlan>) before addressing a subinterface.";
const ENCAP_IN_USE: &str = "% That VLAN is already used by another subinterface of this port.";
const ENCAP_NOT_SUB: &str = "% Encapsulation can only be set on a subinterface.";
const NO_SUCH_INTERFACE: &str = "% No such interface.";
const SVI_ONLY: &str = "% Addresses go on VLAN interfaces (interface vlan N), not on switch ports.";
const BAD_BRIDGE_PRIORITY: &str = "% Bridge Priority must be in increments of 4096.\n% Allowed values are:\n  0     4096  8192  12288 16384 20480 24576 28672\n  32768 36864 40960 45056 49152 53248 57344 61440";
const BAD_PORT_PRIORITY: &str = "% Port priority must be a multiple of 16.";
const NO_SPAN_INSTANCE: &str = "No spanning tree instance exists.";
const NEGOTIATE_CONFLICT: &str =
    "Command rejected: Conflict between 'nonegotiate' and 'dynamic' status.";
const PORT_ONLY: &str = "% switchport commands apply to switch ports, not VLAN interfaces.";
/// A prompt that asks for a secret; the UI must not echo what is typed after it.
pub const PASSWORD_PROMPT: &str = "Password: ";
const BAD_SECRETS: &str = "% Bad secrets";
const BAD_PASSWORDS: &str = "% Bad passwords";
const NOT_PRESENT: &str = "startup-config is not present";
const ASK_SAVE: &str = "System configuration has been modified. Save? [yes/no]: ";
const ASK_RELOAD: &str = "Proceed with reload? [confirm]";
const ASK_ERASE: &str = "Erasing the saved configuration. [confirm]";
const ERASED: &str = "Erase of the saved configuration complete.";
const ANSWER_YES_NO: &str = "% Please answer 'yes' or 'no'.";
const BAD_RANGE: &str = "% The last address is below the first.";
const NO_SUCH_EXCLUSION: &str = "% That range is not excluded.";
const NO_SUCH_POOL: &str = "% No such pool.";
const NO_SUCH_HELPER: &str = "% That helper address is not configured.";
const DHCP_NO_ANSWER: &str = "No DHCP server answered; it keeps trying for a few more seconds.";
const BAD_ACL_NUMBER: &str = "% Invalid input detected: no such access list number.";
const ACL_KIND_CLASH: &str =
    "% An access list of that name already exists and is of the other kind.";
const NO_SUCH_ACL: &str = "% No such access list.";
const NO_SUCH_ACE: &str = "% No such entry in the access list.";
const NO_SUCH_SEQ: &str = "% No entry has that sequence number.";
const DUP_SEQ: &str = "% Duplicate sequence number.";
const SEQ_OVERFLOW: &str = "% Sequence number overflow.";
const OSPF_ONE_PROCESS: &str = "% Only one OSPF process is supported.";
const OSPF_NOT_RUNNING: &str = "% OSPF is not configured.";
const OSPF_BAD_AREA: &str = "% Invalid area: use a number or a dotted address.";
const OSPF_BAD_OPTION: &str = "% Invalid option: use always, metric <n>, metric-type <1|2>.";
const OSPF_NO_NETWORK: &str = "% That network statement is not configured.";
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
    /// `line console` / `line vty` configuration. Hosts point it at their only mode.
    config_line: ModeId,
    /// `ip dhcp pool` configuration. Hosts point it at their only mode.
    config_dhcp: ModeId,
    /// `ip access-list standard|extended <name>`. Hosts point both at their only mode.
    config_std_nacl: ModeId,
    config_ext_nacl: ModeId,
    /// `router ospf <n>`. Hosts point it at their only mode.
    config_router: ModeId,
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
const ENCAP: u32 = 60;
const ENCAP_NATIVE: u32 = 61;
const NO_ENCAP: u32 = 62;
const NO_INTERFACE: u32 = 63;
const SVI_ENTER: u32 = 70;
const NO_SVI: u32 = 71;
const IP_ROUTING: u32 = 72;
const NO_IP_ROUTING: u32 = 73;
const DEFAULT_GW: u32 = 74;
const NO_DEFAULT_GW: u32 = 75;
const ENABLE_SECRET: u32 = 80;
const ENABLE_PASSWORD: u32 = 82;
const NO_ENABLE_SECRET: u32 = 84;
const NO_ENABLE_PASSWORD: u32 = 85;
const SERVICE_PWENC: u32 = 86;
const NO_SERVICE_PWENC: u32 = 87;
const LINE_CONSOLE: u32 = 88;
const LINE_VTY: u32 = 89;
const LINE_PASSWORD: u32 = 90;
const NO_LINE_PASSWORD: u32 = 92;
const LINE_LOGIN: u32 = 93;
const NO_LINE_LOGIN: u32 = 94;
const BANNER: u32 = 95;
const NO_BANNER: u32 = 96;
const SHOW_START: u32 = 97;
const RELOAD: u32 = 98;
const ERASE_START: u32 = 99;
const IPCONFIG_DHCP: u32 = 110;
const IPCONFIG_RELEASE: u32 = 111;
const IPCONFIG_RENEW: u32 = 112;
const SERVICE_DHCP: u32 = 120;
const NO_SERVICE_DHCP: u32 = 121;
const EXCLUDE: u32 = 122;
const NO_EXCLUDE: u32 = 123;
const DHCP_POOL: u32 = 124;
const NO_DHCP_POOL: u32 = 125;
const POOL_NETWORK: u32 = 126;
const POOL_ROUTER: u32 = 127;
const POOL_DNS: u32 = 128;
const POOL_LEASE: u32 = 129;
const NO_POOL_NETWORK: u32 = 130;
const NO_POOL_ROUTER: u32 = 131;
const NO_POOL_DNS: u32 = 132;
const HELPER: u32 = 133;
const NO_HELPER: u32 = 134;
const SHOW_DHCP_BINDING: u32 = 135;
const SHOW_DHCP_POOL: u32 = 136;
const SHOW_DHCP_CONFLICT: u32 = 137;
const CLEAR_DHCP: u32 = 138;
const CLEAR_DHCP_ADDR: u32 = 139;
const TRACERT: u32 = 150;
const ROUTER_PING: u32 = 151;
const TRACEROUTE: u32 = 152;
const SW_MODE_AUTO: u32 = 180;
const SW_MODE_DESIRABLE: u32 = 181;
const SW_NONEGOTIATE: u32 = 182;
const NO_SW_NONEGOTIATE: u32 = 183;
const SHOW_SWITCHPORT: u32 = 184;
const ACL_PERMIT: u32 = 200;
const ACL_DENY: u32 = 201;
const ACL_REMARK: u32 = 202;
const ACL_NO_SEQ: u32 = 203;
const ACL_NO_PERMIT: u32 = 204;
const ACL_NO_DENY: u32 = 205;
const NO_ACL_NUMBER: u32 = 206;
const ACL_STD: u32 = 207;
const ACL_EXT: u32 = 208;
const NO_ACL_STD: u32 = 209;
const NO_ACL_EXT: u32 = 210;
const ACCESS_GROUP_IN: u32 = 211;
const NO_ACCESS_GROUP_IN: u32 = 212;
const NO_ACCESS_GROUP_ANY_IN: u32 = 213;
const ACCESS_GROUP_OUT: u32 = 220;
const NO_ACCESS_GROUP_OUT: u32 = 221;
const NO_ACCESS_GROUP_ANY_OUT: u32 = 222;
const ACCESS_CLASS: u32 = 214;
const NO_ACCESS_CLASS: u32 = 215;
const SHOW_ACL: u32 = 216;
const SHOW_ACL_ONE: u32 = 217;
const CLEAR_ACL: u32 = 218;
const CLEAR_ACL_ONE: u32 = 219;
const ROUTER_OSPF: u32 = 230;
const NO_ROUTER_OSPF: u32 = 231;
const OSPF_ROUTER_ID: u32 = 232;
const NO_OSPF_ROUTER_ID: u32 = 233;
const OSPF_NETWORK: u32 = 234;
const NO_OSPF_NETWORK: u32 = 235;
const OSPF_PASSIVE: u32 = 236;
const NO_OSPF_PASSIVE: u32 = 237;
const OSPF_REF_BW: u32 = 238;
const NO_OSPF_REF_BW: u32 = 239;
const IF_OSPF_COST: u32 = 240;
const IF_OSPF_PRIORITY: u32 = 241;
const IF_OSPF_HELLO: u32 = 242;
const IF_OSPF_DEAD: u32 = 243;
const IF_OSPF_P2P: u32 = 244;
const IF_OSPF_BROADCAST: u32 = 245;
const NO_IF_OSPF_COST: u32 = 246;
const NO_IF_OSPF_PRIORITY: u32 = 247;
const NO_IF_OSPF_HELLO: u32 = 248;
const NO_IF_OSPF_DEAD: u32 = 249;
const NO_IF_OSPF_NETWORK: u32 = 250;
const SHOW_OSPF: u32 = 251;
const SHOW_OSPF_NBR: u32 = 252;
const SHOW_OSPF_IF: u32 = 253;
const SHOW_OSPF_DB: u32 = 254;
const CLEAR_OSPF: u32 = 255;
const IF_MTU: u32 = 259;
const NO_IF_MTU: u32 = 260;
const IF_OSPF_MTU_IGNORE: u32 = 261;
const NO_IF_OSPF_MTU_IGNORE: u32 = 262;
const SHOW_PROTOCOLS: u32 = 263;
const OSPF_DEFAULT_INFO: u32 = 256;
const OSPF_DEFAULT_INFO_OPTS: u32 = 257;
const NO_OSPF_DEFAULT_INFO: u32 = 258;
const SPAN_MODE_PVST: u32 = 160;
const SPAN_MODE_RAPID: u32 = 161;
const NO_SPAN_MODE: u32 = 162;
const SPAN_PRIORITY: u32 = 163;
const NO_SPAN_PRIORITY: u32 = 164;
const SPAN_ROOT_PRIMARY: u32 = 165;
const SPAN_ROOT_SECONDARY: u32 = 166;
const SPAN_VLAN_ON: u32 = 167;
const SPAN_VLAN_OFF: u32 = 168;
const PORTFAST: u32 = 169;
const NO_PORTFAST: u32 = 170;
const SPAN_COST: u32 = 171;
const NO_SPAN_COST: u32 = 172;
const SPAN_PORT_PRIO: u32 = 173;
const NO_SPAN_PORT_PRIO: u32 = 174;
const SHOW_SPAN: u32 = 175;
const SHOW_SPAN_VLAN: u32 = 176;

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
    let config_line = g.mode(
        "config-line",
        "{host}(config-line)#",
        Some(config),
        Some(config),
    );
    let config_dhcp = g.mode(
        "config-dhcp",
        "{host}(dhcp-config)#",
        Some(config),
        Some(config),
    );
    let config_std_nacl = g.mode(
        "config-std-nacl",
        "{host}(config-std-nacl)#",
        Some(config),
        Some(config),
    );
    let config_ext_nacl = g.mode(
        "config-ext-nacl",
        "{host}(config-ext-nacl)#",
        Some(config),
        Some(config),
    );
    let config_router = g.mode(
        "config-router",
        "{host}(config-router)#",
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
        &[config],
        "hostname <name:word>",
        &["Set the device name", "New name"],
        h(HOSTNAME),
    );
    g.cmd(
        &[config],
        "interface <name:word>",
        &["Configure an interface", "Interface, e.g. g0/0 or g0/0.10"],
        h(INTERFACE),
    );
    g.cmd(
        &[config],
        "no interface <name:word>",
        &[
            "Negate",
            "Delete a subinterface",
            "Subinterface, e.g. g0/0.10",
        ],
        h(NO_INTERFACE),
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
        "encapsulation dot1q <vlan:int(1..4094)>",
        &["Set the encapsulation", "802.1Q tagging", "VLAN id"],
        h(ENCAP),
    );
    g.cmd(
        &[config_if],
        "encapsulation dot1q <vlan:int(1..4094)> native",
        &[
            "Set the encapsulation",
            "802.1Q tagging",
            "VLAN id",
            "Untagged frames belong here",
        ],
        h(ENCAP_NATIVE),
    );
    g.cmd(
        &[config_if],
        "no encapsulation dot1q",
        &["Negate", "Set the encapsulation", "802.1Q tagging"],
        h(NO_ENCAP),
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
    mgmt_commands(&mut g, privileged, config, config_line);
    dhcp_commands(&mut g, privileged, config, config_if, config_dhcp);
    acl_commands(
        &mut g,
        privileged,
        config,
        config_if,
        config_line,
        config_std_nacl,
        config_ext_nacl,
    );
    ospf_commands(&mut g, privileged, config, config_if, config_router);
    probe_commands(&mut g, privileged);
    disc::discovery_commands(&mut g, user, privileged, config, config_if);
    (
        g,
        Modes {
            user,
            privileged,
            config,
            config_if,
            config_vlan: config,
            config_line,
            config_dhcp,
            config_std_nacl,
            config_ext_nacl,
            config_router,
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
        "ipconfig dhcp",
        &["Addressing", "Get the address from a DHCP server"],
        h(IPCONFIG_DHCP),
    );
    g.cmd(
        &[m],
        "ipconfig release",
        &["Addressing", "Give the DHCP address back"],
        h(IPCONFIG_RELEASE),
    );
    g.cmd(
        &[m],
        "ipconfig renew",
        &["Addressing", "Renew the DHCP lease"],
        h(IPCONFIG_RENEW),
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
    g.cmd(
        &[m],
        "tracert <dst:ipv4>",
        &["Trace the route to a destination", "Destination"],
        h(TRACERT),
    );
    g.cmd(
        &[m],
        "tracert -h <max:int(1..255)> <dst:ipv4>",
        &[
            "Trace the route to a destination",
            "Maximum number of hops",
            "Hops",
            "Destination",
        ],
        h(TRACERT),
    );
    (
        g,
        Modes {
            user: m,
            privileged: m,
            config: m,
            config_if: m,
            config_vlan: m,
            config_line: m,
            config_dhcp: m,
            config_std_nacl: m,
            config_ext_nacl: m,
            config_router: m,
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
    let config_line = g.mode(
        "config-line",
        "{host}(config-line)#",
        Some(config),
        Some(config),
    );
    let config_dhcp = g.mode(
        "config-dhcp",
        "{host}(dhcp-config)#",
        Some(config),
        Some(config),
    );
    let config_std_nacl = g.mode(
        "config-std-nacl",
        "{host}(config-std-nacl)#",
        Some(config),
        Some(config),
    );
    let config_ext_nacl = g.mode(
        "config-ext-nacl",
        "{host}(config-ext-nacl)#",
        Some(config),
        Some(config),
    );
    let config_router = g.mode(
        "config-router",
        "{host}(config-router)#",
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
        "switchport mode dynamic auto",
        &[
            "Switch port settings",
            "Port mode",
            "Negotiate",
            "Trunk if the neighbour asks",
        ],
        h(SW_MODE_AUTO),
    );
    g.cmd(
        &[config_if],
        "switchport mode dynamic desirable",
        &[
            "Switch port settings",
            "Port mode",
            "Negotiate",
            "Ask the neighbour to trunk",
        ],
        h(SW_MODE_DESIRABLE),
    );
    g.cmd(
        &[config_if],
        "switchport nonegotiate",
        &["Switch port settings", "Send no trunk negotiation"],
        h(SW_NONEGOTIATE),
    );
    g.cmd(
        &[config_if],
        "no switchport nonegotiate",
        &["Negate", "Switch port settings", "Negotiate trunking again"],
        h(NO_SW_NONEGOTIATE),
    );
    g.cmd(
        &[user, privileged],
        "show interfaces <name:word> switchport",
        &[
            "Show",
            "Interface information",
            "Interface, e.g. fa0/1",
            "Switching settings",
        ],
        h(SHOW_SWITCHPORT),
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
        &[config],
        "interface vlan <id:int(1..4094)>",
        &["Configure an interface", "VLAN interface", "VLAN id"],
        h(SVI_ENTER),
    );
    g.cmd(
        &[config],
        "no interface vlan <id:int(1..4094)>",
        &["Negate", "Delete an interface", "VLAN interface", "VLAN id"],
        h(NO_SVI),
    );
    g.cmd(
        &[config],
        "ip routing",
        &["IP", "Route between VLAN interfaces"],
        h(IP_ROUTING),
    );
    g.cmd(
        &[config],
        "no ip routing",
        &["Negate", "IP", "Stop routing between VLAN interfaces"],
        h(NO_IP_ROUTING),
    );
    g.cmd(
        &[config],
        "ip default-gateway <gw:ipv4>",
        &["IP", "Gateway used while not routing", "Gateway address"],
        h(DEFAULT_GW),
    );
    g.cmd(
        &[config],
        "no ip default-gateway",
        &["Negate", "IP", "Gateway used while not routing"],
        h(NO_DEFAULT_GW),
    );
    g.cmd(
        &[config],
        "ip route <net:ipv4> <mask:mask> <via:ipv4>",
        &[
            "IP",
            "Static route",
            "Destination network",
            "Network mask",
            "Next hop",
        ],
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
    mgmt_commands(&mut g, privileged, config, config_line);
    dhcp_commands(&mut g, privileged, config, config_if, config_dhcp);
    acl_commands(
        &mut g,
        privileged,
        config,
        config_if,
        config_line,
        config_std_nacl,
        config_ext_nacl,
    );
    ospf_commands(&mut g, privileged, config, config_if, config_router);
    probe_commands(&mut g, privileged);
    span_commands(&mut g, user, privileged, config, config_if);
    m3b::port_security_commands(&mut g, user, privileged, config, config_if);
    etherchannel::channel_commands(&mut g, user, privileged, config, config_if);
    disc::discovery_commands(&mut g, user, privileged, config, config_if);
    (
        g,
        Modes {
            user,
            privileged,
            config,
            config_if,
            config_vlan,
            config_line,
            config_dhcp,
            config_std_nacl,
            config_ext_nacl,
            config_router,
        },
    )
}

impl Shell {
    /// Start OSPF's upkeep timer if it is not running.
    fn ospf_kick(&self, w: &mut World) {
        match self.kind {
            Kind::Switch => {
                w.with_device::<L2Switch, _>(self.device, |s, ctx| s.l3_mut().ospf_kick(ctx))
            }
            _ => w.with_device::<Router, _>(self.device, |r, ctx| r.ospf_kick(ctx)),
        }
    }

    /// The index of the interface a name like `g0/1`, `g0/1.10` or `Vlan10` stands for.
    fn l3_iface_by_name(&self, w: &World, name: &str) -> Option<usize> {
        match self.kind {
            Kind::Switch => parse_vlan_name(name).and_then(|v| self.l3(w).svi_index(v)),
            _ => {
                let n = w.device::<Router>(self.device).n_ports() as usize;
                let (p, sub) = parse_router_ifname(name, n)?;
                match sub {
                    None => Some(p),
                    Some(num) => self.l3(w).interfaces().iter().position(|i| {
                        i.sub
                            .is_some_and(|s| s.parent.0 as usize == p && s.number == num)
                    }),
                }
            }
        }
    }

    fn ospf_cmd(&mut self, w: &mut World, handler: u32, a: &Args, m: &Modes) -> String {
        let now = w.kernel.now();
        match handler {
            ROUTER_OSPF => {
                let pid = a.int("pid").unwrap() as u16;
                let ok = self.with_l3(w, |r| match &r.ospf {
                    Some(o) => o.process == pid,
                    None => {
                        r.ospf = Some(Ospf::new(pid));
                        true
                    }
                });
                if !ok {
                    return OSPF_ONE_PROCESS.to_string();
                }
                self.ospf_kick(w);
                self.session.enter(m.config_router, Some(pid.to_string()));
            }
            NO_ROUTER_OSPF => {
                let pid = a.int("pid").unwrap() as u16;
                let found = self.with_l3(w, |r| {
                    let found = r.ospf.as_ref().is_some_and(|o| o.process == pid);
                    if found {
                        r.remove_ospf();
                    }
                    found
                });
                if !found {
                    return OSPF_NOT_RUNNING.to_string();
                }
            }
            OSPF_ROUTER_ID | NO_OSPF_ROUTER_ID => {
                let id = a.ipv4("id").map(Ipv4Addr);
                self.with_l3(w, |r| {
                    if let Some(o) = &mut r.ospf {
                        o.router_id_cfg = id;
                        // A new router ID restarts the process.
                        o.reset();
                    }
                    r.ospf_config_changed();
                });
            }
            OSPF_NETWORK | NO_OSPF_NETWORK => {
                let Some(area) = parse_area(a.word("area").unwrap()) else {
                    return OSPF_BAD_AREA.to_string();
                };
                let stmt = OspfNetStmt {
                    net: Ipv4Addr(a.ipv4("net").unwrap()),
                    wild: Ipv4Addr(a.ipv4("wild").unwrap()),
                    area,
                };
                let r = self.with_l3(w, |r| {
                    let o = r.ospf.as_mut().ok_or(OSPF_NOT_RUNNING)?;
                    if handler == OSPF_NETWORK {
                        o.add_network(stmt);
                    } else {
                        let n = o.networks.len();
                        o.networks.retain(|x| *x != stmt);
                        if o.networks.len() == n {
                            return Err(OSPF_NO_NETWORK);
                        }
                    }
                    r.ospf_config_changed();
                    Ok(())
                });
                if let Err(e) = r {
                    return e.to_string();
                }
            }
            OSPF_PASSIVE | NO_OSPF_PASSIVE => {
                let name = a.word("name").unwrap();
                let Some(i) = self.l3_iface_by_name(w, name) else {
                    return format!("{UNKNOWN_INTERFACE} {name}");
                };
                let on = handler == OSPF_PASSIVE;
                self.with_l3(w, |r| {
                    r.set_ospf_cfg(i, |c| c.passive = on);
                    r.ospf_config_changed();
                });
            }
            OSPF_REF_BW | NO_OSPF_REF_BW => {
                let mbps = a
                    .int("mbps")
                    .map_or(netdev::OSPF_DEFAULT_REF_BW, |v| v as u32);
                self.with_l3(w, |r| {
                    if let Some(o) = &mut r.ospf {
                        o.ref_bw = mbps;
                    }
                    r.ospf_config_changed();
                });
            }
            IF_OSPF_COST..=NO_IF_OSPF_NETWORK | IF_MTU..=NO_IF_OSPF_MTU_IGNORE => {
                let Some(i) = self.current_iface(w) else {
                    return NO_SUCH_INTERFACE.to_string();
                };
                let n = a.int("n");
                self.with_l3(w, |r| {
                    r.set_ospf_cfg(i, |c| match handler {
                        IF_OSPF_COST => c.cost = n.map(|v| v as u32),
                        NO_IF_OSPF_COST => c.cost = None,
                        IF_OSPF_PRIORITY => c.priority = n.unwrap_or(1) as u8,
                        NO_IF_OSPF_PRIORITY => c.priority = 1,
                        IF_OSPF_HELLO => c.hello = n.map(|v| v as u16),
                        NO_IF_OSPF_HELLO => c.hello = None,
                        IF_OSPF_DEAD => c.dead = n.map(|v| v as u32),
                        NO_IF_OSPF_DEAD => c.dead = None,
                        IF_OSPF_P2P => c.point_to_point = true,
                        IF_MTU => c.mtu = n.map(|v| v as u16),
                        NO_IF_MTU => c.mtu = None,
                        IF_OSPF_MTU_IGNORE => c.mtu_ignore = true,
                        NO_IF_OSPF_MTU_IGNORE => c.mtu_ignore = false,
                        _ => c.point_to_point = false,
                    });
                    r.ospf_config_changed();
                });
            }
            SHOW_OSPF => return show_ospf(self.l3(w)),
            SHOW_PROTOCOLS => return show_protocols(self.l3(w)),
            SHOW_OSPF_NBR => return show_ospf_neighbors(self.l3(w), now),
            SHOW_OSPF_IF => return show_ospf_interfaces(self.l3(w)),
            SHOW_OSPF_DB => return show_ospf_database(self.l3(w), now),
            OSPF_DEFAULT_INFO | OSPF_DEFAULT_INFO_OPTS | NO_OSPF_DEFAULT_INFO => {
                let info = if handler == NO_OSPF_DEFAULT_INFO {
                    None
                } else {
                    let mut d = netdev::OspfDefaultInfo::default();
                    let opts = a.word("opts").unwrap_or("");
                    let mut it = opts.split_whitespace();
                    while let Some(t) = it.next() {
                        match t {
                            "always" => d.always = true,
                            "metric" => match it.next().and_then(|v| v.parse::<u32>().ok()) {
                                Some(v) if v < 0xff_ffff => d.metric = v,
                                _ => return OSPF_BAD_OPTION.to_string(),
                            },
                            "metric-type" => match it.next() {
                                Some("1") => d.e2 = false,
                                Some("2") => d.e2 = true,
                                _ => return OSPF_BAD_OPTION.to_string(),
                            },
                            _ => return OSPF_BAD_OPTION.to_string(),
                        }
                    }
                    Some(d)
                };
                let r: Result<(), &str> = self.with_l3(w, |r| {
                    let o = r.ospf.as_mut().ok_or(OSPF_NOT_RUNNING)?;
                    o.default_info = info;
                    r.ospf_config_changed();
                    Ok(())
                });
                if let Err(e) = r {
                    return e.to_string();
                }
            }
            CLEAR_OSPF => {
                self.with_l3(w, |r| {
                    if let Some(o) = &mut r.ospf {
                        o.reset();
                    }
                    r.ospf_config_changed();
                });
            }
            _ => {}
        }
        String::new()
    }
}

/// `1` or `0.0.0.1`: an OSPF area.
fn parse_area(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut v = 0u32;
    for p in parts {
        v = v << 8 | p.parse::<u8>().ok()? as u32;
    }
    Some(v)
}

fn ospf_config(r: &Router) -> String {
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let mut s = format!("router ospf {}\n", o.process);
    if let Some(id) = o.router_id_cfg {
        s += &format!(" router-id {id}\n");
    }
    if o.ref_bw != netdev::OSPF_DEFAULT_REF_BW {
        s += &format!(" auto-cost reference-bandwidth {}\n", o.ref_bw);
    }
    for n in &o.networks {
        s += &format!(" network {} {} area {}\n", n.net, n.wild, n.area);
    }
    if let Some(d) = o.default_info {
        s += " default-information originate";
        if d.always {
            s += " always";
        }
        if d.metric != 1 {
            s += &format!(" metric {}", d.metric);
        }
        if !d.e2 {
            s += " metric-type 1";
        }
        s += "\n";
    }
    for (i, ifc) in r.interfaces().iter().enumerate() {
        if ifc.ospf.passive {
            s += &format!(" passive-interface {}\n", iface_name(r, i));
        }
    }
    s + "!\n"
}

fn ospf_if_lines(c: &netdev::OspfIfCfg) -> String {
    let mut s = String::new();
    if let Some(v) = c.cost {
        s += &format!(" ip ospf cost {v}\n");
    }
    if c.priority != 1 {
        s += &format!(" ip ospf priority {}\n", c.priority);
    }
    if let Some(v) = c.hello {
        s += &format!(" ip ospf hello-interval {v}\n");
    }
    if let Some(v) = c.dead {
        s += &format!(" ip ospf dead-interval {v}\n");
    }
    if c.point_to_point {
        s += " ip ospf network point-to-point\n";
    }
    if let Some(v) = c.mtu {
        s += &format!(" ip mtu {v}\n");
    }
    if c.mtu_ignore {
        s += " ip ospf mtu-ignore\n";
    }
    s
}

fn show_ospf(r: &Router) -> String {
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let rid = o
        .router_id()
        .map_or("0.0.0.0".to_string(), |i| i.to_string());
    let mut s = format!(" Routing Process \"ospf {}\" with ID {rid}", o.process);
    if o.is_asbr() {
        s += "\n It is an autonomous system boundary router";
    }
    if o.is_abr() {
        s += "\n It is an area border router";
    }
    s += &format!("\n Reference bandwidth unit is {} mbps", o.ref_bw);
    let areas = o.areas();
    s += &format!(
        "\n Number of areas in this router is {}. {} normal 0 stub 0 nssa",
        areas.len(),
        areas.len()
    );
    let db = o.database(SimTime::ZERO);
    for a in areas {
        let n = o.interfaces().filter(|f| f.area == a).count();
        let lsas = db.iter().filter(|(x, _, _)| *x == Some(a)).count();
        s += &format!(
            "\n    Area {}\n        Number of interfaces in this area is {n}\n        Number of LSA {lsas}",
            if a == 0 { "BACKBONE(0)".to_string() } else { a.to_string() }
        );
    }
    s
}

fn show_protocols(r: &Router) -> String {
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let rid = o
        .router_id()
        .map_or("0.0.0.0".to_string(), |i| i.to_string());
    let areas = o.areas();
    let mut s = format!(
        "Routing Protocol is \"ospf {}\"\n  Router ID {rid}\n  Number of areas in this router is {}. {} normal 0 stub 0 nssa",
        o.process,
        areas.len(),
        areas.len()
    );
    if o.is_asbr() {
        s += "\n  It is an autonomous system boundary router";
    }
    if o.is_abr() {
        s += "\n  It is an area border router";
    }
    s += "\n  Routing for Networks:";
    for n in &o.networks {
        s += &format!("\n    {} {} area {}", n.net, n.wild, n.area);
    }
    let passive: Vec<String> = r
        .interfaces()
        .iter()
        .enumerate()
        .filter(|(_, ifc)| ifc.ospf.passive)
        .map(|(i, _)| iface_name(r, i))
        .collect();
    if !passive.is_empty() {
        s += "\n  Passive Interface(s):";
        for p in passive {
            s += &format!("\n    {p}");
        }
    }
    s + "\n  Distance: (default is 110)"
}

fn show_ospf_neighbors(r: &Router, now: SimTime) -> String {
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let mut s = format!(
        "{:<16}{:<6}{:<16}{:<12}{:<16}{}",
        "Neighbor ID", "Pri", "State", "Dead Time", "Address", "Interface"
    );
    for f in o.interfaces() {
        for n in f.nbrs.values() {
            let role = if f.cfg.point_to_point {
                "-"
            } else if n.ip == f.dr {
                "DR"
            } else if n.ip == f.bdr {
                "BDR"
            } else {
                "DROTHER"
            };
            s += &format!(
                "\n{:<16}{:<6}{:<16}{:<12}{:<16}{}",
                n.id.to_string(),
                n.priority,
                format!("{}/{}", n.state.name(), role),
                hms(n.dead_in(now)),
                n.ip.to_string(),
                iface_name(r, f.index)
            );
        }
    }
    s
}

fn show_ospf_interfaces(r: &Router) -> String {
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let rid = o.router_id().unwrap_or(Ipv4Addr::UNSPECIFIED);
    let mut blocks = Vec::new();
    for f in o.interfaces() {
        let who = |ip: Ipv4Addr| -> Ipv4Addr {
            if ip == f.addr {
                rid
            } else {
                f.nbrs
                    .values()
                    .find(|n| n.ip == ip)
                    .map_or(Ipv4Addr::UNSPECIFIED, |n| n.id)
            }
        };
        let p2p = f.cfg.point_to_point;
        let mut s = format!(
            "{} is up, line protocol is up\n  Internet Address {}/{}, Area {}\n  Process ID {}, Router ID {rid}, Network Type {}, Cost: {}\n  Transmit Delay is 1 sec, State {}, Priority {}",
            iface_name(r, f.index),
            f.addr,
            f.len,
            f.area,
            o.process,
            if p2p { "POINT_TO_POINT" } else { "BROADCAST" },
            o.cost_of(&f.cfg),
            if f.cfg.passive { "DR" } else { f.state.name() },
            f.cfg.priority,
        );
        if f.cfg.passive {
            s += "\n  No Hellos (Passive interface)";
        } else {
            if f.dr != Ipv4Addr::UNSPECIFIED {
                s += &format!(
                    "\n  Designated Router (ID) {}, Interface address {}",
                    who(f.dr),
                    f.dr
                );
            }
            if f.bdr != Ipv4Addr::UNSPECIFIED {
                s += &format!(
                    "\n  Backup Designated router (ID) {}, Interface address {}",
                    who(f.bdr),
                    f.bdr
                );
            }
            let dead = f.cfg.dead_interval();
            s += &format!(
                "\n  Timer intervals configured, Hello {}, Dead {dead}, Wait {dead}, Retransmit 5",
                f.cfg.hello_interval()
            );
            let adj = f
                .nbrs
                .values()
                .filter(|n| n.state == OspfNbrState::Full)
                .count();
            s += &format!(
                "\n  Neighbor Count is {}, Adjacent neighbor count is {adj}",
                f.nbrs.len()
            );
        }
        blocks.push(s);
    }
    blocks.join("\n")
}

fn show_ospf_database(r: &Router, now: SimTime) -> String {
    use wire::{LSA_ASBR_SUMMARY, LSA_EXTERNAL, LSA_NETWORK, LSA_ROUTER, LSA_SUMMARY};
    let Some(o) = &r.ospf else {
        return String::new();
    };
    let rid = o
        .router_id()
        .map_or("0.0.0.0".to_string(), |i| i.to_string());
    let db = o.database(now);
    let mut s = format!(
        "            OSPF Router with ID ({rid}) (Process ID {})",
        o.process
    );
    let row = |h: &wire::LsaHeader, tail: String| {
        format!(
            "\n{:<16}{:<16}{:<12}0x{:08X} 0x{:04X}{tail}",
            h.id.to_string(),
            h.adv_router.to_string(),
            h.age,
            h.seq as u32,
            h.checksum
        )
    };
    let mut areas: Vec<u32> = db.iter().filter_map(|(a, _, _)| *a).collect();
    areas.sort();
    areas.dedup();
    for a in areas {
        for (ty, title, tail_head) in [
            (LSA_ROUTER, "Router Link States", "Link count"),
            (LSA_NETWORK, "Net Link States", ""),
            (LSA_SUMMARY, "Summary Net Link States", ""),
            (LSA_ASBR_SUMMARY, "Summary ASB Link States", ""),
        ] {
            let rows: Vec<_> = db
                .iter()
                .filter(|(x, h, _)| *x == Some(a) && h.ls_type == ty)
                .collect();
            if rows.is_empty() {
                continue;
            }
            s += &format!("\n\n                {title} (Area {a})\n\n");
            s += format!(
                "{:<16}{:<16}{:<12}{:<11}{:<9}{tail_head}",
                "Link ID", "ADV Router", "Age", "Seq#", "Checksum"
            )
            .trim_end();
            for (_, h, b) in rows {
                let tail = match b {
                    wire::LsaBody::Router { links, .. } => format!("   {}", links.len()),
                    _ => String::new(),
                };
                s += &row(h, tail);
            }
        }
    }
    let ext: Vec<_> = db
        .iter()
        .filter(|(x, h, _)| x.is_none() && h.ls_type == LSA_EXTERNAL)
        .collect();
    if !ext.is_empty() {
        s += "\n\n                Type-5 AS External Link States\n\n";
        s += &format!(
            "{:<16}{:<16}{:<12}{:<11}{:<9}{}",
            "Link ID", "ADV Router", "Age", "Seq#", "Checksum", "Tag"
        );
        for (_, h, b) in ext {
            let tag = match b {
                wire::LsaBody::External { tag, .. } => *tag,
                _ => 0,
            };
            s += &row(h, format!("   {tag}"));
        }
    }
    s
}

/// `ping` and `traceroute` on routers and layer 3 switches.
fn probe_commands(g: &mut Grammar, privileged: ModeId) {
    let h = Action::Handler;
    g.cmd(
        &[privileged],
        "ping <dst:ipv4>",
        &["Send echo requests", "Destination"],
        h(ROUTER_PING),
    );
    g.cmd(
        &[privileged],
        "traceroute <dst:ipv4>",
        &["Trace the route to a destination", "Destination"],
        h(TRACEROUTE),
    );
    g.cmd(
        &[privileged],
        "traceroute <dst:ipv4> ttl <min:int(1..255)> <max:int(1..255)>",
        &[
            "Trace the route to a destination",
            "Destination",
            "Set the TTL range",
            "First TTL",
            "Last TTL",
        ],
        h(TRACEROUTE),
    );
}

/// Spanning tree: mode, per-VLAN bridge priority, port settings and `show spanning-tree`.
fn span_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
) {
    let h = Action::Handler;
    let st = "Spanning tree subsystem";
    g.cmd(
        &[config],
        "spanning-tree mode pvst",
        &[st, "Protocol mode", "Per-VLAN spanning tree (802.1D)"],
        h(SPAN_MODE_PVST),
    );
    g.cmd(
        &[config],
        "spanning-tree mode rapid-pvst",
        &[st, "Protocol mode", "Per-VLAN rapid spanning tree (802.1w)"],
        h(SPAN_MODE_RAPID),
    );
    g.cmd(
        &[config],
        "no spanning-tree mode",
        &["Negate", st, "Turn spanning tree off"],
        h(NO_SPAN_MODE),
    );
    let vl = "VLAN list, e.g. 10,20-30";
    g.cmd(
        &[config],
        "spanning-tree vlan <list:word>",
        &[st, "VLANs", vl],
        h(SPAN_VLAN_ON),
    );
    g.cmd(
        &[config],
        "no spanning-tree vlan <list:word>",
        &["Negate", st, "VLANs", vl],
        h(SPAN_VLAN_OFF),
    );
    g.cmd(
        &[config],
        "spanning-tree vlan <list:word> priority <prio:int(0..61440)>",
        &[
            st,
            "VLANs",
            vl,
            "Bridge priority",
            "Priority, a multiple of 4096",
        ],
        h(SPAN_PRIORITY),
    );
    g.cmd(
        &[config],
        "no spanning-tree vlan <list:word> priority",
        &["Negate", st, "VLANs", vl, "Back to the default priority"],
        h(NO_SPAN_PRIORITY),
    );
    g.cmd(
        &[config],
        "spanning-tree vlan <list:word> root primary",
        &[st, "VLANs", vl, "Make this the root", "Win the election"],
        h(SPAN_ROOT_PRIMARY),
    );
    g.cmd(
        &[config],
        "spanning-tree vlan <list:word> root secondary",
        &[st, "VLANs", vl, "Make this the root", "Be the next best"],
        h(SPAN_ROOT_SECONDARY),
    );
    g.cmd(
        &[config_if],
        "spanning-tree portfast",
        &[st, "Forward at once (a port facing a host)"],
        h(PORTFAST),
    );
    g.cmd(
        &[config_if],
        "no spanning-tree portfast",
        &["Negate", st, "Take part in the election"],
        h(NO_PORTFAST),
    );
    g.cmd(
        &[config_if],
        "spanning-tree cost <cost:int(1..200000000)>",
        &[st, "Path cost", "Cost"],
        h(SPAN_COST),
    );
    g.cmd(
        &[config_if],
        "no spanning-tree cost",
        &["Negate", st, "Back to the default cost"],
        h(NO_SPAN_COST),
    );
    g.cmd(
        &[config_if],
        "spanning-tree port-priority <prio:int(0..240)>",
        &[st, "Port priority", "Priority, a multiple of 16"],
        h(SPAN_PORT_PRIO),
    );
    g.cmd(
        &[config_if],
        "no spanning-tree port-priority",
        &["Negate", st, "Back to the default priority"],
        h(NO_SPAN_PORT_PRIO),
    );
    g.cmd(
        &[user, privileged],
        "show spanning-tree",
        &["Show", "Spanning tree information"],
        h(SHOW_SPAN),
    );
    g.cmd(
        &[user, privileged],
        "show spanning-tree vlan <id:int(1..4094)>",
        &["Show", "Spanning tree information", "One VLAN", "VLAN id"],
        h(SHOW_SPAN_VLAN),
    );
}

/// DHCP server configuration, relay and show commands, the same on routers and switches.
fn dhcp_commands(
    g: &mut Grammar,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
    pool: ModeId,
) {
    let h = Action::Handler;
    for (pattern, help, id) in [
        (
            "service dhcp",
            &["Service settings", "The DHCP server and relay"][..],
            SERVICE_DHCP,
        ),
        (
            "no service dhcp",
            &[
                "Negate",
                "Service settings",
                "Turn the DHCP server and relay off",
            ][..],
            NO_SERVICE_DHCP,
        ),
        (
            "ip dhcp excluded-address <low:ipv4>",
            &["IP", "DHCP server", "Never hand out", "Address"][..],
            EXCLUDE,
        ),
        (
            "ip dhcp excluded-address <low:ipv4> <high:ipv4>",
            &[
                "IP",
                "DHCP server",
                "Never hand out",
                "First address",
                "Last address",
            ][..],
            EXCLUDE,
        ),
        (
            "no ip dhcp excluded-address <low:ipv4>",
            &["Negate", "IP", "DHCP server", "Never hand out", "Address"][..],
            NO_EXCLUDE,
        ),
        (
            "no ip dhcp excluded-address <low:ipv4> <high:ipv4>",
            &[
                "Negate",
                "IP",
                "DHCP server",
                "Never hand out",
                "First address",
                "Last address",
            ][..],
            NO_EXCLUDE,
        ),
        (
            "ip dhcp pool <name:word>",
            &[
                "IP",
                "DHCP server",
                "Configure an address pool",
                "Pool name",
            ][..],
            DHCP_POOL,
        ),
        (
            "no ip dhcp pool <name:word>",
            &[
                "Negate",
                "IP",
                "DHCP server",
                "Delete an address pool",
                "Pool name",
            ][..],
            NO_DHCP_POOL,
        ),
        (
            "ip helper-address <server:ipv4>",
            &["IP", "Forward DHCP requests", "Server address"][..],
            HELPER,
        ),
        (
            "no ip helper-address <server:ipv4>",
            &[
                "Negate",
                "IP",
                "Stop forwarding DHCP requests",
                "Server address",
            ][..],
            NO_HELPER,
        ),
    ] {
        let modes: &[ModeId] = if id == HELPER || id == NO_HELPER {
            &[config_if]
        } else {
            &[config]
        };
        g.cmd(modes, pattern, help, h(id));
    }
    for (pattern, help, id) in [
        (
            "network <net:ipv4> <mask:mask>",
            &["Addresses to hand out", "Network", "Network mask"][..],
            POOL_NETWORK,
        ),
        (
            "default-router <a:ipv4>",
            &["Gateway for clients", "Address"][..],
            POOL_ROUTER,
        ),
        (
            "dns-server <a:ipv4>",
            &["DNS servers for clients", "Address"][..],
            POOL_DNS,
        ),
        (
            "dns-server <a:ipv4> <b:ipv4>",
            &["DNS servers for clients", "Address", "Address"][..],
            POOL_DNS,
        ),
        (
            "dns-server <a:ipv4> <b:ipv4> <c:ipv4>",
            &["DNS servers for clients", "Address", "Address", "Address"][..],
            POOL_DNS,
        ),
        (
            "lease <days:int(0..365)>",
            &["Lease time", "Days"][..],
            POOL_LEASE,
        ),
        (
            "lease <days:int(0..365)> <hours:int(0..23)>",
            &["Lease time", "Days", "Hours"][..],
            POOL_LEASE,
        ),
        (
            "lease <days:int(0..365)> <hours:int(0..23)> <minutes:int(0..59)>",
            &["Lease time", "Days", "Hours", "Minutes"][..],
            POOL_LEASE,
        ),
        (
            "no network",
            &["Negate", "Addresses to hand out"][..],
            NO_POOL_NETWORK,
        ),
        (
            "no default-router",
            &["Negate", "Gateway for clients"][..],
            NO_POOL_ROUTER,
        ),
        (
            "no dns-server",
            &["Negate", "DNS servers for clients"][..],
            NO_POOL_DNS,
        ),
    ] {
        g.cmd(&[pool], pattern, help, h(id));
    }
    g.cmd(
        &[config, pool],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    g.cmd(&[pool], "exit", &["Up one level"], Action::Exit);
    g.cmd(
        &[privileged],
        "show ip dhcp binding",
        &["Show", "IP information", "DHCP server", "Leases"][..],
        h(SHOW_DHCP_BINDING),
    );
    g.cmd(
        &[privileged],
        "show ip dhcp pool",
        &["Show", "IP information", "DHCP server", "Address pools"][..],
        h(SHOW_DHCP_POOL),
    );
    g.cmd(
        &[privileged],
        "show ip dhcp conflict",
        &[
            "Show",
            "IP information",
            "DHCP server",
            "Addresses clients reported in use",
        ][..],
        h(SHOW_DHCP_CONFLICT),
    );
    g.cmd(
        &[privileged],
        "clear ip dhcp binding *",
        &[
            "Clear",
            "IP information",
            "DHCP server",
            "Leases",
            "All of them",
        ][..],
        h(CLEAR_DHCP),
    );
    g.cmd(
        &[privileged],
        "clear ip dhcp binding <addr:ipv4>",
        &[
            "Clear",
            "IP information",
            "DHCP server",
            "Leases",
            "Address",
        ][..],
        h(CLEAR_DHCP_ADDR),
    );
}

impl Shell {
    /// Add or remove a line of an access list: `access-list N permit ...` in global
    /// configuration, or `permit ...` inside a named list.
    fn acl_entry_cmd(&mut self, w: &mut World, handler: u32, a: &Args, m: &Modes) -> String {
        let (name, kind) = match a.int("num") {
            Some(n) => {
                let Some(kind) = acl_kind_of_number(n as u32) else {
                    return BAD_ACL_NUMBER.to_string();
                };
                if handler == NO_ACL_NUMBER {
                    if !self.with_l3(w, |r| r.acls.remove(&n.to_string())) {
                        return NO_SUCH_ACL.to_string();
                    }
                    return String::new();
                }
                (n.to_string(), kind)
            }
            None => {
                let Some(name) = self.session.context().map(str::to_string) else {
                    return NO_SUCH_ACL.to_string();
                };
                let kind = if self.session.mode() == m.config_std_nacl {
                    AclKind::Standard
                } else {
                    AclKind::Extended
                };
                (name, kind)
            }
        };
        let seq = a.int("seq").map(|s| s as u32);
        if handler == ACL_NO_SEQ {
            let seq = seq.expect("parsed");
            return if self.with_l3(w, |r| {
                r.acls.get_mut(&name).is_some_and(|l| l.remove_seq(seq))
            }) {
                String::new()
            } else {
                NO_SUCH_SEQ.to_string()
            };
        }
        let rest = a.word("rest").unwrap_or("").trim().to_string();
        let body = if handler == ACL_REMARK {
            AclBody::Remark(rest)
        } else {
            let action = if matches!(handler, ACL_PERMIT | ACL_NO_PERMIT) {
                "permit"
            } else {
                "deny"
            };
            let mut toks = vec![action];
            toks.extend(rest.split_whitespace());
            match parse_acl_rule(kind, &toks) {
                Ok(r) => AclBody::Rule(r),
                Err(e) => return e.0,
            }
        };
        if matches!(handler, ACL_NO_PERMIT | ACL_NO_DENY) {
            return if self.with_l3(w, |r| {
                r.acls.get_mut(&name).is_some_and(|l| l.remove_body(&body))
            }) {
                String::new()
            } else {
                NO_SUCH_ACE.to_string()
            };
        }
        self.with_l3(w, |r| match r.acls.open(&name, kind) {
            Err(_) => ACL_KIND_CLASH.to_string(),
            Ok(acl) => match acl.add(seq, body) {
                Ok(_) => String::new(),
                Err(AclAddError::DuplicateSeq) => DUP_SEQ.to_string(),
                Err(AclAddError::SeqOverflow) => SEQ_OVERFLOW.to_string(),
            },
        })
    }
}

/// Access list commands, the same on routers and layer 3 switches.
fn acl_commands(
    g: &mut Grammar,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
    line: ModeId,
    std: ModeId,
    ext: ModeId,
) {
    let h = Action::Handler;
    const SEQ: &str = "int(1..2147483647)";
    for (pattern, help, id) in [
        (
            "access-list <num:int(1..2699)> permit <rest:rest>".to_string(),
            vec![
                "Add an access list entry",
                "List number",
                "Allow matching packets",
                "Conditions",
            ],
            ACL_PERMIT,
        ),
        (
            "access-list <num:int(1..2699)> deny <rest:rest>".to_string(),
            vec![
                "Add an access list entry",
                "List number",
                "Drop matching packets",
                "Conditions",
            ],
            ACL_DENY,
        ),
        (
            "access-list <num:int(1..2699)> remark <rest:rest>".to_string(),
            vec![
                "Add an access list entry",
                "List number",
                "A comment",
                "Text",
            ],
            ACL_REMARK,
        ),
        (
            "no access-list <num:int(1..2699)>".to_string(),
            vec!["Negate", "Delete an access list", "List number"],
            NO_ACL_NUMBER,
        ),
        (
            "ip access-list standard <name:word>".to_string(),
            vec![
                "IP",
                "Named access list",
                "Standard (source address only)",
                "List name or number",
            ],
            ACL_STD,
        ),
        (
            "ip access-list extended <name:word>".to_string(),
            vec![
                "IP",
                "Named access list",
                "Extended (protocol, addresses, ports)",
                "List name or number",
            ],
            ACL_EXT,
        ),
        (
            "no ip access-list standard <name:word>".to_string(),
            vec![
                "Negate",
                "IP",
                "Named access list",
                "Standard",
                "List name or number",
            ],
            NO_ACL_STD,
        ),
        (
            "no ip access-list extended <name:word>".to_string(),
            vec![
                "Negate",
                "IP",
                "Named access list",
                "Extended",
                "List name or number",
            ],
            NO_ACL_EXT,
        ),
    ] {
        g.cmd(&[config], &pattern, &help, h(id));
    }
    for mode in [std, ext] {
        for (pattern, help, id) in [
            (
                "permit <rest:rest>".to_string(),
                vec!["Allow matching packets", "Conditions"],
                ACL_PERMIT,
            ),
            (
                "deny <rest:rest>".to_string(),
                vec!["Drop matching packets", "Conditions"],
                ACL_DENY,
            ),
            (
                "remark <rest:rest>".to_string(),
                vec!["A comment", "Text"],
                ACL_REMARK,
            ),
            (
                format!("<seq:{SEQ}> permit <rest:rest>"),
                vec!["Sequence number", "Allow matching packets", "Conditions"],
                ACL_PERMIT,
            ),
            (
                format!("<seq:{SEQ}> deny <rest:rest>"),
                vec!["Sequence number", "Drop matching packets", "Conditions"],
                ACL_DENY,
            ),
            (
                format!("<seq:{SEQ}> remark <rest:rest>"),
                vec!["Sequence number", "A comment", "Text"],
                ACL_REMARK,
            ),
            (
                format!("no <seq:{SEQ}>"),
                vec!["Negate", "Sequence number of the entry"],
                ACL_NO_SEQ,
            ),
            (
                "no permit <rest:rest>".to_string(),
                vec!["Negate", "Allow matching packets", "Conditions"],
                ACL_NO_PERMIT,
            ),
            (
                "no deny <rest:rest>".to_string(),
                vec!["Negate", "Drop matching packets", "Conditions"],
                ACL_NO_DENY,
            ),
        ] {
            g.cmd(&[mode], &pattern, &help, h(id));
        }
        g.cmd(&[mode], "exit", &["Up one level"], Action::Exit);
    }
    g.cmd(
        &[config, std, ext],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    for (pattern, help, id) in [
        (
            "ip access-group <acl:word> in",
            &[
                "IP",
                "Apply an access list",
                "List name or number",
                "To arriving packets",
            ][..],
            ACCESS_GROUP_IN,
        ),
        (
            "ip access-group <acl:word> out",
            &[
                "IP",
                "Apply an access list",
                "List name or number",
                "To forwarded packets",
            ][..],
            ACCESS_GROUP_OUT,
        ),
        (
            "no ip access-group <acl:word> in",
            &[
                "Negate",
                "IP",
                "Apply an access list",
                "List name or number",
                "To arriving packets",
            ][..],
            NO_ACCESS_GROUP_IN,
        ),
        (
            "no ip access-group <acl:word> out",
            &[
                "Negate",
                "IP",
                "Apply an access list",
                "List name or number",
                "To forwarded packets",
            ][..],
            NO_ACCESS_GROUP_OUT,
        ),
        (
            "no ip access-group in",
            &[
                "Negate",
                "IP",
                "Apply an access list",
                "To arriving packets",
            ][..],
            NO_ACCESS_GROUP_ANY_IN,
        ),
        (
            "no ip access-group out",
            &[
                "Negate",
                "IP",
                "Apply an access list",
                "To forwarded packets",
            ][..],
            NO_ACCESS_GROUP_ANY_OUT,
        ),
    ] {
        g.cmd(&[config_if], pattern, help, h(id));
    }
    g.cmd(
        &[line],
        "access-class <acl:word> in",
        &[
            "Restrict access by an access list",
            "List name or number",
            "Incoming connections",
        ],
        h(ACCESS_CLASS),
    );
    g.cmd(
        &[line],
        "no access-class <acl:word> in",
        &[
            "Negate",
            "Restrict access by an access list",
            "List name or number",
            "Incoming connections",
        ],
        h(NO_ACCESS_CLASS),
    );
    g.cmd(
        &[privileged],
        "show access-lists",
        &["Show", "Access lists"],
        h(SHOW_ACL),
    );
    g.cmd(
        &[privileged],
        "show access-lists <name:word>",
        &["Show", "Access lists", "List name or number"],
        h(SHOW_ACL_ONE),
    );
    g.cmd(
        &[privileged],
        "show ip access-lists",
        &["Show", "IP information", "Access lists"],
        h(SHOW_ACL),
    );
    g.cmd(
        &[privileged],
        "show ip access-lists <name:word>",
        &[
            "Show",
            "IP information",
            "Access lists",
            "List name or number",
        ],
        h(SHOW_ACL_ONE),
    );
    g.cmd(
        &[privileged],
        "clear access-list counters",
        &["Clear", "Access lists", "Match counters"],
        h(CLEAR_ACL),
    );
    g.cmd(
        &[privileged],
        "clear access-list counters <name:word>",
        &[
            "Clear",
            "Access lists",
            "Match counters",
            "List name or number",
        ],
        h(CLEAR_ACL_ONE),
    );
}

/// OSPF commands, the same on routers and layer 3 switches.
fn ospf_commands(
    g: &mut Grammar,
    privileged: ModeId,
    config: ModeId,
    config_if: ModeId,
    router: ModeId,
) {
    let h = Action::Handler;
    for (pattern, help, id) in [
        (
            "router ospf <pid:int(1..65535)>",
            &[
                "Routing protocol",
                "Open Shortest Path First",
                "Process number",
            ][..],
            ROUTER_OSPF,
        ),
        (
            "no router ospf <pid:int(1..65535)>",
            &[
                "Negate",
                "Routing protocol",
                "Open Shortest Path First",
                "Process number",
            ][..],
            NO_ROUTER_OSPF,
        ),
    ] {
        g.cmd(&[config], pattern, help, h(id));
    }
    for (pattern, help, id) in [
        (
            "router-id <id:ipv4>",
            &["Set the router ID", "Router ID"][..],
            OSPF_ROUTER_ID,
        ),
        (
            "no router-id",
            &["Negate", "Use the highest interface address"][..],
            NO_OSPF_ROUTER_ID,
        ),
        (
            "network <net:ipv4> <wild:ipv4> area <area:word>",
            &[
                "Enable OSPF on interfaces",
                "Address",
                "Wildcard bits",
                "Set the area",
                "Area number or address",
            ][..],
            OSPF_NETWORK,
        ),
        (
            "no network <net:ipv4> <wild:ipv4> area <area:word>",
            &[
                "Negate",
                "Enable OSPF on interfaces",
                "Address",
                "Wildcard bits",
                "Set the area",
                "Area number or address",
            ][..],
            NO_OSPF_NETWORK,
        ),
        (
            "passive-interface <name:word>",
            &["Send no hellos and form no neighbours", "Interface"][..],
            OSPF_PASSIVE,
        ),
        (
            "no passive-interface <name:word>",
            &[
                "Negate",
                "Send no hellos and form no neighbours",
                "Interface",
            ][..],
            NO_OSPF_PASSIVE,
        ),
        (
            "auto-cost reference-bandwidth <mbps:int(1..4294967)>",
            &[
                "Interface cost",
                "Reference bandwidth",
                "Megabits per second",
            ][..],
            OSPF_REF_BW,
        ),
        (
            "no auto-cost reference-bandwidth",
            &["Negate", "Interface cost", "Reference bandwidth"][..],
            NO_OSPF_REF_BW,
        ),
        (
            "default-information originate",
            &[
                "Control distribution of default information",
                "Advertise a default route",
            ][..],
            OSPF_DEFAULT_INFO,
        ),
        (
            "default-information originate <opts:rest>",
            &[
                "Control distribution of default information",
                "Advertise a default route",
                "always, metric <n>, metric-type <1|2>",
            ][..],
            OSPF_DEFAULT_INFO_OPTS,
        ),
        (
            "no default-information originate",
            &[
                "Negate",
                "Control distribution of default information",
                "Advertise a default route",
            ][..],
            NO_OSPF_DEFAULT_INFO,
        ),
    ] {
        g.cmd(&[router], pattern, help, h(id));
    }
    g.cmd(
        &[config, router],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    g.cmd(&[router], "exit", &["Up one level"], Action::Exit);
    for (pattern, help, id) in [
        (
            "ip ospf cost <n:int(1..65535)>",
            &["IP", "OSPF", "Interface cost", "Cost"][..],
            IF_OSPF_COST,
        ),
        (
            "ip ospf priority <n:int(0..255)>",
            &[
                "IP",
                "OSPF",
                "Priority in the designated router election",
                "Priority",
            ][..],
            IF_OSPF_PRIORITY,
        ),
        (
            "ip ospf hello-interval <n:int(1..65535)>",
            &["IP", "OSPF", "Time between hellos", "Seconds"][..],
            IF_OSPF_HELLO,
        ),
        (
            "ip ospf dead-interval <n:int(1..65535)>",
            &[
                "IP",
                "OSPF",
                "Time before a silent neighbour is dead",
                "Seconds",
            ][..],
            IF_OSPF_DEAD,
        ),
        (
            "ip ospf network point-to-point",
            &["IP", "OSPF", "Network type", "Point to point"][..],
            IF_OSPF_P2P,
        ),
        (
            "ip ospf network broadcast",
            &["IP", "OSPF", "Network type", "Broadcast"][..],
            IF_OSPF_BROADCAST,
        ),
        (
            "no ip ospf cost",
            &["Negate", "IP", "OSPF", "Interface cost"][..],
            NO_IF_OSPF_COST,
        ),
        (
            "no ip ospf priority",
            &["Negate", "IP", "OSPF", "Priority"][..],
            NO_IF_OSPF_PRIORITY,
        ),
        (
            "no ip ospf hello-interval",
            &["Negate", "IP", "OSPF", "Time between hellos"][..],
            NO_IF_OSPF_HELLO,
        ),
        (
            "no ip ospf dead-interval",
            &[
                "Negate",
                "IP",
                "OSPF",
                "Time before a silent neighbour is dead",
            ][..],
            NO_IF_OSPF_DEAD,
        ),
        (
            "ip mtu <n:int(68..1500)>",
            &["IP", "Maximum transmission unit (OSPF only)", "Bytes"][..],
            IF_MTU,
        ),
        (
            "no ip mtu",
            &["Negate", "IP", "Maximum transmission unit"][..],
            NO_IF_MTU,
        ),
        (
            "ip ospf mtu-ignore",
            &["IP", "OSPF", "Do not check the neighbour's MTU"][..],
            IF_OSPF_MTU_IGNORE,
        ),
        (
            "no ip ospf mtu-ignore",
            &["Negate", "IP", "OSPF", "Do not check the neighbour's MTU"][..],
            NO_IF_OSPF_MTU_IGNORE,
        ),
        (
            "no ip ospf network",
            &["Negate", "IP", "OSPF", "Network type"][..],
            NO_IF_OSPF_NETWORK,
        ),
    ] {
        g.cmd(&[config_if], pattern, help, h(id));
    }
    for (pattern, help, id) in [
        (
            "show ip ospf",
            &["Show", "IP information", "OSPF"][..],
            SHOW_OSPF,
        ),
        (
            "show ip protocols",
            &["Show", "IP information", "Routing protocols"][..],
            SHOW_PROTOCOLS,
        ),
        (
            "show ip ospf neighbor",
            &["Show", "IP information", "OSPF", "Neighbours"][..],
            SHOW_OSPF_NBR,
        ),
        (
            "show ip ospf interface",
            &["Show", "IP information", "OSPF", "Interfaces"][..],
            SHOW_OSPF_IF,
        ),
        (
            "show ip ospf database",
            &["Show", "IP information", "OSPF", "Link state database"][..],
            SHOW_OSPF_DB,
        ),
        (
            "clear ip ospf process",
            &["Clear", "IP information", "OSPF", "Restart the process"][..],
            CLEAR_OSPF,
        ),
    ] {
        g.cmd(&[privileged], pattern, help, h(id));
    }
}

/// Management-plane commands, the same on routers and switches.
fn mgmt_commands(g: &mut Grammar, privileged: ModeId, config: ModeId, line: ModeId) {
    let h = Action::Handler;
    for (pattern, help, id) in [
        (
            "enable secret <pw:word>",
            &[
                "Set a password for privileged mode",
                "Stored hashed",
                "Password",
            ][..],
            ENABLE_SECRET,
        ),
        (
            "enable password <pw:word>",
            &[
                "Set a password for privileged mode",
                "Stored as typed",
                "Password",
            ][..],
            ENABLE_PASSWORD,
        ),
        (
            "no enable secret",
            &[
                "Negate",
                "Set a password for privileged mode",
                "Remove the secret",
            ][..],
            NO_ENABLE_SECRET,
        ),
        (
            "no enable password",
            &[
                "Negate",
                "Set a password for privileged mode",
                "Remove the password",
            ][..],
            NO_ENABLE_PASSWORD,
        ),
        (
            "service password-encryption",
            &[
                "Service settings",
                "Show stored passwords in encrypted form",
            ][..],
            SERVICE_PWENC,
        ),
        (
            "no service password-encryption",
            &[
                "Negate",
                "Service settings",
                "Show stored passwords in the clear",
            ][..],
            NO_SERVICE_PWENC,
        ),
        (
            "line console <n:int(0..0)>",
            &["Configure a line", "Console line", "Line number"][..],
            LINE_CONSOLE,
        ),
        (
            "line vty <first:int(0..15)> <last:int(0..15)>",
            &[
                "Configure a line",
                "Virtual terminal lines",
                "First line",
                "Last line",
            ][..],
            LINE_VTY,
        ),
        (
            "banner motd <text:rest>",
            &[
                "Set a login banner",
                "Message of the day",
                "Delimiter, then the text",
            ][..],
            BANNER,
        ),
        (
            "no banner motd",
            &[
                "Negate",
                "Set a login banner",
                "Remove the message of the day",
            ][..],
            NO_BANNER,
        ),
    ] {
        g.cmd(&[config], pattern, help, h(id));
    }
    for (pattern, help, id) in [
        (
            "password <pw:word>",
            &["Set the line password", "Password"][..],
            LINE_PASSWORD,
        ),
        (
            "no password",
            &["Negate", "Remove the line password"][..],
            NO_LINE_PASSWORD,
        ),
        ("login", &["Ask for the line password"][..], LINE_LOGIN),
        (
            "no login",
            &["Negate", "Do not ask for a password"][..],
            NO_LINE_LOGIN,
        ),
    ] {
        g.cmd(&[line], pattern, help, h(id));
    }
    g.cmd(
        &[config, line],
        "end",
        &["Back to privileged"],
        Action::End(privileged),
    );
    g.cmd(&[line], "exit", &["Up one level"], Action::Exit);
    g.cmd(
        &[privileged],
        "show startup-config",
        &["Show", "Saved configuration"],
        h(SHOW_START),
    );
    g.cmd(
        &[privileged],
        "copy running-config startup-config",
        &["Copy", "Current configuration", "Saved configuration"],
        h(COPY_RUN_START),
    );
    g.cmd(
        &[privileged],
        "write",
        &["Save the configuration"],
        h(COPY_RUN_START),
    );
    g.cmd(
        &[privileged],
        "write memory",
        &["Save the configuration", "To the saved configuration"],
        h(COPY_RUN_START),
    );
    g.cmd(
        &[privileged],
        "erase startup-config",
        &["Erase", "The saved configuration"],
        h(ERASE_START),
    );
    g.cmd(&[privileged], "reload", &["Restart the device"], h(RELOAD));
}

fn dotted(len: u8) -> Ipv4Addr {
    Ipv4Addr::from_u32(mask(len))
}

fn if_name(port: usize) -> String {
    format!("GigabitEthernet0/{port}")
}

/// Name of logical interface `i`: `GigabitEthernet0/1`, or `GigabitEthernet0/1.10` for a
/// subinterface.
pub(crate) fn iface_name(r: &Router, i: usize) -> String {
    let ifc = &r.interfaces()[i];
    match (ifc.svi, ifc.sub) {
        (Some(v), _) => format!("Vlan{v}"),
        (None, Some(s)) => format!("{}.{}", if_name(s.parent.0 as usize), s.number),
        (None, None) => if_name(i),
    }
}

/// Like [`parse_ifname`] but also accepts a subinterface suffix: `g0/0.10` gives `(0, Some(10))`.
pub fn parse_router_ifname(s: &str, n_ports: usize) -> Option<(usize, Option<u16>)> {
    match s.split_once('.') {
        None => parse_ifname(s, n_ports).map(|p| (p, None)),
        Some((base, num)) => {
            let num: u16 = num.parse().ok()?;
            parse_ifname(base, n_ports).map(|p| (p, Some(num)))
        }
    }
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

/// What a terminal is waiting for instead of a command.
#[derive(Clone, Debug)]
enum Pending {
    /// `enable` asked for the password.
    Enable {
        tries: u8,
    },
    /// A new console session asked for the line password.
    Login {
        tries: u8,
    },
    /// `banner motd` text is arriving; it ends at the line holding the delimiter.
    Banner {
        delim: char,
        text: Vec<String>,
    },
    /// `reload` asks whether to save first.
    ReloadSave,
    ReloadConfirm,
    EraseConfirm,
}

/// One terminal on one device.
#[derive(Clone)]
pub struct Shell {
    grammar: Arc<Grammar>,
    modes: Modes,
    kind: Kind,
    session: Session,
    pub device: DeviceId,
    pending: Option<Pending>,
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
            pending: None,
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
            pending: None,
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
            pending: None,
        }
    }

    pub fn prompt(&self, w: &World) -> String {
        match &self.pending {
            Some(Pending::Enable { .. } | Pending::Login { .. }) => return PASSWORD_PROMPT.into(),
            Some(Pending::Banner { .. }) => return String::new(),
            Some(Pending::ReloadSave) => return ASK_SAVE.into(),
            Some(Pending::ReloadConfirm) => return ASK_RELOAD.into(),
            Some(Pending::EraseConfirm) => return ASK_ERASE.into(),
            None => {}
        }
        let name = match self.kind {
            Kind::Router => w.device::<Router>(self.device).name.clone(),
            Kind::Host => w.device::<Host>(self.device).name.clone(),
            Kind::Switch => w.device::<L2Switch>(self.device).name.clone(),
        };
        self.session.prompt(&self.grammar, &name)
    }

    /// Saved configuration from `copy running-config startup-config`.
    pub fn startup_config<'a>(&self, w: &'a World) -> Option<&'a str> {
        match self.kind {
            Kind::Router => w.device::<Router>(self.device).startup.as_deref(),
            Kind::Switch => w.device::<L2Switch>(self.device).startup.as_deref(),
            Kind::Host => None,
        }
    }

    /// Text shown when a terminal opens: the login banner, and for a console that needs a
    /// password, the login notice (after which the prompt asks for it).
    pub fn greeting(&mut self, w: &World) -> String {
        let Some(m) = self.mgmt(w) else {
            return String::new();
        };
        let mut out = String::new();
        if let Some(b) = &m.banner_motd {
            out += b;
            out += "\n";
        }
        if m.console_needs_login() {
            if !out.is_empty() {
                out += "\n";
            }
            out += "User Access Verification\n";
            self.pending = Some(Pending::Login { tries: 0 });
        }
        out.trim_end().to_string()
    }

    fn mgmt<'a>(&self, w: &'a World) -> Option<&'a Mgmt> {
        match self.kind {
            Kind::Router => Some(&w.device::<Router>(self.device).mgmt),
            Kind::Switch => Some(&w.device::<L2Switch>(self.device).mgmt),
            Kind::Host => None,
        }
    }

    fn with_mgmt<R>(&self, w: &mut World, f: impl FnOnce(&mut Mgmt) -> R) -> R {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, _| f(&mut s.mgmt)),
            _ => w.with_device::<Router, _>(self.device, |r, _| f(&mut r.mgmt)),
        }
    }

    fn device_name(&self, w: &World) -> String {
        match self.kind {
            Kind::Router => w.device::<Router>(self.device).name.clone(),
            Kind::Host => w.device::<Host>(self.device).name.clone(),
            Kind::Switch => w.device::<L2Switch>(self.device).name.clone(),
        }
    }

    /// The running configuration as text.
    fn running_text(&self, w: &World) -> String {
        match self.kind {
            Kind::Router => running_config(w.device::<Router>(self.device)),
            Kind::Switch => switch_config(w.device::<L2Switch>(self.device)),
            Kind::Host => String::new(),
        }
    }

    fn set_startup(&self, w: &mut World, text: Option<String>) {
        match self.kind {
            Kind::Router => w.with_device::<Router, _>(self.device, |r, _| r.startup = text),
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, _| s.startup = text),
            Kind::Host => {}
        }
    }

    /// Whether the running configuration differs from the saved one.
    fn unsaved(&self, w: &World) -> bool {
        self.startup_config(w).map(str::trim) != Some(self.running_text(w).trim())
    }

    /// Feed config text through the CLI as if typed in configuration mode. Lines the CLI
    /// rejects come back as messages (`config line N: ...`) instead of failing the replay, so
    /// an old file still opens after the grammar changes. Banner text is passed through
    /// untouched, blank lines and `!` included.
    pub fn replay(&mut self, w: &mut World, cfg: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for line in ["enable", "configure terminal"] {
            self.exec(w, line);
        }
        for (n, raw) in cfg.lines().enumerate() {
            let in_banner = matches!(self.pending, Some(Pending::Banner { .. }));
            let line = if in_banner { raw } else { raw.trim() };
            if !in_banner && (line.is_empty() || line == "!" || line == "end") {
                continue;
            }
            match self.exec(w, line) {
                Some(out) if out.is_empty() => {}
                // Starting a multi-line banner prints instructions meant for a person.
                Some(_) if matches!(self.pending, Some(Pending::Banner { .. })) && !in_banner => {}
                Some(out) => problems.push(format!(
                    "config line {}: `{line}`: {}",
                    n + 1,
                    out.lines().last().unwrap_or("")
                )),
                None => break,
            }
        }
        problems
    }

    /// `reload`: forget the running configuration, then boot from the saved one.
    fn reload(&mut self, w: &mut World) -> String {
        let startup = self.startup_config(w).map(str::to_string);
        match self.kind {
            Kind::Router => w.with_device::<Router, _>(self.device, |r, _| r.factory_reset()),
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, _| s.factory_reset()),
            Kind::Host => {}
        }
        let mut out = String::from("Reload requested.");
        if let Some(cfg) = startup {
            let mut boot = match self.kind {
                Kind::Switch => Shell::switch(self.device),
                _ => Shell::router(self.device),
            };
            for p in boot.replay(w, &cfg) {
                out += &format!("\n{}: {p}", self.device_name(w));
            }
        }
        self.session = Session::new(self.modes.user);
        out += "\nSystem restarted.";
        let greeting = self.greeting(w);
        if !greeting.is_empty() {
            out += "\n\n";
            out += &greeting;
        }
        out
    }

    /// One line typed while a prompt is open instead of a command.
    fn answer(&mut self, w: &mut World, pending: Pending, line: &str) -> String {
        let yes = |l: &str| matches!(l.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        let no = |l: &str| matches!(l.trim().to_ascii_lowercase().as_str(), "n" | "no");
        match pending {
            Pending::Enable { tries } => {
                if self.mgmt(w).is_some_and(|m| m.check_enable(line)) {
                    self.session.enter(self.modes.privileged, None);
                } else if tries + 1 >= 3 {
                    return BAD_SECRETS.to_string();
                } else {
                    self.pending = Some(Pending::Enable { tries: tries + 1 });
                }
                String::new()
            }
            Pending::Login { tries } => {
                let ok = self
                    .mgmt(w)
                    .and_then(|m| m.console.password.as_deref())
                    .is_none_or(|p| p == line);
                if ok {
                    return String::new();
                }
                // Three misses print a notice and start over; the console never locks out.
                self.pending = Some(Pending::Login {
                    tries: (tries + 1) % 3,
                });
                if tries + 1 >= 3 {
                    BAD_PASSWORDS.to_string()
                } else {
                    String::new()
                }
            }
            Pending::Banner { delim, mut text } => {
                match line.find(delim) {
                    Some(end) => {
                        if end > 0 {
                            text.push(line[..end].to_string());
                        }
                        let banner = text.join("\n");
                        self.with_mgmt(w, |m| m.banner_motd = Some(banner));
                    }
                    None => {
                        text.push(line.to_string());
                        self.pending = Some(Pending::Banner { delim, text });
                    }
                }
                String::new()
            }
            Pending::ReloadSave => {
                if yes(line) {
                    let text = self.running_text(w);
                    self.set_startup(w, Some(text));
                } else if !no(line) {
                    self.pending = Some(Pending::ReloadSave);
                    return ANSWER_YES_NO.to_string();
                }
                self.pending = Some(Pending::ReloadConfirm);
                String::new()
            }
            Pending::ReloadConfirm => {
                if line.trim().is_empty() || yes(line) {
                    self.reload(w)
                } else {
                    String::new()
                }
            }
            Pending::EraseConfirm => {
                if line.trim().is_empty() || yes(line) {
                    self.set_startup(w, None);
                    ERASED.to_string()
                } else {
                    String::new()
                }
            }
        }
    }

    pub fn help(&self, line: &str) -> String {
        if self.pending.is_some() {
            return String::new();
        }
        render_help(&self.session.help(&self.grammar, line))
    }

    pub fn tab(&self, line: &str) -> String {
        if self.pending.is_some() {
            return line.to_string();
        }
        self.session.tab(&self.grammar, line)
    }

    /// Run one line; returns the text to show (possibly empty). `None` means the session ended.
    /// Start in global configuration mode, with no password asked: for the settings panels, which
    /// act for the owner of the lab rather than for someone at a login prompt.
    pub fn enter_config(&mut self) {
        if self.kind != Kind::Host {
            self.session.enter(self.modes.config, None);
        }
    }

    pub fn exec(&mut self, w: &mut World, line: &str) -> Option<String> {
        if let Some(p) = self.pending.take() {
            return Some(self.answer(w, p, line));
        }
        let prompt = self.prompt(w);
        match self.session.exec(&self.grammar, line) {
            Exec::Empty | Exec::Moved => Some(String::new()),
            Exec::Logout => None,
            Exec::Error(e) => Some(render_error(&prompt, &e)),
            Exec::Run { handler, args } => {
                let out = self.run(w, handler, &args, line);
                self.settle(w);
                Some(out)
            }
        }
    }

    /// Run a command; inside `interface range` once for each port, unless it is a `do` command.
    fn run(&mut self, w: &mut World, handler: u32, a: &Args, line: &str) -> String {
        let Some(ports) = self
            .range_ports()
            .filter(|_| !line.trim_start().starts_with("do "))
        else {
            return self.run_one(w, handler, a);
        };
        let saved = self.session.context().map(str::to_string);
        let mut outs: Vec<String> = vec![];
        for p in ports {
            self.session
                .enter(self.modes.config_if, Some(p.to_string()));
            let o = self.run_one(w, handler, a);
            if !o.is_empty() && !outs.contains(&o) {
                outs.push(o);
            }
        }
        self.session.enter(self.modes.config_if, saved);
        outs.join("\n")
    }

    /// After a command: a switch looks at its cables and gets spanning tree going if it is on.
    fn settle(&self, w: &mut World) {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, ctx| s.refresh(ctx)),
            Kind::Router => w.with_device::<Router, _>(self.device, |r, ctx| r.disc_kick(ctx)),
            Kind::Host => {}
        }
    }

    fn run_one(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        let m = self.modes;
        match handler {
            SERVICE_DHCP | NO_SERVICE_DHCP => {
                let on = handler == SERVICE_DHCP;
                self.with_l3(w, |r| r.dhcp.enabled = on);
            }
            EXCLUDE | NO_EXCLUDE => {
                let low = Ipv4Addr(a.ipv4("low").unwrap());
                let high = a.ipv4("high").map_or(low, Ipv4Addr);
                if high.to_u32() < low.to_u32() {
                    return BAD_RANGE.to_string();
                }
                if handler == EXCLUDE {
                    self.with_l3(w, |r| r.dhcp.exclude(low, high));
                } else if !self.with_l3(w, |r| r.dhcp.include(low, high)) {
                    return NO_SUCH_EXCLUSION.to_string();
                }
            }
            ROUTER_OSPF..=NO_OSPF_DEFAULT_INFO | IF_MTU..=SHOW_PROTOCOLS => {
                return self.ospf_cmd(w, handler, a, &m)
            }
            300..=329 => return self.m3b_cmd(w, handler, a),
            330..=349 => return self.channel_cmd(w, handler, a),
            350..=379 => return self.disc_cmd(w, handler, a),
            ACL_PERMIT | ACL_DENY | ACL_REMARK | ACL_NO_SEQ | ACL_NO_PERMIT | ACL_NO_DENY
            | NO_ACL_NUMBER => return self.acl_entry_cmd(w, handler, a, &m),
            ACL_STD | ACL_EXT | NO_ACL_STD | NO_ACL_EXT => {
                let name = a.word("name").unwrap().to_string();
                let kind = if matches!(handler, ACL_STD | NO_ACL_STD) {
                    AclKind::Standard
                } else {
                    AclKind::Extended
                };
                if matches!(handler, NO_ACL_STD | NO_ACL_EXT) {
                    let found = self.with_l3(w, |r| {
                        r.acls.get(&name).is_some_and(|l| l.kind == kind) && r.acls.remove(&name)
                    });
                    if !found {
                        return NO_SUCH_ACL.to_string();
                    }
                } else {
                    if self.with_l3(w, |r| r.acls.open(&name, kind).is_err()) {
                        return ACL_KIND_CLASH.to_string();
                    }
                    let mode = if kind == AclKind::Standard {
                        m.config_std_nacl
                    } else {
                        m.config_ext_nacl
                    };
                    self.session.enter(mode, Some(name));
                }
            }
            ACCESS_GROUP_IN
            | ACCESS_GROUP_OUT
            | NO_ACCESS_GROUP_IN
            | NO_ACCESS_GROUP_OUT
            | NO_ACCESS_GROUP_ANY_IN
            | NO_ACCESS_GROUP_ANY_OUT => {
                let Some(i) = self.current_iface(w) else {
                    return NO_SUCH_INTERFACE.to_string();
                };
                let inbound = matches!(
                    handler,
                    ACCESS_GROUP_IN | NO_ACCESS_GROUP_IN | NO_ACCESS_GROUP_ANY_IN
                );
                let name = matches!(handler, ACCESS_GROUP_IN | ACCESS_GROUP_OUT)
                    .then(|| a.word("acl").unwrap().to_string());
                self.with_l3(w, |r| r.set_acl(i, inbound, name));
            }
            ACCESS_CLASS | NO_ACCESS_CLASS => {
                let name = (handler == ACCESS_CLASS).then(|| a.word("acl").unwrap().to_string());
                let console = self.session.context() == Some("console");
                self.with_mgmt(w, |mg| {
                    let l = if console {
                        &mut mg.console
                    } else {
                        &mut mg.vty
                    };
                    l.access_class_in = name;
                });
            }
            SHOW_ACL | SHOW_ACL_ONE => {
                let only = a.word("name").map(str::to_string);
                return self.with_l3(w, |r| acl_show(&r.acls, only.as_deref()));
            }
            CLEAR_ACL | CLEAR_ACL_ONE => {
                let only = a.word("name").map(str::to_string);
                self.with_l3(w, |r| r.acls.clear_hits(only.as_deref()));
            }
            DHCP_POOL => {
                let name = a.word("name").unwrap().to_string();
                self.with_l3(w, |r| r.dhcp.pool_mut(&name).lease);
                self.session.enter(m.config_dhcp, Some(name));
            }
            NO_DHCP_POOL => {
                let name = a.word("name").unwrap().to_string();
                if !self.with_l3(w, |r| r.dhcp.remove_pool(&name)) {
                    return NO_SUCH_POOL.to_string();
                }
            }
            POOL_NETWORK | POOL_ROUTER | POOL_DNS | POOL_LEASE | NO_POOL_NETWORK
            | NO_POOL_ROUTER | NO_POOL_DNS => {
                let Some(name) = self.session.context().map(str::to_string) else {
                    return NO_SUCH_POOL.to_string();
                };
                let ip = |n: &str| a.ipv4(n).map(Ipv4Addr);
                let dns: Vec<Ipv4Addr> = ["a", "b", "c"].iter().filter_map(|n| ip(n)).collect();
                let network = a.ipv4("net").zip(a.mask("mask"));
                let lease = a.int("days").map(|d| {
                    let h = a.int("hours").unwrap_or(0);
                    let mi = a.int("minutes").unwrap_or(0);
                    SimTime::from_secs(((d * 24 + h) * 60 + mi) as u64 * 60)
                });
                self.with_l3(w, |r| {
                    let p = r.dhcp.pool_mut(&name);
                    match handler {
                        POOL_NETWORK => {
                            let (net, len) = network.expect("parsed");
                            p.network =
                                Some((Ipv4Addr::from_u32(Ipv4Addr(net).to_u32() & mask(len)), len));
                        }
                        POOL_ROUTER => p.default_router = ip("a"),
                        POOL_DNS => p.dns = dns,
                        POOL_LEASE => p.lease = lease.expect("parsed"),
                        NO_POOL_NETWORK => p.network = None,
                        NO_POOL_ROUTER => p.default_router = None,
                        _ => p.dns.clear(),
                    }
                });
            }
            HELPER | NO_HELPER => {
                let server = Ipv4Addr(a.ipv4("server").unwrap());
                let Some(i) = self.current_iface(w) else {
                    return NO_SUCH_INTERFACE.to_string();
                };
                if handler == HELPER {
                    self.with_l3(w, |r| r.add_helper(i, server));
                } else if !self.with_l3(w, |r| r.remove_helper(i, server)) {
                    return NO_SUCH_HELPER.to_string();
                }
            }
            SHOW_DHCP_BINDING => {
                let now = w.kernel.now();
                let rows = self.with_l3(w, |r| r.dhcp.bindings(now));
                return show_dhcp_bindings(&rows, now);
            }
            SHOW_DHCP_POOL => {
                let now = w.kernel.now();
                let used = self.with_l3(w, |r| r.dhcp.bindings(now));
                return show_dhcp_pools(self.l3(w), &used);
            }
            SHOW_DHCP_CONFLICT => {
                let c = self.l3(w).dhcp.conflicts();
                return if c.is_empty() {
                    "No conflicts.".to_string()
                } else {
                    c.iter()
                        .map(Ipv4Addr::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                };
            }
            CLEAR_DHCP | CLEAR_DHCP_ADDR => {
                let addr = a.ipv4("addr").map(Ipv4Addr);
                self.with_l3(w, |r| r.dhcp.clear(addr));
            }
            IPCONFIG_DHCP | IPCONFIG_RENEW => {
                w.with_device::<Host, _>(id, |h, c| {
                    if handler == IPCONFIG_DHCP {
                        h.dhcp_start(c)
                    } else {
                        h.dhcp_renew(c)
                    }
                });
                // Give the exchange time to finish; stop as soon as it has.
                let deadline = w.kernel.now() + SimTime::from_secs(8);
                w.run_until_or(deadline, |w| {
                    w.device::<Host>(id).dhcp.state == netdev::DhcpState::Bound
                });
                let h = w.device::<Host>(id);
                return if h.dhcp.state == netdev::DhcpState::Bound {
                    show_ipconfig(h, w.kernel.now())
                } else {
                    DHCP_NO_ANSWER.to_string()
                };
            }
            IPCONFIG_RELEASE => {
                w.with_device::<Host, _>(id, |h, c| h.dhcp_release(c));
                let t = w.kernel.now() + SimTime::from_millis(100);
                w.run_until(t);
                return "Address released.".to_string();
            }
            ENABLE => {
                if self.mgmt(w).is_some_and(Mgmt::enable_protected) {
                    self.pending = Some(Pending::Enable { tries: 0 });
                } else {
                    self.session.enter(m.privileged, None);
                }
            }
            ENABLE_SECRET => {
                let stored = stored_secret(&self.device_name(w), a.word("pw").unwrap());
                self.with_mgmt(w, |m| m.enable_secret = Some(stored));
            }
            ENABLE_PASSWORD => {
                let plain = plain_password(a.word("pw").unwrap());
                self.with_mgmt(w, |m| m.enable_password = Some(plain));
            }
            NO_ENABLE_SECRET => self.with_mgmt(w, |m| m.enable_secret = None),
            NO_ENABLE_PASSWORD => self.with_mgmt(w, |m| m.enable_password = None),
            SERVICE_PWENC | NO_SERVICE_PWENC => {
                self.with_mgmt(w, |m| m.encrypt_passwords = handler == SERVICE_PWENC)
            }
            LINE_CONSOLE => self.session.enter(m.config_line, Some("console".into())),
            LINE_VTY => {
                let range = (a.int("first").unwrap() as u8, a.int("last").unwrap() as u8);
                self.with_mgmt(w, |m| m.vty_range = range);
                self.session.enter(m.config_line, Some("vty".into()));
            }
            LINE_PASSWORD | NO_LINE_PASSWORD | LINE_LOGIN | NO_LINE_LOGIN => {
                let password = match handler {
                    LINE_PASSWORD => Some(Some(plain_password(a.word("pw").unwrap()))),
                    NO_LINE_PASSWORD => Some(None),
                    _ => None,
                };
                let console = self.session.context() == Some("console");
                self.with_mgmt(w, |mg| {
                    let line = if console {
                        &mut mg.console
                    } else {
                        &mut mg.vty
                    };
                    match (password, handler) {
                        (Some(p), _) => line.password = p,
                        (None, LINE_LOGIN) => line.login = true,
                        (None, _) => line.login = false,
                    }
                });
            }
            BANNER => {
                let text = a.word("text").unwrap();
                let delim = text.chars().next().unwrap_or('#');
                let body = &text[delim.len_utf8()..];
                match body.find(delim) {
                    Some(end) => {
                        let b = body[..end].to_string();
                        self.with_mgmt(w, |mg| mg.banner_motd = Some(b));
                    }
                    None => {
                        let first = (!body.trim().is_empty()).then(|| body.to_string());
                        self.pending = Some(Pending::Banner {
                            delim,
                            text: first.into_iter().collect(),
                        });
                        return format!("Enter TEXT message.  End with the character '{delim}'.");
                    }
                }
            }
            NO_BANNER => self.with_mgmt(w, |mg| mg.banner_motd = None),
            SHOW_START => {
                return self
                    .startup_config(w)
                    .map_or_else(|| NOT_PRESENT.to_string(), str::to_string)
            }
            RELOAD => {
                self.pending = Some(if self.unsaved(w) {
                    Pending::ReloadSave
                } else {
                    Pending::ReloadConfirm
                });
            }
            ERASE_START => self.pending = Some(Pending::EraseConfirm),
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
                let n = w.device::<Router>(id).n_ports() as usize;
                let Some((p, sub)) = parse_router_ifname(name, n) else {
                    return format!("{UNKNOWN_INTERFACE} {name}");
                };
                let idx = match sub {
                    None => Some(p),
                    Some(num) => w.with_device::<Router, _>(id, |r, _| {
                        r.add_subinterface(PortId(p as u16), num)
                    }),
                };
                match idx {
                    Some(i) => self.session.enter(m.config_if, Some(i.to_string())),
                    None => return format!("{UNKNOWN_INTERFACE} {name}"),
                }
            }
            NO_INTERFACE => {
                let name = a.word("name").unwrap();
                let n = w.device::<Router>(id).n_ports() as usize;
                let found = parse_router_ifname(name, n).and_then(|(p, sub)| {
                    let num = sub?;
                    w.device::<Router>(id).interfaces().iter().position(|i| {
                        i.sub
                            .is_some_and(|s| s.parent.0 as usize == p && s.number == num)
                    })
                });
                match found {
                    Some(i) => {
                        w.with_device::<Router, _>(id, |r, _| r.remove_subinterface(i));
                    }
                    None => return format!("{UNKNOWN_INTERFACE} {name}"),
                }
            }
            ENCAP | ENCAP_NATIVE | NO_ENCAP => {
                let i = self
                    .session
                    .context()
                    .and_then(|c| c.parse::<usize>().ok())
                    .unwrap_or(0);
                let vlan = a.int("vlan").map(|v| v as u16);
                let native = handler == ENCAP_NATIVE;
                let r = w.with_device::<Router, _>(id, |r, _| r.set_encapsulation(i, vlan, native));
                match r {
                    Ok(()) => {}
                    Err(EncapError::NotSubinterface) => return ENCAP_NOT_SUB.to_string(),
                    Err(EncapError::InUse) => return ENCAP_IN_USE.to_string(),
                }
            }
            IP_ADDRESS | NO_IP_ADDRESS | SHUTDOWN | NO_SHUTDOWN if self.kind == Kind::Switch => {
                return self.svi_interface_cmd(w, handler, a);
            }
            SVI_ENTER => {
                let vlan = a.int("id").unwrap() as u16;
                w.with_device::<L2Switch, _>(id, |s, _| s.add_svi(vlan));
                self.session.enter(m.config_if, Some(format!("vlan{vlan}")));
            }
            NO_SVI => {
                let vlan = a.int("id").unwrap() as u16;
                if !w.with_device::<L2Switch, _>(id, |s, _| s.remove_svi(vlan)) {
                    return NO_SUCH_INTERFACE.to_string();
                }
            }
            IP_ROUTING | NO_IP_ROUTING => {
                let on = handler == IP_ROUTING;
                w.with_device::<L2Switch, _>(id, |s, _| s.l3_mut().set_routing(on));
            }
            DEFAULT_GW | NO_DEFAULT_GW => {
                let gw = a.ipv4("gw").map(Ipv4Addr);
                w.with_device::<L2Switch, _>(id, |s, _| s.l3_mut().set_default_gateway(gw));
            }
            IP_ADDRESS | NO_IP_ADDRESS | SHUTDOWN | NO_SHUTDOWN => {
                let port = PortId(
                    self.session
                        .context()
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(0),
                );
                let needs_vlan = w
                    .device::<Router>(id)
                    .interfaces()
                    .get(port.0 as usize)
                    .and_then(|i| i.sub)
                    .is_some_and(|s| s.vlan.is_none());
                if handler == IP_ADDRESS && needs_vlan {
                    return NEEDS_ENCAP.to_string();
                }
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
                    self.with_l3(w, |r| r.add_static(route));
                } else if self.l3(w).statics().contains(&route) {
                    self.with_l3(w, |r| r.remove_static(route));
                } else {
                    return NO_ROUTE_ENTRY.to_string();
                }
            }
            SW_INTERFACE => {
                let name = a.word("name").unwrap();
                // `Vlan10`, the form the running config prints, is the same as `vlan 10`.
                if let Some(vlan) = parse_vlan_name(name) {
                    w.with_device::<L2Switch, _>(id, |s, _| s.add_svi(vlan));
                    self.session.enter(m.config_if, Some(format!("vlan{vlan}")));
                    return String::new();
                }
                if let Some(g) = etherchannel::parse_po_name(name) {
                    w.with_device::<L2Switch, _>(id, |s, _| s.ensure_group(g));
                    self.session.enter(m.config_if, Some(format!("po:{g}")));
                    return String::new();
                }
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
            SW_MODE_AUTO | SW_MODE_DESIRABLE | SW_NONEGOTIATE | NO_SW_NONEGOTIATE => {
                return self.switchport(w, handler, a)
            }
            SHOW_SWITCHPORT => {
                let name = a.word("name").unwrap();
                let sw = w.device::<L2Switch>(id);
                return match parse_switch_ifname(name, sw.n_ports() as usize) {
                    Some(p) => show_switchport(sw, p),
                    None => format!("{UNKNOWN_INTERFACE} {name}"),
                };
            }
            SW_MODE_ACCESS | SW_MODE_TRUNK | NO_SW_MODE | SW_ACCESS_VLAN | NO_SW_ACCESS_VLAN
            | SW_NATIVE | NO_SW_NATIVE | SW_ALLOWED | SW_ALLOWED_ADD | SW_ALLOWED_REMOVE
            | SW_ALLOWED_ALL | SW_ALLOWED_NONE => return self.switchport(w, handler, a),
            SHOW_RUN if self.kind == Kind::Switch => {
                return switch_config(w.device::<L2Switch>(id));
            }
            SPAN_MODE_PVST | SPAN_MODE_RAPID | NO_SPAN_MODE => {
                let mode = match handler {
                    SPAN_MODE_PVST => Some(StpMode::Stp),
                    SPAN_MODE_RAPID => Some(StpMode::Rapid),
                    _ => None,
                };
                w.with_device::<L2Switch, _>(id, |s, _| s.configure_spanning(|c| c.mode = mode));
            }
            SPAN_PRIORITY | NO_SPAN_PRIORITY | SPAN_ROOT_PRIMARY | SPAN_ROOT_SECONDARY
            | SPAN_VLAN_ON | SPAN_VLAN_OFF => return self.spanning_vlans(w, handler, a),
            PORTFAST | NO_PORTFAST | SPAN_COST | NO_SPAN_COST | SPAN_PORT_PRIO
            | NO_SPAN_PORT_PRIO => return self.spanning_port(w, handler, a),
            SHOW_SPAN => return show_spanning_tree(w.device::<L2Switch>(id), None),
            SHOW_SPAN_VLAN => {
                let v = a.int("id").unwrap() as u16;
                return show_spanning_tree(w.device::<L2Switch>(id), Some(v));
            }
            SHOW_MACS => return show_macs(w.device::<L2Switch>(id), w.kernel.now()),
            SHOW_VLAN => return show_vlan_brief(w.device::<L2Switch>(id)),
            SHOW_TRUNK => return show_trunks(w.device::<L2Switch>(id)),
            SHOW_RUN => return running_config(w.device::<Router>(id)),
            SHOW_ROUTE => return show_routes(self.l3(w)),
            SHOW_BRIEF => return show_brief(self.l3(w)),
            COPY_RUN_START => {
                let text = self.running_text(w);
                self.set_startup(w, Some(text));
                return SAVED.to_string();
            }
            IPCONFIG => return show_ipconfig(w.device::<Host>(id), w.kernel.now()),
            IPCONFIG_SET => {
                let (ip, len) = (Ipv4Addr(a.ipv4("addr").unwrap()), a.mask("mask").unwrap());
                w.with_device::<Host, _>(id, |h, _| {
                    h.dhcp_disable();
                    h.ip = ip;
                    h.prefix_len = len;
                });
            }
            IPCONFIG_GW => {
                let gw = Ipv4Addr(a.ipv4("gw").unwrap());
                w.with_device::<Host, _>(id, |h, _| h.gateway = Some(gw));
            }
            PING => return ping(w, id, Ipv4Addr(a.ipv4("dst").unwrap())),
            TRACERT => {
                let max = a.int("max").map_or(30, |m| m as u8);
                return trace::tracert(w, id, Ipv4Addr(a.ipv4("dst").unwrap()), max);
            }
            ROUTER_PING | TRACEROUTE => {
                let origin = if self.kind == Kind::Switch {
                    Origin::Switch
                } else {
                    Origin::Router
                };
                let dst = Ipv4Addr(a.ipv4("dst").unwrap());
                return if handler == ROUTER_PING {
                    trace::router_ping(w, origin, id, dst)
                } else {
                    let first = a.int("min").map_or(1, |m| m as u8);
                    let last = a.int("max").map_or(30, |m| m as u8);
                    trace::traceroute(w, origin, id, dst, first, last.max(first))
                };
            }
            _ => {}
        }
        String::new()
    }
}

impl Shell {
    /// The layer 3 half of this device: the router itself, or a switch's SVIs.
    fn l3<'a>(&self, w: &'a World) -> &'a Router {
        match self.kind {
            Kind::Switch => w.device::<L2Switch>(self.device).l3(),
            _ => w.device::<Router>(self.device),
        }
    }

    fn with_l3<R>(&self, w: &mut World, f: impl FnOnce(&mut Router) -> R) -> R {
        match self.kind {
            Kind::Switch => w.with_device::<L2Switch, _>(self.device, |s, _| f(s.l3_mut())),
            _ => w.with_device::<Router, _>(self.device, |r, _| f(r)),
        }
    }

    /// The logical interface the session is configuring: a router interface by number, a
    /// switch's SVI by VLAN.
    fn current_iface(&self, w: &World) -> Option<usize> {
        let ctx = self.session.context()?;
        match self.kind {
            Kind::Switch => {
                let vlan = ctx.strip_prefix("vlan")?.parse().ok()?;
                self.l3(w).svi_index(vlan)
            }
            _ => ctx.parse().ok(),
        }
    }

    /// `ip address` and `shutdown` inside `interface vlan N` on a switch.
    fn svi_interface_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        // `shutdown` on a switch port, not an SVI.
        if let (SHUTDOWN | NO_SHUTDOWN, Some(target)) = (handler, self.cfg_target()) {
            w.with_device::<L2Switch, _>(self.device, |s, _| {
                target.apply(s, |c| c.shutdown = handler == SHUTDOWN)
            });
            return String::new();
        }
        let Some(vlan) = self
            .session
            .context()
            .and_then(|c| c.strip_prefix("vlan"))
            .and_then(|v| v.parse::<u16>().ok())
        else {
            return SVI_ONLY.to_string();
        };
        let Some(i) = self.l3(w).svi_index(vlan) else {
            return NO_SUCH_INTERFACE.to_string();
        };
        let port = PortId(i as u16);
        self.with_l3(w, |r| match handler {
            IP_ADDRESS => r.set_address(
                port,
                Some((Ipv4Addr(a.ipv4("addr").unwrap()), a.mask("mask").unwrap())),
            ),
            NO_IP_ADDRESS => r.set_address(port, None),
            SHUTDOWN => r.set_shutdown(port, true),
            _ => r.set_shutdown(port, false),
        });
        String::new()
    }

    /// Global `spanning-tree vlan ...` commands.
    fn spanning_vlans(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let text = a.word("list").unwrap_or_default();
        let Some(vlans) = parse_vlan_list(text) else {
            return format!("{BAD_VLAN_LIST} {text}");
        };
        let prio = a.int("prio").map_or(0, |p| p as u16);
        if handler == SPAN_PRIORITY && prio % 4096 != 0 {
            return BAD_BRIDGE_PRIORITY.to_string();
        }
        w.with_device::<L2Switch, _>(self.device, |s, _| {
            // `root primary` looks at who is root now, so work that out before changing anything.
            let wanted: Vec<(u16, u16)> = vlans
                .iter()
                .map(|v| {
                    let current = s
                        .tree(*v)
                        .map(|t| t.root().root.priority.saturating_sub(*v));
                    let p = match (handler, current) {
                        (SPAN_ROOT_SECONDARY, _) => 28672,
                        (SPAN_ROOT_PRIMARY, Some(r))
                            if r <= 24576 && !s.tree(*v).is_some_and(|t| t.is_root()) =>
                        {
                            r.saturating_sub(4096)
                        }
                        (SPAN_ROOT_PRIMARY, Some(_)) | (SPAN_ROOT_PRIMARY, None) => 24576,
                        _ => prio,
                    };
                    (*v, p)
                })
                .collect();
            s.configure_spanning(|c| {
                for (v, p) in wanted {
                    match handler {
                        SPAN_PRIORITY | SPAN_ROOT_PRIMARY | SPAN_ROOT_SECONDARY => {
                            c.priority.insert(v, p);
                        }
                        NO_SPAN_PRIORITY => {
                            c.priority.remove(&v);
                        }
                        SPAN_VLAN_ON => {
                            c.disabled.remove(&v);
                        }
                        _ => {
                            c.disabled.insert(v);
                        }
                    }
                }
            });
        });
        String::new()
    }

    /// `spanning-tree portfast`, `cost` and `port-priority` inside an interface.
    fn spanning_port(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let Some(target) = self.cfg_target() else {
            return PORT_ONLY.to_string();
        };
        let prio = a.int("prio").map_or(128, |p| p as u8);
        if handler == SPAN_PORT_PRIO && prio % 16 != 0 {
            return BAD_PORT_PRIORITY.to_string();
        }
        let cost = a.int("cost").map(|c| c as u32);
        w.with_device::<L2Switch, _>(self.device, |s, _| {
            target.apply(s, |c| match handler {
                PORTFAST => c.portfast = true,
                NO_PORTFAST => c.portfast = false,
                SPAN_COST => c.stp_cost = cost,
                NO_SPAN_COST => c.stp_cost = None,
                SPAN_PORT_PRIO => c.stp_priority = Some(prio),
                _ => c.stp_priority = None,
            })
        });
        String::new()
    }

    /// The `switchport ...` family, for the interface the session is in.
    fn switchport(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        // Inside `interface vlan N` the context is not a port number.
        let Some(target) = self.cfg_target() else {
            return PORT_ONLY.to_string();
        };
        let mut notice = String::new();
        {
            let c = target
                .config(w.device::<L2Switch>(id))
                .expect("port in range");
            let dynamic_now = c.mode.is_dynamic();
            if (handler == SW_NONEGOTIATE && dynamic_now)
                || (matches!(handler, SW_MODE_AUTO | SW_MODE_DESIRABLE) && c.nonegotiate)
            {
                return NEGOTIATE_CONFLICT.to_string();
            }
        }
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
            target.apply(s, |c| match handler {
                SW_MODE_ACCESS => c.mode = PortMode::Access,
                NO_SW_MODE | SW_MODE_AUTO => c.mode = PortMode::DynamicAuto,
                SW_MODE_DESIRABLE => c.mode = PortMode::DynamicDesirable,
                SW_MODE_TRUNK => c.mode = PortMode::Trunk,
                SW_NONEGOTIATE => c.nonegotiate = true,
                NO_SW_NONEGOTIATE => c.nonegotiate = false,
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

/// `vlan10` or `Vlan10`: the one-word spelling of an SVI name.
fn parse_vlan_name(s: &str) -> Option<u16> {
    let rest = s.to_ascii_lowercase();
    let n: u16 = rest.strip_prefix("vlan")?.parse().ok()?;
    (1..=4094).contains(&n).then_some(n)
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

/// The start of a config: password encryption, hostname, enable secret and password.
fn mgmt_head(m: &Mgmt, hostname: &str) -> String {
    let mut o = String::new();
    if m.encrypt_passwords {
        o += "service password-encryption\n!\n";
    }
    o += &format!("hostname {hostname}\n!\n");
    if let Some(sec) = &m.enable_secret {
        o += &format!("enable secret wb1${sec}\n");
    }
    if let Some(p) = &m.enable_password {
        o += &format!(
            "enable password {}\n",
            password_text(p, m.encrypt_passwords)
        );
    }
    if m.enable_protected() {
        o += "!\n";
    }
    o
}

/// The end of a config: the login banner and the console and vty lines.
/// The access lists, as configuration text, with a `!` after them.
fn acl_block(r: &Router) -> String {
    let t = acl_config_text(&r.acls);
    if t.is_empty() {
        t
    } else {
        t + "!\n"
    }
}

fn mgmt_tail(m: &Mgmt) -> String {
    let mut o = String::new();
    if let Some(b) = &m.banner_motd {
        let d = banner_delimiter(b);
        o += &format!("banner motd {d}\n{b}\n{d}\n!\n");
    }
    for (header, line) in [
        ("line console 0".to_string(), &m.console),
        (
            format!("line vty {} {}", m.vty_range.0, m.vty_range.1),
            &m.vty,
        ),
    ] {
        if line.password.is_none() && !line.login && line.access_class_in.is_none() {
            continue;
        }
        o += &format!("{header}\n");
        if let Some(p) = &line.password {
            o += &format!(" password {}\n", password_text(p, m.encrypt_passwords));
        }
        if line.login {
            o += " login\n";
        }
        if let Some(a) = &line.access_class_in {
            o += &format!(" access-class {a} in\n");
        }
        o += "!\n";
    }
    o
}

pub fn running_config(r: &Router) -> String {
    let mut o = mgmt_head(&r.mgmt, &r.name);
    o += &disc::disc_global_lines(&r.disc);
    o += &dhcp_config(r);
    for (i, ifc) in r.interfaces().iter().enumerate() {
        o += &format!("interface {}\n", iface_name(r, i));
        if let Some(s) = ifc.sub {
            if let Some(v) = s.vlan {
                o += &format!(
                    " encapsulation dot1q {v}{}\n",
                    if s.native { " native" } else { "" }
                );
            }
        }
        match ifc.addr {
            Some((ip, len)) => o += &format!(" ip address {ip} {}\n", dotted(len)),
            None => o += " no ip address\n",
        }
        for h in &ifc.helpers {
            o += &format!(" ip helper-address {h}\n");
        }
        if let Some(n) = &ifc.acl_in {
            o += &format!(" ip access-group {n} in\n");
        }
        if let Some(n) = &ifc.acl_out {
            o += &format!(" ip access-group {n} out\n");
        }
        o += &ospf_if_lines(&ifc.ospf);
        if i < r.n_ports() as usize {
            o += &disc::disc_port_lines(&r.disc, i);
        }
        // A subinterface is up unless told otherwise, so only a shut one needs a line.
        if ifc.sub.is_none() {
            o += if ifc.shutdown {
                " shutdown\n"
            } else {
                " no shutdown\n"
            };
        } else if ifc.shutdown {
            o += " shutdown\n";
        }
        o += "!\n";
    }
    o += &ospf_config(r);
    for s in r.statics() {
        o += &format!("ip route {} {} {}\n", s.net, dotted(s.len), s.via);
    }
    o += &acl_block(r);
    o += &mgmt_tail(&r.mgmt);
    o + "end"
}

/// The switch's configuration as commands the shell accepts back: the hostname, VLANs other than
/// VLAN 1, and every port that differs from the defaults (access, VLAN 1, all VLANs allowed).
pub fn switch_config(s: &L2Switch) -> String {
    let mut o = mgmt_head(&s.mgmt, &s.name);
    for (id, name) in s.vlans().filter(|(id, _)| *id != DEFAULT_VLAN) {
        o += &format!("vlan {id}\n");
        if name != format!("VLAN{id:04}") {
            o += &format!(" name {name}\n");
        }
        o += "!\n";
    }
    o += &spanning_config(s);
    o += &m3b::errdisable_lines(s);
    o += &disc::disc_global_lines(&s.disc);
    if s.l3().routing() {
        o += "ip routing\n!\n";
    }
    o += &dhcp_config(s.l3());
    let default = netdev::PortConfig::default();
    for (g, c) in s.groups() {
        o += &format!("interface Port-channel{g}\n");
        o += &port_body(c);
        o += "!\n";
    }
    for p in 0..s.n_ports() {
        let c = s.port(kernel::PortId(p)).expect("port in range");
        if *c == default && !s.disc.port_changed(p as usize) {
            continue;
        }
        o += &format!("interface FastEthernet0/{}\n", p + 1);
        o += &port_body(c);
        o += &disc::disc_port_lines(&s.disc, p as usize);
        o += "!\n";
    }
    let l3 = s.l3();
    for ifc in l3.interfaces() {
        let Some(v) = ifc.svi else { continue };
        o += &format!("interface Vlan{v}\n");
        match ifc.addr {
            Some((ip, len)) => o += &format!(" ip address {ip} {}\n", dotted(len)),
            None => o += " no ip address\n",
        }
        for h in &ifc.helpers {
            o += &format!(" ip helper-address {h}\n");
        }
        if let Some(n) = &ifc.acl_in {
            o += &format!(" ip access-group {n} in\n");
        }
        if let Some(n) = &ifc.acl_out {
            o += &format!(" ip access-group {n} out\n");
        }
        o += &ospf_if_lines(&ifc.ospf);
        if ifc.shutdown {
            o += " shutdown\n";
        }
        o += "!\n";
    }
    o += &ospf_config(l3);
    if let Some(gw) = l3.default_gateway() {
        o += &format!("ip default-gateway {gw}\n");
    }
    for r in l3.statics() {
        o += &format!("ip route {} {} {}\n", r.net, dotted(r.len), r.via);
    }
    o += &acl_block(l3);
    o += &mgmt_tail(&s.mgmt);
    o + "end"
}

/// The lines of a switch port or port-channel inside its `interface` block.
fn port_body(c: &netdev::PortConfig) -> String {
    let mut o = String::new();
    match c.mode {
        PortMode::Trunk => o += " switchport mode trunk\n",
        PortMode::Access => o += " switchport mode access\n",
        PortMode::DynamicDesirable => o += " switchport mode dynamic desirable\n",
        PortMode::DynamicAuto => {}
    }
    if c.nonegotiate {
        o += " switchport nonegotiate\n";
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
    o += &m3b::psec_lines(c);
    if c.shutdown {
        o += " shutdown\n";
    }
    if c.portfast {
        o += " spanning-tree portfast\n";
    }
    if let Some(cost) = c.stp_cost {
        o += &format!(" spanning-tree cost {cost}\n");
    }
    if let Some(p) = c.stp_priority {
        o += &format!(" spanning-tree port-priority {p}\n");
    }
    if let Some(m) = c.channel {
        o += &format!(" channel-group {} mode {}\n", m.group, m.mode.word());
    }
    o
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
            etherchannel::port_label(s, e.port.0 as usize)
        );
    }
    if table.is_empty() {
        o += "\n(no addresses learned)";
    }
    o
}

/// The global spanning tree lines of a running config.
fn spanning_config(s: &L2Switch) -> String {
    let c = s.spanning();
    let mut o = String::new();
    match c.mode {
        Some(StpMode::Stp) => o += "spanning-tree mode pvst\n",
        Some(StpMode::Rapid) => o += "spanning-tree mode rapid-pvst\n",
        None => {}
    }
    for v in &c.disabled {
        o += &format!("no spanning-tree vlan {v}\n");
    }
    for (v, p) in &c.priority {
        o += &format!("spanning-tree vlan {v} priority {p}\n");
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

/// `show spanning-tree`, for every VLAN with a tree or for one.
fn show_spanning_tree(s: &L2Switch, only: Option<u16>) -> String {
    let trees: Vec<(u16, &netdev::Stp)> = s
        .trees()
        .filter(|(v, _)| only.is_none_or(|o| o == *v))
        .collect();
    if trees.is_empty() {
        return match only {
            Some(v) => format!("Spanning tree instance(s) for vlan {v} does not exist."),
            None => NO_SPAN_INSTANCE.to_string(),
        };
    }
    let mut blocks = vec![];
    for (v, t) in trees {
        let hello = "Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec";
        let protocol = match t.mode() {
            StpMode::Stp => "ieee",
            StpMode::Rapid => "rstp",
        };
        let root = t.root();
        let mut o = format!("VLAN{v:04}\n  Spanning tree enabled protocol {protocol}\n");
        o += &format!(
            "  Root ID    Priority    {}\n             Address     {}\n",
            root.root.priority, root.root.mac
        );
        match t.root_port() {
            None => o += "             This bridge is the root\n",
            Some(rp) => {
                o += &format!(
                    "             Cost        {}\n             Port        {} ({})\n",
                    root.cost,
                    rp + 1,
                    match s.bundle_of(kernel::PortId(rp as u16)) {
                        Some((g, _)) => format!("Port-channel{g}"),
                        None => format!("FastEthernet0/{}", rp + 1),
                    }
                )
            }
        }
        o += &format!("             {hello}\n\n");
        let own = t.bridge();
        o += &format!(
            "  Bridge ID  Priority    {}  (priority {} sys-id-ext {v})\n             Address     {}\n             {hello}\n             Aging Time  {} sec\n\n",
            own.priority,
            own.priority - v,
            own.mac,
            s.aging.0 / 1_000_000_000
        );
        o += &format!(
            "{:<20}{:<5}{:<4}{:<10}{:<9}{}\n{:-<19} {:-<4} {:-<3} {:-<9} {:-<8} {:-<32}",
            "Interface", "Role", "Sts", "Cost", "Prio.Nbr", "Type", "", "", "", "", "", ""
        );
        for p in 0..t.n_ports() {
            let pv = t.port(p);
            let role = match pv.role {
                StpRole::Disabled => continue,
                StpRole::Root => "Root",
                StpRole::Designated => "Desg",
                StpRole::Alternate => "Altn",
                StpRole::Backup => "Back",
            };
            let sts = match pv.state {
                StpState::Discarding => "BLK",
                StpState::Listening => "LIS",
                StpState::Learning => "LRN",
                StpState::Forwarding => "FWD",
            };
            let kind = match (pv.edge, pv.compat) {
                (_, true) => "P2p Peer(STP)",
                (true, _) => "P2p Edge",
                _ => "P2p",
            };
            o += &format!(
                "\n{:<20}{:<5}{:<4}{:<10}{:<9}{}",
                etherchannel::port_label(s, p),
                role,
                sts,
                pv.cost,
                format!("{}.{}", pv.priority, p + 1),
                kind
            );
        }
        blocks.push(o);
    }
    blocks.join("\n\n")
}

fn show_vlan_brief(s: &L2Switch) -> String {
    let mut o = format!("{:<5}{:<33}{:<10}{}", "VLAN", "Name", "Status", "Ports");
    o += &format!("\n{:-<4} {:-<32} {:-<9} {:-<31}", "", "", "", "");
    for (id, name) in s.vlans() {
        let ports: Vec<String> = (0..s.n_ports())
            .filter(|p| {
                let c = s.port(kernel::PortId(*p)).expect("port in range");
                !s.is_trunk(kernel::PortId(*p))
                    && c.access_vlan == id
                    && s.bundle_of(kernel::PortId(*p)).is_none()
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

/// `show interfaces fa0/1 switchport`.
fn show_switchport(s: &L2Switch, p: usize) -> String {
    let c = s.port(kernel::PortId(p as u16)).expect("port in range");
    let admin = match c.mode {
        PortMode::Access => "static access",
        PortMode::Trunk => "trunk",
        PortMode::DynamicAuto => "dynamic auto",
        PortMode::DynamicDesirable => "dynamic desirable",
    };
    let trunk = s.is_trunk(kernel::PortId(p as u16));
    let vlan = |v: u16, name: &str| {
        if v == DEFAULT_VLAN {
            format!("{v} ({name})")
        } else {
            v.to_string()
        }
    };
    let allowed = if c.allowed == all_vlans() {
        "ALL".to_string()
    } else if c.allowed.is_empty() {
        "NONE".to_string()
    } else {
        format_vlan_list(&c.allowed)
    };
    format!(
        "Name: {}\nSwitchport: Enabled\nAdministrative Mode: {admin}\nOperational Mode: {}\n\
Administrative Trunking Encapsulation: dot1q\nOperational Trunking Encapsulation: {}\n\
Negotiation of Trunking: {}\nAccess Mode VLAN: {}\nTrunking Native Mode VLAN: {}\n\
Trunking VLANs Enabled: {allowed}",
        fa(p),
        if trunk { "trunk" } else { "static access" },
        if trunk { "dot1q" } else { "native" },
        if c.nonegotiate || c.mode == PortMode::Access {
            "Off"
        } else {
            "On"
        },
        vlan(c.access_vlan, "default"),
        vlan(c.native_vlan, "default"),
    )
}

fn show_trunks(s: &L2Switch) -> String {
    let trunks: Vec<(usize, &netdev::PortConfig)> = (0..s.n_ports())
        .filter_map(|p| s.port(kernel::PortId(p)).map(|c| (p as usize, c)))
        .filter(|(p, _)| s.is_trunk(kernel::PortId(*p as u16)) && etherchannel::listed(s, *p))
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
            etherchannel::port_label(s, *p),
            match c.mode {
                PortMode::DynamicAuto => "auto",
                PortMode::DynamicDesirable => "desirable",
                _ => "on",
            },
            "802.1q",
            "trunking",
            c.native_vlan
        );
    }
    o += &format!("\n\n{:<12}{}", "Port", "Vlans allowed on trunk");
    for (p, c) in &trunks {
        o += &format!(
            "\n{:<12}{}",
            etherchannel::port_label(s, *p),
            format_vlan_list(&c.allowed)
        );
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
        o += &format!(
            "\n{:<12}{}",
            etherchannel::port_label(s, *p),
            format_vlan_list(&active)
        );
    }
    o
}

fn show_routes(r: &Router) -> String {
    let mut o = String::from(if r.ospf.is_some() {
        "Codes: C connected, S static, O OSPF, IA OSPF inter area, E1/E2 OSPF external type 1/2\n"
    } else {
        "Codes: C connected, S static\n"
    });
    let mut any = false;
    for rt in r.fib().routes() {
        any = true;
        o += "\n";
        let net = format!("{}/{}", rt.net, rt.len);
        match (rt.kind, rt.via) {
            (RouteKind::Connected, _) => {
                o += &format!(
                    "C  {net}  directly connected, {}",
                    iface_name(r, rt.port.0 as usize)
                )
            }
            (RouteKind::Static, Some(v)) => {
                o += &format!("S  {net}  via {v}, {}", iface_name(r, rt.port.0 as usize))
            }
            (RouteKind::Static, None) => o += &format!("S  {net}"),
            (
                k @ (RouteKind::Ospf
                | RouteKind::OspfInter
                | RouteKind::OspfExt1
                | RouteKind::OspfExt2),
                v,
            ) => {
                let code = match k {
                    RouteKind::Ospf => "O",
                    RouteKind::OspfInter => "O IA",
                    RouteKind::OspfExt1 => "O E1",
                    _ => "O E2",
                };
                o += &format!(
                    "{code}  {net}  [110/{}] via {}, {}",
                    rt.metric,
                    v.map_or("?".to_string(), |v| v.to_string()),
                    iface_name(r, rt.port.0 as usize)
                )
            }
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
        // Down for lack of a working link layer (an SVI with no port in its VLAN), as opposed to
        // being shut down.
        let st = if r.is_up(i) {
            "up"
        } else if ifc.line_down && !ifc.shutdown {
            "down"
        } else {
            "admin down"
        };
        o += &format!("\n{:<22}{:<18}{}", iface_name(r, i), addr, st);
    }
    o
}

fn show_ipconfig(h: &Host, now: SimTime) -> String {
    let mut o = format!("Address : {}\nMask    : {}", h.ip, dotted(h.prefix_len));
    if let Some(g) = h.gateway {
        o += &format!("\nGateway : {g}");
    }
    if !h.dns.is_empty() {
        let dns: Vec<String> = h.dns.iter().map(Ipv4Addr::to_string).collect();
        o += &format!("\nDNS     : {}", dns.join(" "));
    }
    if h.dhcp.enabled {
        use netdev::DhcpState::*;
        o += "\nDHCP    : ";
        o += match h.dhcp.state {
            Off => "released",
            Selecting => "looking for a server",
            Requesting => "requesting an address",
            Bound => "bound",
            Renewing => "renewing",
            Rebinding => "rebinding",
        };
        if let Some((server, _)) = h.dhcp.server.filter(|_| h.ip != Ipv4Addr::UNSPECIFIED) {
            o += &format!(", server {server}");
        }
        if let Some(left) = h
            .dhcp_lease_left(now)
            .filter(|_| h.ip != Ipv4Addr::UNSPECIFIED)
        {
            o += &format!(", lease {} left", hms(left));
        }
    }
    o
}

/// `HH:MM:SS`; hours may pass 24.
fn hms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Client identifier bytes as the real command groups them: `0100.0000.0101`.
fn client_text(id: &[u8]) -> String {
    id.chunks(2)
        .map(|c| c.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .collect::<Vec<_>>()
        .join(".")
}

fn show_dhcp_bindings(rows: &[netdev::Binding], now: SimTime) -> String {
    let mut o = format!(
        "{:<16}{:<20}{:<20}{}",
        "IP address", "Client-ID", "Lease expires in", "Type"
    );
    for b in rows {
        let left = b.expires.0.saturating_sub(now.0) / 1_000_000_000;
        let kind = match b.state {
            netdev::BindingState::Offered => "Offered",
            netdev::BindingState::Bound => "Automatic",
        };
        o += &format!(
            "\n{:<16}{:<20}{:<20}{kind}",
            b.ip.to_string(),
            client_text(&b.client),
            hms(left)
        );
    }
    if rows.is_empty() {
        o += "\n(no bindings)";
    }
    o
}

fn lease_text(lease: SimTime) -> (u64, u64, u64) {
    let mins = lease.0 / 60_000_000_000;
    (mins / 1440, mins / 60 % 24, mins % 60)
}

fn show_dhcp_pools(r: &Router, used: &[netdev::Binding]) -> String {
    let mut out = Vec::new();
    for p in r.dhcp.pools() {
        let mut o = format!("Pool {} :", p.name);
        match p.network {
            Some((n, len)) => {
                let in_use = used
                    .iter()
                    .filter(|b| b.ip.to_u32() & mask(len) == n.to_u32())
                    .count();
                o += &format!("\n Network          : {n}/{len}");
                o += &format!("\n Addresses in use : {in_use}");
            }
            None => o += "\n Network          : (not set)",
        }
        if let Some(g) = p.default_router {
            o += &format!("\n Default router   : {g}");
        }
        if !p.dns.is_empty() {
            let dns: Vec<String> = p.dns.iter().map(Ipv4Addr::to_string).collect();
            o += &format!("\n DNS servers      : {}", dns.join(" "));
        }
        let (d, h, m) = lease_text(p.lease);
        o += &format!("\n Lease            : {d} days {h} hours {m} minutes");
        out.push(o);
    }
    if out.is_empty() {
        "(no pools)".to_string()
    } else {
        out.join("\n\n")
    }
}

/// The DHCP server's part of a config: `no service dhcp`, excluded ranges, then pools.
fn dhcp_config(r: &Router) -> String {
    let mut o = String::new();
    if !r.dhcp.enabled {
        o += "no service dhcp\n!\n";
    }
    for (lo, hi) in r.dhcp.excluded() {
        if lo == hi {
            o += &format!("ip dhcp excluded-address {lo}\n");
        } else {
            o += &format!("ip dhcp excluded-address {lo} {hi}\n");
        }
    }
    if !r.dhcp.excluded().is_empty() {
        o += "!\n";
    }
    for p in r.dhcp.pools() {
        o += &format!("ip dhcp pool {}\n", p.name);
        if let Some((n, len)) = p.network {
            o += &format!(" network {n} {}\n", dotted(len));
        }
        if let Some(g) = p.default_router {
            o += &format!(" default-router {g}\n");
        }
        if !p.dns.is_empty() {
            let dns: Vec<String> = p.dns.iter().take(3).map(Ipv4Addr::to_string).collect();
            o += &format!(" dns-server {}\n", dns.join(" "));
        }
        let (d, h, m) = lease_text(p.lease);
        if (d, h, m) != (1, 0, 0) {
            o += &match (h, m) {
                (0, 0) => format!(" lease {d}\n"),
                (h, 0) => format!(" lease {d} {h}\n"),
                (h, m) => format!(" lease {d} {h} {m}\n"),
            };
        }
        o += "!\n";
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
