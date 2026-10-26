use crate::{MacAddr, ParseError};

/// Where Dynamic Trunking Protocol frames go.
pub const DTP_MULTICAST: MacAddr = MacAddr([0x01, 0x00, 0x0c, 0xcc, 0xcc, 0xcc]);

/// What a port is configured to do about trunking, as DTP announces it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DtpMode {
    /// `switchport mode trunk`.
    On,
    /// `switchport mode access`.
    Off,
    /// `dynamic desirable`: asks the other side to trunk.
    Desirable,
    /// `dynamic auto`: trunks if asked.
    Auto,
}

/// A Dynamic Trunking Protocol message: the sender's trunking mode and whether it is trunking now,
/// the encapsulation it would use, and its (VTP) domain. Carried in an LLC/SNAP frame (OUI
/// 00:00:0c, protocol 0x2004) after a length field.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dtp {
    pub mode: DtpMode,
    /// The sender is operating as a trunk right now.
    pub trunking: bool,
    pub domain: Vec<u8>,
    pub sender: MacAddr,
}

const SNAP: [u8; 8] = [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x0c, 0x20, 0x04];
const DOT1Q_TYPE: u8 = 0xa5;

impl Dtp {
    pub fn parse(b: &[u8]) -> Result<Dtp, ParseError> {
        let b = b.strip_prefix(&SNAP).ok_or(ParseError::Unsupported)?;
        if b.first() != Some(&1) {
            return Err(ParseError::Unsupported);
        }
        let mut rest = &b[1..];
        let (mut mode, mut trunking, mut domain, mut sender) = (None, false, vec![], None);
        while rest.len() >= 4 {
            let ty = u16::from_be_bytes([rest[0], rest[1]]);
            let len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
            if ty == 0 && len == 0 {
                break; // padding
            }
            if len < 4 || len > rest.len() {
                return Err(ParseError::BadLength);
            }
            let v = &rest[4..len];
            match (ty, v) {
                (1, d) => domain = d.to_vec(),
                (2, [s]) => {
                    mode = match s & 0x0f {
                        1 => Some(DtpMode::On),
                        2 => Some(DtpMode::Off),
                        3 => Some(DtpMode::Desirable),
                        4 => Some(DtpMode::Auto),
                        _ => return Err(ParseError::Unsupported),
                    };
                    trunking = s & 0x80 != 0;
                }
                (4, m) if m.len() == 6 => sender = Some(MacAddr(m.try_into().unwrap())),
                _ => {}
            }
            rest = &rest[len..];
        }
        Ok(Dtp {
            mode: mode.ok_or(ParseError::Truncated)?,
            trunking,
            domain,
            sender: sender.ok_or(ParseError::Truncated)?,
        })
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = SNAP.to_vec();
        o.push(1);
        let mut tlv = |ty: u16, v: &[u8]| {
            o.extend_from_slice(&ty.to_be_bytes());
            o.extend_from_slice(&((v.len() + 4) as u16).to_be_bytes());
            o.extend_from_slice(v);
        };
        tlv(1, &self.domain);
        let low = match self.mode {
            DtpMode::On => 1,
            DtpMode::Off => 2,
            DtpMode::Desirable => 3,
            DtpMode::Auto => 4,
        };
        tlv(2, &[low | if self.trunking { 0x80 } else { 0 }]);
        tlv(3, &[DOT1Q_TYPE]);
        tlv(4, &self.sender.0);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(mode: DtpMode) -> Dtp {
        Dtp {
            mode,
            trunking: true,
            domain: b"lab".to_vec(),
            sender: MacAddr([2, 0xbb, 0xff, 1, 0, 3]),
        }
    }

    #[test]
    fn every_mode_round_trips() {
        for m in [DtpMode::On, DtpMode::Off, DtpMode::Desirable, DtpMode::Auto] {
            let d = sample(m);
            assert_eq!(Dtp::parse(&d.emit()), Ok(d));
        }
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(Dtp::parse(&[]), Err(ParseError::Unsupported));
        let mut raw = sample(DtpMode::Auto).emit();
        raw.truncate(raw.len() - 3);
        assert!(Dtp::parse(&raw).is_err());
        let mut raw = sample(DtpMode::Auto).emit();
        raw[8] = 7; // an unknown version
        assert_eq!(Dtp::parse(&raw), Err(ParseError::Unsupported));
    }

    proptest::proptest! {
        #[test]
        fn anything_that_parses_round_trips(tail in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..50)) {
            let mut raw = SNAP.to_vec();
            raw.push(1);
            raw.extend(tail);
            if let Ok(d) = Dtp::parse(&raw) {
                proptest::prop_assert_eq!(Dtp::parse(&d.emit()), Ok(d));
            }
        }
    }
}
