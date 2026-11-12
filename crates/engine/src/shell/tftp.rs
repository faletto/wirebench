//! `copy` to and from a TFTP server (routers and switches) and the TFTP server on a PC
//! (`tftpd`). The client runs inside the command: it sends, lets simulated time pass until an
//! answer arrives and sends again after a silence, as the real one does in the background.
//! Handler ids 542..549 and 550..559.

use super::*;
use netdev::{Tcp, TFTP_MAX_RETRIES};
use wire::{TftpMode, TftpPacket, TFTP_BLOCK_SIZE, TFTP_PORT};

pub(super) const COPY_RUN_TFTP: u32 = 542;
pub(super) const COPY_START_TFTP: u32 = 543;
pub(super) const COPY_TFTP_RUN: u32 = 544;
pub(super) const COPY_TFTP_START: u32 = 545;
pub(super) const TFTPD_START: u32 = 550;
pub(super) const TFTPD_STOP: u32 = 551;
pub(super) const TFTPD_LIST: u32 = 552;
pub(super) const TFTPD_SHOW: u32 = 553;
pub(super) const TFTPD_DELETE: u32 = 554;
pub(super) const TFTPD_LOG: u32 = 555;

const BAD_URL: &str = "% Invalid input detected: the address is tftp://<ip address>/<file>.";
/// How long the client waits for an answer before it sends its last packet again.
const WAIT: u64 = 2;
const FIRST_PORT: u16 = 49300;

pub(super) fn copy_commands(g: &mut Grammar, privileged: ModeId) {
    let h = Action::Handler;
    let copy = "Copy from one file to another";
    let cmds: &[(&str, &[&str], u32)] = &[
        (
            "copy running-config <url:word>",
            &[
                copy,
                "Current configuration",
                "Destination, tftp://<address>/<file>",
            ],
            COPY_RUN_TFTP,
        ),
        (
            "copy startup-config <url:word>",
            &[
                copy,
                "Saved configuration",
                "Destination, tftp://<address>/<file>",
            ],
            COPY_START_TFTP,
        ),
        (
            "copy <url:word> running-config",
            &[
                copy,
                "Source, tftp://<address>/<file>",
                "Merge into the current configuration",
            ],
            COPY_TFTP_RUN,
        ),
        (
            "copy <url:word> startup-config",
            &[
                copy,
                "Source, tftp://<address>/<file>",
                "Replace the saved configuration",
            ],
            COPY_TFTP_START,
        ),
    ];
    for (line, help, id) in cmds {
        g.cmd(&[privileged], line, help, h(*id));
    }
}

pub(super) fn tftpd_commands(g: &mut Grammar, m: ModeId) {
    let h = Action::Handler;
    let cmds: &[(&str, &[&str], u32)] = &[
        (
            "tftpd start",
            &["TFTP server", "Start listening on UDP 69"],
            TFTPD_START,
        ),
        ("tftpd stop", &["TFTP server", "Stop listening"], TFTPD_STOP),
        (
            "tftpd list",
            &["TFTP server", "List the files it holds"],
            TFTPD_LIST,
        ),
        (
            "tftpd log",
            &["TFTP server", "Show what it has done"],
            TFTPD_LOG,
        ),
        (
            "tftpd show <file:word>",
            &["TFTP server", "Show a file as text", "File name"],
            TFTPD_SHOW,
        ),
        (
            "tftpd delete <file:word>",
            &["TFTP server", "Delete a file", "File name"],
            TFTPD_DELETE,
        ),
    ];
    for (pattern, help, id) in cmds {
        g.cmd(&[m], pattern, help, h(*id));
    }
}

/// `tftp://10.0.0.5/dir/name` as (address, file name).
fn parse_url(url: &str) -> Option<(Ipv4Addr, String)> {
    let rest = url.strip_prefix("tftp://")?;
    let (host, file) = rest.split_once('/')?;
    let parts: Vec<u8> = host
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let addr = <[u8; 4]>::try_from(parts).ok()?;
    (!file.is_empty()).then(|| (Ipv4Addr(addr), file.to_string()))
}

pub(super) fn with_udp<R>(
    w: &mut World,
    kind: Kind,
    dev: DeviceId,
    f: impl FnOnce(&mut Tcp, &mut dyn FnMut(&mut Ctx, Ipv4Addr, u16, u16, &[u8]) -> bool, &mut Ctx) -> R,
) -> R {
    match kind {
        Kind::Host => w.with_device::<Host, _>(dev, |h, c| {
            let mut tcp = std::mem::replace(&mut h.tcp, Tcp::new(0));
            let r = f(
                &mut tcp,
                &mut |c, d, sp, dp, b| {
                    h.udp_send(c, d, sp, dp, b);
                    true
                },
                c,
            );
            h.tcp = tcp;
            r
        }),
        Kind::Router => w.with_device::<Router, _>(dev, |rt, c| {
            let mut tcp = std::mem::replace(&mut rt.tcp, Tcp::new(0));
            let mut sent = false;
            let r = f(
                &mut tcp,
                &mut |c, d, sp, dp, b| {
                    sent = rt.udp_send(c, d, sp, dp, b);
                    sent
                },
                c,
            );
            rt.tcp = tcp;
            rt.flush_outbox(c);
            r
        }),
        Kind::Switch => w.with_device::<L2Switch, _>(dev, |s, c| {
            s.originate(c, |rt, c| {
                let mut tcp = std::mem::replace(&mut rt.tcp, Tcp::new(0));
                let r = f(
                    &mut tcp,
                    &mut |c, d, sp, dp, b| rt.udp_send(c, d, sp, dp, b),
                    c,
                );
                rt.tcp = tcp;
                r
            })
        }),
    }
}

