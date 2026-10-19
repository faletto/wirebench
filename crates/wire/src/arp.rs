use crate::{Ipv4Addr, MacAddr, ParseError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArpOp {
    Request,
    Reply,
}

/// ARP for Ethernet/IPv4 only (RFC 826).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArpPacket {
    pub op: ArpOp,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip: Ipv4Addr,
}

impl ArpPacket {
    pub const LEN: usize = 28;

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < Self::LEN {
            return Err(ParseError::Truncated);
        }
        // htype=1, ptype=0x0800, hlen=6, plen=4
        if b[0..6] != [0, 1, 8, 0, 6, 4] {
            return Err(ParseError::Unsupported);
        }
        let op = match u16::from_be_bytes([b[6], b[7]]) {
            1 => ArpOp::Request,
            2 => ArpOp::Reply,
            _ => return Err(ParseError::Unsupported),
        };
        Ok(ArpPacket {
            op,
            sender_mac: MacAddr(b[8..14].try_into().unwrap()),
            sender_ip: Ipv4Addr(b[14..18].try_into().unwrap()),
            target_mac: MacAddr(b[18..24].try_into().unwrap()),
            target_ip: Ipv4Addr(b[24..28].try_into().unwrap()),
        })
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = vec![
            0,
            1,
            8,
            0,
            6,
            4,
            0,
            if self.op == ArpOp::Request { 1 } else { 2 },
        ];
        o.extend_from_slice(&self.sender_mac.0);
        o.extend_from_slice(&self.sender_ip.0);
        o.extend_from_slice(&self.target_mac.0);
        o.extend_from_slice(&self.target_ip.0);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arp_roundtrip() {
        let p = ArpPacket {
            op: ArpOp::Request,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::default(),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        assert_eq!(ArpPacket::parse(&p.emit()).unwrap(), p);
        assert_eq!(p.emit().len(), ArpPacket::LEN);
    }

    #[test]
    fn arp_rejects_garbage() {
        assert_eq!(ArpPacket::parse(&[0; 10]), Err(ParseError::Truncated));
        assert_eq!(ArpPacket::parse(&[0; 28]), Err(ParseError::Unsupported));
    }
}
