//! Remote login: `telnet` and `ssh` clients on hosts, routers and switches, and the vty login
//! (usernames, `login local`, `transport input`, `crypto key generate rsa`) on routers and
//! switches.
//!
//! A remote session is a real TCP connection between two simulated devices, so ACLs, routing,
//! lost cables and `show tcp brief` all behave. The far device's terminal is a [`Shell`] kept in
//! the client's shell (`Shell::remote`): the client's lines travel over the connection, are run
//! by that shell, and its output travels back. The far device only does anything while a client
//! is typing at it, which is the only time anyone can see.
//!
//! One shortcut: the far shell's *prompt* is read directly rather than sent as text, as the
//! rest of the terminal handles prompts separately from output.

use super::*;
use netdev::{Tcp, TcpFailure, TcpState, TELNET_PORT, User, SSH_PORT};
use wire::{
    ssh_ident, ssh_parse_ident, telnet_escape, SshDecoder, SshIdent, SshPacket, TelnetDecoder,
    TelnetOptions, TELNET_ECHO, TELNET_SGA, SSH_MSG_CHANNEL_DATA, SSH_MSG_DISCONNECT,
    SSH_MSG_USERAUTH_FAILURE, SSH_MSG_USERAUTH_REQUEST, SSH_MSG_USERAUTH_SUCCESS,
};

// Handler ids 430..469.
pub(super) const USERNAME_SECRET: u32 = 430;
pub(super) const USERNAME_PASSWORD: u32 = 431;
pub(super) const USERNAME_PRIV_SECRET: u32 = 432;
pub(super) const USERNAME_PRIV_PASSWORD: u32 = 433;
pub(super) const NO_USERNAME: u32 = 434;
pub(super) const DOMAIN_NAME: u32 = 435;
pub(super) const NO_DOMAIN_NAME: u32 = 436;
pub(super) const KEYGEN: u32 = 437;
pub(super) const KEYGEN_BITS: u32 = 438;
pub(super) const KEYZEROIZE: u32 = 439;
pub(super) const LOGIN_LOCAL: u32 = 440;
pub(super) const TRANSPORT_TELNET: u32 = 441;
pub(super) const TRANSPORT_SSH: u32 = 442;
pub(super) const TRANSPORT_ALL: u32 = 443;
pub(super) const TRANSPORT_NONE: u32 = 444;
pub(super) const IP_SSH_VERSION: u32 = 445;
pub(super) const TELNET: u32 = 446;
pub(super) const SSH: u32 = 447;
pub(super) const SHOW_IP_SSH: u32 = 448;
pub(super) const SHOW_TCP_BRIEF: u32 = 449;

pub(super) const USERNAME_PROMPT: &str = "Username: ";
const NO_ROUTE: &str = "% Destination unreachable; gateway or host down";
const REFUSED: &str = "% Connection refused by remote host";
const TIMED_OUT_REMOTE: &str = "% Connection timed out; remote host not responding";
const NEED_DOMAIN: &str = "% Please define a domain-name first.";
const AUTH_FAILED: &str = "% Authentication failed.";
const NO_PASSWORD_SET: &str = "Password required, but none set";
const VTY_ONLY: &str = "% This applies to the vty lines.";
const BAD_SSH_SERVER: &str = "% Protocol mismatch: the far end did not answer as an SSH server.";
const V1_ONLY: &str = "% SSH protocol version 1 is not supported.";