pub(super) fn udp_of(w: &World, kind: Kind, dev: DeviceId) -> &Tcp {
    match kind {
        Kind::Host => &w.device::<Host>(dev).tcp,
        Kind::Router => &w.device::<Router>(dev).tcp,
        Kind::Switch => &w.device::<L2Switch>(dev).l3().tcp,
    }
}

/// One TFTP transfer from this device.
struct Client {
    kind: Kind,
    dev: DeviceId,
    server: Ipv4Addr,
    port: u16,
    /// The server's port for this transfer, known after its first answer.
    tid: Option<u16>,
    last: Vec<u8>,
}

impl Client {
    fn new(w: &mut World, kind: Kind, dev: DeviceId, server: Ipv4Addr) -> Client {
        let mut port = FIRST_PORT;
        while !with_udp(w, kind, dev, |t, _, _| t.udp_bind(port)) {
            port += 1;
        }
        Client {
            kind,
            dev,
            server,
            port,
            tid: None,
            last: Vec::new(),
        }
    }

    fn send(&mut self, w: &mut World, p: &TftpPacket) -> bool {
        self.last = p.emit();
        self.resend(w)
    }

    fn resend(&mut self, w: &mut World) -> bool {
        let (dst, dport) = (self.server, self.tid.unwrap_or(TFTP_PORT));
        let (sport, data) = (self.port, self.last.clone());
        with_udp(w, self.kind, self.dev, |_, send, c| {
            send(c, dst, sport, dport, &data)
        })
    }

    /// The next datagram from the server, waiting up to `WAIT` seconds of simulated time and
    /// sending the last packet again after each silence. Anything from another address or, once
    /// the server's port is known, another port is answered with error 5 and ignored.
    fn next(&mut self, w: &mut World) -> Result<TftpPacket, String> {
        let (kind, dev, port) = (self.kind, self.dev, self.port);
        for _ in 0..=TFTP_MAX_RETRIES {
            let deadline = w.kernel.now() + SimTime::from_secs(WAIT);
            loop {
                w.run_until_or(deadline, |w| {
                    udp_of(w, kind, dev)
                        .udp
                        .get(&port)
                        .is_some_and(|s| !s.inbox.is_empty())
                });
                let got = with_udp(w, kind, dev, |t, _, _| t.udp_recv(port));
                let Some((src, sport, data)) = got else { break };
                if src != self.server || self.tid.is_some_and(|t| t != sport) {
                    let err = TftpPacket::Error {
                        code: wire::TFTP_ERR_UNKNOWN_ID,
                        message: "Unknown transfer ID".into(),
                    }
                    .emit();
                    with_udp(w, kind, dev, |_, send, c| send(c, src, port, sport, &err));
                    continue;
                }
                let Ok(p) = TftpPacket::parse(&data) else {
                    continue;
                };
                self.tid.get_or_insert(sport);
                return Ok(p);
            }
            if !self.resend(w) {
                return Err("No route to host".into());
            }
        }
        Err("Timed out".into())
    }

    fn close(&self, w: &mut World) {
        let port = self.port;
        with_udp(w, self.kind, self.dev, |t, _, _| t.udp_close(port));
    }

    fn get(&mut self, w: &mut World, file: &str) -> Result<Vec<u8>, String> {
        if !self.send(
            w,
            &TftpPacket::Read {
                file: file.into(),
                mode: TftpMode::Octet,
            },
        ) {
            return Err("No route to host".into());
        }
        let mut data = Vec::new();
        let mut expect = 1u16;
        loop {
            match self.next(w)? {
                TftpPacket::Data { block, data: d } if block == expect => {
                    data.extend_from_slice(&d);
                    self.send(w, &TftpPacket::Ack { block });
                    if d.len() < TFTP_BLOCK_SIZE {
                        return Ok(data);
                    }
                    expect = expect.wrapping_add(1);
                }
                // The server did not hear our ACK and sent the block again.
                TftpPacket::Data { .. } => {
                    self.resend(w);
                }
                TftpPacket::Error { message, .. } => return Err(message),
                _ => {}
            }
        }
    }

