//! Names: `ip name-server`, `ip host`, `ip domain-lookup`, `ip dns server`, `show hosts` on routers
//! and switches; `nslookup`, `netsh ... set dns` and the `dnsd` server on PCs. A host name where
//! `ping`, `traceroute`, `tracert`, `telnet` or `ssh` expects an address is looked up first and
//! replaced by the address. The lookup runs inside the command, letting simulated time pass until
//! an answer arrives. Handler ids 560..579.

use super::tftp::{udp_of, with_udp};
use super::*;
use netdev::{dns_canonical, DNS_MAX_SERVERS};
use wire::{DnsData, DnsMessage, DNS_NXDOMAIN, DNS_OK, DNS_PORT};

pub(super) const NAME_SERVER: u32 = 560;
pub(super) const NO_NAME_SERVER: u32 = 561;
pub(super) const DOMAIN_LOOKUP: u32 = 562;
pub(super) const NO_DOMAIN_LOOKUP: u32 = 563;
pub(super) const IP_HOST: u32 = 564;
pub(super) const NO_IP_HOST: u32 = 565;
pub(super) const DNS_SERVER_ON: u32 = 566;
pub(super) const DNS_SERVER_OFF: u32 = 567;
pub(super) const SHOW_HOSTS: u32 = 568;
pub(super) const NSLOOKUP: u32 = 569;
pub(super) const NSLOOKUP_AT: u32 = 570;
pub(super) const NETSH_DNS: u32 = 571;
pub(super) const DNSD_START: u32 = 572;
pub(super) const DNSD_STOP: u32 = 573;
pub(super) const DNSD_ADD: u32 = 574;
pub(super) const DNSD_DELETE: u32 = 575;
pub(super) const DNSD_LIST: u32 = 576;

const NO_HOST: &str = "% Unrecognized host or address, or protocol not running.";
const TOO_MANY_SERVERS: &str = "% At most six name servers.";
const NO_SUCH_SERVER: &str = "% That name server is not configured.";
const NO_SUCH_HOST: &str = "% That host is not in the table.";
const BAD_NAME: &str =
    "% Invalid input detected: a host name is letters, digits, dots and hyphens.";
/// How long to wait for an answer, and how many times a server is asked.
const WAIT: u64 = 2;
const TRIES: u32 = 2;
const FIRST_PORT: u16 = 49600;

pub(super) fn dns_commands(g: &mut Grammar, user: ModeId, privileged: ModeId, config: ModeId) {
    let h = Action::Handler;
    let ip = "IP configuration subcommands";
    let cmds: &[(&[ModeId], &str, &[&str], u32)] = &[
        (
            &[config],
            "ip name-server <a:ipv4>",
            &[ip, "Name server for host names", "Its address"],
            NAME_SERVER,
        ),
        (
            &[config],
            "no ip name-server <a:ipv4>",
            &["Negate", ip, "Name server", "Its address"],
            NO_NAME_SERVER,
        ),
        (
            &[config],
            "ip domain-lookup",
            &[ip, "Look up host names with a name server"],
            DOMAIN_LOOKUP,
        ),
        (
            &[config],
            "no ip domain-lookup",
            &["Negate", ip, "Look up host names with a name server"],
            NO_DOMAIN_LOOKUP,
        ),
        (
            &[config],
            "ip host <name:word> <addr:ipv4>",
            &[ip, "A name and its address", "The name", "Its address"],
            IP_HOST,
        ),
        (
            &[config],
            "no ip host <name:word>",
            &["Negate", ip, "A name and its address", "The name"],
            NO_IP_HOST,
        ),
        (
            &[config],
            "ip dns server",
            &[ip, "DNS", "Answer queries from the host table"],
            DNS_SERVER_ON,
        ),
        (
            &[config],
            "no ip dns server",
            &["Negate", ip, "DNS", "Answer queries from the host table"],
            DNS_SERVER_OFF,
        ),
        (
            &[user, privileged],
            "show hosts",
            &[
                "Show",
                "IP domain-name, lookup style, name servers, and host table",
            ],
            SHOW_HOSTS,
        ),
    ];
    for (modes, line, help, id) in cmds {
        g.cmd(modes, line, help, h(*id));
    }
}

