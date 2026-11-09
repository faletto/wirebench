//! The PC's command prompt: Windows-style commands, messages and prompt. There is no `?` help
//! and no modes; `help` lists what exists.

use super::*;

// Handler ids 470..479.
pub(super) const NETSH_STATIC: u32 = 470;
pub(super) const NETSH_STATIC_GW: u32 = 471;
pub(super) const HELP: u32 = 472;
pub(super) const CLS: u32 = 473;

/// Output that asks the terminal to clear its screen (what `cls` returns).
pub const CLEAR_SCREEN: &str = "\u{c}";

pub(super) const PROMPT: &str = "C:\\>";

const HELP_TEXT: &str = "\
IPCONFIG          Show the address, mask, gateway and DHCP state
                  IPCONFIG /ALL | /RELEASE | /RENEW
NETSH INTERFACE IP SET ADDRESS name STATIC address mask [gateway]
                  Set the address by hand
NETSH INTERFACE IP SET ADDRESS name DHCP
                  Get the address from a DHCP server
NETSH INTERFACE IP SHOW CONFIG
                  Show the address, as IPCONFIG does
PING address      Send echo requests
TRACERT [-h max] address
                  Trace the route to an address
TELNET address    Open a telnet connection
SSH -l user address
                  Open an SSH connection
CLS               Clear the screen
HELP              This list
EXIT              Close the window";

/// What a new window prints first.
pub const GREETING: &str = "wirebench PC command prompt. Type HELP for a list of commands.";

pub(super) fn pc_commands(g: &mut Grammar, m: ModeId) {
    let h = Action::Handler;
    let cmds: &[(&str, &[&str], u32)] = &[
        ("ipconfig /all", &["Show addressing", "In full"], IPCONFIG),
        (
            "ipconfig /release",
            &["Addressing", "Give the DHCP address back"],
            IPCONFIG_RELEASE,
        ),
        (
            "ipconfig /renew",
            &["Addressing", "Renew the DHCP lease"],
            IPCONFIG_RENEW,
        ),
        (
            "netsh interface ip set address <name:word> static <addr:ipv4> <mask:mask>",
            &[
                "Network shell",
                "Interfaces",
                "IP",
                "Set",
                "Address",
                "Interface name",
                "Fixed address",
                "Address",
                "Network mask",
            ],
            NETSH_STATIC,
        ),
        (
            "netsh interface ip set address <name:word> static <addr:ipv4> <mask:mask> <gw:ipv4>",
            &[
                "Network shell",
                "Interfaces",
                "IP",
                "Set",
                "Address",
                "Interface name",
                "Fixed address",
                "Address",
                "Network mask",
                "Default gateway",
            ],
            NETSH_STATIC_GW,
        ),
        (
            "netsh interface ip set address <name:word> dhcp",
            &[
                "Network shell",
                "Interfaces",
                "IP",
                "Set",
                "Address",
                "Interface name",
                "Get the address from a DHCP server",
            ],
            IPCONFIG_DHCP,
        ),
        (
            "netsh interface ip show config",
            &["Network shell", "Interfaces", "IP", "Show", "Addressing"],
            IPCONFIG,
        ),
        ("help", &["List the commands"], HELP),
        ("cls", &["Clear the screen"], CLS),
    ];
    for (pattern, help, id) in cmds {
        g.cmd(&[m], pattern, help, h(*id));
    }
}

impl Shell {
    pub(super) fn pc_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        match handler {
            NETSH_STATIC | NETSH_STATIC_GW => {
                let ip = Ipv4Addr(a.ipv4("addr").unwrap());
                let len = a.mask("mask").unwrap();
                let gw = a.ipv4("gw").map(Ipv4Addr);
                w.with_device::<Host, _>(self.device, |h, _| {
                    h.dhcp_disable();
                    h.ip = ip;
                    h.prefix_len = len;
                    h.gateway = gw;
                });
                String::new()
            }
            HELP => HELP_TEXT.to_string(),
            CLS => CLEAR_SCREEN.to_string(),
            _ => String::new(),
        }
    }
}

/// Commands at a command prompt do not care about case. Everything is lowered except the user
/// name after `-l` (names are case-sensitive).
pub(super) fn normalise(line: &str) -> String {
    let mut after_l = false;
    line.split(' ')
        .map(|t| {
            let keep = after_l;
            after_l = t == "-l" || t == "-L";
            if keep {
                t.to_string()
            } else {
                t.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Windows' wording for a line the grammar rejected.
pub(super) fn error_text(line: &str, e: &ParseError) -> String {
    let first = line.split_whitespace().next().unwrap_or("");
    let at_first_word = line.len() - line.trim_start().len() == e.caret;
    if e.kind == ErrorKind::Invalid && at_first_word {
        format!("'{first}' is not recognized as an internal or external command,\noperable program or batch file.")
    } else {
        "The syntax of the command is incorrect.".to_string()
    }
}
