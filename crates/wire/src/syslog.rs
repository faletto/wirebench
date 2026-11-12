//! Syslog messages over UDP port 514, in the BSD format of RFC 3164: `<PRI>text`, where PRI is
//! facility * 8 + severity. Everything after the `>` is kept as it came (devices put a timestamp,
//! sequence number and `%FAC-SEV-MNEMONIC:` tag there; a collector shows it unchanged).

use crate::ParseError;

pub const SYSLOG_PORT: u16 = 514;
/// The longest message RFC 3164 allows, including the priority.
pub const MAX_LEN: usize = 1024;

/// Severity names, emergencies (0) to debugging (7).
pub const SEVERITIES: [&str; 8] = [
    "emergencies",
    "alerts",
    "critical",
    "errors",
    "warnings",
    "notifications",
    "informational",
    "debugging",
];

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SyslogMessage {
    /// 0..=23 (16 is local0, the one network devices usually use; 23 is local7).
    pub facility: u8,
    /// 0..=7.
    pub severity: u8,
    pub text: String,
}

impl SyslogMessage {
    pub fn pri(&self) -> u8 {
        self.facility * 8 + self.severity
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = format!("<{}>", self.pri()).into_bytes();
        o.extend_from_slice(self.text.as_bytes());
        o.truncate(MAX_LEN);
        o
    }

    pub fn parse(b: &[u8]) -> Result<SyslogMessage, ParseError> {
        if b.first() != Some(&b'<') {
            return Err(ParseError::BadLength);
        }
        let close = b
            .iter()
            .take(5)
            .position(|&c| c == b'>')
            .ok_or(ParseError::BadLength)?;
        let digits = &b[1..close];
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(ParseError::BadLength);
        }
        let pri: u32 = std::str::from_utf8(digits).unwrap().parse().unwrap();
        if pri > 191 {
            return Err(ParseError::BadLength);
        }
        Ok(SyslogMessage {
            facility: (pri / 8) as u8,
            severity: (pri % 8) as u8,
            text: String::from_utf8_lossy(&b[close + 1..]).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_priority() {
        let m = SyslogMessage {
            facility: 23,
            severity: 5,
            text: "12: *Mar  1 00:00:05.123: %LINK-3-UPDOWN: Interface Fa0/1, changed state to up"
                .into(),
        };
        assert!(m.emit().starts_with(b"<189>12: "));
        assert_eq!(SyslogMessage::parse(&m.emit()), Ok(m));
    }

    #[test]
    fn bad_priorities_are_refused() {
        for bad in [
            &b""[..],
            b"hello",
            b"<>x",
            b"<a>x",
            b"<192>x",
            b"<1234>x",
            b"<12",
        ] {
            assert!(SyslogMessage::parse(bad).is_err(), "{bad:?}");
        }
        assert!(SyslogMessage::parse(b"<191>").is_ok());
        assert!(SyslogMessage::parse(b"<0>").is_ok());
    }

    #[test]
    fn long_text_is_cut_and_junk_never_panics() {
        let m = SyslogMessage {
            facility: 1,
            severity: 1,
            text: "x".repeat(3000),
        };
        assert_eq!(m.emit().len(), MAX_LEN);
        for n in 0..40u8 {
            let _ = SyslogMessage::parse(&vec![b'<' ^ (n & 1); n as usize]);
            let _ = SyslogMessage::parse(&[0xff; 6][..(n % 7) as usize]);
        }
    }
}
