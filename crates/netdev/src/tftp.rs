//! A TFTP server (RFC 1350) for a PC: files by name in memory, transfers on their own ports.
//! The caller hands it datagrams and the time, and sends what it returns.

use kernel::SimTime;
use std::collections::BTreeMap;
use wire::{
    Ipv4Addr, TftpPacket, TFTP_BLOCK_SIZE, TFTP_ERR_DISK_FULL, TFTP_ERR_FILE_NOT_FOUND,
    TFTP_ERR_ILLEGAL_OP, TFTP_ERR_UNKNOWN_ID, TFTP_PORT,
};

/// How long to wait for the other end before sending the last packet again.
pub const RETRY: SimTime = SimTime::from_secs(3);
/// Times the last packet is sent again before the transfer is dropped.
pub const MAX_RETRIES: u8 = 5;
/// The first port transfers are served from.
const FIRST_TID: u16 = 49152;
/// Block numbers are 16 bits and wrap is not supported.
const MAX_FILE: usize = 65535 * TFTP_BLOCK_SIZE;

/// A datagram for the caller to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub dst: Ipv4Addr,
    pub dst_port: u16,
    pub src_port: u16,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug)]
enum Dir {
    /// Sending `data` to a reader; `block` is the one on the wire.
    Send { data: Vec<u8>, block: u16 },
    /// Receiving a file; `block` is the last one acknowledged.
    Recv { data: Vec<u8>, block: u16 },
}

#[derive(Clone, Debug)]
struct Xfer {
    file: String,
    peer: Ipv4Addr,
    peer_port: u16,
    tid: u16,
    dir: Dir,
    last: Vec<u8>,
    sent_at: SimTime,
    tries: u8,
}

#[derive(Clone, Debug, Default)]
pub struct TftpServer {
    pub running: bool,
    pub files: BTreeMap<String, Vec<u8>>,
    xfers: Vec<Xfer>,
    next_tid: u16,
    /// What happened, oldest first, for `tftpd log`.
    pub log: Vec<String>,
}

impl TftpServer {
    pub fn wants_tick(&self) -> bool {
        self.running && !self.xfers.is_empty()
    }

    /// Whether `port` is one a transfer is being served from.
    pub fn owns_port(&self, port: u16) -> bool {
        self.xfers.iter().any(|x| x.tid == port)
    }

    fn data_packet(data: &[u8], block: u16) -> Vec<u8> {
        let from = (block as usize - 1) * TFTP_BLOCK_SIZE;
        let to = (from + TFTP_BLOCK_SIZE).min(data.len());
        TftpPacket::Data {
            block,
            data: data[from.min(data.len())..to].to_vec(),
        }
        .emit()
    }

    fn error(dst: Ipv4Addr, dst_port: u16, src_port: u16, code: u16, msg: &str) -> Out {
        Out {
            dst,
            dst_port,
            src_port,
            payload: TftpPacket::Error {
                code,
                message: msg.to_string(),
            }
            .emit(),
        }
    }

