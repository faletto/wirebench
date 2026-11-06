use crate::{internet_checksum, Ipv4Addr, ParseError};

/// TCP segment header and payload (RFC 9293 section 3.1). Options are skipped when parsing and
/// never written. The checksum is not kept: it is computed from the addresses when emitting and
/// checked by [`TcpSegment::checksum_ok`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TcpSegment {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    /// The eight flag bits, CWR to FIN (see the `FIN`... constants).
    pub flags: u8,
    pub window: u16,
    pub payload: Vec<u8>,
}

impl TcpSegment {
    pub const HEADER_LEN: usize = 20;
    pub const FIN: u8 = 0x01;
    pub const SYN: u8 = 0x02;
    pub const RST: u8 = 0x04;
    pub const PSH: u8 = 0x08;
    pub const ACK: u8 = 0x10;

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < Self::HEADER_LEN {
            return Err(ParseError::Truncated);
        }
        let header = (b[12] >> 4) as usize * 4;
        if header < Self::HEADER_LEN || header > b.len() {
            return Err(ParseError::BadLength);
        }
        Ok(TcpSegment {
            src_port: u16::from_be_bytes([b[0], b[1]]),
            dst_port: u16::from_be_bytes([b[2], b[3]]),
            seq: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            ack: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            flags: b[13],
            window: u16::from_be_bytes([b[14], b[15]]),
            payload: b[header..].to_vec(),
        })
    }

    /// Serialise for a packet from `src` to `dst` (the checksum covers them).
    pub fn emit(&self, src: Ipv4Addr, dst: Ipv4Addr) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::HEADER_LEN + self.payload.len());
        o.extend_from_slice(&self.src_port.to_be_bytes());
        o.extend_from_slice(&self.dst_port.to_be_bytes());
        o.extend_from_slice(&self.seq.to_be_bytes());
        o.extend_from_slice(&self.ack.to_be_bytes());
        o.push(5 << 4);
        o.push(self.flags);
        o.extend_from_slice(&self.window.to_be_bytes());
        o.extend_from_slice(&[0, 0, 0, 0]);
        o.extend_from_slice(&self.payload);
        let c = checksum(src, dst, &o);
        o[16..18].copy_from_slice(&c.to_be_bytes());
        o
    }

    /// Whether the checksum in `raw` is right for a segment from `src` to `dst`.
    pub fn checksum_ok(raw: &[u8], src: Ipv4Addr, dst: Ipv4Addr) -> bool {
        raw.len() >= Self::HEADER_LEN && checksum(src, dst, raw) == 0
    }
}

/// Internet checksum of the pseudo header plus the segment (checksum field included).
fn checksum(src: Ipv4Addr, dst: Ipv4Addr, segment: &[u8]) -> u16 {
    let mut b = Vec::with_capacity(12 + segment.len());
    b.extend_from_slice(&src.0);
    b.extend_from_slice(&dst.0);
    b.push(0);
    b.push(6);
    b.extend_from_slice(&(segment.len() as u16).to_be_bytes());
    b.extend_from_slice(segment);
    internet_checksum(&b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn tcp_roundtrip_and_checksum(
            sp in any::<u16>(), dp in any::<u16>(), seq in any::<u32>(), ack in any::<u32>(),
            flags in any::<u8>(), window in any::<u16>(),
            s in any::<[u8;4]>(), d in any::<[u8;4]>(),
            payload in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let t = TcpSegment { src_port: sp, dst_port: dp, seq, ack, flags, window, payload };
            let raw = t.emit(Ipv4Addr(s), Ipv4Addr(d));
            prop_assert_eq!(TcpSegment::parse(&raw).unwrap(), t);
            prop_assert!(TcpSegment::checksum_ok(&raw, Ipv4Addr(s), Ipv4Addr(d)));
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..48)) {
            let _ = TcpSegment::parse(&b);
        }
    }

    #[test]
    fn short_and_inconsistent_headers_are_rejected() {
        assert_eq!(TcpSegment::parse(&[0; 19]), Err(ParseError::Truncated));
        let mut b = [0u8; 20];
        b[12] = 4 << 4; // header shorter than the fixed part
        assert_eq!(TcpSegment::parse(&b), Err(ParseError::BadLength));
        b[12] = 6 << 4; // header longer than the bytes there are
        assert_eq!(TcpSegment::parse(&b), Err(ParseError::BadLength));
    }

    #[test]
    fn options_are_skipped() {
        let t = TcpSegment {
            src_port: 1,
            dst_port: 2,
            seq: 3,
            ack: 4,
            flags: TcpSegment::SYN,
            window: 5,
            payload: vec![9],
        };
        let mut raw = t.emit(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST);
        raw[12] = 6 << 4;
        raw.splice(20..20, [2, 4, 5, 180]); // an MSS option
        assert_eq!(TcpSegment::parse(&raw).unwrap(), t);
    }
}
