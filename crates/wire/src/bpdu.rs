use crate::{MacAddr, ParseError};

/// A bridge identifier: 16 bits of priority (the VLAN is folded into the low 12 by the sender)
/// then the bridge's MAC. Ordered as the standard orders them, lower is better.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BridgeId {
    pub priority: u16,
    pub mac: MacAddr,
}

impl BridgeId {
    pub fn to_bytes(self) -> [u8; 8] {
        let mut o = [0; 8];
        o[..2].copy_from_slice(&self.priority.to_be_bytes());
        o[2..].copy_from_slice(&self.mac.0);
        o
    }

    pub fn from_bytes(b: &[u8]) -> BridgeId {
        BridgeId {
            priority: u16::from_be_bytes([b[0], b[1]]),
            mac: MacAddr(b[2..8].try_into().unwrap()),
        }
    }
}

/// Port role carried in an RSTP BPDU's flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BpduRole {
    Unknown,
    AlternateOrBackup,
    Root,
    Designated,
}

/// What a BPDU is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BpduKind {
    /// Original 802.1D configuration BPDU.
    Config,
    /// Topology change notification: just the type byte, sent towards the root.
    Tcn,
    /// 802.1D-2004 rapid BPDU (version 2), same fields plus role and handshake flags.
    Rapid,
}

/// A spanning tree BPDU (802.1D-2004 clause 9) with its LLC header. Times are in units of
/// 1/256 second, as on the wire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bpdu {
    pub kind: BpduKind,
    pub topology_change: bool,
    pub topology_change_ack: bool,
    pub proposal: bool,
    pub agreement: bool,
    pub learning: bool,
    pub forwarding: bool,
    pub role: BpduRole,
    pub root: BridgeId,
    pub root_cost: u32,
    pub bridge: BridgeId,
    pub port: u16,
    pub message_age: u16,
    pub max_age: u16,
    pub hello_time: u16,
    pub forward_delay: u16,
}

/// The destination of per-VLAN BPDUs: the address Cisco's PVST+ uses, so a capture of this
/// simulator looks like one of that.
pub const PVST_MULTICAST: MacAddr = MacAddr([0x01, 0x00, 0x0c, 0xcc, 0xcc, 0xcd]);

impl Bpdu {
    const LLC: [u8; 3] = [0x42, 0x42, 0x03];

