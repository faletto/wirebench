//! What a device heard back after sending a probe (a ping, a traceroute packet): the replies
//! land in a log the console reads, matching them to probes by the identifiers they quote.

use kernel::SimTime;
use wire::{IcmpErrorKind, Ipv4Addr};

/// The most replies a device remembers; the oldest are dropped first.
pub const PROBE_LOG_LIMIT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeReply {
    /// When it arrived.
    pub at: SimTime,
    /// The address it came from (for an error, the router or host that sent it).
    pub from: Ipv4Addr,
    pub body: ProbeBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeBody {
    EchoReply {
        ident: u16,
        seq: u16,
    },
    /// An ICMP error and the start of the packet that caused it (IP header plus 8 bytes).
    Error {
        kind: IcmpErrorKind,
        original: Vec<u8>,
    },
}

impl ProbeReply {
    /// For an error quoting a UDP packet: its destination port.
    pub fn quoted_udp_dport(&self) -> Option<u16> {
        match &self.body {
            ProbeBody::Error { original, .. } if original.len() >= 24 && original[9] == 17 => {
                Some(u16::from_be_bytes([original[22], original[23]]))
            }
            _ => None,
        }
    }

    /// For an echo reply, or an error quoting an echo request: its identifier and sequence number.
    pub fn echo_id(&self) -> Option<(u16, u16)> {
        match &self.body {
            ProbeBody::EchoReply { ident, seq } => Some((*ident, *seq)),
            ProbeBody::Error { original, .. }
                if original.len() >= 28 && original[9] == 1 && original[20] == 8 =>
            {
                Some((
                    u16::from_be_bytes([original[24], original[25]]),
                    u16::from_be_bytes([original[26], original[27]]),
                ))
            }
            _ => None,
        }
    }
}

/// Append to a log, forgetting the oldest entries past the limit.
pub fn record(log: &mut Vec<ProbeReply>, r: ProbeReply) {
    log.push(r);
    if log.len() > PROBE_LOG_LIMIT {
        log.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(original: Vec<u8>) -> ProbeReply {
        ProbeReply {
            at: SimTime::ZERO,
            from: Ipv4Addr::new(10, 0, 0, 1),
            body: ProbeBody::Error {
                kind: IcmpErrorKind::TimeExceeded,
                original,
            },
        }
    }

    #[test]
    fn identifiers_are_read_from_the_quoted_packet() {
        let mut udp = vec![0u8; 28];
        udp[9] = 17;
        udp[22..24].copy_from_slice(&33436u16.to_be_bytes());
        assert_eq!(err(udp.clone()).quoted_udp_dport(), Some(33436));
        assert_eq!(err(udp).echo_id(), None);
        let mut echo = vec![0u8; 28];
        echo[9] = 1;
        echo[20] = 8;
        echo[24..26].copy_from_slice(&7u16.to_be_bytes());
        echo[26..28].copy_from_slice(&9u16.to_be_bytes());
        assert_eq!(err(echo.clone()).echo_id(), Some((7, 9)));
        assert_eq!(err(echo).quoted_udp_dport(), None);
        assert_eq!(
            err(vec![0; 5]).echo_id(),
            None,
            "short quotes are not trusted"
        );
    }

    #[test]
    fn the_log_is_bounded() {
        let mut log = Vec::new();
        for i in 0..PROBE_LOG_LIMIT + 5 {
            record(
                &mut log,
                ProbeReply {
                    at: SimTime(i as u64),
                    from: Ipv4Addr::UNSPECIFIED,
                    body: ProbeBody::EchoReply { ident: 0, seq: 0 },
                },
            );
        }
        assert_eq!(log.len(), PROBE_LOG_LIMIT);
        assert_eq!(log[0].at, SimTime(5));
    }
}