pub(super) fn pc_dns_commands(g: &mut Grammar, m: ModeId) {
    let h = Action::Handler;
    let cmds: &[(&str, &[&str], u32)] = &[
        (
            "nslookup <name:word>",
            &["Look up a name", "Name"],
            NSLOOKUP,
        ),
        (
            "nslookup <name:word> <server:ipv4>",
            &["Look up a name", "Name", "Ask this server"],
            NSLOOKUP_AT,
        ),
        (
            "netsh interface ip set dns <if:word> static <addr:ipv4>",
            &[
                "Network shell",
                "Interfaces",
                "IP",
                "Set",
                "DNS server",
                "Interface name",
                "Fixed address",
                "Address",
            ],
            NETSH_DNS,
        ),
        (
            "dnsd start",
            &["DNS server", "Start listening on UDP 53"],
            DNSD_START,
        ),
        ("dnsd stop", &["DNS server", "Stop listening"], DNSD_STOP),
        (
            "dnsd add <name:word> <addr:ipv4>",
            &["DNS server", "Add a name", "The name", "Its address"],
            DNSD_ADD,
        ),
        (
            "dnsd delete <name:word>",
            &["DNS server", "Delete a name", "The name"],
            DNSD_DELETE,
        ),
        (
            "dnsd list",
            &["DNS server", "List the names it holds"],
            DNSD_LIST,
        ),
    ];
    for (pattern, help, id) in cmds {
        g.cmd(&[m], pattern, help, h(*id));
    }
}

/// The running-config lines that belong to name resolution.
pub(super) fn dns_lines(d: &netdev::DnsResolver) -> String {
    let mut o = String::new();
    if !d.lookup {
        o += "no ip domain-lookup\n";
    }
    for s in &d.servers {
        o += &format!("ip name-server {s}\n");
    }
    for (name, addr) in &d.hosts {
        o += &format!("ip host {name} {addr}\n");
    }
    if d.serve {
        o += "ip dns server\n";
    }
    if !o.is_empty() {
        o += "!\n";
    }
    o
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 253
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !n.starts_with(['.', '-'])
}

/// What one server said.
enum Said {
    Addr(Ipv4Addr),
    /// The name does not exist (or has no address).
    No,
    /// No answer, or one that cannot be used.
    Silent,
}

/// Ask `server` for the address of `name`.
fn ask(w: &mut World, kind: Kind, dev: DeviceId, server: Ipv4Addr, name: &str) -> Said {
    let mut port = FIRST_PORT;
    while !with_udp(w, kind, dev, |t, _, _| t.udp_bind(port)) {
        port += 1;
    }
    let id = (w.kernel.now().0 / 1000) as u16 ^ port;
    let query = DnsMessage::query(id, name).emit();
    let mut said = Said::Silent;
    'tries: for _ in 0..TRIES {
        if !with_udp(w, kind, dev, |_, send, c| {
            send(c, server, port, DNS_PORT, &query)
        }) {
            break;
        }
        let deadline = w.kernel.now() + SimTime::from_secs(WAIT);
        loop {
            w.run_until_or(deadline, |w| {
                udp_of(w, kind, dev)
                    .udp
                    .get(&port)
                    .is_some_and(|s| !s.inbox.is_empty())
            });
            let Some((src, _, data)) = with_udp(w, kind, dev, |t, _, _| t.udp_recv(port)) else {
                break;
            };
            let Ok(m) = DnsMessage::parse(&data) else {
                continue;
            };
            if src != server || !m.response || m.id != id {
                continue;
            }
            said = match m.rcode {
                DNS_OK => m
                    .answers
                    .iter()
                    .find_map(|r| match r.data {
                        DnsData::A(a) => Some(Said::Addr(a)),
                        _ => None,
                    })
                    .unwrap_or(Said::No),
                DNS_NXDOMAIN => Said::No,
                _ => Said::Silent,
            };
            break 'tries;
        }
    }
    with_udp(w, kind, dev, |t, _, _| t.udp_close(port));
    said
}

/// How a lookup came out.
enum Lookup {
    Found {
        addr: Ipv4Addr,
        via: Option<Ipv4Addr>,
    },
    NotFound,
    NoAnswer,
}

impl Shell {
    fn resolver_servers(&self, w: &World) -> Vec<Ipv4Addr> {
        match self.kind {
            Kind::Host => w.device::<Host>(self.device).dns.clone(),
            _ => self.l3(w).dns.servers.clone(),
        }
    }

    /// The names to try for `name`: as typed if it has a dot, else with the default domain.
    fn candidates(&self, w: &World, name: &str) -> Vec<String> {
        let name = dns_canonical(name);
        let domain = self.mgmt(w).and_then(|m| m.domain_name.clone());
        match domain {
            Some(d) if !name.contains('.') => vec![format!("{name}.{}", dns_canonical(&d)), name],
            _ => vec![name],
        }
    }