pub(super) fn remote_commands(
    g: &mut Grammar,
    user: ModeId,
    privileged: ModeId,
    config: ModeId,
    line: ModeId,
) {
    let h = Action::Handler;
    let cfg: &[(&str, &[&str], u32)] = &[
        (
            "username <name:word> secret <pw:word>",
            &["Local user", "Name", "Hashed password", "Password"],
            USERNAME_SECRET,
        ),
        (
            "username <name:word> password <pw:word>",
            &["Local user", "Name", "Password as typed", "Password"],
            USERNAME_PASSWORD,
        ),
        (
            "username <name:word> privilege <lvl:int(0..15)> secret <pw:word>",
            &[
                "Local user",
                "Name",
                "Privilege level",
                "Level",
                "Hashed password",
                "Password",
            ],
            USERNAME_PRIV_SECRET,
        ),
        (
            "username <name:word> privilege <lvl:int(0..15)> password <pw:word>",
            &[
                "Local user",
                "Name",
                "Privilege level",
                "Level",
                "Password as typed",
                "Password",
            ],
            USERNAME_PRIV_PASSWORD,
        ),
        (
            "no username <name:word>",
            &["Negate", "Local user", "Name"],
            NO_USERNAME,
        ),
        (
            "ip domain-name <name:word>",
            &["IP", "Default domain name", "Name"],
            DOMAIN_NAME,
        ),
        (
            "no ip domain-name",
            &["Negate", "IP", "Default domain name"],
            NO_DOMAIN_NAME,
        ),
        (
            "crypto key generate rsa",
            &["Cryptography", "Keys", "Generate", "RSA key pair"],
            KEYGEN,
        ),
        (
            "crypto key generate rsa modulus <bits:int(360..4096)>",
            &["Cryptography", "Keys", "Generate", "RSA key pair", "Size", "Bits"],
            KEYGEN_BITS,
        ),
        (
            "crypto key zeroize rsa",
            &["Cryptography", "Keys", "Remove", "The RSA key pair"],
            KEYZEROIZE,
        ),
        (
            "ip ssh version 2",
            &["IP", "SSH", "Protocol version", "Version 2"],
            IP_SSH_VERSION,
        ),
    ];
    for (pattern, help, id) in cfg {
        g.cmd(&[config], pattern, help, h(*id));
    }
    let lines: &[(&str, &[&str], u32)] = &[
        (
            "login local",
            &["Ask for a username and password", "Use the local usernames"],
            LOGIN_LOCAL,
        ),
        (
            "transport input telnet",
            &["Line transport", "Incoming connections", "Telnet only"],
            TRANSPORT_TELNET,
        ),
        (
            "transport input ssh",
            &["Line transport", "Incoming connections", "SSH only"],
            TRANSPORT_SSH,
        ),
        (
            "transport input all",
            &["Line transport", "Incoming connections", "Telnet and SSH"],
            TRANSPORT_ALL,
        ),
        (
            "transport input none",
            &["Line transport", "Incoming connections", "No incoming connections"],
            TRANSPORT_NONE,
        ),
    ];
    for (pattern, help, id) in lines {
        g.cmd(&[line], pattern, help, h(*id));
    }
    g.cmd(
        &[user, privileged],
        "telnet <dst:ipv4>",
        &["Open a telnet connection", "Address of the device"],
        h(TELNET),
    );
    g.cmd(
        &[user, privileged],
        "ssh -l <user:word> <dst:ipv4>",
        &[
            "Open an SSH connection",
            "Log in as",
            "User name",
            "Address of the device",
        ],
        h(SSH),
    );
    g.cmd(
        &[user, privileged],
        "show ip ssh",
        &["Show", "IP", "SSH server state"],
        h(SHOW_IP_SSH),
    );
    g.cmd(
        &[user, privileged],
        "show tcp brief",
        &["Show", "TCP", "Connections, one line each"],
        h(SHOW_TCP_BRIEF),
    );
}

/// The host's half: it can open sessions but has no vty lines.
pub(super) fn host_remote_commands(g: &mut Grammar, m: ModeId) {
    let h = Action::Handler;
    g.cmd(
        &[m],
        "telnet <dst:ipv4>",
        &["Open a telnet connection", "Address of the device"],
        h(TELNET),
    );
    g.cmd(
        &[m],
        "ssh -l <user:word> <dst:ipv4>",
        &[
            "Open an SSH connection",
            "Log in as",
            "User name",
            "Address of the device",
        ],
        h(SSH),
    );
}

// ---- sockets, whatever the device ----

/// What `Host` and `Router` both offer; a switch gives its router half.
trait Net {
    fn open(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, port: u16) -> Option<u32>;
    fn write(&mut self, ctx: &mut Ctx, conn: u32, data: &[u8]) -> bool;
    fn take(&mut self, ctx: &mut Ctx, conn: u32) -> Vec<u8>;
    fn shut(&mut self, ctx: &mut Ctx, conn: u32);
    fn abort(&mut self, ctx: &mut Ctx, conn: u32);
}

