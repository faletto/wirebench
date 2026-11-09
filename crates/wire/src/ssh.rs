//! The part of SSH (RFC 4253, 4252, 4254) a teaching simulator needs on the wire: the
//! identification string, and framed messages for authentication and session data.
//!
//! This is **not** a secure channel. Key exchange, encryption and MACs are not simulated: the
//! framing is `uint32 length, byte message-number, payload` (RFC 4253 section 6 without padding,
//! compression or a MAC), with the message numbers and string encodings of the real protocol.
//! A student sees the same sequence of events (version exchange, userauth, a data channel) that a
//! packet capture of a real session would show, minus the encrypted bytes.

use crate::ParseError;

pub const SSH_PORT: u16 = 22;

pub const MSG_DISCONNECT: u8 = 1;
pub const MSG_USERAUTH_REQUEST: u8 = 50;
pub const MSG_USERAUTH_FAILURE: u8 = 51;
pub const MSG_USERAUTH_SUCCESS: u8 = 52;
pub const MSG_CHANNEL_DATA: u8 = 94;

/// Software version string we announce.
pub const SOFTWARE: &str = "wirebench_1.0";

/// The line each side sends first.
pub fn ident() -> Vec<u8> {
    format!("SSH-2.0-{SOFTWARE}\r\n").into_bytes()
}

/// What an identification line (without its line end) says, or why it is not acceptable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ident {
    /// Protocol 2.0 (or 1.99, which also speaks 2.0), with the software version.
    V2(String),
    /// Protocol 1.x: not supported here.
    V1,
    NotSsh,
}

pub fn parse_ident(line: &str) -> Ident {
    let Some(rest) = line.strip_prefix("SSH-") else {
        return Ident::NotSsh;
    };
    let Some((proto, software)) = rest.split_once('-') else {
        return Ident::NotSsh;
    };
    match proto {
        "2.0" | "1.99" => Ident::V2(software.to_string()),
        p if p.starts_with("1.") => Ident::V1,
        _ => Ident::NotSsh,
    }
}

/// `string` of RFC 4251 section 5.
pub fn put_string(o: &mut Vec<u8>, s: &[u8]) {
    o.extend_from_slice(&(s.len() as u32).to_be_bytes());
    o.extend_from_slice(s);
}

pub fn get_string<'a>(b: &mut &'a [u8]) -> Result<&'a [u8], ParseError> {
    if b.len() < 4 {
        return Err(ParseError::Truncated);
    }
    let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    if b.len() < 4 + n {
        return Err(ParseError::Truncated);
    }
    let s = &b[4..4 + n];
    *b = &b[4 + n..];
    Ok(s)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub msg: u8,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn emit(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(5 + self.payload.len());
        o.extend_from_slice(&(self.payload.len() as u32 + 1).to_be_bytes());
        o.push(self.msg);
        o.extend_from_slice(&self.payload);
        o
    }

    /// `SSH_MSG_USERAUTH_REQUEST` for the password method.
    pub fn auth_password(user: &str, password: &str) -> Packet {
        let mut p = vec![];
        put_string(&mut p, user.as_bytes());
        put_string(&mut p, b"ssh-connection");
        put_string(&mut p, b"password");
        p.push(0);
        put_string(&mut p, password.as_bytes());
        Packet {
            msg: MSG_USERAUTH_REQUEST,
            payload: p,
        }
    }

    /// The user and password out of a password `USERAUTH_REQUEST`.
    pub fn parse_auth(&self) -> Result<(String, String), ParseError> {
        let mut b = self.payload.as_slice();
        let user = get_string(&mut b)?;
        let _service = get_string(&mut b)?;
        let method = get_string(&mut b)?;
        if method != b"password" || b.is_empty() {
            return Err(ParseError::BadLength);
        }
        b = &b[1..];
        let pw = get_string(&mut b)?;
        Ok((
            String::from_utf8_lossy(user).into_owned(),
            String::from_utf8_lossy(pw).into_owned(),
        ))
    }

    pub fn data(bytes: &[u8]) -> Packet {
        let mut p = vec![];
        put_string(&mut p, bytes);
        Packet {
            msg: MSG_CHANNEL_DATA,
            payload: p,
        }
    }

    pub fn parse_data(&self) -> Result<Vec<u8>, ParseError> {
        let mut b = self.payload.as_slice();
        Ok(get_string(&mut b)?.to_vec())
    }

    pub fn disconnect(reason: &str) -> Packet {
        let mut p = vec![];
        p.extend_from_slice(&11u32.to_be_bytes()); // SSH_DISCONNECT_BY_APPLICATION
        put_string(&mut p, reason.as_bytes());
        put_string(&mut p, b"");
        Packet {
            msg: MSG_DISCONNECT,
            payload: p,
        }
    }
}