    fn lookup(&self, w: &mut World, name: &str) -> Lookup {
        let (kind, dev) = (self.kind, self.device);
        let names = self.candidates(w, name);
        if kind != Kind::Host {
            for n in &names {
                if let Some(a) = self.l3(w).dns.hosts.get(n) {
                    return Lookup::Found {
                        addr: *a,
                        via: None,
                    };
                }
            }
            if !self.l3(w).dns.lookup {
                return Lookup::NotFound;
            }
        }
        let servers = self.resolver_servers(w);
        if servers.is_empty() {
            return Lookup::NotFound;
        }
        let mut silent = true;
        for n in &names {
            for s in &servers {
                match ask(w, kind, dev, *s, n) {
                    Said::Addr(addr) => {
                        return Lookup::Found {
                            addr,
                            via: Some(*s),
                        }
                    }
                    Said::No => {
                        silent = false;
                        break;
                    }
                    Said::Silent => {}
                }
            }
        }
        if silent {
            Lookup::NoAnswer
        } else {
            Lookup::NotFound
        }
    }

    /// If `line` is `ping`, `traceroute`, `tracert`, `telnet` or `ssh -l` aimed at a host name,
    /// look it up. Ok(None) when the line has no name in it, Ok(Some((line with the address,
    /// text to print first))) when it had, Err(text) when the name did not resolve.
    pub(super) fn resolve_in_line(
        &self,
        w: &mut World,
        line: &str,
    ) -> Result<Option<(String, String)>, String> {
        let mut t: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let m = &self.modes;
        let mode = self.session.mode();
        if self.kind != Kind::Host && mode != m.user && mode != m.privileged {
            return Ok(None);
        }
        let Some(cmd) = t.first().map(|c| c.to_ascii_lowercase()) else {
            return Ok(None);
        };
        let at = match (cmd.as_str(), t.len()) {
            ("ping" | "telnet", 2) => 1,
            ("traceroute", 2 | 5) => 1,
            ("tracert", 2) => 1,
            ("tracert", 4) if t[1].eq_ignore_ascii_case("-h") => 3,
            ("ssh", 4) if t[1].eq_ignore_ascii_case("-l") => 3,
            _ => return Ok(None),
        };
        let name = t[at].clone();
        // Digits and dots alone are a mistyped address, not a name.
        let numeric = name.chars().all(|c| c.is_ascii_digit() || c == '.');
        if numeric || name.contains('?') || !valid_name(&name) {
            return Ok(None);
        }
        let host = self.kind == Kind::Host;
        match self.lookup(w, &name) {
            Lookup::Found { addr, via } => {
                t[at] = addr.to_string();
                let note = match (host, via, cmd.as_str()) {
                    (true, _, "ping") => format!("Pinging {name} [{addr}]:"),
                    (true, _, "tracert") => format!("Tracing route to {name} [{addr}]"),
                    (true, _, _) => String::new(),
                    (false, Some(s), _) => {
                        format!("Translating \"{name}\"...domain server ({s}) [OK]")
                    }
                    (false, None, _) => String::new(),
                };
                Ok(Some((t.join(" "), note)))
            }
            Lookup::NotFound | Lookup::NoAnswer => Err(match (host, cmd.as_str()) {
                (true, "ping") => format!(
                    "Ping request could not find host {name}. Please check the name and try again."
                ),
                (true, "tracert") => format!("Unable to resolve target system name {name}."),
                (true, _) => format!("Could not resolve host {name}."),
                (false, _) => {
                    let servers = self.resolver_servers(w);
                    match servers.first() {
                        Some(s) if self.l3(w).dns.lookup => {
                            format!("Translating \"{name}\"...domain server ({s})\n{NO_HOST}")
                        }
                        _ => NO_HOST.to_string(),
                    }
                }
            }),
        }
    }

    fn show_hosts(&self, w: &World) -> String {
        let r = &self.l3(w).dns;
        let domain = self.mgmt(w).and_then(|m| m.domain_name.clone());
        let mut o = match domain {
            Some(d) => format!("Default domain is {d}"),
            None => "Default domain is not set".to_string(),
        };
        o += if r.lookup {
            "\nName/address lookup uses domain service"
        } else {
            "\nName/address lookup is disabled"
        };
        if r.servers.is_empty() {
            o += "\nName servers are not set";
        } else {
            let s: Vec<String> = r.servers.iter().map(Ipv4Addr::to_string).collect();
            o += &format!("\nName servers are {}", s.join(", "));
        }
        o += "\n\nCodes: UN - unknown, EX - expired, OK - OK, ?? - revalidate\n       temp - temporary, perm - permanent\n       NA - Not Applicable None - Not defined\n\nHost                      Port Flags      Age Type   Address(es)";
        for (name, addr) in &r.hosts {
            o += &format!("\n{name:<25} None (perm, OK)  0   IP      {addr}");
        }
        o
    }

