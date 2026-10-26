use crate::{internet_checksum, Ipv4Addr, ParseError};

/// UDP datagram (RFC 768). The checksum is not kept: it is computed from the addresses when
/// emitting and checked, when present, by [`UdpDatagram::checksum_ok`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UdpDatagram {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: Vec<u8>,
}

impl UdpDatagram {
    pub const HEADER_LEN: usize = 8;

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < Self::HEADER_LEN {
            return Err(ParseError::Truncated);
        }
        let len = u16::from_be_bytes([b[4], b[5]]) as usize;
        if len < Self::HEADER_LEN || len > b.len() {
            return Err(ParseError::BadLength);
        }
        Ok(UdpDatagram {
            src_port: u16::from_be_bytes([b[0], b[1]]),
            dst_port: u16::from_be_bytes([b[2], b[3]]),
            // Ethernet padding beyond the UDP length is discarded.
            payload: b[Self::HEADER_LEN..len].to_vec(),
        })
    }

    /// Serialise for a packet from `src` to `dst` (the checksum covers them).
    pub fn emit(&self, src: Ipv4Addr, dst: Ipv4Addr) -> Vec<u8> {
        let len = (Self::HEADER_LEN + self.payload.len()) as u16;
        let mut o = Vec::with_capacity(len as usize);
        o.extend_from_slice(&self.src_port.to_be_bytes());
        o.extend_from_slice(&self.dst_port.to_be_bytes());
        o.extend_from_slice(&len.to_be_bytes());
        o.extend_from_slice(&[0, 0]);
        o.extend_from_slice(&self.payload);
        let c = match checksum(src, dst, &o) {
            0 => 0xffff,
            c => c,
        };
        o[6..8].copy_from_slice(&c.to_be_bytes());
        o
    }

    /// Whether the checksum in `raw` (the bytes this datagram was parsed from) is right for a
    /// packet from `src` to `dst`. A zero checksum means "none sent" and is accepted.
    pub fn checksum_ok(raw: &[u8], src: Ipv4Addr, dst: Ipv4Addr) -> bool {
        if raw.len() < Self::HEADER_LEN {
            return false;
        }
        let len = (u16::from_be_bytes([raw[4], raw[5]]) as usize).min(raw.len());
        raw[6..8] == [0, 0] || checksum(src, dst, &raw[..len]) == 0
    }
}

/// Internet checksum of the pseudo header plus the datagram (all of it, checksum field included).
fn checksum(src: Ipv4Addr, dst: Ipv4Addr, datagram: &[u8]) -> u16 {
    let mut b = Vec::with_capacity(12 + datagram.len());
    b.extend_from_slice(&src.0);
    b.extend_from_slice(&dst.0);
    b.push(0);
    b.push(17);
    b.extend_from_slice(&(datagram.len() as u16).to_be_bytes());
    b.extend_from_slice(datagram);
    internet_checksum(&b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn udp_roundtrip_and_checksum(
            sp in any::<u16>(), dp in any::<u16>(),
            s in any::<[u8;4]>(), d in any::<[u8;4]>(),
            payload in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let u = UdpDatagram { src_port: sp, dst_port: dp, payload };
            let raw = u.emit(Ipv4Addr(s), Ipv4Addr(d));
            prop_assert_eq!(UdpDatagram::parse(&raw).unwrap(), u);
            prop_assert!(UdpDatagram::checksum_ok(&raw, Ipv4Addr(s), Ipv4Addr(d)));
        }

        #[test]
        fn a_flipped_bit_breaks_the_checksum(
            s in any::<[u8;4]>(), d in any::<[u8;4]>(),
            payload in proptest::collection::vec(any::<u8>(), 1..32),
            bit in 0usize..8, at in 8usize..40,
        ) {
            let u = UdpDatagram { src_port: 68, dst_port: 67, payload };
            let mut raw = u.emit(Ipv4Addr(s), Ipv4Addr(d));
            let at = 8 + (at - 8) % (raw.len() - 8);
            raw[at] ^= 1 << bit;
            prop_assert!(!UdpDatagram::checksum_ok(&raw, Ipv4Addr(s), Ipv4Addr(d)));
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..32)) {
            let _ = UdpDatagram::parse(&b);
        }
    }

    #[test]
    fn short_and_inconsistent_lengths_are_rejected() {
        assert_eq!(UdpDatagram::parse(&[0; 7]), Err(ParseError::Truncated));
        let mut b = vec![0, 68, 0, 67, 0, 7, 0, 0];
        assert_eq!(UdpDatagram::parse(&b), Err(ParseError::BadLength));
        b[5] = 20;
        assert_eq!(UdpDatagram::parse(&b), Err(ParseError::BadLength));
    }

    #[test]
    fn padding_after_the_datagram_is_ignored() {
        let u = UdpDatagram {
            src_port: 1,
            dst_port: 2,
            payload: vec![9],
        };
        let mut raw = u.emit(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST);
        raw.extend_from_slice(&[0; 20]);
        assert_eq!(UdpDatagram::parse(&raw).unwrap(), u);
    }
}