impl Net for Host {
    fn open(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, port: u16) -> Option<u32> {
        self.tcp_open(ctx, dst, port)
    }
    fn write(&mut self, ctx: &mut Ctx, conn: u32, data: &[u8]) -> bool {
        self.tcp_write(ctx, conn, data)
    }
    fn take(&mut self, ctx: &mut Ctx, conn: u32) -> Vec<u8> {
        self.tcp_take(ctx, conn)
    }
    fn shut(&mut self, ctx: &mut Ctx, conn: u32) {
        self.tcp_shut(ctx, conn)
    }
    fn abort(&mut self, ctx: &mut Ctx, conn: u32) {
        self.tcp_abort(ctx, conn)
    }
}

impl Net for Router {
    fn open(&mut self, ctx: &mut Ctx, dst: Ipv4Addr, port: u16) -> Option<u32> {
        self.tcp_open(ctx, dst, port)
    }
    fn write(&mut self, ctx: &mut Ctx, conn: u32, data: &[u8]) -> bool {
        self.tcp_write(ctx, conn, data)
    }
    fn take(&mut self, ctx: &mut Ctx, conn: u32) -> Vec<u8> {
        self.tcp_take(ctx, conn)
    }
    fn shut(&mut self, ctx: &mut Ctx, conn: u32) {
        self.tcp_shut(ctx, conn)
    }
    fn abort(&mut self, ctx: &mut Ctx, conn: u32) {
        self.tcp_abort(ctx, conn)
    }
}


#[derive(Clone, Copy, Debug)]
struct Endpoint {
    dev: DeviceId,
    kind: Kind,
    conn: u32,
}

fn with_net<R>(
    w: &mut World,
    kind: Kind,
    dev: DeviceId,
    f: impl FnOnce(&mut dyn Net, &mut Ctx) -> R,
) -> R {
    match kind {
        Kind::Host => w.with_device::<Host, _>(dev, |h, c| f(h, c)),
        Kind::Router => w.with_device::<Router, _>(dev, |r, c| {
            let x = f(r, c);
            r.flush_outbox(c);
            x
        }),
        Kind::Switch => w.with_device::<L2Switch, _>(dev, |s, c| s.originate(c, |r, c| f(r, c))),
    }
}

fn tcp_of(w: &World, kind: Kind, dev: DeviceId) -> &Tcp {
    match kind {
        Kind::Host => &w.device::<Host>(dev).tcp,
        Kind::Router => &w.device::<Router>(dev).tcp,
        Kind::Switch => &w.device::<L2Switch>(dev).l3().tcp,
    }
}

