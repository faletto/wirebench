//! First hop redundancy packets: HSRP version 1 (RFC 2281) and VRRP version 2 (RFC 3768).

use crate::{internet_checksum, Ipv4Addr, MacAddr, ParseError};

/// HSRP speaks UDP port 1985 to 224.0.0.2, VRRP uses IP protocol 112 to 224.0.0.18.
pub const HSRP_PORT: u16 = 1985;
pub const HSRP_GROUP: Ipv4Addr = Ipv4Addr([224, 0, 0, 2]);
pub const VRRP_PROTO: u8 = 112;
pub const VRRP_GROUP: Ipv4Addr = Ipv4Addr([224, 0, 0, 18]);
/// The Ethernet multicast addresses those map to.
pub const HSRP_MAC: MacAddr = MacAddr([0x01, 0x00, 0x5e, 0x00, 0x00, 0x02]);
pub const VRRP_MAC: MacAddr = MacAddr([0x01, 0x00, 0x5e, 0x00, 0x00, 0x12]);

/// The virtual MAC of HSRP group `g`: `0000.0c07.acNN` (RFC 2281).
pub fn hsrp_vmac(g: u8) -> MacAddr {
    MacAddr([0x00, 0x00, 0x0c, 0x07, 0xac, g])
}

/// The virtual MAC of VRRP group `g`: `0000.5e00.01NN` (RFC 3768).
pub fn vrrp_vmac(g: u8) -> MacAddr {
    MacAddr([0x00, 0x00, 0x5e, 0x00, 0x01, g])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HsrpOp {
    Hello,
    Coup,
    Resign,
}

/// HSRP state numbers as they appear on the wire.
pub mod hsrp_state {
    pub const INITIAL: u8 = 0;
    pub const LEARN: u8 = 1;
    pub const LISTEN: u8 = 2;
    pub const SPEAK: u8 = 4;
    pub const STANDBY: u8 = 8;
    pub const ACTIVE: u8 = 16;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hsrp {
    pub op: HsrpOp,
    pub state: u8,
    pub hello: u8,
    pub hold: u8,
    pub priority: u8,
    pub group: u8,
    pub vip: Ipv4Addr,
}

const AUTH: [u8; 8] = *b"cisco\0\0\0";

impl Hsrp {
    pub const LEN: usize = 20;

    pub fn emit(&self) -> Vec<u8> {
        let mut o = vec![
            0,
            match self.op {
                HsrpOp::Hello => 0,
                HsrpOp::Coup => 1,
                HsrpOp::Resign => 2,
            },
            self.state,
            self.hello,
            self.hold,
            self.priority,
            self.group,
            0,
        ];
        o.extend_from_slice(&AUTH);
        o.extend_from_slice(&self.vip.0);
        o
    }

    pub fn parse(b: &[u8]) -> Result<Hsrp, ParseError> {
        if b.len() < Self::LEN {
            return Err(ParseError::Truncated);
        }
        if b[0] != 0 {
            return Err(ParseError::Unsupported);
        }
        let op = match b[1] {
            0 => HsrpOp::Hello,
            1 => HsrpOp::Coup,
            2 => HsrpOp::Resign,
            _ => return Err(ParseError::Unsupported),
        };
        Ok(Hsrp {
            op,
            state: b[2],
            hello: b[3],
            hold: b[4],
            priority: b[5],
            group: b[6],
            vip: Ipv4Addr(b[16..20].try_into().unwrap()),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vrrp {
    pub group: u8,
    pub priority: u8,
    /// Advertisement interval in seconds.
    pub advert: u8,
    pub ips: Vec<Ipv4Addr>,
}

impl Vrrp {
    pub fn emit(&self) -> Vec<u8> {
        let mut o = vec![
            0x21,
            self.group,
            self.priority,
            self.ips.len() as u8,
            0,
            self.advert,
            0,
            0,
        ];
        for ip in &self.ips {
            o.extend_from_slice(&ip.0);
        }
        o.extend_from_slice(&[0; 8]);
        let c = internet_checksum(&o);
        o[6..8].copy_from_slice(&c.to_be_bytes());
        o
    }

    pub fn parse(b: &[u8]) -> Result<Vrrp, ParseError> {
        if b.len() < 8 {
            return Err(ParseError::Truncated);
        }
        if b[0] != 0x21 {
            return Err(ParseError::Unsupported);
        }
        let n = b[3] as usize;
        if b.len() < 8 + 4 * n {
            return Err(ParseError::BadLength);
        }
        let mut copy = b.to_vec();
        copy[6] = 0;
        copy[7] = 0;
        if internet_checksum(&copy) != u16::from_be_bytes([b[6], b[7]]) {
            return Err(ParseError::BadChecksum);
        }
        Ok(Vrrp {
            group: b[1],
            priority: b[2],
            advert: b[5],
            ips: (0..n)
                .map(|i| Ipv4Addr(b[8 + 4 * i..12 + 4 * i].try_into().unwrap()))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsrp_round_trips_and_has_the_documented_size() {
        let h = Hsrp {
            op: HsrpOp::Coup,
            state: hsrp_state::ACTIVE,
            hello: 3,
            hold: 10,
            priority: 110,
            group: 7,
            vip: Ipv4Addr([10, 0, 0, 254]),
        };
        assert_eq!(h.emit().len(), 20);
        assert_eq!(Hsrp::parse(&h.emit()), Ok(h));
    }

    #[test]
    fn vrrp_round_trips_and_checks_its_checksum() {
        let v = Vrrp {
            group: 3,
            priority: 255,
            advert: 1,
            ips: vec![Ipv4Addr([10, 0, 0, 1]), Ipv4Addr([10, 0, 0, 2])],
        };
        let mut b = v.emit();
        assert_eq!(Vrrp::parse(&b), Ok(v));
        b[9] ^= 1;
        assert_eq!(Vrrp::parse(&b), Err(ParseError::BadChecksum));
    }

    #[test]
    fn virtual_macs_follow_the_rfcs() {
        assert_eq!(hsrp_vmac(1).to_string(), "0000.0c07.ac01");
        assert_eq!(vrrp_vmac(10).to_string(), "0000.5e00.010a");
    }

    #[test]
    fn junk_is_refused_not_panicked_on() {
        let b = Vrrp {
            group: 1,
            priority: 1,
            advert: 1,
            ips: vec![Ipv4Addr([1, 2, 3, 4])],
        }
        .emit();
        for n in 0..b.len() {
            let _ = Vrrp::parse(&b[..n]);
            let _ = Hsrp::parse(&b[..n]);
        }
        assert_eq!(Hsrp::parse(&[9; 20]), Err(ParseError::Unsupported));
    }
}
