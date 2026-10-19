use crate::{MacAddr, ParseError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EtherType(pub u16);

impl EtherType {
    pub const IPV4: EtherType = EtherType(0x0800);
    pub const ARP: EtherType = EtherType(0x0806);
    pub const DOT1Q: EtherType = EtherType(0x8100);
}

/// 802.1Q tag: priority code point, drop eligible, VLAN id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VlanTag {
    pub pcp: u8,
    pub dei: bool,
    pub vid: u16,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EthernetFrame {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub vlan: Option<VlanTag>,
    pub ethertype: EtherType,
    pub payload: Vec<u8>,
}

impl EthernetFrame {
    pub const MIN_LEN: usize = 14;

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < Self::MIN_LEN {
            return Err(ParseError::Truncated);
        }
        let dst = MacAddr(b[0..6].try_into().unwrap());
        let src = MacAddr(b[6..12].try_into().unwrap());
        let et = u16::from_be_bytes([b[12], b[13]]);
        if et == EtherType::DOT1Q.0 {
            if b.len() < 18 {
                return Err(ParseError::Truncated);
            }
            let tci = u16::from_be_bytes([b[14], b[15]]);
            Ok(EthernetFrame {
                dst,
                src,
                vlan: Some(VlanTag {
                    pcp: (tci >> 13) as u8,
                    dei: tci & 0x1000 != 0,
                    vid: tci & 0x0fff,
                }),
                ethertype: EtherType(u16::from_be_bytes([b[16], b[17]])),
                payload: b[18..].to_vec(),
            })
        } else {
            Ok(EthernetFrame {
                dst,
                src,
                vlan: None,
                ethertype: EtherType(et),
                payload: b[14..].to_vec(),
            })
        }
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(18 + self.payload.len());
        o.extend_from_slice(&self.dst.0);
        o.extend_from_slice(&self.src.0);
        if let Some(t) = self.vlan {
            o.extend_from_slice(&EtherType::DOT1Q.0.to_be_bytes());
            let tci = ((t.pcp as u16 & 7) << 13) | ((t.dei as u16) << 12) | (t.vid & 0x0fff);
            o.extend_from_slice(&tci.to_be_bytes());
        }
        o.extend_from_slice(&self.ethertype.0.to_be_bytes());
        o.extend_from_slice(&self.payload);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn arb_frame(tagged: bool) -> impl Strategy<Value = EthernetFrame> {
        (
            any::<[u8; 6]>(),
            any::<[u8; 6]>(),
            0u8..8,
            any::<bool>(),
            0u16..4096,
            any::<u16>().prop_filter("not 802.1Q", |e| *e != 0x8100),
            proptest::collection::vec(any::<u8>(), 0..64),
        )
            .prop_map(move |(d, s, pcp, dei, vid, et, payload)| EthernetFrame {
                dst: MacAddr(d),
                src: MacAddr(s),
                vlan: tagged.then_some(VlanTag { pcp, dei, vid }),
                ethertype: EtherType(et),
                payload,
            })
    }

    proptest! {
        #[test]
        fn ethernet_roundtrip(f in arb_frame(false)) {
            prop_assert_eq!(EthernetFrame::parse(&f.emit()).unwrap(), f);
        }

        #[test]
        fn vlan_tag_roundtrip(f in arb_frame(true)) {
            prop_assert_eq!(EthernetFrame::parse(&f.emit()).unwrap(), f);
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = EthernetFrame::parse(&b);
        }
    }

    #[test]
    fn rejects_short_frame() {
        assert_eq!(EthernetFrame::parse(&[0; 13]), Err(ParseError::Truncated));
        let mut b = vec![0u8; 14];
        b[12] = 0x81;
        b[13] = 0x00;
        assert_eq!(EthernetFrame::parse(&b), Err(ParseError::Truncated));
    }
}
