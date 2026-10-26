use crate::{Ipv4Addr, MacAddr, ParseError};

/// DHCP over Ethernet (RFC 2131, options per RFC 2132): the fixed 236-byte BOOTP header, the
/// magic cookie and a list of options. Only Ethernet clients (`htype` 1) are supported, and the
/// `sname` and `file` fields are not kept (they are written as zeros).
///
/// The options this simulation uses are typed fields; anything else is kept in `unknown`, in
/// order, so a message survives parse then emit unchanged. Emit is canonical: the same message
/// always gives the same bytes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DhcpMessage {
    pub op: DhcpOp,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    /// The 16 flag bits; bit 15 is the broadcast flag.
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: MacAddr,
    pub msg_type: Option<DhcpType>,
    pub subnet_mask: Option<Ipv4Addr>,
    pub routers: Vec<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub requested_ip: Option<Ipv4Addr>,
    pub lease_secs: Option<u32>,
    pub server_id: Option<Ipv4Addr>,
    pub param_request: Vec<u8>,
    /// Option 56: a human-readable reason (NAK, DECLINE).
    pub message: Option<Vec<u8>>,
    pub t1_secs: Option<u32>,
    pub t2_secs: Option<u32>,
    pub client_id: Option<Vec<u8>>,
    pub unknown: Vec<(u8, Vec<u8>)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DhcpOp {
    #[default]
    Request,
    Reply,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DhcpType {
    Discover,
    Offer,
    Request,
    Decline,
    Ack,
    Nak,
    Release,
    Inform,
}

impl DhcpType {
    fn code(self) -> u8 {
        match self {
            DhcpType::Discover => 1,
            DhcpType::Offer => 2,
            DhcpType::Request => 3,
            DhcpType::Decline => 4,
            DhcpType::Ack => 5,
            DhcpType::Nak => 6,
            DhcpType::Release => 7,
            DhcpType::Inform => 8,
        }
    }

    fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => DhcpType::Discover,
            2 => DhcpType::Offer,
            3 => DhcpType::Request,
            4 => DhcpType::Decline,
            5 => DhcpType::Ack,
            6 => DhcpType::Nak,
            7 => DhcpType::Release,
            8 => DhcpType::Inform,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            DhcpType::Discover => "discover",
            DhcpType::Offer => "offer",
            DhcpType::Request => "request",
            DhcpType::Decline => "decline",
            DhcpType::Ack => "ack",
            DhcpType::Nak => "nak",
            DhcpType::Release => "release",
            DhcpType::Inform => "inform",
        }
    }
}

const MAGIC: [u8; 4] = [0x63, 0x82, 0x53, 0x63];
const FIXED_LEN: usize = 236;
const BROADCAST_FLAG: u16 = 0x8000;

// Option codes (RFC 2132).
const PAD: u8 = 0;
const SUBNET_MASK: u8 = 1;
const ROUTERS: u8 = 3;
const DNS: u8 = 6;
const REQUESTED_IP: u8 = 50;
const LEASE_TIME: u8 = 51;
const MSG_TYPE: u8 = 53;
const SERVER_ID: u8 = 54;
const PARAM_REQUEST: u8 = 55;
const MESSAGE: u8 = 56;
const T1: u8 = 58;
const T2: u8 = 59;
const CLIENT_ID: u8 = 61;
const END: u8 = 255;

/// Options that are typed fields: they never appear in `unknown`.
fn is_typed(code: u8) -> bool {
    matches!(
        code,
        PAD | SUBNET_MASK
            | ROUTERS
            | DNS
            | REQUESTED_IP
            | LEASE_TIME
            | MSG_TYPE
            | SERVER_ID
            | PARAM_REQUEST
            | MESSAGE
            | T1
            | T2
            | CLIENT_ID
            | END
    )
}

impl DhcpMessage {
    pub fn broadcast(&self) -> bool {
        self.flags & BROADCAST_FLAG != 0
    }

