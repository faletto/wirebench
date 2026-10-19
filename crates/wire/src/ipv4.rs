use crate::{internet_checksum, Ipv4Addr, ParseError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IpProto(pub u8);

impl IpProto {
    pub const ICMP: IpProto = IpProto(1);
    pub const TCP: IpProto = IpProto(6);
    pub const UDP: IpProto = IpProto(17);
}

/// IPv4 packet without options (IHL = 5). Packets with options parse as `Unsupported` for now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ipv4Packet {
    pub dscp_ecn: u8,
    pub ident: u16,
    pub flags_frag: u16,
    pub ttl: u8,
    pub proto: IpProto,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub payload: Vec<u8>,
}

impl Ipv4Packet {
    pub const HEADER_LEN: usize = 20;

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < Self::HEADER_LEN {
            return Err(ParseError::Truncated);
        }
        if b[0] >> 4 != 4 {
            return Err(ParseError::Unsupported);
        }
        if b[0] & 0x0f != 5 {
            return Err(ParseError::Unsupported);
        }
        let total = u16::from_be_bytes([b[2], b[3]]) as usize;
        if total < Self::HEADER_LEN || total > b.len() {
            return Err(ParseError::BadLength);
        }
        if internet_checksum(&b[..Self::HEADER_LEN]) != 0 {
            return Err(ParseError::BadChecksum);
        }
        Ok(Ipv4Packet {
            dscp_ecn: b[1],
            ident: u16::from_be_bytes([b[4], b[5]]),
            flags_frag: u16::from_be_bytes([b[6], b[7]]),
            ttl: b[8],
            proto: IpProto(b[9]),
            src: Ipv4Addr(b[12..16].try_into().unwrap()),
            dst: Ipv4Addr(b[16..20].try_into().unwrap()),
            // Ethernet padding beyond `total` is discarded.
            payload: b[Self::HEADER_LEN..total].to_vec(),
        })
    }

    pub fn emit(&self) -> Vec<u8> {
        let total = (Self::HEADER_LEN + self.payload.len()) as u16;
        let mut o = Vec::with_capacity(total as usize);
        o.push(0x45);
        o.push(self.dscp_ecn);
        o.extend_from_slice(&total.to_be_bytes());
        o.extend_from_slice(&self.ident.to_be_bytes());
        o.extend_from_slice(&self.flags_frag.to_be_bytes());
        o.push(self.ttl);
        o.push(self.proto.0);
        o.extend_from_slice(&[0, 0]);
        o.extend_from_slice(&self.src.0);
        o.extend_from_slice(&self.dst.0);
        let c = internet_checksum(&o);
        o[10..12].copy_from_slice(&c.to_be_bytes());
        o.extend_from_slice(&self.payload);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn ipv4_roundtrip(
            ident in any::<u16>(), ttl in any::<u8>(), proto in any::<u8>(),
            s in any::<[u8;4]>(), d in any::<[u8;4]>(),
            payload in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let p = Ipv4Packet {
                dscp_ecn: 0, ident, flags_frag: 0, ttl, proto: IpProto(proto),
                src: Ipv4Addr(s), dst: Ipv4Addr(d), payload,
            };
            prop_assert_eq!(Ipv4Packet::parse(&p.emit()).unwrap(), p);
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = Ipv4Packet::parse(&b);
        }
    }

    #[test]
    fn corrupt_header_fails_checksum() {
        let p = Ipv4Packet {
            dscp_ecn: 0,
            ident: 1,
            flags_frag: 0,
            ttl: 64,
            proto: IpProto::ICMP,
            src: Ipv4Addr::new(10, 0, 0, 1),
            dst: Ipv4Addr::new(10, 0, 0, 2),
            payload: vec![1, 2],
        };
        let mut b = p.emit();
        b[8] ^= 1;
        assert_eq!(Ipv4Packet::parse(&b), Err(ParseError::BadChecksum));
    }

    #[test]
    fn trailing_padding_is_dropped() {
        let p = Ipv4Packet {
            dscp_ecn: 0,
            ident: 1,
            flags_frag: 0,
            ttl: 64,
            proto: IpProto::ICMP,
            src: Ipv4Addr::new(10, 0, 0, 1),
            dst: Ipv4Addr::new(10, 0, 0, 2),
            payload: vec![9],
        };
        let mut b = p.emit();
        b.extend_from_slice(&[0; 20]);
        assert_eq!(Ipv4Packet::parse(&b).unwrap(), p);
    }
}