/// Collects a TCP byte stream into the identification line and then packets.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    got_ident: bool,
}

impl Decoder {
    /// The peer's identification line, once it has arrived (before any packet).
    pub fn take_ident(&mut self, bytes: &[u8]) -> Option<String> {
        self.buf.extend_from_slice(bytes);
        if self.got_ident {
            return None;
        }
        let end = self.buf.iter().position(|b| *b == b'\n')?;
        let line: Vec<u8> = self.buf.drain(..=end).collect();
        self.got_ident = true;
        Some(
            String::from_utf8_lossy(&line)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        )
    }

    pub fn has_ident(&self) -> bool {
        self.got_ident
    }

    /// Add bytes and return the packets now complete. A length that is absurd (or zero) is a
    /// protocol error: `Err`.
    pub fn packets(&mut self, bytes: &[u8]) -> Result<Vec<Packet>, ParseError> {
        self.buf.extend_from_slice(bytes);
        let mut out = vec![];
        loop {
            if self.buf.len() < 5 {
                return Ok(out);
            }
            let n = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]])
                as usize;
            if n == 0 || n > 35_000 {
                return Err(ParseError::BadLength);
            }
            if self.buf.len() < 4 + n {
                return Ok(out);
            }
            let msg = self.buf[4];
            let payload = self.buf[5..4 + n].to_vec();
            self.buf.drain(..4 + n);
            out.push(Packet { msg, payload });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identification_lines() {
        assert_eq!(parse_ident("SSH-2.0-OpenSSH_9.6"), Ident::V2("OpenSSH_9.6".into()));
        assert_eq!(parse_ident("SSH-1.99-x"), Ident::V2("x".into()));
        assert_eq!(parse_ident("SSH-1.5-old"), Ident::V1);
        assert_eq!(parse_ident("HELO"), Ident::NotSsh);
        assert_eq!(String::from_utf8(ident()).unwrap(), "SSH-2.0-wirebench_1.0\r\n");
    }

    #[test]
    fn auth_and_data_round_trip_across_split_reads() {
        let mut stream = ident();
        stream.extend(Packet::auth_password("admin", "s3cret").emit());
        stream.extend(Packet::data(b"show version\r").emit());
        let mut d = Decoder::default();
        let mut ident_seen = None;
        let mut pkts = vec![];
        for chunk in stream.chunks(7) {
            if !d.has_ident() {
                ident_seen = d.take_ident(chunk).or(ident_seen);
                if d.has_ident() {
                    pkts.extend(d.packets(&[]).unwrap());
                }
            } else {
                pkts.extend(d.packets(chunk).unwrap());
            }
        }
        assert_eq!(ident_seen.as_deref(), Some("SSH-2.0-wirebench_1.0"));
        assert_eq!(pkts.len(), 2);
        assert_eq!(pkts[0].parse_auth().unwrap(), ("admin".into(), "s3cret".into()));
        assert_eq!(pkts[1].parse_data().unwrap(), b"show version\r");
    }

    #[test]
    fn nonsense_lengths_are_errors_not_panics() {
        let mut d = Decoder::default();
        assert!(d.packets(&[0xff, 0xff, 0xff, 0xff, 1]).is_err());
        let mut d = Decoder::default();
        assert!(d.packets(&[0, 0, 0, 0, 1]).is_err());
    }
}
