//! OSPFv2 packets and the LSAs they carry (RFC 2328 appendices A.3 and A.4): Hello, Database
//! Description, Link State Request, Update and Acknowledgment; router and network LSAs. Other LSA
//! types are carried as bytes. Only null and simple-password authentication are understood.

use crate::{internet_checksum, Ipv4Addr, ParseError};

/// 224.0.0.5: all routers speaking OSPF.
pub const ALL_SPF_ROUTERS: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 5);
/// 224.0.0.6: the designated router and its backup.
pub const ALL_DR_ROUTERS: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 6);

/// Hello options: this router can take AS-external LSAs (the area is not a stub).
pub const OPT_E: u8 = 0x02;

/// Database Description flags.
pub const DBD_INIT: u8 = 0x04;
pub const DBD_MORE: u8 = 0x02;
pub const DBD_MASTER: u8 = 0x01;

/// LSA types carried in structure here.
pub const LSA_ROUTER: u8 = 1;
pub const LSA_NETWORK: u8 = 2;
/// Summary of a network in another area.
pub const LSA_SUMMARY: u8 = 3;
/// Summary of how to reach an AS boundary router in another area.
pub const LSA_ASBR_SUMMARY: u8 = 4;
pub const LSA_EXTERNAL: u8 = 5;

/// Router-LSA flags: this router is an area border router / an AS boundary router.
pub const ROUTER_B: u8 = 0x01;
pub const ROUTER_E: u8 = 0x02;

/// The metric that means "unreachable" in summary and external LSAs.
pub const LS_INFINITY: u32 = 0x00ff_ffff;

/// Router-LSA link types.
pub const LINK_P2P: u8 = 1;
pub const LINK_TRANSIT: u8 = 2;
pub const LINK_STUB: u8 = 3;

/// Age at which an LSA is being flushed (seconds).
pub const MAX_AGE: u16 = 3600;
/// Sequence numbers run from this up to `MAX_SEQ`.
pub const INITIAL_SEQ: i32 = i32::MIN + 1;
pub const MAX_SEQ: i32 = i32::MAX;

