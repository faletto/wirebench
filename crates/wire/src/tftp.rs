//! TFTP (RFC 1350): read and write requests, data, acknowledgements and errors. No option
//! extensions (RFC 2347), so block size is always 512.

use crate::ParseError;

pub const TFTP_PORT: u16 = 69;
pub const BLOCK_SIZE: usize = 512;

/// Error codes of RFC 1350.
pub const ERR_UNDEFINED: u16 = 0;
pub const ERR_FILE_NOT_FOUND: u16 = 1;
pub const ERR_ACCESS: u16 = 2;
pub const ERR_DISK_FULL: u16 = 3;
pub const ERR_ILLEGAL_OP: u16 = 4;
pub const ERR_UNKNOWN_ID: u16 = 5;
pub const ERR_EXISTS: u16 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Netascii,
    Octet,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Netascii => "netascii",
            Mode::Octet => "octet",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TftpPacket {
    Read { file: String, mode: Mode },
    Write { file: String, mode: Mode },
    Data { block: u16, data: Vec<u8> },
    Ack { block: u16 },
    Error { code: u16, message: String },
}

impl TftpPacket {
    pub fn emit(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            TftpPacket::Read { file, mode } | TftpPacket::Write { file, mode } => {
                let op: u16 = if matches!(self, TftpPacket::Read { .. }) {
                    1
                } else {
                    2
                };
                o.extend_from_slice(&op.to_be_bytes());
                o.extend_from_slice(file.as_bytes());
                o.push(0);
                o.extend_from_slice(mode.name().as_bytes());
                o.push(0);
            }
            TftpPacket::Data { block, data } => {
                o.extend_from_slice(&3u16.to_be_bytes());
                o.extend_from_slice(&block.to_be_bytes());
                o.extend_from_slice(&data[..data.len().min(BLOCK_SIZE)]);
            }
            TftpPacket::Ack { block } => {
                o.extend_from_slice(&4u16.to_be_bytes());
                o.extend_from_slice(&block.to_be_bytes());
            }
            TftpPacket::Error { code, message } => {
                o.extend_from_slice(&5u16.to_be_bytes());
                o.extend_from_slice(&code.to_be_bytes());
                o.extend_from_slice(message.as_bytes());
                o.push(0);
            }
        }
        o
    }

    pub fn parse(b: &[u8]) -> Result<TftpPacket, ParseError> {
        if b.len() < 4 {
            return Err(ParseError::Truncated);
        }
        let op = u16::from_be_bytes([b[0], b[1]]);
        let num = u16::from_be_bytes([b[2], b[3]]);
        match op {
            1 | 2 => {
                let mut parts = b[2..].split(|&c| c == 0);
                let file = parts
                    .next()
                    .filter(|f| !f.is_empty())
                    .ok_or(ParseError::BadLength)?;
                let mode = parts.next().ok_or(ParseError::Truncated)?;
                // The mode must be closed by a zero byte of its own.
                if b[2..].iter().filter(|&&c| c == 0).count() < 2 {
                    return Err(ParseError::Truncated);
                }
                let mode = match String::from_utf8_lossy(mode).to_ascii_lowercase().as_str() {
                    "netascii" => Mode::Netascii,
                    "octet" => Mode::Octet,
                    _ => return Err(ParseError::Unsupported),
                };
                let file = String::from_utf8_lossy(file).into_owned();
                Ok(if op == 1 {
                    TftpPacket::Read { file, mode }
                } else {
                    TftpPacket::Write { file, mode }
                })
            }
            3 => {
                if b.len() > 4 + BLOCK_SIZE {
                    return Err(ParseError::BadLength);
                }
                Ok(TftpPacket::Data {
                    block: num,
                    data: b[4..].to_vec(),
                })
            }
            4 => {
                if b.len() != 4 {
                    return Err(ParseError::BadLength);
                }
                Ok(TftpPacket::Ack { block: num })
            }
            5 => {
                let m = &b[4..];
                let end = m
                    .iter()
                    .position(|&c| c == 0)
                    .ok_or(ParseError::Truncated)?;
                Ok(TftpPacket::Error {
                    code: num,
                    message: String::from_utf8_lossy(&m[..end]).into_owned(),
                })
            }
            _ => Err(ParseError::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_packet_kind_round_trips() {
        for p in [
            TftpPacket::Read {
                file: "r1-confg".into(),
                mode: Mode::Octet,
            },
            TftpPacket::Write {
                file: "dir/r2.cfg".into(),
                mode: Mode::Netascii,
            },
            TftpPacket::Data {
                block: 1,
                data: vec![7; BLOCK_SIZE],
            },
            TftpPacket::Data {
                block: 65535,
                data: vec![],
            },
            TftpPacket::Ack { block: 0 },
            TftpPacket::Error {
                code: ERR_FILE_NOT_FOUND,
                message: "File not found".into(),
            },
        ] {
            assert_eq!(TftpPacket::parse(&p.emit()), Ok(p));
        }
    }

    #[test]
    fn the_layout_is_the_rfc_one() {
        let rrq = TftpPacket::Read {
            file: "a".into(),
            mode: Mode::Octet,
        }
        .emit();
        assert_eq!(rrq, b"\x00\x01a\x00octet\x00");
        assert_eq!(TftpPacket::Ack { block: 258 }.emit(), [0, 4, 1, 2]);
        assert_eq!(
            TftpPacket::Error {
                code: 1,
                message: "x".into()
            }
            .emit(),
            b"\x00\x05\x00\x01x\x00"
        );
    }

    #[test]
    fn modes_are_case_insensitive_and_junk_is_refused() {
        assert!(TftpPacket::parse(b"\x00\x01f\x00OcTeT\x00").is_ok());
        assert_eq!(
            TftpPacket::parse(b"\x00\x01f\x00mail\x00"),
            Err(ParseError::Unsupported)
        );
        assert!(TftpPacket::parse(b"\x00\x01f\x00octet").is_err());
        assert!(TftpPacket::parse(b"\x00\x01\x00octet\x00").is_err());
        assert!(TftpPacket::parse(b"\x00\x09\x00\x00").is_err());
        assert!(TftpPacket::parse(
            &[0, 3, 0, 1]
                .iter()
                .copied()
                .chain([0u8; 513])
                .collect::<Vec<_>>()
        )
        .is_err());
        assert!(TftpPacket::parse(&[0, 4, 0, 1, 9]).is_err());
        assert!(TftpPacket::parse(b"\x00\x05\x00\x01no end").is_err());
        for n in 0..24usize {
            for fill in [0u8, 1, 2, 3, 5, 0xff] {
                let _ = TftpPacket::parse(&vec![fill; n]);
            }
        }
    }
}
