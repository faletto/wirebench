use clish::{Action, Grammar, ModeId};

pub struct Modes {
    pub user: ModeId,
    pub privileged: ModeId,
    pub config: ModeId,
    pub config_if: ModeId,
}

pub const H_ENABLE: u32 = 1;
pub const H_CONF_T: u32 = 2;
pub const H_DISABLE: u32 = 3;
pub const H_HOSTNAME: u32 = 20;
pub const H_INTERFACE: u32 = 21;
pub const H_IP_ROUTE: u32 = 22;
pub const H_IP_ADDR: u32 = 30;
pub const H_SHUTDOWN: u32 = 31;
pub const H_NO_SHUTDOWN: u32 = 32;
pub const H_SHOW_RUN: u32 = 10;
pub const H_SHOW_ROUTE: u32 = 11;
pub const H_SHOW_BRIEF: u32 = 12;

/// A small router-flavoured grammar, written for these tests.
pub fn demo() -> (Grammar, Modes) {
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

    g.cmd(
        &[user],
        "enable",
        &["Become privileged"],
        Action::Handler(H_ENABLE),
    );
    g.cmd(
        &[user, privileged],
        "exit",
        &["Leave the session"],
        Action::Exit,
    );
    g.cmd(
        &[privileged],
        "disable",
        &["Drop privileges"],
        Action::Handler(H_DISABLE),
    );
    g.cmd(
        &[privileged],
        "configure terminal",
        &["Configure", "From this terminal"],
        Action::Handler(H_CONF_T),
    );
    g.cmd(
        &[privileged],
        "show running-config",
        &["Show", "Current configuration"],
        Action::Handler(H_SHOW_RUN),
    );
    g.cmd(
        &[privileged],
        "show ip route",
        &["Show", "IP information", "Routing table"],
        Action::Handler(H_SHOW_ROUTE),
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
        Action::Handler(H_SHOW_BRIEF),
    );
    g.cmd(
        &[privileged],
        "show interfaces",
        &["Show", "Interface details"],
        Action::Handler(13),
    );
    g.cmd(
        &[config],
        "hostname <name:word>",
        &["Set the device name", "New name"],
        Action::Handler(H_HOSTNAME),
    );
    g.cmd(
        &[config],
        "interface <name:word>",
        &["Configure an interface", "Interface name"],
        Action::Handler(H_INTERFACE),
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
        Action::Handler(H_IP_ROUTE),
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
        Action::Handler(H_IP_ADDR),
    );
    g.cmd(
        &[config_if],
        "shutdown",
        &["Disable the interface"],
        Action::Handler(H_SHUTDOWN),
    );
    g.cmd(
        &[config_if],
        "no shutdown",
        &["Negate", "Enable the interface"],
        Action::Handler(H_NO_SHUTDOWN),
    );
    (
        g,
        Modes {
            user,
            privileged,
            config,
            config_if,
        },
    )
}