impl Endpoint {
    fn write(&self, w: &mut World, data: &[u8]) {
        let conn = self.conn;
        with_net(w, self.kind, self.dev, |n, c| n.write(c, conn, data));
    }
    fn take(&self, w: &mut World) -> Vec<u8> {
        let conn = self.conn;
        with_net(w, self.kind, self.dev, |n, c| n.take(c, conn))
    }
    fn shut(&self, w: &mut World) {
        let conn = self.conn;
        with_net(w, self.kind, self.dev, |n, c| n.shut(c, conn));
    }
    fn abort(&self, w: &mut World) {
        let conn = self.conn;
        with_net(w, self.kind, self.dev, |n, c| n.abort(c, conn));
    }
    /// Whether the connection is gone or the other end has said goodbye.
    fn ended(&self, w: &World) -> bool {
        match tcp_of(w, self.kind, self.dev).conn(self.conn) {
            None => true,
            Some(c) => c.state == TcpState::Closed || c.peer_closed,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Proto {
    Telnet,
    Ssh,
}

/// The far end of a session, held by the shell that opened it.
#[derive(Clone)]
pub(super) struct Remote {
    proto: Proto,
    addr: Ipv4Addr,
    client: Endpoint,
    server: Endpoint,
    /// The terminal on the far device.
    pub(super) shell: Box<Shell>,
    c_tel: TelnetDecoder,
    c_opts: TelnetOptions,
    s_tel: TelnetDecoder,
    s_opts: TelnetOptions,
    c_ssh: SshDecoder,
    s_ssh: SshDecoder,
    /// Server side: input after the last line end.
    partial: Vec<u8>,
    /// Server side: SSH authentication has succeeded.
    authed: bool,
    server_done: bool,
}

fn to_wire(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n").into_bytes()
}

fn from_wire(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| *c != '\r' && *c != '\0')
        .collect()
}

impl Remote {
    fn client_send(&self, w: &mut World, text: &[u8]) {
        match self.proto {
            Proto::Telnet => self.client.write(w, &telnet_escape(text)),
            Proto::Ssh => self.client.write(w, &SshPacket::data(text).emit()),
        }
    }

    fn server_send(&self, w: &mut World, text: &str) {
        if text.is_empty() {
            return;
        }
        let bytes = to_wire(text);
        match self.proto {
            Proto::Telnet => self.server.write(w, &telnet_escape(&bytes)),
            Proto::Ssh => self.server.write(w, &SshPacket::data(&bytes).emit()),
        }
    }

    /// The far device reads what has arrived and answers it.
    fn serve(&mut self, w: &mut World) {
        let bytes = self.server.take(w);
        // A login that failed for good (or no password to ask for) closes without waiting for input.
        if self.shell.take_close() {
            self.server_done = true;
        }
        if self.server_done {
            self.server.shut(w);
            return;
        }
        if bytes.is_empty() {
            return;
        }
        let mut input = vec![];
        match self.proto {
            Proto::Telnet => {
                let (data, cmds) = self.s_tel.feed(&bytes);
                for n in cmds {
                    if let Some(r) = self.s_opts.answer(n, &[TELNET_ECHO, TELNET_SGA]) {
                        self.server.write(w, &r.bytes());
                    }
                }
                input = data;
            }
            Proto::Ssh => {
                match self.s_ssh.packets(&bytes) {
                    Ok(pkts) => {
                        for p in pkts {
                            match p.msg {
                                SSH_MSG_CHANNEL_DATA if self.authed => {
                                    if let Ok(d) = p.parse_data() {
                                        input.extend(d);
                                    }
                                }
                                SSH_MSG_DISCONNECT => self.server_done = true,
                                _ => {}
                            }
                        }
                    }
                    Err(_) => self.server_done = true,
                }
            }
        }
        self.partial.extend(input);
        let mut out = String::new();
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n' || *b == b'\r') {
            let raw: Vec<u8> = self.partial.drain(..=end).collect();
            // CR LF (or CR NUL) is one line end: swallow the second byte.
            if raw.last() == Some(&b'\r') && matches!(self.partial.first(), Some(b'\n' | 0)) {
                self.partial.remove(0);
            }
            let line = from_wire(&raw[..raw.len() - 1]);
            match self.shell.exec(w, &line) {
                Some(text) => {
                    if !text.is_empty() {
                        out += &text;
                        out += "\n";
                    }
                }
                None => self.server_done = true,
            }
            if self.shell.take_close() {
                self.server_done = true;
            }
            if self.server_done {
                break;
            }
        }
        let out = out.trim_end_matches('\n').to_string();
        if !out.is_empty() {
            self.server_send(w, &(out + "\n"));
        }
        if self.server_done {
            self.server.shut(w);
        }
    }

    /// The client reads what the far end sent. Returns the text and whether the session is over.
    fn receive(&mut self, w: &mut World) -> (String, bool) {
        let bytes = self.client.take(w);
        let mut text = vec![];
        match self.proto {
            Proto::Telnet => {
                let (data, cmds) = self.c_tel.feed(&bytes);
                for n in cmds {
                    if let Some(r) = self.c_opts.answer(n, &[TELNET_ECHO, TELNET_SGA]) {
                        self.client.write(w, &r.bytes());
                    }
                }
                text = data;
            }
            Proto::Ssh => {
                if let Ok(pkts) = self.c_ssh.packets(&bytes) {
                    for p in pkts {
                        if p.msg == SSH_MSG_CHANNEL_DATA {
                            text.extend(p.parse_data().unwrap_or_default());
                        }
                    }
                }
            }
        }
        (from_wire(&text), self.client.ended(w))
    }

    /// One round: the client's line goes out, the far device acts on it, the answer comes back.
    pub(super) fn exec(&mut self, w: &mut World, line: &str) -> (String, bool) {
        let mut sent = line.as_bytes().to_vec();
        sent.extend_from_slice(b"\r\n");
        self.client_send(w, &sent);
        self.settle(w)
    }

    /// Let the network carry what is in flight, let the far device react, carry the answer.
    fn settle(&mut self, w: &mut World) -> (String, bool) {
        w.run_to_idle();
        self.serve(w);
        w.run_to_idle();
        let (mut text, mut over) = self.receive(w);
        // Answers to negotiation and the closing handshake need another pass.
        w.run_to_idle();
        self.serve(w);
        w.run_to_idle();
        let (more, over2) = self.receive(w);
        text += &more;
        over |= over2;
        if over {
            self.client.shut(w);
            w.run_to_idle();
            let tail = format!("[Connection to {} closed by foreign host]", self.addr);
            let t = text.trim_end_matches('\n').to_string();
            text = if t.is_empty() { tail } else { format!("{t}\n\n{tail}") };
        }
        (text.trim_end_matches('\n').to_string(), over)
    }
}

// ---- the far device's login ----

impl Shell {
    /// Whether the far end asked for the connection to be closed (three bad passwords).
    pub(super) fn take_close(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    /// Start a terminal on this device for a session that came in over the network, after any
    /// authentication the transport did. Returns the text to show first.
    fn open_vty(&mut self, w: &World, authed: Option<&User>) -> String {
        let Some(m) = self.mgmt(w) else {
            return String::new();
        };
        let mut out = String::new();
        if let Some(b) = &m.banner_motd {
            out += b;
            out += "\n";
        }
        if let Some(u) = authed {
            if u.privilege >= 15 {
                self.session.enter(self.modes.privileged, None);
            }
        } else if m.vty.login_local {
            out += "\nUser Access Verification\n\n";
            self.pending = Some(Pending::VtyUser { tries: 0 });
        } else if m.vty.login {
            if m.vty.password.is_none() {
                out += &format!("\n{NO_PASSWORD_SET}\n");
                self.close_requested = true;
            } else {
                out += "\nUser Access Verification\n\n";
                self.pending = Some(Pending::VtyPassword { tries: 0, user: None });
            }
        }
        out
    }

    /// One answer typed at a vty login prompt.
    pub(super) fn answer_vty(&mut self, w: &mut World, pending: Pending, line: &str) -> String {
        let (tries, user) = match pending {
            Pending::VtyUser { tries } => {
                self.pending = Some(Pending::VtyPassword {
                    tries,
                    user: Some(line.trim().to_string()),
                });
                return String::new();
            }
            Pending::VtyPassword { tries, user } => (tries, user),
            _ => return String::new(),
        };
        let found = self.mgmt(w).and_then(|m| match &user {
            Some(name) => m.user(name).filter(|u| u.check(line)).cloned().map(Some),
            None => m
                .vty
                .password
                .as_deref()
                .is_some_and(|p| p == line)
                .then_some(None),
        });
        if let Some(u) = found {
            if u.is_some_and(|u| u.privilege >= 15) {
                self.session.enter(self.modes.privileged, None);
            }
            return String::new();
        }
        if tries + 1 >= 3 {
            self.close_requested = true;
            return BAD_PASSWORDS.to_string();
        }
        self.pending = Some(if user.is_some() {
            Pending::VtyUser { tries: tries + 1 }
        } else {
            Pending::VtyPassword { tries: tries + 1, user: None }
        });
        if user.is_some() {
            "% Login invalid".to_string()
        } else {
            String::new()
        }
    }

    /// The password the person typed after `ssh -l user host`.
    pub(super) fn answer_ssh_password(
        &mut self,
        w: &mut World,
        user: String,
        dst: Ipv4Addr,
        password: &str,
    ) -> String {
        self.connect(w, Proto::Ssh, dst, Some((user, password.to_string())))
    }

    pub(super) fn remote_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let device = self.device_name(w);
        match handler {
            USERNAME_SECRET..=USERNAME_PRIV_PASSWORD => {
                let name = a.word("name").unwrap().to_string();
                let hashed = matches!(handler, USERNAME_SECRET | USERNAME_PRIV_SECRET);
                let word = a.word("pw").unwrap();
                let secret = if hashed {
                    stored_secret(&device, word)
                } else {
                    plain_password(word)
                };
                let privilege = a.int("lvl").map_or(1, |l| l as u8);
                self.with_mgmt(w, |m| {
                    m.users.retain(|u| u.name != name);
                    m.users.push(User {
                        name,
                        secret,
                        hashed,
                        privilege,
                    });
                });
            }
            NO_USERNAME => {
                let name = a.word("name").unwrap().to_string();
                let found = self.with_mgmt(w, |m| {
                    let n = m.users.len();
                    m.users.retain(|u| u.name != name);
                    m.users.len() != n
                });
                if !found {
                    return "% That user is not configured.".to_string();
                }
            }
            DOMAIN_NAME => {
                let n = a.word("name").unwrap().to_string();
                self.with_mgmt(w, |m| m.domain_name = Some(n));
            }
            NO_DOMAIN_NAME => self.with_mgmt(w, |m| m.domain_name = None),
            KEYGEN | KEYGEN_BITS => {
                let bits = a.int("bits").map_or(1024, |b| b as u16);
                let Some(domain) = self.mgmt(w).and_then(|m| m.domain_name.clone()) else {
                    return NEED_DOMAIN.to_string();
                };
                self.with_mgmt(w, |m| m.rsa_bits = Some(bits));
                return format!(
                    "The name for the keys will be: {device}.{domain}\n% The key modulus size is {bits} bits\n% Generating {bits} bit RSA keys, keys will be non-exportable...\n[OK] (elapsed time was 0 seconds)"
                );
            }
            KEYZEROIZE => {
                if self.mgmt(w).is_none_or(|m| m.rsa_bits.is_none()) {
                    return "% No Signature RSA Keys found in configuration.".to_string();
                }
                self.with_mgmt(w, |m| m.rsa_bits = None);
                return "% All RSA keys will be removed.".to_string();
            }
            IP_SSH_VERSION => {}
            LOGIN_LOCAL => {
                if self.session.context() == Some("console") {
                    return VTY_ONLY.to_string();
                }
                self.with_mgmt(w, |m| {
                    m.vty.login = true;
                    m.vty.login_local = true;
                });
            }
            TRANSPORT_TELNET..=TRANSPORT_NONE => {
                if self.session.context() == Some("console") {
                    return VTY_ONLY.to_string();
                }
                let (telnet, ssh) = match handler {
                    TRANSPORT_TELNET => (true, false),
                    TRANSPORT_SSH => (false, true),
                    TRANSPORT_ALL => (true, true),
                    _ => (false, false),
                };
                self.with_mgmt(w, |m| m.vty.transport = netdev::Transport { telnet, ssh });
            }
            TELNET => return self.connect(w, Proto::Telnet, Ipv4Addr(a.ipv4("dst").unwrap()), None),
            SSH => {
                let dst = Ipv4Addr(a.ipv4("dst").unwrap());
                let user = a.word("user").unwrap().to_string();
                self.pending = Some(Pending::SshPassword { user, dst });
                return String::new();
            }
            SHOW_IP_SSH => {
                let on = self.mgmt(w).is_some_and(Mgmt::ssh_enabled);
                return if on {
                    "SSH Enabled - version 2.0\nAuthentication timeout: 120 secs; Authentication retries: 3".to_string()
                } else {
                    "SSH Disabled - version 2.0\n%Please create RSA keys and set an IP domain-name to enable SSH.".to_string()
                };
            }
            SHOW_TCP_BRIEF => return self.show_tcp_brief(w),
            _ => {}
        }
        String::new()
    }

