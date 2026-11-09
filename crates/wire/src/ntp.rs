//! NTP version 4 packets (RFC 5905), without extension fields or authentication.

use crate::ParseError;

pub const NTP_PORT: u16 = 123;
pub const MODE_CLIENT: u8 = 3;
pub const MODE_SERVER: u8 = 4;
/// Seconds from 1900-01-01 (the NTP era) to 1970-01-01.
pub const UNIX_TO_NTP: u64 = 2_208_988_800;

/// A 64-bit NTP timestamp: seconds since 1900 and a binary fraction of a second.
pub type Timestamp = u64;

/// Unix nanoseconds to an NTP timestamp. Only era 0 (until 2036) is represented; later times wrap.
pub fn to_timestamp(unix_ns: i64) -> Timestamp {
    let ns = unix_ns.max(0) as u128;
    let secs = ns / 1_000_000_000 + u128::from(UNIX_TO_NTP);
    let frac = ((ns % 1_000_000_000) << 32) / 1_000_000_000;
    (((secs as u64) & 0xffff_ffff) << 32) | frac as u64
}

/// An NTP timestamp to Unix nanoseconds.
pub fn from_timestamp(t: Timestamp) -> i64 {
    let secs = (t >> 32).saturating_sub(UNIX_TO_NTP) as i64;
    let frac = ((t & 0xffff_ffff) * 1_000_000_000) >> 32;
    secs * 1_000_000_000 + frac as i64
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NtpPacket {
    /// Leap indicator; 3 means the clock is unsynchronised.
    pub leap: u8,
    pub version: u8,
    pub mode: u8,
    pub stratum: u8,
    /// log2 seconds.
    pub poll: i8,
    pub precision: i8,
    /// 16.16 fixed point seconds.
    pub root_delay: u32,
    pub root_dispersion: u32,
    pub ref_id: [u8; 4],
    pub reference: Timestamp,
    pub origin: Timestamp,
    pub receive: Timestamp,
    pub transmit: Timestamp,
}

impl NtpPacket {
    pub const LEN: usize = 48;

    pub fn emit(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.push((self.leap << 6) | ((self.version & 7) << 3) | (self.mode & 7));
        o.push(self.stratum);
        o.push(self.poll as u8);
        o.push(self.precision as u8);
        o.extend_from_slice(&self.root_delay.to_be_bytes());
        o.extend_from_slice(&self.root_dispersion.to_be_bytes());
        o.extend_from_slice(&self.ref_id);
        for t in [self.reference, self.origin, self.receive, self.transmit] {
            o.extend_from_slice(&t.to_be_bytes());
        }
        o
    }

    pub fn parse(b: &[u8]) -> Result<NtpPacket, ParseError> {
        if b.len() < Self::LEN {
            return Err(ParseError::Truncated);
        }
        let version = (b[0] >> 3) & 7;
        if !(1..=4).contains(&version) {
            return Err(ParseError::Unsupported);
        }
        let ts = |i: usize| u64::from_be_bytes(b[i..i + 8].try_into().unwrap());
        Ok(NtpPacket {
            leap: b[0] >> 6,
            version,
            mode: b[0] & 7,
            stratum: b[1],
            poll: b[2] as i8,
            precision: b[3] as i8,
            root_delay: u32::from_be_bytes(b[4..8].try_into().unwrap()),
            root_dispersion: u32::from_be_bytes(b[8..12].try_into().unwrap()),
            ref_id: b[12..16].try_into().unwrap(),
            reference: ts(16),
            origin: ts(24),
            receive: ts(32),
            transmit: ts(40),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_round_trips() {
        let p = NtpPacket {
            leap: 0,
            version: 4,
            mode: MODE_SERVER,
            stratum: 3,
            poll: 6,
            precision: -18,
            root_delay: 0x0001_8000,
            root_dispersion: 0x0000_4000,
            ref_id: [10, 0, 0, 1],
            reference: to_timestamp(1_700_000_000_123_456_789),
            origin: 1,
            receive: 2,
            transmit: 3,
        };
        assert_eq!(NtpPacket::parse(&p.emit()), Ok(p));
    }

    #[test]
    fn timestamps_convert_both_ways_to_the_nanosecond_or_nearly() {
        for ns in [
            0_i64,
            1,
            999_999_999,
            1_700_000_000_500_000_000,
            2_000_000_000_000_000_001,
        ] {
            let back = from_timestamp(to_timestamp(ns));
            assert!((back - ns).abs() <= 1, "{ns} came back as {back}");
        }
    }

    #[test]
    fn short_and_wrong_version_are_refused() {
        assert_eq!(NtpPacket::parse(&[0; 47]), Err(ParseError::Truncated));
        assert_eq!(NtpPacket::parse(&[0; 48]), Err(ParseError::Unsupported));
        for n in 0..48 {
            let _ = NtpPacket::parse(&vec![0x23; n]);
        }
    }
}
