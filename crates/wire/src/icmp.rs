use crate::{internet_checksum, ParseError};

/// ICMP echo request/reply (RFC 792).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IcmpEcho {
    pub reply: bool,
    pub ident: u16,
    pub seq: u16,
    pub data: Vec<u8>,
}

impl IcmpEcho {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < 8 {
            return Err(ParseError::Truncated);
        }
        let reply = match (b[0], b[1]) {
            (8, 0) => false,
            (0, 0) => true,
            _ => return Err(ParseError::Unsupported),
        };
        if internet_checksum(b) != 0 {
            return Err(ParseError::BadChecksum);
        }
        Ok(IcmpEcho {
            reply,
            ident: u16::from_be_bytes([b[4], b[5]]),
            seq: u16::from_be_bytes([b[6], b[7]]),
            data: b[8..].to_vec(),
        })
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = vec![if self.reply { 0 } else { 8 }, 0, 0, 0];
        o.extend_from_slice(&self.ident.to_be_bytes());
        o.extend_from_slice(&self.seq.to_be_bytes());
        o.extend_from_slice(&self.data);
        let c = internet_checksum(&o);
        o[2..4].copy_from_slice(&c.to_be_bytes());
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn icmp_echo_roundtrip(
            reply in any::<bool>(), ident in any::<u16>(), seq in any::<u16>(),
            data in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let p = IcmpEcho { reply, ident, seq, data };
            prop_assert_eq!(IcmpEcho::parse(&p.emit()).unwrap(), p);
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = IcmpEcho::parse(&b);
        }
    }
}