const HEADER_LEN: usize = 24;
const LSA_HEADER_LEN: usize = 20;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hello {
    pub mask: Ipv4Addr,
    pub hello_interval: u16,
    pub options: u8,
    pub priority: u8,
    pub dead_interval: u32,
    pub dr: Ipv4Addr,
    pub bdr: Ipv4Addr,
    pub neighbors: Vec<Ipv4Addr>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dbd {
    pub mtu: u16,
    pub options: u8,
    pub flags: u8,
    pub seq: u32,
    pub headers: Vec<LsaHeader>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LsRequest {
    pub ls_type: u32,
    pub id: Ipv4Addr,
    pub adv_router: Ipv4Addr,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OspfBody {
    Hello(Hello),
    Dbd(Dbd),
    Request(Vec<LsRequest>),
    Update(Vec<Lsa>),
    Ack(Vec<LsaHeader>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OspfPacket {
    pub router_id: Ipv4Addr,
    pub area: Ipv4Addr,
    /// 0 none, 1 simple password.
    pub auth_type: u16,
    pub auth: [u8; 8],
    pub body: OspfBody,
}

/// The 20 bytes every LSA starts with, as listed in Database Description and Acknowledgment
/// packets. `checksum` and `length` are the sender's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LsaHeader {
    pub age: u16,
    pub options: u8,
    pub ls_type: u8,
    pub id: Ipv4Addr,
    pub adv_router: Ipv4Addr,
    pub seq: i32,
    pub checksum: u16,
    pub length: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RouterLink {
    pub id: Ipv4Addr,
    pub data: Ipv4Addr,
    pub kind: u8,
    pub metric: u16,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LsaBody {
    Router {
        flags: u8,
        links: Vec<RouterLink>,
    },
    Network {
        mask: Ipv4Addr,
        routers: Vec<Ipv4Addr>,
    },
    /// Type 3 (a network in another area; `mask` is its mask) or type 4 (an AS boundary router
    /// in another area; `mask` is zero).
    Summary {
        ls_type: u8,
        mask: Ipv4Addr,
        metric: u32,
    },
    /// Type 5: a route from outside OSPF. `e2` is a type 2 metric (not added to the cost of
    /// reaching the boundary router). TOS metrics are not kept.
    External {
        mask: Ipv4Addr,
        e2: bool,
        metric: u32,
        forwarding: Ipv4Addr,
        tag: u32,
    },
    /// Any other type, unread.
    Other {
        ls_type: u8,
        data: Vec<u8>,
    },
}

/// A whole LSA. Its length and checksum are worked out when it is written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lsa {
    pub age: u16,
    pub options: u8,
    pub id: Ipv4Addr,
    pub adv_router: Ipv4Addr,
    pub seq: i32,
    pub body: LsaBody,
}

fn ip(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr(b[..4].try_into().unwrap())
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().unwrap())
}

// ---- LSAs ----

/// Fletcher sums of `data` modulo 255 (ISO 8473 annex C, as RFC 2328 12.1.7 uses).
fn fletcher(data: &[u8]) -> (i64, i64) {
    let (mut c0, mut c1) = (0i64, 0i64);
    for &b in data {
        c0 = (c0 + b as i64) % 255;
        c1 = (c1 + c0) % 255;
    }
    (c0, c1)
}

/// Fill in the checksum of a whole LSA (bytes 16 and 17); the age (bytes 0 and 1) is left out of
/// the sum so it can change in transit.
fn set_lsa_checksum(lsa: &mut [u8]) {
    lsa[16] = 0;
    lsa[17] = 0;
    let data = &lsa[2..];
    let (c0, c1) = fletcher(data);
    let l = data.len() as i64;
    // The checksum's first byte is the 15th of `data`.
    let mut x = ((l - 15) * c0 - c1).rem_euclid(255);
    let mut y = (c1 - (l - 14) * c0).rem_euclid(255);
    if x == 0 {
        x = 255;
    }
    if y == 0 {
        y = 255;
    }
    lsa[16] = x as u8;
    lsa[17] = y as u8;
}

/// Whether the checksum in a whole LSA is right.
fn lsa_checksum_ok(lsa: &[u8]) -> bool {
    fletcher(&lsa[2..]) == (0, 0)
}

impl LsaHeader {
    fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < LSA_HEADER_LEN {
            return Err(ParseError::Truncated);
        }
        Ok(LsaHeader {
            age: u16::from_be_bytes([b[0], b[1]]),
            options: b[2],
            ls_type: b[3],
            id: ip(&b[4..]),
            adv_router: ip(&b[8..]),
            seq: be32(&b[12..]) as i32,
            checksum: u16::from_be_bytes([b[16], b[17]]),
            length: u16::from_be_bytes([b[18], b[19]]),
        })
    }

    fn emit(&self, o: &mut Vec<u8>) {
        o.extend_from_slice(&self.age.to_be_bytes());
        o.push(self.options);
        o.push(self.ls_type);
        o.extend_from_slice(&self.id.0);
        o.extend_from_slice(&self.adv_router.0);
        o.extend_from_slice(&(self.seq as u32).to_be_bytes());
        o.extend_from_slice(&self.checksum.to_be_bytes());
        o.extend_from_slice(&self.length.to_be_bytes());
    }

    /// Whether two headers name the same LSA (type, id, advertising router).
    pub fn same_lsa(&self, other: &LsaHeader) -> bool {
        (self.ls_type, self.id, self.adv_router) == (other.ls_type, other.id, other.adv_router)
    }
}

impl Lsa {
    pub fn ls_type(&self) -> u8 {
        match &self.body {
            LsaBody::Router { .. } => LSA_ROUTER,
            LsaBody::Network { .. } => LSA_NETWORK,
            LsaBody::Summary { ls_type, .. } => *ls_type,
            LsaBody::External { .. } => LSA_EXTERNAL,
            LsaBody::Other { ls_type, .. } => *ls_type,
        }
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut body = Vec::new();
        match &self.body {
            LsaBody::Router { flags, links } => {
                body.push(*flags);
                body.push(0);
                body.extend_from_slice(&(links.len() as u16).to_be_bytes());
                for l in links {
                    body.extend_from_slice(&l.id.0);
                    body.extend_from_slice(&l.data.0);
                    body.push(l.kind);
                    body.push(0); // no TOS metrics
                    body.extend_from_slice(&l.metric.to_be_bytes());
                }
            }
            LsaBody::Network { mask, routers } => {
                body.extend_from_slice(&mask.0);
                for r in routers {
                    body.extend_from_slice(&r.0);
                }
            }
            LsaBody::Summary { mask, metric, .. } => {
                body.extend_from_slice(&mask.0);
                body.extend_from_slice(&(metric & LS_INFINITY).to_be_bytes());
            }
            LsaBody::External {
                mask,
                e2,
                metric,
                forwarding,
                tag,
            } => {
                body.extend_from_slice(&mask.0);
                body.extend_from_slice(
                    &(((*e2 as u32) << 31) | (metric & LS_INFINITY)).to_be_bytes(),
                );
                body.extend_from_slice(&forwarding.0);
                body.extend_from_slice(&tag.to_be_bytes());
            }
            LsaBody::Other { data, .. } => body.extend_from_slice(data),
        }
        let mut o = Vec::with_capacity(LSA_HEADER_LEN + body.len());
        LsaHeader {
            age: self.age,
            options: self.options,
            ls_type: self.ls_type(),
            id: self.id,
            adv_router: self.adv_router,
            seq: self.seq,
            checksum: 0,
            length: (LSA_HEADER_LEN + body.len()) as u16,
        }
        .emit(&mut o);
        o.extend_from_slice(&body);
        set_lsa_checksum(&mut o);
        o
    }

    /// The header this LSA is listed under in Database Description and Acknowledgment packets.
    pub fn header(&self) -> LsaHeader {
        LsaHeader::parse(&self.emit()).expect("an emitted LSA has a header")
    }

    /// Read one LSA from the start of `b`; returns it and how many bytes it took.
    pub fn parse(b: &[u8]) -> Result<(Lsa, usize), ParseError> {
        let h = LsaHeader::parse(b)?;
        let len = h.length as usize;
        if len < LSA_HEADER_LEN || len > b.len() {
            return Err(ParseError::BadLength);
        }
        let raw = &b[..len];
        if !lsa_checksum_ok(raw) {
            return Err(ParseError::BadChecksum);
        }
        let data = &raw[LSA_HEADER_LEN..];
        let body = match h.ls_type {
            LSA_ROUTER => {
                if data.len() < 4 {
                    return Err(ParseError::Truncated);
                }
                let n = u16::from_be_bytes([data[2], data[3]]) as usize;
                let mut at = 4;
                let mut links = Vec::new();
                for _ in 0..n {
                    if data.len() < at + 12 {
                        return Err(ParseError::BadLength);
                    }
                    let tos = data[at + 9] as usize;
                    links.push(RouterLink {
                        id: ip(&data[at..]),
                        data: ip(&data[at + 4..]),
                        kind: data[at + 8],
                        metric: u16::from_be_bytes([data[at + 10], data[at + 11]]),
                    });
                    // TOS metrics are obsolete (RFC 2328 appendix F): skipped.
                    at += 12 + 4 * tos;
                    if data.len() < at {
                        return Err(ParseError::BadLength);
                    }
                }
                LsaBody::Router {
                    flags: data[0],
                    links,
                }
            }
            LSA_NETWORK => {
                if data.len() < 4 || (data.len() - 4) % 4 != 0 {
                    return Err(ParseError::BadLength);
                }
                LsaBody::Network {
                    mask: ip(data),
                    routers: data[4..].chunks_exact(4).map(ip).collect(),
                }
            }
            t @ (LSA_SUMMARY | LSA_ASBR_SUMMARY) => {
                // Mask, then TOS 0 and a 24-bit metric; further TOS metrics are obsolete.
                if data.len() < 8 || (data.len() - 8) % 4 != 0 {
                    return Err(ParseError::BadLength);
                }
                LsaBody::Summary {
                    ls_type: t,
                    mask: ip(data),
                    metric: be32(&data[4..]) & LS_INFINITY,
                }
            }
            LSA_EXTERNAL => {
                if data.len() < 16 || (data.len() - 16) % 8 != 0 {
                    return Err(ParseError::BadLength);
                }
                LsaBody::External {
                    mask: ip(data),
                    e2: data[4] & 0x80 != 0,
                    metric: be32(&data[4..]) & LS_INFINITY,
                    forwarding: ip(&data[8..]),
                    tag: be32(&data[12..]),
                }
            }
            t => LsaBody::Other {
                ls_type: t,
                data: data.to_vec(),
            },
        };
        Ok((
            Lsa {
                age: h.age,
                options: h.options,
                id: h.id,
                adv_router: h.adv_router,
                seq: h.seq,
                body,
            },
            len,
        ))
    }
}

// ---- packets ----

impl OspfPacket {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < HEADER_LEN {
            return Err(ParseError::Truncated);
        }
        if b[0] != 2 {
            return Err(ParseError::Unsupported);
        }
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        if len < HEADER_LEN || len > b.len() {
            return Err(ParseError::BadLength);
        }
        let auth_type = u16::from_be_bytes([b[14], b[15]]);
        if auth_type > 1 {
            return Err(ParseError::Unsupported);
        }
        // The checksum covers the packet without the 8 bytes of authentication data.
        let mut summed = b[..len].to_vec();
        summed.drain(16..24);
        if internet_checksum(&summed) != 0 {
            return Err(ParseError::BadChecksum);
        }
        let d = &b[HEADER_LEN..len];
        let body = match b[1] {
            1 => {
                if d.len() < 20 || (d.len() - 20) % 4 != 0 {
                    return Err(ParseError::BadLength);
                }
                OspfBody::Hello(Hello {
                    mask: ip(d),
                    hello_interval: u16::from_be_bytes([d[4], d[5]]),
                    options: d[6],
                    priority: d[7],
                    dead_interval: be32(&d[8..]),
                    dr: ip(&d[12..]),
                    bdr: ip(&d[16..]),
                    neighbors: d[20..].chunks_exact(4).map(ip).collect(),
                })
            }
            2 => {
                if d.len() < 8 || (d.len() - 8) % LSA_HEADER_LEN != 0 {
                    return Err(ParseError::BadLength);
                }
                OspfBody::Dbd(Dbd {
                    mtu: u16::from_be_bytes([d[0], d[1]]),
                    options: d[2],
                    flags: d[3],
                    seq: be32(&d[4..]),
                    headers: d[8..]
                        .chunks_exact(LSA_HEADER_LEN)
                        .map(LsaHeader::parse)
                        .collect::<Result<_, _>>()?,
                })
            }
            3 => {
                if d.len() % 12 != 0 {
                    return Err(ParseError::BadLength);
                }
                OspfBody::Request(
                    d.chunks_exact(12)
                        .map(|c| LsRequest {
                            ls_type: be32(c),
                            id: ip(&c[4..]),
                            adv_router: ip(&c[8..]),
                        })
                        .collect(),
                )
            }
            4 => {
                if d.len() < 4 {
                    return Err(ParseError::Truncated);
                }
                let n = be32(d) as usize;
                let mut at = 4;
                let mut lsas = Vec::new();
                for _ in 0..n {
                    let (lsa, used) = Lsa::parse(&d[at..])?;
                    lsas.push(lsa);
                    at += used;
                }
                if at != d.len() {
                    return Err(ParseError::BadLength);
                }
                OspfBody::Update(lsas)
            }
            5 => {
                if d.len() % LSA_HEADER_LEN != 0 {
                    return Err(ParseError::BadLength);
                }
                OspfBody::Ack(
                    d.chunks_exact(LSA_HEADER_LEN)
                        .map(LsaHeader::parse)
                        .collect::<Result<_, _>>()?,
                )
            }
            _ => return Err(ParseError::Unsupported),
        };
        Ok(OspfPacket {
            router_id: ip(&b[4..]),
            area: ip(&b[8..]),
            auth_type,
            auth: b[16..24].try_into().unwrap(),
            body,
        })
    }

    pub fn emit(&self) -> Vec<u8> {
        let mut d = Vec::new();
        let kind = match &self.body {
            OspfBody::Hello(h) => {
                d.extend_from_slice(&h.mask.0);
                d.extend_from_slice(&h.hello_interval.to_be_bytes());
                d.push(h.options);
                d.push(h.priority);
                d.extend_from_slice(&h.dead_interval.to_be_bytes());
                d.extend_from_slice(&h.dr.0);
                d.extend_from_slice(&h.bdr.0);
                for n in &h.neighbors {
                    d.extend_from_slice(&n.0);
                }
                1
            }
            OspfBody::Dbd(x) => {
                d.extend_from_slice(&x.mtu.to_be_bytes());
                d.push(x.options);
                d.push(x.flags);
                d.extend_from_slice(&x.seq.to_be_bytes());
                for h in &x.headers {
                    h.emit(&mut d);
                }
                2
            }
            OspfBody::Request(rs) => {
                for r in rs {
                    d.extend_from_slice(&r.ls_type.to_be_bytes());
                    d.extend_from_slice(&r.id.0);
                    d.extend_from_slice(&r.adv_router.0);
                }
                3
            }
            OspfBody::Update(lsas) => {
                d.extend_from_slice(&(lsas.len() as u32).to_be_bytes());
                for l in lsas {
                    d.extend_from_slice(&l.emit());
                }
                4
            }
            OspfBody::Ack(hs) => {
                for h in hs {
                    h.emit(&mut d);
                }
                5
            }
        };
        let mut o = Vec::with_capacity(HEADER_LEN + d.len());
        o.push(2);
        o.push(kind);
        o.extend_from_slice(&((HEADER_LEN + d.len()) as u16).to_be_bytes());
        o.extend_from_slice(&self.router_id.0);
        o.extend_from_slice(&self.area.0);
        o.extend_from_slice(&[0, 0]);
        o.extend_from_slice(&self.auth_type.to_be_bytes());
        o.extend_from_slice(&self.auth);
        o.extend_from_slice(&d);
        let mut summed = o.clone();
        summed.drain(16..24);
        let c = internet_checksum(&summed);
        o[12..14].copy_from_slice(&c.to_be_bytes());
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

    fn lsa_header() -> impl Strategy<Value = LsaHeader> {
        (
            any::<u16>(),
            any::<u8>(),
            any::<u8>(),
            addr(),
            addr(),
            any::<i32>(),
            any::<u16>(),
            any::<u16>(),
        )
            .prop_map(
                |(age, options, ls_type, id, adv_router, seq, checksum, length)| LsaHeader {
                    age,
                    options,
                    ls_type,
                    id,
                    adv_router,
                    seq,
                    checksum,
                    length,
                },
            )
    }

    fn lsa() -> impl Strategy<Value = Lsa> {
        let link = (addr(), addr(), 1u8..5, any::<u16>()).prop_map(|(id, data, kind, metric)| {
            RouterLink {
                id,
                data,
                kind,
                metric,
            }
        });
        let body = prop_oneof![
            (any::<u8>(), proptest::collection::vec(link, 0..5))
                .prop_map(|(flags, links)| LsaBody::Router { flags, links }),
            (addr(), proptest::collection::vec(addr(), 0..5))
                .prop_map(|(mask, routers)| LsaBody::Network { mask, routers }),
            (3u8..=4, addr(), 0u32..=LS_INFINITY).prop_map(|(ls_type, mask, metric)| {
                LsaBody::Summary {
                    ls_type,
                    mask,
                    metric,
                }
            }),
            (
                addr(),
                any::<bool>(),
                0u32..=LS_INFINITY,
                addr(),
                any::<u32>()
            )
                .prop_map(|(mask, e2, metric, forwarding, tag)| LsaBody::External {
                    mask,
                    e2,
                    metric,
                    forwarding,
                    tag
                }),
            (6u8..=255, proptest::collection::vec(any::<u8>(), 0..24))
                .prop_map(|(ls_type, data)| LsaBody::Other { ls_type, data }),
        ];
        (
            any::<u16>(),
            any::<u8>(),
            addr(),
            addr(),
            any::<i32>(),
            body,
        )
            .prop_map(|(age, options, id, adv_router, seq, body)| Lsa {
                age,
                options,
                id,
                adv_router,
                seq,
                body,
            })
    }

    fn body() -> impl Strategy<Value = OspfBody> {
        prop_oneof![
            (
                addr(),
                any::<u16>(),
                any::<u8>(),
                any::<u8>(),
                any::<u32>(),
                addr(),
                addr(),
                proptest::collection::vec(addr(), 0..5)
            )
                .prop_map(
                    |(
                        mask,
                        hello_interval,
                        options,
                        priority,
                        dead_interval,
                        dr,
                        bdr,
                        neighbors,
                    )| {
                        OspfBody::Hello(Hello {
                            mask,
                            hello_interval,
                            options,
                            priority,
                            dead_interval,
                            dr,
                            bdr,
                            neighbors,
                        })
                    }
                ),
            (
                any::<u16>(),
                any::<u8>(),
                any::<u8>(),
                any::<u32>(),
                proptest::collection::vec(lsa_header(), 0..4)
            )
                .prop_map(|(mtu, options, flags, seq, headers)| OspfBody::Dbd(Dbd {
                    mtu,
                    options,
                    flags,
                    seq,
                    headers
                })),
            proptest::collection::vec((any::<u32>(), addr(), addr()), 0..4).prop_map(|v| {
                OspfBody::Request(
                    v.into_iter()
                        .map(|(ls_type, id, adv_router)| LsRequest {
                            ls_type,
                            id,
                            adv_router,
                        })
                        .collect(),
                )
            }),
            proptest::collection::vec(lsa(), 0..4).prop_map(OspfBody::Update),
            proptest::collection::vec(lsa_header(), 0..4).prop_map(OspfBody::Ack),
        ]
    }

    proptest! {
        #[test]
        fn packets_roundtrip(router_id in addr(), area in addr(), simple in any::<bool>(), auth in any::<[u8; 8]>(), body in body()) {
            let p = OspfPacket { router_id, area, auth_type: simple as u16, auth, body };
            prop_assert_eq!(OspfPacket::parse(&p.emit()).unwrap(), p);
        }

        #[test]
        fn lsa_checksums_verify_and_catch_a_flipped_bit(l in lsa(), at in any::<usize>(), bit in 0u8..8) {
            let raw = l.emit();
            prop_assert!(lsa_checksum_ok(&raw));
            // Everything but the age is protected, and a single flipped bit always changes a
            // sum modulo 255.
            let mut bad = raw.clone();
            let i = 2 + at % (raw.len() - 2);
            bad[i] ^= 1 << bit;
            prop_assert!(!lsa_checksum_ok(&bad));
        }

        #[test]
        fn the_age_can_change_without_breaking_the_checksum(l in lsa(), age in any::<u16>()) {
            let mut raw = l.emit();
            raw[..2].copy_from_slice(&age.to_be_bytes());
            let (back, n) = Lsa::parse(&raw).unwrap();
            prop_assert_eq!(n, raw.len());
            prop_assert_eq!(back.age, age);
        }

        #[test]
        fn parse_never_panics(b in proptest::collection::vec(any::<u8>(), 0..96)) {
            let _ = OspfPacket::parse(&b);
            let _ = Lsa::parse(&b);
        }
    }

    #[test]
    fn a_hello_has_the_layout_of_appendix_a() {
        let p = OspfPacket {
            router_id: Ipv4Addr::new(1, 1, 1, 1),
            area: Ipv4Addr::new(0, 0, 0, 0),
            auth_type: 0,
            auth: [0; 8],
            body: OspfBody::Hello(Hello {
                mask: Ipv4Addr::new(255, 255, 255, 0),
                hello_interval: 10,
                options: OPT_E,
                priority: 1,
                dead_interval: 40,
                dr: Ipv4Addr::new(10, 0, 0, 2),
                bdr: Ipv4Addr::UNSPECIFIED,
                neighbors: vec![Ipv4Addr::new(2, 2, 2, 2)],
            }),
        };
        let b = p.emit();
        assert_eq!(b.len(), 24 + 20 + 4);
        assert_eq!(&b[..2], &[2, 1], "version 2, type Hello");
        assert_eq!(&b[2..4], &[0, 48], "packet length");
        assert_eq!(&b[4..8], &[1, 1, 1, 1]);
        assert_eq!(&b[24..28], &[255, 255, 255, 0], "network mask");
        assert_eq!(&b[28..30], &[0, 10], "hello interval");
        assert_eq!((b[30], b[31]), (OPT_E, 1), "options, priority");
        assert_eq!(&b[32..36], &[0, 0, 0, 40], "dead interval");
        assert_eq!(&b[36..40], &[10, 0, 0, 2], "designated router");
        assert_eq!(&b[44..48], &[2, 2, 2, 2], "a neighbour");
        assert_eq!(
            internet_checksum(&{
                let mut s = b.clone();
                s.drain(16..24);
                s
            }),
            0
        );
    }

    #[test]
    fn summary_and_external_lsas_have_the_layout_of_appendix_a() {
        let sum = Lsa {
            age: 0,
            options: OPT_E,
            id: Ipv4Addr::new(10, 1, 0, 0),
            adv_router: Ipv4Addr::new(1, 1, 1, 1),
            seq: INITIAL_SEQ,
            body: LsaBody::Summary {
                ls_type: LSA_SUMMARY,
                mask: Ipv4Addr::new(255, 255, 0, 0),
                metric: 0x0102_0304 & LS_INFINITY,
            },
        };
        let b = sum.emit();
        assert_eq!(b.len(), 28);
        assert_eq!(b[3], 3);
        assert_eq!(&b[20..24], &[255, 255, 0, 0], "mask");
        assert_eq!(&b[24..28], &[0, 2, 3, 4], "TOS 0 and a 24-bit metric");
        let ext = Lsa {
            age: 0,
            options: OPT_E,
            id: Ipv4Addr::UNSPECIFIED,
            adv_router: Ipv4Addr::new(1, 1, 1, 1),
            seq: INITIAL_SEQ,
            body: LsaBody::External {
                mask: Ipv4Addr::UNSPECIFIED,
                e2: true,
                metric: 20,
                forwarding: Ipv4Addr::UNSPECIFIED,
                tag: 7,
            },
        };
        let b = ext.emit();
        assert_eq!(b.len(), 36);
        assert_eq!(b[3], 5);
        assert_eq!(&b[24..28], &[0x80, 0, 0, 20], "E bit and metric");
        assert_eq!(&b[32..36], &[0, 0, 0, 7], "tag");
        assert_eq!(Lsa::parse(&b).unwrap().0, ext);
    }

    #[test]
    fn a_router_lsa_has_the_layout_of_appendix_a() {
        let l = Lsa {
            age: 7,
            options: OPT_E,
            id: Ipv4Addr::new(1, 1, 1, 1),
            adv_router: Ipv4Addr::new(1, 1, 1, 1),
            seq: INITIAL_SEQ,
            body: LsaBody::Router {
                flags: 0,
                links: vec![RouterLink {
                    id: Ipv4Addr::new(10, 0, 0, 0),
                    data: Ipv4Addr::new(255, 255, 255, 0),
                    kind: LINK_STUB,
                    metric: 1,
                }],
            },
        };
        let b = l.emit();
        assert_eq!(b.len(), 20 + 4 + 12);
        assert_eq!(&b[18..20], &[0, 36], "length counts the header");
        assert_eq!(b[3], LSA_ROUTER);
        assert_eq!(&b[12..16], &[0x80, 0, 0, 1], "first sequence number");
        assert_eq!(&b[22..24], &[0, 1], "one link");
        assert_eq!(&b[32..34], &[3, 0], "stub, no TOS");
        assert!(lsa_checksum_ok(&b));
        assert_eq!(l.header().length, 36);
    }

    #[test]
    fn what_is_wrong_is_named() {
        let p = OspfPacket {
            router_id: Ipv4Addr::new(1, 1, 1, 1),
            area: Ipv4Addr::UNSPECIFIED,
            auth_type: 0,
            auth: [0; 8],
            body: OspfBody::Ack(vec![]),
        };
        let good = p.emit();
        assert_eq!(OspfPacket::parse(&good[..23]), Err(ParseError::Truncated));
        let mut b = good.clone();
        b[0] = 3;
        assert_eq!(OspfPacket::parse(&b), Err(ParseError::Unsupported));
        let mut b = good.clone();
        b[15] = 2; // cryptographic authentication
        assert_eq!(OspfPacket::parse(&b), Err(ParseError::Unsupported));
        let mut b = good.clone();
        b[5] ^= 1;
        assert_eq!(OspfPacket::parse(&b), Err(ParseError::BadChecksum));
        let mut b = good.clone();
        b[3] = 99;
        assert_eq!(OspfPacket::parse(&b), Err(ParseError::BadLength));
        // The authentication data is not part of the sum, so it may be anything.
        let mut b = good;
        b[16] = 0xaa;
        assert!(OspfPacket::parse(&b).is_ok());
    }
}