    pub(super) fn dns_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        match handler {
            NAME_SERVER => {
                let s = Ipv4Addr(a.ipv4("a").unwrap());
                let full = self.with_l3(w, |r| {
                    if r.dns.servers.contains(&s) {
                        false
                    } else if r.dns.servers.len() >= DNS_MAX_SERVERS {
                        true
                    } else {
                        r.dns.servers.push(s);
                        false
                    }
                });
                if full {
                    return TOO_MANY_SERVERS.to_string();
                }
            }
            NO_NAME_SERVER => {
                let s = Ipv4Addr(a.ipv4("a").unwrap());
                if !self.with_l3(w, |r| {
                    let n = r.dns.servers.len();
                    r.dns.servers.retain(|x| *x != s);
                    r.dns.servers.len() != n
                }) {
                    return NO_SUCH_SERVER.to_string();
                }
            }
            DOMAIN_LOOKUP => self.with_l3(w, |r| r.dns.lookup = true),
            NO_DOMAIN_LOOKUP => self.with_l3(w, |r| r.dns.lookup = false),
            IP_HOST => {
                let name = a.word("name").unwrap_or_default();
                if !valid_name(name) {
                    return BAD_NAME.to_string();
                }
                let (name, addr) = (dns_canonical(name), Ipv4Addr(a.ipv4("addr").unwrap()));
                self.with_l3(w, |r| r.dns.hosts.insert(name, addr));
            }
            NO_IP_HOST => {
                let name = dns_canonical(a.word("name").unwrap_or_default());
                if self.with_l3(w, |r| r.dns.hosts.remove(&name)).is_none() {
                    return NO_SUCH_HOST.to_string();
                }
            }
            DNS_SERVER_ON => self.with_l3(w, |r| r.dns.serve = true),
            DNS_SERVER_OFF => self.with_l3(w, |r| r.dns.serve = false),
            SHOW_HOSTS => return self.show_hosts(w),
            _ => return self.pc_dns_cmd(w, handler, a),
        }
        String::new()
    }

    fn pc_dns_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        match handler {
            NSLOOKUP | NSLOOKUP_AT => {
                let name = a.word("name").unwrap_or_default().to_string();
                let servers = match a.ipv4("server") {
                    Some(s) => vec![Ipv4Addr(s)],
                    None => self.resolver_servers(w),
                };
                let Some(first) = servers.first().copied() else {
                    return "*** Default servers are not available".to_string();
                };
                let head = format!("Server:  UnKnown\nAddress:  {first}\n");
                let full = self.candidates(w, &name).remove(0);
                match ask(w, Kind::Host, id, first, &full) {
                    Said::Addr(addr) => format!("{head}\nName:    {full}\nAddress:  {addr}"),
                    Said::No => format!("{head}\n*** UnKnown can't find {name}: Non-existent domain"),
                    Said::Silent => format!("DNS request timed out.\n    timeout was {WAIT} seconds.\n{head}\n*** Request to UnKnown timed-out"),
                }
            }
            NETSH_DNS => {
                let addr = Ipv4Addr(a.ipv4("addr").unwrap());
                w.with_device::<Host, _>(id, |h, _| h.dns = vec![addr]);
                String::new()
            }
            DNSD_START => {
                w.with_device::<Host, _>(id, |h, _| h.dnsd.running = true);
                "DNS server listening on UDP port 53.".to_string()
            }
            DNSD_STOP => {
                w.with_device::<Host, _>(id, |h, _| h.dnsd.running = false);
                String::new()
            }
            DNSD_ADD => {
                let name = a.word("name").unwrap_or_default();
                if !valid_name(name) {
                    return BAD_NAME.to_string();
                }
                let (name, addr) = (dns_canonical(name), Ipv4Addr(a.ipv4("addr").unwrap()));
                w.with_device::<Host, _>(id, |h, _| h.dnsd.names.insert(name, addr));
                String::new()
            }
            DNSD_DELETE => {
                let name = dns_canonical(a.word("name").unwrap_or_default());
                if w.with_device::<Host, _>(id, |h, _| h.dnsd.names.remove(&name))
                    .is_some()
                {
                    String::new()
                } else {
                    format!("The name {name} does not exist.")
                }
            }
            _ => {
                let d = &w.device::<Host>(id).dnsd;
                let mut o = format!(
                    "DNS server is {}. {} name{}, {} quer{} answered.",
                    if d.running { "running" } else { "stopped" },
                    d.names.len(),
                    if d.names.len() == 1 { "" } else { "s" },
                    d.queries,
                    if d.queries == 1 { "y" } else { "ies" }
                );
                for (n, a) in &d.names {
                    o += &format!("\n{a:<15} {n}");
                }
                o
            }
        }
    }
}