    /// A datagram for `dst_port` (69, or a transfer's port) from `src`:`src_port`.
    pub fn on_packet(
        &mut self,
        now: SimTime,
        src: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        data: &[u8],
    ) -> Vec<Out> {
        if !self.running {
            return vec![];
        }
        let Ok(pkt) = TftpPacket::parse(data) else {
            return vec![Self::error(
                src,
                src_port,
                dst_port,
                TFTP_ERR_ILLEGAL_OP,
                "Illegal TFTP operation",
            )];
        };
        if dst_port == TFTP_PORT {
            return self.on_request(now, src, src_port, pkt);
        }
        let Some(i) = self.xfers.iter().position(|x| x.tid == dst_port) else {
            return vec![];
        };
        if (self.xfers[i].peer, self.xfers[i].peer_port) != (src, src_port) {
            return vec![Self::error(
                src,
                src_port,
                dst_port,
                TFTP_ERR_UNKNOWN_ID,
                "Unknown transfer ID",
            )];
        }
        let x = &mut self.xfers[i];
        let mut out = Vec::new();
        let mut done = false;
        match (&mut x.dir, pkt) {
            (_, TftpPacket::Error { message, .. }) => {
                self.log.push(format!(
                    "{} with {src}: error from the peer: {message}",
                    x.file
                ));
                done = true;
            }
            (Dir::Send { data, block }, TftpPacket::Ack { block: acked }) if acked == *block => {
                if data.len() / TFTP_BLOCK_SIZE < *block as usize {
                    // The last block (shorter than a full one) is acknowledged: finished.
                    self.log
                        .push(format!("sent {} ({} bytes) to {src}", x.file, data.len()));
                    done = true;
                } else {
                    *block += 1;
                    x.last = Self::data_packet(data, *block);
                    x.sent_at = now;
                    x.tries = 0;
                    out.push(Out {
                        dst: src,
                        dst_port: src_port,
                        src_port: x.tid,
                        payload: x.last.clone(),
                    });
                }
            }
            (Dir::Recv { data, block }, TftpPacket::Data { block: n, data: d }) => {
                if n == block.wrapping_add(1) {
                    if data.len() + d.len() > MAX_FILE {
                        out.push(Self::error(
                            src,
                            src_port,
                            x.tid,
                            TFTP_ERR_DISK_FULL,
                            "Disk full",
                        ));
                        done = true;
                    } else {
                        data.extend_from_slice(&d);
                        *block = n;
                        x.last = TftpPacket::Ack { block: n }.emit();
                        x.sent_at = now;
                        x.tries = 0;
                        out.push(Out {
                            dst: src,
                            dst_port: src_port,
                            src_port: x.tid,
                            payload: x.last.clone(),
                        });
                        if d.len() < TFTP_BLOCK_SIZE {
                            self.log.push(format!(
                                "received {} ({} bytes) from {src}",
                                x.file,
                                data.len()
                            ));
                            self.files.insert(x.file.clone(), std::mem::take(data));
                            done = true;
                        }
                    }
                } else if n == *block {
                    // The acknowledgement was lost: say it again.
                    out.push(Out {
                        dst: src,
                        dst_port: src_port,
                        src_port: x.tid,
                        payload: TftpPacket::Ack { block: n }.emit(),
                    });
                }
            }
            // A duplicate acknowledgement or a stray packet: ignored (RFC 1123: never resend on a
            // duplicate ACK).
            (_, TftpPacket::Ack { .. } | TftpPacket::Data { .. }) => {}
            _ => out.push(Self::error(
                src,
                src_port,
                x.tid,
                TFTP_ERR_ILLEGAL_OP,
                "Illegal TFTP operation",
            )),
        }
        if done {
            self.xfers.remove(i);
        }
        out
    }

    fn on_request(
        &mut self,
        now: SimTime,
        src: Ipv4Addr,
        src_port: u16,
        pkt: TftpPacket,
    ) -> Vec<Out> {
        let tid = FIRST_TID + self.next_tid % 1000;
        let (file, write) = match pkt {
            TftpPacket::Read { file, .. } => (file, false),
            TftpPacket::Write { file, .. } => (file, true),
            _ => {
                return vec![Self::error(
                    src,
                    src_port,
                    TFTP_PORT,
                    TFTP_ERR_ILLEGAL_OP,
                    "Illegal TFTP operation",
                )]
            }
        };
        // A retransmitted request for a transfer already running is not a new one.
        if self
            .xfers
            .iter()
            .any(|x| x.peer == src && x.peer_port == src_port && x.file == file)
        {
            return vec![];
        }
        self.next_tid += 1;
        let (dir, last) = if write {
            (
                Dir::Recv {
                    data: Vec::new(),
                    block: 0,
                },
                TftpPacket::Ack { block: 0 }.emit(),
            )
        } else {
            match self.files.get(&file) {
                Some(data) => {
                    let first = Self::data_packet(data, 1);
                    (
                        Dir::Send {
                            data: data.clone(),
                            block: 1,
                        },
                        first,
                    )
                }
                None => {
                    self.log.push(format!("{src} asked for {file}: not found"));
                    return vec![Self::error(
                        src,
                        src_port,
                        TFTP_PORT,
                        TFTP_ERR_FILE_NOT_FOUND,
                        "File not found",
                    )];
                }
            }
        };
        self.xfers.push(Xfer {
            file,
            peer: src,
            peer_port: src_port,
            tid,
            dir,
            last: last.clone(),
            sent_at: now,
            tries: 0,
        });
        vec![Out {
            dst: src,
            dst_port: src_port,
            src_port: tid,
            payload: last,
        }]
    }