    pub fn set_broadcast(&mut self, on: bool) {
        if on {
            self.flags |= BROADCAST_FLAG;
        } else {
            self.flags &= !BROADCAST_FLAG;
        }
    }

    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < FIXED_LEN + MAGIC.len() {
            return Err(ParseError::Truncated);
        }
        let op = match b[0] {
            1 => DhcpOp::Request,
            2 => DhcpOp::Reply,
            _ => return Err(ParseError::Unsupported),
        };
        if b[1] != 1 || b[2] != 6 {
            return Err(ParseError::Unsupported);
        }
        if b[FIXED_LEN..FIXED_LEN + 4] != MAGIC {
            return Err(ParseError::Unsupported);
        }
        let addr = |at: usize| Ipv4Addr(b[at..at + 4].try_into().unwrap());
        let mut m = DhcpMessage {
            op,
            hops: b[3],
            xid: u32::from_be_bytes(b[4..8].try_into().unwrap()),
            secs: u16::from_be_bytes([b[8], b[9]]),
            flags: u16::from_be_bytes([b[10], b[11]]),
            ciaddr: addr(12),
            yiaddr: addr(16),
            siaddr: addr(20),
            giaddr: addr(24),
            chaddr: MacAddr(b[28..34].try_into().unwrap()),
            ..DhcpMessage::default()
        };
        let mut i = FIXED_LEN + MAGIC.len();
        while i < b.len() {
            let code = b[i];
            i += 1;
            match code {
                PAD => continue,
                END => break,
                _ => {}
            }
            let len = *b.get(i).ok_or(ParseError::Truncated)? as usize;
            let v = b.get(i + 1..i + 1 + len).ok_or(ParseError::Truncated)?;
            i += 1 + len;
            m.set_option(code, v)?;
        }
        Ok(m)
    }

    fn set_option(&mut self, code: u8, v: &[u8]) -> Result<(), ParseError> {
        let one = |v: &[u8]| -> Result<Ipv4Addr, ParseError> {
            <[u8; 4]>::try_from(v)
                .map(Ipv4Addr)
                .map_err(|_| ParseError::BadLength)
        };
        let many = |v: &[u8]| -> Result<Vec<Ipv4Addr>, ParseError> {
            if v.is_empty() || v.len() % 4 != 0 {
                return Err(ParseError::BadLength);
            }
            Ok(v.chunks(4)
                .map(|c| Ipv4Addr(c.try_into().unwrap()))
                .collect())
        };
        let secs = |v: &[u8]| -> Result<u32, ParseError> {
            <[u8; 4]>::try_from(v)
                .map(u32::from_be_bytes)
                .map_err(|_| ParseError::BadLength)
        };
        match code {
            SUBNET_MASK => self.subnet_mask = Some(one(v)?),
            ROUTERS => self.routers = many(v)?,
            DNS => self.dns = many(v)?,
            REQUESTED_IP => self.requested_ip = Some(one(v)?),
            LEASE_TIME => self.lease_secs = Some(secs(v)?),
            MSG_TYPE => {
                let [c] = v else {
                    return Err(ParseError::BadLength);
                };
                self.msg_type = Some(DhcpType::from_code(*c).ok_or(ParseError::Unsupported)?);
            }
            SERVER_ID => self.server_id = Some(one(v)?),
            PARAM_REQUEST => self.param_request = v.to_vec(),
            MESSAGE => self.message = Some(v.to_vec()),
            T1 => self.t1_secs = Some(secs(v)?),
            T2 => self.t2_secs = Some(secs(v)?),
            CLIENT_ID => {
                if v.is_empty() {
                    return Err(ParseError::BadLength);
                }
                self.client_id = Some(v.to_vec());
            }
            _ => self.unknown.push((code, v.to_vec())),
        }
        Ok(())
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut o = vec![0u8; FIXED_LEN];
        o[0] = match self.op {
            DhcpOp::Request => 1,
            DhcpOp::Reply => 2,
        };
        o[1] = 1;
        o[2] = 6;
        o[3] = self.hops;
        o[4..8].copy_from_slice(&self.xid.to_be_bytes());
        o[8..10].copy_from_slice(&self.secs.to_be_bytes());
        o[10..12].copy_from_slice(&self.flags.to_be_bytes());
        o[12..16].copy_from_slice(&self.ciaddr.0);
        o[16..20].copy_from_slice(&self.yiaddr.0);
        o[20..24].copy_from_slice(&self.siaddr.0);
        o[24..28].copy_from_slice(&self.giaddr.0);
        o[28..34].copy_from_slice(&self.chaddr.0);
        o.extend_from_slice(&MAGIC);
        let mut opt = |code: u8, v: &[u8]| {
            // An option's length is one byte; the typed fields are capped so they fit.
            let v = &v[..v.len().min(255)];
            o.push(code);
            o.push(v.len() as u8);
            o.extend_from_slice(v);
        };
        if let Some(t) = self.msg_type {
            opt(MSG_TYPE, &[t.code()]);
        }
        if let Some(a) = self.subnet_mask {
            opt(SUBNET_MASK, &a.0);
        }
        let list = |l: &[Ipv4Addr]| -> Vec<u8> { l.iter().take(63).flat_map(|a| a.0).collect() };
        if !self.routers.is_empty() {
            opt(ROUTERS, &list(&self.routers));
        }
        if !self.dns.is_empty() {
            opt(DNS, &list(&self.dns));
        }
        if let Some(a) = self.requested_ip {
            opt(REQUESTED_IP, &a.0);
        }
        if let Some(s) = self.lease_secs {
            opt(LEASE_TIME, &s.to_be_bytes());
        }
        if let Some(a) = self.server_id {
            opt(SERVER_ID, &a.0);
        }
        if !self.param_request.is_empty() {
            opt(PARAM_REQUEST, &self.param_request);
        }
        if let Some(m) = &self.message {
            opt(MESSAGE, m);
        }
        if let Some(s) = self.t1_secs {
            opt(T1, &s.to_be_bytes());
        }
        if let Some(s) = self.t2_secs {
            opt(T2, &s.to_be_bytes());
        }
        if let Some(c) = &self.client_id {
            if !c.is_empty() {
                opt(CLIENT_ID, c);
            }
        }
        for (code, v) in self.unknown.iter().filter(|(c, _)| !is_typed(*c)) {
            opt(*code, v);
        }
        o.push(END);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn addr() -> impl Strategy<Value = Ipv4Addr> {
        any::<[u8; 4]>().prop_map(Ipv4Addr)
    }

    fn arb_message() -> impl Strategy<Value = DhcpMessage> {
        let types = prop::option::of(prop::sample::select(vec![
            DhcpType::Discover,
            DhcpType::Offer,
            DhcpType::Request,
            DhcpType::Decline,
            DhcpType::Ack,
            DhcpType::Nak,
            DhcpType::Release,
            DhcpType::Inform,
        ]));
        let base = (
            any::<bool>(),
            any::<u8>(),
            any::<u32>(),
            any::<u16>(),
            any::<u16>(),
            (addr(), addr(), addr(), addr()),
            any::<[u8; 6]>(),
            types,
        );
        let opts = (
            prop::option::of(addr()),
            prop::collection::vec(addr(), 0..4),
            prop::collection::vec(addr(), 0..4),
            prop::option::of(addr()),
            prop::option::of(any::<u32>()),
            prop::option::of(addr()),
            prop::collection::vec(any::<u8>(), 0..8),
            prop::option::of(prop::collection::vec(any::<u8>(), 0..16)),
            (
                prop::option::of(any::<u32>()),
                prop::option::of(any::<u32>()),
            ),
            prop::option::of(prop::collection::vec(any::<u8>(), 1..8)),
            prop::collection::vec(
                (any::<u8>(), prop::collection::vec(any::<u8>(), 0..6)),
                0..3,
            ),
        );
        (base, opts).prop_map(
            |(
                (reply, hops, xid, secs, flags, (ci, yi, si, gi), mac, msg_type),
                (mask, routers, dns, req, lease, sid, prl, message, (t1, t2), cid, unknown),
            )| DhcpMessage {
                op: if reply {
                    DhcpOp::Reply
                } else {
                    DhcpOp::Request
                },
                hops,
                xid,
                secs,
                flags,
                ciaddr: ci,
                yiaddr: yi,
                siaddr: si,
                giaddr: gi,
                chaddr: MacAddr(mac),
                msg_type,
                subnet_mask: mask,
                routers,
                dns,
                requested_ip: req,
                lease_secs: lease,
                server_id: sid,
                param_request: prl,
                message,
                t1_secs: t1,
                t2_secs: t2,
                client_id: cid,
                unknown: unknown.into_iter().filter(|(c, _)| !is_typed(*c)).collect(),
            },
        )
    }

    proptest! {
        #[test]
        fn dhcp_roundtrip(m in arb_message()) {
            prop_assert_eq!(DhcpMessage::parse(&m.emit()).unwrap(), m);
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..400)) {
            let _ = DhcpMessage::parse(&b);
        }

        /// Anything that parses survives emit then parse unchanged.
        #[test]
        fn parsed_messages_are_stable(tail in proptest::collection::vec(any::<u8>(), 0..64)) {
            let mut b = DhcpMessage::default().emit();
            b.pop();
            b.extend_from_slice(&tail);
            if let Ok(m) = DhcpMessage::parse(&b) {
                prop_assert_eq!(DhcpMessage::parse(&m.emit()).unwrap(), m);
            }
        }
    }

    fn discover() -> DhcpMessage {
        DhcpMessage {
            xid: 0x1234_5678,
            chaddr: MacAddr([2, 0, 0, 0, 0, 1]),
            msg_type: Some(DhcpType::Discover),
            param_request: vec![1, 3, 6],
            ..DhcpMessage::default()
        }
    }

    #[test]
    fn a_discover_has_the_standard_layout() {
        let b = discover().emit();
        assert_eq!(b[0], 1, "BOOTREQUEST");
        assert_eq!((b[1], b[2]), (1, 6), "Ethernet, six-byte addresses");
        assert_eq!(&b[4..8], &[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(&b[28..34], &[2, 0, 0, 0, 0, 1]);
        assert_eq!(&b[236..240], &[0x63, 0x82, 0x53, 0x63], "magic cookie");
        assert_eq!(&b[240..243], &[53, 1, 1], "message type option");
        assert_eq!(*b.last().unwrap(), 255, "end option");
    }

    #[test]
    fn the_broadcast_flag_is_bit_15() {
        let mut m = discover();
        assert!(!m.broadcast());
        m.set_broadcast(true);
        assert_eq!(m.flags, 0x8000);
        assert!(DhcpMessage::parse(&m.emit()).unwrap().broadcast());
        m.set_broadcast(false);
        assert_eq!(m.flags, 0);
    }

    #[test]
    fn pad_options_and_trailing_bytes_are_skipped() {
        let mut b = discover().emit();
        b.pop();
        b.extend_from_slice(&[0, 0, 0, 255, 9, 9, 9]);
        assert_eq!(DhcpMessage::parse(&b).unwrap(), discover());
    }

    #[test]
    fn bad_messages_are_refused() {
        let good = discover().emit();
        assert_eq!(DhcpMessage::parse(&good[..100]), Err(ParseError::Truncated));
        let mut b = good.clone();
        b[0] = 3;
        assert_eq!(DhcpMessage::parse(&b), Err(ParseError::Unsupported));
        let mut b = good.clone();
        b[1] = 6;
        assert_eq!(
            DhcpMessage::parse(&b),
            Err(ParseError::Unsupported),
            "not Ethernet"
        );
        let mut b = good.clone();
        b[236] = 0;
        assert_eq!(
            DhcpMessage::parse(&b),
            Err(ParseError::Unsupported),
            "no cookie"
        );
        // Message type with the wrong length, an address option cut short, a type that is not one.
        for opt in [&[53u8, 2, 1, 1][..], &[50, 3, 1, 2, 3][..], &[53, 1, 9][..]] {
            let mut b = good[..240].to_vec();
            b.extend_from_slice(opt);
            b.push(255);
            assert!(DhcpMessage::parse(&b).is_err(), "{opt:?}");
        }
        let mut b = good[..240].to_vec();
        b.extend_from_slice(&[54, 4, 1, 2]);
        assert_eq!(DhcpMessage::parse(&b), Err(ParseError::Truncated));
    }

    #[test]
    fn unknown_options_are_kept_in_order() {
        let mut m = discover();
        m.unknown = vec![(82, vec![1, 2]), (200, vec![]), (82, vec![3])];
        let back = DhcpMessage::parse(&m.emit()).unwrap();
        assert_eq!(back.unknown, m.unknown);
    }
}