    fn show_tcp_brief(&self, w: &World) -> String {
        let t = tcp_of(w, self.kind, self.device);
        let mut o = format!("{:<5} {:<24}{:<24}(state)", "TCB", "Local Address", "Foreign Address");
        for c in t.conns() {
            o += &format!(
                "\n{:<5} {:<24}{:<24}{}",
                c.id,
                format!("{}.{}", c.local.0, c.local.1),
                format!("{}.{}", c.remote.0, c.remote.1),
                c.state.name()
            );
        }
        for port in t.udp.keys() {
            o += &format!("\n{:<5} {:<24}{:<24}UDP", "-", format!("*.{port}"), "*.*");
        }
        o
    }

    /// Open a session to `dst`. For SSH, `auth` is the user and password already typed.
    pub(super) fn connect(
        &mut self,
        w: &mut World,
        proto: Proto,
        dst: Ipv4Addr,
        auth: Option<(String, String)>,
    ) -> String {
        let (kind, dev) = (self.kind, self.device);
        let port = if proto == Proto::Telnet { TELNET_PORT } else { SSH_PORT };
        let Some(conn) = with_net(w, kind, dev, |n, c| n.open(c, dst, port)) else {
            return NO_ROUTE.to_string();
        };
        let me = (kind, dev);
        let started = w.kernel.now();
        let done = move |w: &World| {
            tcp_of(w, me.0, me.1)
                .conn(conn)
                .is_none_or(|c| c.state != TcpState::SynSent)
        };
        w.run_until_or(started + SimTime::from_secs(60), done);
        w.run_to_idle();
        let (state, failure, local) = match tcp_of(w, kind, dev).conn(conn) {
            Some(c) => (c.state, c.failure, c.local),
            None => (TcpState::Closed, None, (dst, 0)),
        };
        if state != TcpState::Established {
            with_net(w, kind, dev, |n, c| n.abort(c, conn));
            return match failure {
                Some(TcpFailure::Refused) => REFUSED.to_string(),
                Some(TcpFailure::TimedOut) | None => TIMED_OUT_REMOTE.to_string(),
                Some(TcpFailure::Reset) => REFUSED.to_string(),
            };
        }
        let client = Endpoint { dev, kind, conn };
        let Some(server) = find_server(w, local, (dst, port)) else {
            client.abort(w);
            return REFUSED.to_string();
        };
        let far = match server.kind {
            Kind::Switch => Shell::switch(server.dev),
            _ => Shell::router(server.dev),
        };
        let mut r = Remote {
            proto,
            addr: dst,
            client,
            server,
            shell: Box::new(far),
            c_tel: TelnetDecoder::default(),
            c_opts: TelnetOptions::default(),
            s_tel: TelnetDecoder::default(),
            s_opts: TelnetOptions::default(),
            c_ssh: SshDecoder::default(),
            s_ssh: SshDecoder::default(),
            partial: vec![],
            authed: false,
            server_done: false,
        };
        let mut out = format!("Trying {dst} ... Open\n");
        match proto {
            Proto::Telnet => {
                // The server offers to echo and to suppress go-ahead, as a telnet daemon does.
                for o in [TELNET_ECHO, TELNET_SGA] {
                    if let Some(n) = r.s_opts.offer(o) {
                        server.write(w, &n.bytes());
                    }
                }
                let greeting = r.shell.open_vty(w, None);
                r.server_send(w, &greeting);
            }
            Proto::Ssh => {
                let Some((user, password)) = auth else {
                    client.abort(w);
                    return AUTH_FAILED.to_string();
                };
                match r.ssh_handshake(w, &user, &password) {
                    Ok(()) => {}
                    Err(msg) => {
                        r.client.shut(w);
                        r.server.shut(w);
                        w.run_to_idle();
                        return msg;
                    }
                }
            }
        }
        let (text, over) = r.settle(w);
        let mut out_text = out.trim_end().to_string();
        if !text.is_empty() {
            out_text += "\n\n";
            out_text += &text;
        }
        out = out_text;
        if !over {
            self.remote = Some(Box::new(r));
        }
        out
    }
}

impl Remote {
    /// Version exchange and password authentication. On success the far terminal is open.
    fn ssh_handshake(&mut self, w: &mut World, user: &str, password: &str) -> Result<(), String> {
        // Client's identification, then the server's.
        self.client.write(w, &ssh_ident());
        w.run_to_idle();
        let theirs = self.server.take(w);
        let ident = self.s_ssh.take_ident(&theirs).unwrap_or_default();
        match ssh_parse_ident(&ident) {
            SshIdent::V2(_) => {}
            SshIdent::V1 => {
                self.server.write(w, b"Protocol major versions differ.\r\n");
                return Err(V1_ONLY.to_string());
            }
            SshIdent::NotSsh => return Err(BAD_SSH_SERVER.to_string()),
        }
        self.server.write(w, &ssh_ident());
        w.run_to_idle();
        let back = self.client.take(w);
        let line = self.c_ssh.take_ident(&back).unwrap_or_default();
        match ssh_parse_ident(&line) {
            SshIdent::V2(_) => {}
            SshIdent::V1 => return Err(V1_ONLY.to_string()),
            SshIdent::NotSsh => return Err(BAD_SSH_SERVER.to_string()),
        }
        // Authentication.
        self.client
            .write(w, &SshPacket::auth_password(user, password).emit());
        w.run_to_idle();
        let bytes = self.server.take(w);
        let pkts = self.s_ssh.packets(&bytes).map_err(|_| BAD_SSH_SERVER.to_string())?;
        let who = pkts
            .iter()
            .find(|p| p.msg == SSH_MSG_USERAUTH_REQUEST)
            .and_then(|p| p.parse_auth().ok())
            .and_then(|(u, pw)| {
                self.shell
                    .mgmt(w)
                    .and_then(|m| m.user(&u))
                    .filter(|x| x.check(&pw))
                    .cloned()
            });
        let reply = SshPacket {
            msg: if who.is_some() {
                SSH_MSG_USERAUTH_SUCCESS
            } else {
                SSH_MSG_USERAUTH_FAILURE
            },
            payload: vec![],
        };
        self.server.write(w, &reply.emit());
        w.run_to_idle();
        let bytes = self.client.take(w);
        let ok = self
            .c_ssh
            .packets(&bytes)
            .map(|p| p.iter().any(|p| p.msg == SSH_MSG_USERAUTH_SUCCESS))
            .unwrap_or(false);
        let Some(u) = who.filter(|_| ok) else {
            self.server
                .write(w, &SshPacket::disconnect("Too many authentication failures").emit());
            return Err(AUTH_FAILED.to_string());
        };
        self.authed = true;
        let greeting = self.shell.open_vty(w, Some(&u));
        self.server_send(w, &greeting);
        Ok(())
    }
}

/// Find the device that accepted the connection from `client` to `server`, by looking at the
/// connection tables. (The shell has no map of the lab, only the world.)
fn find_server(w: &mut World, client: (Ipv4Addr, u16), server: (Ipv4Addr, u16)) -> Option<Endpoint> {
    for i in 0..w.device_count() {
        let dev = DeviceId(i as u32);
        let kind = if w.is_device::<Router>(dev) {
            Kind::Router
        } else if w.is_device::<L2Switch>(dev) {
            Kind::Switch
        } else {
            continue;
        };
        let conn = tcp_of(w, kind, dev)
            .conns()
            .find(|c| c.remote == client && c.local == server)
            .map(|c| c.id);
        if let Some(conn) = conn {
            // Take it off the list of connections waiting to be accepted.
            match kind {
                Kind::Router => w.with_device::<Router, _>(dev, |r, _| while r.tcp.accept().is_some() {}),
                _ => w.with_device::<L2Switch, _>(dev, |s, _| {
                    while s.l3_mut().tcp.accept().is_some() {}
                }),
            }
            return Some(Endpoint { dev, kind, conn });
        }
    }
    None
}

/// The lines of a running config for users, domain name and the vty transport.
pub(super) fn remote_head_lines(m: &Mgmt) -> String {
    let mut o = String::new();
    if let Some(d) = &m.domain_name {
        o += &format!("ip domain-name {d}\n");
    }
    for u in &m.users {
        let priv_part = if u.privilege == 1 {
            String::new()
        } else {
            format!(" privilege {}", u.privilege)
        };
        if u.hashed {
            o += &format!("username {}{priv_part} secret wb1${}\n", u.name, u.secret);
        } else {
            o += &format!(
                "username {}{priv_part} password {}\n",
                u.name,
                password_text(&u.secret, m.encrypt_passwords)
            );
        }
    }
    if m.domain_name.is_some() || !m.users.is_empty() {
        o += "!\n";
    }
    o
}
