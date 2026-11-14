//! DNS messages (RFC 1035) over UDP port 53: the header, questions and answer records. A and
//! CNAME records are understood; any other record type keeps its data as bytes. Authority and
//! additional records are skipped when reading and never written. Names are read with compression
//! pointers and written without.

use crate::{Ipv4Addr, ParseError};

pub const DNS_PORT: u16 = 53;
pub const TYPE_A: u16 = 1;
pub const TYPE_CNAME: u16 = 5;
pub const CLASS_IN: u16 = 1;
pub const RCODE_OK: u8 = 0;
pub const RCODE_FORMERR: u8 = 1;
pub const RCODE_SERVFAIL: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;
pub const RCODE_NOTIMP: u8 = 4;
pub const RCODE_REFUSED: u8 = 5;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RData {
    A(Ipv4Addr),
    Cname(String),
    Other(Vec<u8>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DnsMessage {
    pub id: u16,
    pub response: bool,
    pub opcode: u8,
    pub authoritative: bool,
    pub truncated: bool,
    pub recursion_desired: bool,
    pub recursion_available: bool,
    pub rcode: u8,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
}

impl DnsMessage {
    /// A standard query for the A record of `name`.
    pub fn query(id: u16, name: &str) -> DnsMessage {
        DnsMessage {
            id,
            response: false,
            opcode: 0,
            authoritative: false,
            truncated: false,
            recursion_desired: true,
            recursion_available: false,
            rcode: RCODE_OK,
            questions: vec![Question {
                name: name.to_string(),
                qtype: TYPE_A,
                qclass: CLASS_IN,
            }],
            answers: Vec::new(),
        }
    }

    /// An answer to `q` carrying `answers` (empty for NXDOMAIN and the like).
    pub fn reply(q: &DnsMessage, rcode: u8, answers: Vec<Record>) -> DnsMessage {
        DnsMessage {
            id: q.id,
            response: true,
            opcode: q.opcode,
            authoritative: rcode == RCODE_OK || rcode == RCODE_NXDOMAIN,
            truncated: false,
            recursion_desired: q.recursion_desired,
            recursion_available: false,
            rcode,
            questions: q.questions.clone(),
            answers,
        }
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(&self.id.to_be_bytes());
        let flags = (u16::from(self.response) << 15)
            | (u16::from(self.opcode & 15) << 11)
            | (u16::from(self.authoritative) << 10)
            | (u16::from(self.truncated) << 9)
            | (u16::from(self.recursion_desired) << 8)
            | (u16::from(self.recursion_available) << 7)
            | u16::from(self.rcode & 15);
        o.extend_from_slice(&flags.to_be_bytes());
        o.extend_from_slice(&(self.questions.len() as u16).to_be_bytes());
        o.extend_from_slice(&(self.answers.len() as u16).to_be_bytes());
        o.extend_from_slice(&[0; 4]);
        for q in &self.questions {
            emit_name(&mut o, &q.name);
            o.extend_from_slice(&q.qtype.to_be_bytes());
            o.extend_from_slice(&q.qclass.to_be_bytes());
        }
        for r in &self.answers {
            emit_name(&mut o, &r.name);
            o.extend_from_slice(&r.rtype.to_be_bytes());
            o.extend_from_slice(&r.class.to_be_bytes());
            o.extend_from_slice(&r.ttl.to_be_bytes());
            let mut rd = Vec::new();
            match &r.data {
                RData::A(a) => rd.extend_from_slice(&a.0),
                RData::Cname(n) => emit_name(&mut rd, n),
                RData::Other(b) => rd.extend_from_slice(b),
            }
            o.extend_from_slice(&(rd.len() as u16).to_be_bytes());
            o.extend_from_slice(&rd);
        }
        o
    }

    pub fn parse(b: &[u8]) -> Result<DnsMessage, ParseError> {
        if b.len() < 12 {
            return Err(ParseError::Truncated);
        }
        let flags = u16::from_be_bytes([b[2], b[3]]);
        let (qd, an) = (
            u16::from_be_bytes([b[4], b[5]]),
            u16::from_be_bytes([b[6], b[7]]),
        );
        let mut at = 12;
        let mut questions = Vec::new();
        for _ in 0..qd {
            let name = read_name(b, &mut at)?;
            let rest = b.get(at..at + 4).ok_or(ParseError::Truncated)?;
            questions.push(Question {
                name,
                qtype: u16::from_be_bytes([rest[0], rest[1]]),
                qclass: u16::from_be_bytes([rest[2], rest[3]]),
            });
            at += 4;
        }
        let mut answers = Vec::new();
        for _ in 0..an {
            let name = read_name(b, &mut at)?;
            let fixed = b.get(at..at + 10).ok_or(ParseError::Truncated)?;
            let rtype = u16::from_be_bytes([fixed[0], fixed[1]]);
            let class = u16::from_be_bytes([fixed[2], fixed[3]]);
            let ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
            let len = u16::from_be_bytes([fixed[8], fixed[9]]) as usize;
            at += 10;
            let rd = b.get(at..at + len).ok_or(ParseError::Truncated)?;
            let data = match (rtype, len) {
                (TYPE_A, 4) => RData::A(Ipv4Addr([rd[0], rd[1], rd[2], rd[3]])),
                (TYPE_A, _) => return Err(ParseError::BadLength),
                (TYPE_CNAME, _) => {
                    let mut p = at;
                    let n = read_name(b, &mut p)?;
                    if p > at + len {
                        return Err(ParseError::BadLength);
                    }
                    RData::Cname(n)
                }
                _ => RData::Other(rd.to_vec()),
            };
            at += len;
            answers.push(Record {
                name,
                rtype,
                class,
                ttl,
                data,
            });
        }
        Ok(DnsMessage {
            id: u16::from_be_bytes([b[0], b[1]]),
            response: flags & 0x8000 != 0,
            opcode: ((flags >> 11) & 15) as u8,
            authoritative: flags & 0x0400 != 0,
            truncated: flags & 0x0200 != 0,
            recursion_desired: flags & 0x0100 != 0,
            recursion_available: flags & 0x0080 != 0,
            rcode: (flags & 15) as u8,
            questions,
            answers,
        })
    }
}

fn emit_name(o: &mut Vec<u8>, name: &str) {
    for label in name
        .trim_end_matches('.')
        .split('.')
        .filter(|l| !l.is_empty())
    {
        let l = &label.as_bytes()[..label.len().min(63)];
        o.push(l.len() as u8);
        o.extend_from_slice(l);
    }
    o.push(0);
}

/// A name starting at `*at`, following compression pointers (backwards only, so they end); `*at`
/// is left after the name as it appears there.
fn read_name(b: &[u8], at: &mut usize) -> Result<String, ParseError> {
    let mut name = String::new();
    let mut p = *at;
    let mut jumped = false;
    let mut hops = 0;
    loop {
        let len = *b.get(p).ok_or(ParseError::Truncated)? as usize;
        if len == 0 {
            if !jumped {
                *at = p + 1;
            }
            return Ok(name);
        }
        if len & 0xc0 == 0xc0 {
            let lo = *b.get(p + 1).ok_or(ParseError::Truncated)? as usize;
            let target = ((len & 0x3f) << 8) | lo;
            hops += 1;
            if target >= p || hops > 64 {
                return Err(ParseError::BadLength);
            }
            if !jumped {
                *at = p + 2;
                jumped = true;
            }
            p = target;
            continue;
        }
        if len > 63 {
            return Err(ParseError::BadLength);
        }
        let label = b.get(p + 1..p + 1 + len).ok_or(ParseError::Truncated)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label));
        if name.len() > 255 {
            return Err(ParseError::BadLength);
        }
        p += 1 + len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_and_its_answer_round_trip() {
        let q = DnsMessage::query(0x1234, "www.example.com");
        assert_eq!(DnsMessage::parse(&q.emit()), Ok(q.clone()));
        let r = DnsMessage::reply(
            &q,
            RCODE_OK,
            vec![
                Record {
                    name: "www.example.com".into(),
                    rtype: TYPE_CNAME,
                    class: CLASS_IN,
                    ttl: 60,
                    data: RData::Cname("web.example.com".into()),
                },
                Record {
                    name: "web.example.com".into(),
                    rtype: TYPE_A,
                    class: CLASS_IN,
                    ttl: 60,
                    data: RData::A(Ipv4Addr::new(10, 0, 0, 5)),
                },
                Record {
                    name: "web.example.com".into(),
                    rtype: 16,
                    class: CLASS_IN,
                    ttl: 5,
                    data: RData::Other(vec![1, b'x']),
                },
            ],
        );
        assert_eq!(DnsMessage::parse(&r.emit()), Ok(r));
    }

    #[test]
    fn the_layout_is_the_rfc_one() {
        let b = DnsMessage::query(1, "a.bc").emit();
        assert_eq!(
            b,
            [0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn compression_pointers_are_followed_but_loops_and_forward_jumps_are_refused() {
        // Header, question "a.bc", then an answer whose name points back at the question.
        let mut b = DnsMessage::query(1, "a.bc").emit();
        b[2] = 0x81;
        b[7] = 1;
        b.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 9, 0, 4, 1, 2, 3, 4]);
        let m = DnsMessage::parse(&b).unwrap();
        assert_eq!(m.answers[0].name, "a.bc");
        assert_eq!(m.answers[0].data, RData::A(Ipv4Addr::new(1, 2, 3, 4)));
        let mut looped = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        looped.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1]);
        assert_eq!(DnsMessage::parse(&looped), Err(ParseError::BadLength));
        let mut fwd = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        fwd.extend_from_slice(&[0xc0, 40, 0, 1, 0, 1]);
        assert!(DnsMessage::parse(&fwd).is_err());
    }

    #[test]
    fn short_and_bent_messages_are_errors_not_panics() {
        assert_eq!(DnsMessage::parse(&[0; 11]), Err(ParseError::Truncated));
        let q = DnsMessage::query(9, "x.y").emit();
        for n in 0..q.len() {
            let _ = DnsMessage::parse(&q[..n]);
        }
        for fill in [0u8, 0x3f, 0xc0, 0xff] {
            for n in 0..40 {
                let mut b = vec![fill; n];
                if n > 5 {
                    b[5] = 1;
                }
                let _ = DnsMessage::parse(&b);
            }
        }
    }
}
