//! LACP data units (IEEE 802.1AX, formerly 802.3ad clause 43), as carried in the "slow protocols"
//! Ethernet type. Public standard; layout written from the standard, not from any product.

use crate::{MacAddr, ParseError};

/// Where LACPDUs go: a link-local address that no bridge forwards.
pub const SLOW_PROTOCOLS: MacAddr = MacAddr([0x01, 0x80, 0xc2, 0x00, 0x00, 0x02]);
/// The Ethernet type of slow protocols.
pub const SLOW_PROTOCOLS_TYPE: u16 = 0x8809;

pub const STATE_ACTIVITY: u8 = 1 << 0;
pub const STATE_TIMEOUT: u8 = 1 << 1;
pub const STATE_AGGREGATION: u8 = 1 << 2;
pub const STATE_SYNC: u8 = 1 << 3;
pub const STATE_COLLECTING: u8 = 1 << 4;
pub const STATE_DISTRIBUTING: u8 = 1 << 5;
pub const STATE_DEFAULTED: u8 = 1 << 6;
pub const STATE_EXPIRED: u8 = 1 << 7;

/// One end's description of itself (actor) or of what it last heard (partner).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LacpInfo {
    pub system_priority: u16,
    pub system: MacAddr,
    pub key: u16,
    pub port_priority: u16,
    pub port: u16,
    pub state: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lacpdu {
    pub actor: LacpInfo,
    pub partner: LacpInfo,
}

const LEN: usize = 110;

fn put(out: &mut Vec<u8>, ty: u8, i: &LacpInfo) {
    out.extend_from_slice(&[ty, 20]);
    out.extend_from_slice(&i.system_priority.to_be_bytes());
    out.extend_from_slice(&i.system.0);
    out.extend_from_slice(&i.key.to_be_bytes());
    out.extend_from_slice(&i.port_priority.to_be_bytes());
    out.extend_from_slice(&i.port.to_be_bytes());
    out.push(i.state);
    out.extend_from_slice(&[0; 3]);
}

fn take(b: &[u8]) -> LacpInfo {
    LacpInfo {
        system_priority: u16::from_be_bytes([b[0], b[1]]),
        system: MacAddr(b[2..8].try_into().unwrap()),
        key: u16::from_be_bytes([b[8], b[9]]),
        port_priority: u16::from_be_bytes([b[10], b[11]]),
        port: u16::from_be_bytes([b[12], b[13]]),
        state: b[14],
    }
}

impl Lacpdu {
    /// The payload after the Ethernet header: subtype 1, version 1, actor, partner, collector,
    /// terminator, padded to 110 bytes.
    pub fn emit(&self) -> Vec<u8> {
        let mut out = vec![1, 1];
        put(&mut out, 1, &self.actor);
        put(&mut out, 2, &self.partner);
        out.extend_from_slice(&[3, 16, 0, 0]);
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&[0, 0]);
        out.resize(LEN, 0);
        out
    }

    pub fn parse(b: &[u8]) -> Result<Lacpdu, ParseError> {
        if b.len() < 2 + 20 + 20 {
            return Err(ParseError::Truncated);
        }
        if b[0] != 1 {
            return Err(ParseError::Unsupported);
        }
        if b[2] != 1 || b[3] != 20 || b[22] != 2 || b[23] != 20 {
            return Err(ParseError::BadLength);
        }
        Ok(Lacpdu {
            actor: take(&b[4..]),
            partner: take(&b[24..]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(n: u8) -> LacpInfo {
        LacpInfo {
            system_priority: 32768,
            system: MacAddr([2, 0, 0, 0, 0, n]),
            key: u16::from(n),
            port_priority: 32768,
            port: u16::from(n) + 1,
            state: STATE_ACTIVITY | STATE_AGGREGATION | STATE_SYNC,
        }
    }

    #[test]
    fn round_trip_and_length() {
        let d = Lacpdu {
            actor: info(1),
            partner: info(2),
        };
        let b = d.emit();
        assert_eq!(b.len(), 110);
        assert_eq!(Lacpdu::parse(&b), Ok(d));
    }

    #[test]
    fn rubbish_is_an_error_not_a_panic() {
        for n in 0..120 {
            let junk = vec![0xa5; n];
            let _ = Lacpdu::parse(&junk);
            let mut b = Lacpdu {
                actor: info(1),
                partner: info(2),
            }
            .emit();
            b.truncate(n);
            let _ = Lacpdu::parse(&b);
        }
        assert_eq!(Lacpdu::parse(&[2; 60]), Err(ParseError::Unsupported));
    }
}