    /// Send the last packet again where the other end has been quiet too long; drop transfers
    /// that never got an answer.
    pub fn tick(&mut self, now: SimTime) -> Vec<Out> {
        let mut out = Vec::new();
        let mut log = Vec::new();
        self.xfers.retain_mut(|x| {
            if now.0.saturating_sub(x.sent_at.0) < RETRY.0 {
                return true;
            }
            if x.tries >= MAX_RETRIES {
                log.push(format!("{} with {}: timed out", x.file, x.peer));
                return false;
            }
            x.tries += 1;
            x.sent_at = now;
            out.push(Out {
                dst: x.peer,
                dst_port: x.peer_port,
                src_port: x.tid,
                payload: x.last.clone(),
            });
            true
        });
        self.log.extend(log);
        out
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.xfers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::TftpMode;

    const A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const T0: SimTime = SimTime::ZERO;

    fn server() -> TftpServer {
        TftpServer {
            running: true,
            ..TftpServer::default()
        }
    }

    fn pkt(o: &Out) -> TftpPacket {
        TftpPacket::parse(&o.payload).unwrap()
    }

    fn read(s: &mut TftpServer, file: &str) -> Vec<Out> {
        let rrq = TftpPacket::Read {
            file: file.into(),
            mode: TftpMode::Octet,
        }
        .emit();
        s.on_packet(T0, A, 4000, TFTP_PORT, &rrq)
    }

    #[test]
    fn a_file_is_read_block_by_block_and_the_short_block_ends_it() {
        let mut s = server();
        s.files.insert("f".into(), vec![7; 1100]);
        let first = read(&mut s, "f");
        assert_eq!(first.len(), 1);
        let tid = first[0].src_port;
        assert_ne!(tid, TFTP_PORT);
        assert!(
            matches!(pkt(&first[0]), TftpPacket::Data { block: 1, ref data } if data.len() == 512)
        );
        let mut got = 512;
        for block in 1..=3 {
            let out = s.on_packet(T0, A, 4000, tid, &TftpPacket::Ack { block }.emit());
            if block < 3 {
                let TftpPacket::Data { block: b, data } = pkt(&out[0]) else {
                    panic!()
                };
                assert_eq!(b, block + 1);
                got += data.len();
            } else {
                assert!(out.is_empty());
            }
        }
        assert_eq!(got, 1100);
        assert!(!s.wants_tick());
    }

    #[test]
    fn a_file_of_whole_blocks_ends_with_an_empty_one() {
        let mut s = server();
        s.files.insert("f".into(), vec![1; 512]);
        let tid = read(&mut s, "f")[0].src_port;
        let out = s.on_packet(T0, A, 4000, tid, &TftpPacket::Ack { block: 1 }.emit());
        assert!(matches!(pkt(&out[0]), TftpPacket::Data { block: 2, ref data } if data.is_empty()));
        assert!(s
            .on_packet(T0, A, 4000, tid, &TftpPacket::Ack { block: 2 }.emit())
            .is_empty());
        assert!(!s.wants_tick());
    }

    #[test]
    fn a_missing_file_is_error_one_and_starts_nothing() {
        let mut s = server();
        let out = read(&mut s, "nope");
        assert!(matches!(
            pkt(&out[0]),
            TftpPacket::Error {
                code: TFTP_ERR_FILE_NOT_FOUND,
                ..
            }
        ));
        assert_eq!(out[0].src_port, TFTP_PORT);
        assert!(!s.wants_tick());
    }

    #[test]
    fn a_file_is_written_and_a_lost_ack_is_answered_again() {
        let mut s = server();
        let wrq = TftpPacket::Write {
            file: "new".into(),
            mode: TftpMode::Octet,
        }
        .emit();
        let out = s.on_packet(T0, A, 4000, TFTP_PORT, &wrq);
        assert_eq!(pkt(&out[0]), TftpPacket::Ack { block: 0 });
        let tid = out[0].src_port;
        let d1 = TftpPacket::Data {
            block: 1,
            data: vec![9; 512],
        }
        .emit();
        assert_eq!(
            pkt(&s.on_packet(T0, A, 4000, tid, &d1)[0]),
            TftpPacket::Ack { block: 1 }
        );
        // The ACK was lost: the same block comes again and is acknowledged again, stored once.
        assert_eq!(
            pkt(&s.on_packet(T0, A, 4000, tid, &d1)[0]),
            TftpPacket::Ack { block: 1 }
        );
        let d2 = TftpPacket::Data {
            block: 2,
            data: vec![8; 5],
        }
        .emit();
        assert_eq!(
            pkt(&s.on_packet(T0, A, 4000, tid, &d2)[0]),
            TftpPacket::Ack { block: 2 }
        );
        assert_eq!(s.files["new"].len(), 517);
        assert!(!s.wants_tick());
    }

    #[test]
    fn a_stranger_on_a_transfer_port_is_told_and_the_transfer_goes_on() {
        let mut s = server();
        s.files.insert("f".into(), vec![0; 600]);
        let tid = read(&mut s, "f")[0].src_port;
        let out = s.on_packet(
            T0,
            Ipv4Addr::new(10, 0, 0, 9),
            5000,
            tid,
            &TftpPacket::Ack { block: 1 }.emit(),
        );
        assert!(matches!(
            pkt(&out[0]),
            TftpPacket::Error {
                code: TFTP_ERR_UNKNOWN_ID,
                ..
            }
        ));
        assert!(s.wants_tick());
    }

    #[test]
    fn silence_resends_the_last_packet_and_in_the_end_drops_the_transfer() {
        let mut s = server();
        s.files.insert("f".into(), vec![0; 600]);
        let first = read(&mut s, "f");
        let mut now = T0;
        assert!(s.tick(now + SimTime::from_secs(1)).is_empty());
        for _ in 0..MAX_RETRIES {
            now = now + RETRY;
            let out = s.tick(now);
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].payload, first[0].payload);
        }
        now = now + RETRY;
        assert!(s.tick(now).is_empty());
        assert!(!s.wants_tick());
        assert!(s.log.last().unwrap().contains("timed out"));
    }

    #[test]
    fn a_stopped_server_says_nothing_and_a_repeated_request_is_not_a_second_transfer() {
        let mut s = TftpServer::default();
        assert!(read(&mut s, "f").is_empty());
        s.running = true;
        s.files.insert("f".into(), vec![1]);
        assert_eq!(read(&mut s, "f").len(), 1);
        assert!(read(&mut s, "f").is_empty());
    }

    #[test]
    fn junk_gets_an_error_not_a_panic() {
        let mut s = server();
        for n in 0..8 {
            let _ = s.on_packet(T0, A, 4000, TFTP_PORT, &vec![0xee; n]);
            let _ = s.on_packet(T0, A, 4000, 49152, &vec![n as u8; n]);
        }
    }
}