    pub fn seconds(s: u16) -> u16 {
        s * 256
    }

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let b = b.strip_prefix(&Self::LLC).ok_or(ParseError::Unsupported)?;
        if b.len() < 4 {
            return Err(ParseError::Truncated);
        }
        if u16::from_be_bytes([b[0], b[1]]) != 0 {
            return Err(ParseError::Unsupported);
        }
        let blank = |kind, flags: u8| Bpdu {
            kind,
            topology_change: flags & 1 != 0,
            topology_change_ack: flags & 0x80 != 0,
            proposal: false,
            agreement: false,
            learning: false,
            forwarding: false,
            role: BpduRole::Unknown,
            root: BridgeId::from_bytes(&[0; 8]),
            root_cost: 0,
            bridge: BridgeId::from_bytes(&[0; 8]),
            port: 0,
            message_age: 0,
            max_age: 0,
            hello_time: 0,
            forward_delay: 0,
        };
        match (b[2], b[3]) {
            (_, 0x80) => Ok(blank(BpduKind::Tcn, 0)),
            (version, 0x00 | 0x02) => {
                if b.len() < 35 {
                    return Err(ParseError::Truncated);
                }
                let rapid = b[3] == 0x02 && version >= 2;
                let flags = b[4];
                let mut p = blank(
                    if rapid {
                        BpduKind::Rapid
                    } else {
                        BpduKind::Config
                    },
                    flags,
                );
                if rapid {
                    p.proposal = flags & 0x02 != 0;
                    p.learning = flags & 0x10 != 0;
                    p.forwarding = flags & 0x20 != 0;
                    p.agreement = flags & 0x40 != 0;
                    p.role = match (flags >> 2) & 3 {
                        1 => BpduRole::AlternateOrBackup,
                        2 => BpduRole::Root,
                        3 => BpduRole::Designated,
                        _ => BpduRole::Unknown,
                    };
                }
                p.root = BridgeId::from_bytes(&b[5..13]);
                p.root_cost = u32::from_be_bytes(b[13..17].try_into().unwrap());
                p.bridge = BridgeId::from_bytes(&b[17..25]);
                p.port = u16::from_be_bytes([b[25], b[26]]);
                p.message_age = u16::from_be_bytes([b[27], b[28]]);
                p.max_age = u16::from_be_bytes([b[29], b[30]]);
                p.hello_time = u16::from_be_bytes([b[31], b[32]]);
                p.forward_delay = u16::from_be_bytes([b[33], b[34]]);
                Ok(p)
            }
            _ => Err(ParseError::Unsupported),
        }
    }

    /// LLC header plus BPDU, ready to be an Ethernet payload whose type field holds its length.
    pub fn emit(&self) -> Vec<u8> {
        let mut o = Self::LLC.to_vec();
        o.extend_from_slice(&[0, 0]);
        if self.kind == BpduKind::Tcn {
            o.extend_from_slice(&[0, 0x80]);
            return o;
        }
        let rapid = self.kind == BpduKind::Rapid;
        o.push(if rapid { 2 } else { 0 });
        o.push(if rapid { 2 } else { 0 });
        let mut flags = u8::from(self.topology_change) | (u8::from(self.topology_change_ack) << 7);
        if rapid {
            flags |= u8::from(self.proposal) << 1;
            flags |= match self.role {
                BpduRole::Unknown => 0,
                BpduRole::AlternateOrBackup => 1,
                BpduRole::Root => 2,
                BpduRole::Designated => 3,
            } << 2;
            flags |= u8::from(self.learning) << 4;
            flags |= u8::from(self.forwarding) << 5;
            flags |= u8::from(self.agreement) << 6;
        }
        o.push(flags);
        o.extend_from_slice(&self.root.to_bytes());
        o.extend_from_slice(&self.root_cost.to_be_bytes());
        o.extend_from_slice(&self.bridge.to_bytes());
        o.extend_from_slice(&self.port.to_be_bytes());
        for t in [
            self.message_age,
            self.max_age,
            self.hello_time,
            self.forward_delay,
        ] {
            o.extend_from_slice(&t.to_be_bytes());
        }
        if rapid {
            o.push(0); // version 1 length
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: BpduKind) -> Bpdu {
        let rapid = kind == BpduKind::Rapid;
        Bpdu {
            kind,
            topology_change: true,
            topology_change_ack: false,
            proposal: rapid,
            agreement: false,
            learning: rapid,
            forwarding: false,
            role: if rapid {
                BpduRole::Designated
            } else {
                BpduRole::Unknown
            },
            root: BridgeId {
                priority: 4097,
                mac: MacAddr([2, 0, 0, 0, 0, 1]),
            },
            root_cost: 19,
            bridge: BridgeId {
                priority: 32769,
                mac: MacAddr([2, 0, 0, 0, 0, 2]),
            },
            port: 0x8001,
            message_age: 256,
            max_age: Bpdu::seconds(20),
            hello_time: Bpdu::seconds(2),
            forward_delay: Bpdu::seconds(15),
        }
    }

    #[test]
    fn config_and_rapid_round_trip() {
        for k in [BpduKind::Config, BpduKind::Rapid] {
            let b = sample(k);
            assert_eq!(Bpdu::parse(&b.emit()), Ok(b));
        }
    }

    #[test]
    fn a_tcn_is_four_bytes_after_the_llc() {
        let raw = sample(BpduKind::Tcn).emit();
        assert_eq!(raw.len(), 3 + 4);
        assert_eq!(Bpdu::parse(&raw).unwrap().kind, BpduKind::Tcn);
    }

    #[test]
    fn bridge_ids_order_by_priority_then_mac() {
        let id = |priority, m| BridgeId {
            priority,
            mac: MacAddr([m; 6]),
        };
        assert!(id(4096, 9) < id(8192, 1));
        assert!(id(4096, 1) < id(4096, 9));
    }

    #[test]
    fn short_or_foreign_input_is_refused_not_panicked_on() {
        assert_eq!(Bpdu::parse(&[]), Err(ParseError::Unsupported));
        assert_eq!(
            Bpdu::parse(&[0x42, 0x42, 0x03, 0, 0]),
            Err(ParseError::Truncated)
        );
        let mut raw = sample(BpduKind::Config).emit();
        raw.truncate(20);
        assert_eq!(Bpdu::parse(&raw), Err(ParseError::Truncated));
        raw = sample(BpduKind::Config).emit();
        raw[4] = 9;
        assert_eq!(Bpdu::parse(&raw), Err(ParseError::Unsupported));
    }

    proptest::proptest! {
        /// What the fuzz target checks: anything that parses survives a trip through `emit`.
        #[test]
        fn anything_that_parses_round_trips(tail in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..60), kind in 0u8..3) {
            let mut raw = vec![0x42, 0x42, 0x03, 0, 0];
            raw.push(if kind == 2 { 2 } else { 0 });
            raw.push(match kind { 0 => 0, 1 => 0x80, _ => 2 });
            raw.extend(tail);
            if let Ok(b) = Bpdu::parse(&raw) {
                proptest::prop_assert_eq!(Bpdu::parse(&b.emit()), Ok(b));
            }
        }
    }
}