    fn put(&mut self, w: &mut World, file: &str, data: &[u8]) -> Result<(), String> {
        if !self.send(
            w,
            &TftpPacket::Write {
                file: file.into(),
                mode: TftpMode::Octet,
            },
        ) {
            return Err("No route to host".into());
        }
        let mut block = 0u16;
        loop {
            match self.next(w)? {
                TftpPacket::Ack { block: b } if b == block => {
                    let from = block as usize * TFTP_BLOCK_SIZE;
                    if block > 0 && from > data.len() {
                        return Ok(());
                    }
                    // The block after the last full one is short or empty: that ends the file.
                    if block > 0 && from == data.len() && data.len() % TFTP_BLOCK_SIZE != 0 {
                        return Ok(());
                    }
                    block += 1;
                    let from = (block as usize - 1) * TFTP_BLOCK_SIZE;
                    let chunk = data
                        [from.min(data.len())..(from + TFTP_BLOCK_SIZE).min(data.len())]
                        .to_vec();
                    self.send(w, &TftpPacket::Data { block, data: chunk });
                }
                TftpPacket::Error { message, .. } => return Err(message),
                _ => {}
            }
        }
    }
}

fn blocks(len: usize) -> usize {
    len / TFTP_BLOCK_SIZE + 1
}

fn rate(bytes: usize, started: SimTime, now: SimTime) -> String {
    let secs = (now.0 - started.0) as f64 / 1e9;
    let per = if secs > 0.0 {
        (bytes as f64 / secs) as u64
    } else {
        0
    };
    format!("{bytes} bytes copied in {secs:.3} secs ({per} bytes/sec)")
}

impl Shell {
    pub(super) fn copy_tftp_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let url = a.word("url").unwrap_or_default().to_string();
        // `copy running-config startup-config` is matched here as a URL; it is the save command.
        if matches!(handler, COPY_RUN_TFTP) && url == "startup-config" {
            let text = self.running_text(w);
            self.set_startup(w, Some(text));
            return SAVED.to_string();
        }
        let Some((server, file)) = parse_url(&url) else {
            return BAD_URL.to_string();
        };
        let started = w.kernel.now();
        let (kind, dev) = (self.kind, self.device);
        let mut client = Client::new(w, kind, dev, server);
        let out = match handler {
            COPY_RUN_TFTP | COPY_START_TFTP => {
                let text = if handler == COPY_RUN_TFTP {
                    Some(self.running_text(w))
                } else {
                    self.startup_config(w).map(str::to_string)
                };
                let Some(text) = text else {
                    client.close(w);
                    return format!("%Error: {NOT_PRESENT}");
                };
                let mut bytes = text.into_bytes();
                bytes.push(b'\n');
                let r = client.put(w, &file, &bytes);
                match r {
                    Ok(()) => format!(
                        "Writing {file} {}\n{}",
                        "!".repeat(blocks(bytes.len()).min(60)),
                        rate(bytes.len(), started, w.kernel.now())
                    ),
                    Err(e) => format!("%Error opening {url} ({e})"),
                }
            }
            _ => match client.get(w, &file) {
                Err(e) => format!("%Error opening {url} ({e})"),
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    let mut out = format!(
                        "Accessing {url}...\nLoading {file} from {server}: !\n[OK - {} bytes]\n\n{}",
                        data.len(),
                        rate(data.len(), started, w.kernel.now())
                    );
                    if handler == COPY_TFTP_START {
                        self.set_startup(w, Some(text.trim_end().to_string()));
                    } else {
                        let mut boot = match kind {
                            Kind::Switch => Shell::switch(dev),
                            _ => Shell::router(dev),
                        };
                        for p in boot.replay(w, &text) {
                            out += &format!("\n{p}");
                        }
                    }
                    out
                }
            },
        };
        client.close(w);
        out
    }

    pub(super) fn tftpd_cmd(&mut self, w: &mut World, handler: u32, a: &Args) -> String {
        let id = self.device;
        match handler {
            TFTPD_START => {
                w.with_device::<Host, _>(id, |h, _| h.tftpd.running = true);
                "TFTP server listening on UDP port 69.".to_string()
            }
            TFTPD_STOP => {
                w.with_device::<Host, _>(id, |h, _| h.tftpd.stop());
                String::new()
            }
            TFTPD_LIST => {
                let t = &w.device::<Host>(id).tftpd;
                let mut o = format!(
                    "TFTP server is {}. {} file{}.",
                    if t.running { "running" } else { "stopped" },
                    t.files.len(),
                    if t.files.len() == 1 { "" } else { "s" }
                );
                for (name, data) in &t.files {
                    o += &format!("\n{:>8}  {name}", data.len());
                }
                o
            }
            TFTPD_LOG => w.device::<Host>(id).tftpd.log.join("\n"),
            TFTPD_SHOW => {
                let name = a.word("file").unwrap_or_default();
                match w.device::<Host>(id).tftpd.files.get(name) {
                    Some(d) => String::from_utf8_lossy(d).trim_end().to_string(),
                    None => format!("The file {name} does not exist."),
                }
            }
            _ => {
                let name = a.word("file").unwrap_or_default().to_string();
                let had =
                    w.with_device::<Host, _>(id, |h, _| h.tftpd.files.remove(&name).is_some());
                if had {
                    String::new()
                } else {
                    format!("The file {name} does not exist.")
                }
            }
        }
    }
}
