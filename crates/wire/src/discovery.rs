//! Neighbour discovery frames: CDP (Cisco's, long publicly documented, for instance by the
//! Wireshark dissector) and LLDP (IEEE 802.1AB). Both carry the sender's name, the port it sent
//! on, what it is and how to reach it. Frames here are whole Ethernet frames.

use crate::{internet_checksum, Ipv4Addr, MacAddr, ParseError};

/// Where CDP goes (shared with DTP; the SNAP protocol id tells them apart).
pub const CDP_MULTICAST: MacAddr = MacAddr([0x01, 0x00, 0x0c, 0xcc, 0xcc, 0xcc]);
/// Where LLDP goes: a link-local address no bridge forwards.
pub const LLDP_MULTICAST: MacAddr = MacAddr([0x01, 0x80, 0xc2, 0x00, 0x00, 0x0e]);
pub const LLDP_TYPE: u16 = 0x88cc;

const SNAP_CDP: [u8; 8] = [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x0c, 0x20, 0x00];

/// Capability bits as CDP states them.
pub mod cap {
    pub const ROUTER: u32 = 0x01;
    pub const SWITCH: u32 = 0x08;
    pub const HOST: u32 = 0x10;
}

/// A CDP advertisement.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cdp {
    pub device_id: String,
    /// The sender's port, long form (`FastEthernet0/1`).
    pub port_id: String,
    pub capabilities: u32,
    pub platform: String,
    pub software: String,
    pub addresses: Vec<Ipv4Addr>,
    /// Seconds the receiver keeps this entry.
    pub holdtime: u8,
}

fn tlv(out: &mut Vec<u8>, ty: u16, v: &[u8]) {
    out.extend_from_slice(&ty.to_be_bytes());
    out.extend_from_slice(&(v.len() as u16 + 4).to_be_bytes());
    out.extend_from_slice(v);
}

impl Cdp {
    /// The whole frame, from `src`.
    pub fn emit(&self, src: MacAddr) -> Vec<u8> {
        let mut body = vec![2, self.holdtime, 0, 0];
        tlv(&mut body, 1, self.device_id.as_bytes());
        let mut a = (self.addresses.len() as u32).to_be_bytes().to_vec();
        for ip in &self.addresses {
            a.extend_from_slice(&[1, 1, 0xcc, 0, 4]);
            a.extend_from_slice(&ip.0);
        }
        tlv(&mut body, 2, &a);
        tlv(&mut body, 3, self.port_id.as_bytes());
        tlv(&mut body, 4, &self.capabilities.to_be_bytes());
        tlv(&mut body, 5, self.software.as_bytes());
        tlv(&mut body, 6, self.platform.as_bytes());
        let sum = internet_checksum(&body);
        body[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut payload = SNAP_CDP.to_vec();
        payload.extend_from_slice(&body);
        let mut f = CDP_MULTICAST.0.to_vec();
        f.extend_from_slice(&src.0);
        f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        f.extend(payload);
        f
    }

    /// Whether this frame is CDP (and not DTP, which shares its address).
    pub fn is_cdp(frame: &[u8]) -> bool {
        frame.len() >= 22 && frame[0..6] == CDP_MULTICAST.0 && frame[14..22] == SNAP_CDP
    }

    pub fn parse(frame: &[u8]) -> Result<(MacAddr, Cdp), ParseError> {
        if !Cdp::is_cdp(frame) {
            return Err(ParseError::Unsupported);
        }
        let len = u16::from_be_bytes([frame[12], frame[13]]) as usize;
        if len < 8 + 4 || 14 + len > frame.len() {
            return Err(ParseError::BadLength);
        }
        let b = &frame[22..14 + len];
        if b.len() < 4 || b[0] != 2 {
            return Err(ParseError::Unsupported);
        }
        let mut sum = b.to_vec();
        sum[2] = 0;
        sum[3] = 0;
        let want = u16::from_be_bytes([b[2], b[3]]);
        if internet_checksum(&sum) != want {
            return Err(ParseError::BadChecksum);
        }
        let mut cdp = Cdp {
            holdtime: b[1],
            ..Cdp::default()
        };
        let mut rest = &b[4..];
        let mut named = false;
        while rest.len() >= 4 {
            let ty = u16::from_be_bytes([rest[0], rest[1]]);
            let l = u16::from_be_bytes([rest[2], rest[3]]) as usize;
            if l < 4 || l > rest.len() {
                return Err(ParseError::BadLength);
            }
            let v = &rest[4..l];
            let text = || String::from_utf8_lossy(v).into_owned();
            match ty {
                1 => {
                    cdp.device_id = text();
                    named = true;
                }
                2 if v.len() >= 4 => {
                    let n = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
                    let mut a = &v[4..];
                    for _ in 0..n {
                        // protocol type, protocol length, protocol (NLPID 0xcc is IP)
                        if a.len() < 3 || a[1] as usize + 2 > a.len() {
                            break;
                        }
                        let pl = a[1] as usize;
                        let ip = a[2..2 + pl] == [0xcc];
                        let rest = &a[2 + pl..];
                        if rest.len() < 2 {
                            break;
                        }
                        let al = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                        if rest.len() < 2 + al {
                            break;
                        }
                        if ip && al == 4 {
                            cdp.addresses.push(Ipv4Addr(rest[2..6].try_into().unwrap()));
                        }
                        a = &rest[2 + al..];
                    }
                }
                3 => cdp.port_id = text(),
                4 if v.len() >= 4 => {
                    cdp.capabilities = u32::from_be_bytes([v[0], v[1], v[2], v[3]])
                }
                5 => cdp.software = text(),
                6 => cdp.platform = text(),
                _ => {}
            }
            rest = &rest[l..];
        }
        if !named {
            return Err(ParseError::Truncated);
        }
        Ok((MacAddr(frame[6..12].try_into().unwrap()), cdp))
    }
}

/// An LLDP advertisement.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Lldp {
    /// Chassis id, subtype 4: the sender's MAC address.
    pub chassis: MacAddr,
    /// Port id, subtype 5 (interface name).
    pub port_id: String,
    pub ttl: u16,
    pub port_description: String,
    pub system_name: String,
    pub system_description: String,
    /// Bit 0 other, 2 bridge, 4 router, 5 telephone, 7 station (802.1AB).
    pub capabilities: u16,
    pub enabled: u16,
    pub management: Option<Ipv4Addr>,
}

pub mod lldp_cap {
    pub const BRIDGE: u16 = 1 << 2;
    pub const ROUTER: u16 = 1 << 4;
    pub const STATION: u16 = 1 << 7;
}

fn lldp_tlv(out: &mut Vec<u8>, ty: u8, v: &[u8]) {
    let head = (u16::from(ty) << 9) | (v.len() as u16 & 0x1ff);
    out.extend_from_slice(&head.to_be_bytes());
    out.extend_from_slice(v);
}

impl Lldp {
    pub fn emit(&self, src: MacAddr) -> Vec<u8> {
        let mut b = Vec::new();
        let mut c = vec![4];
        c.extend_from_slice(&self.chassis.0);
        lldp_tlv(&mut b, 1, &c);
        let mut p = vec![5];
        p.extend_from_slice(self.port_id.as_bytes());
        lldp_tlv(&mut b, 2, &p);
        lldp_tlv(&mut b, 3, &self.ttl.to_be_bytes());
        if !self.port_description.is_empty() {
            lldp_tlv(&mut b, 4, self.port_description.as_bytes());
        }
        lldp_tlv(&mut b, 5, self.system_name.as_bytes());
        lldp_tlv(&mut b, 6, self.system_description.as_bytes());
        let mut caps = self.capabilities.to_be_bytes().to_vec();
        caps.extend_from_slice(&self.enabled.to_be_bytes());
        lldp_tlv(&mut b, 7, &caps);
        if let Some(ip) = self.management {
            // address length, subtype 1 (IPv4), address, interface numbering 1 (unknown), number, OID length
            let mut m = vec![5, 1];
            m.extend_from_slice(&ip.0);
            m.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
            lldp_tlv(&mut b, 8, &m);
        }
        lldp_tlv(&mut b, 0, &[]);
        let mut f = LLDP_MULTICAST.0.to_vec();
        f.extend_from_slice(&src.0);
        f.extend_from_slice(&LLDP_TYPE.to_be_bytes());
        f.extend(b);
        if f.len() < 60 {
            f.resize(60, 0);
        }
        f
    }

    pub fn parse(frame: &[u8]) -> Result<(MacAddr, Lldp), ParseError> {
        if frame.len() < 14 || frame[0..6] != LLDP_MULTICAST.0 {
            return Err(ParseError::Unsupported);
        }
        if u16::from_be_bytes([frame[12], frame[13]]) != LLDP_TYPE {
            return Err(ParseError::Unsupported);
        }
        let mut rest = &frame[14..];
        let mut l = Lldp::default();
        let (mut chassis, mut port, mut ttl) = (false, false, false);
        loop {
            if rest.len() < 2 {
                return Err(ParseError::Truncated);
            }
            let head = u16::from_be_bytes([rest[0], rest[1]]);
            let (ty, len) = ((head >> 9) as u8, (head & 0x1ff) as usize);
            if rest.len() < 2 + len {
                return Err(ParseError::BadLength);
            }
            let v = &rest[2..2 + len];
            rest = &rest[2 + len..];
            let text = || String::from_utf8_lossy(v).into_owned();
            match ty {
                0 => break,
                1 if len == 7 && v[0] == 4 => {
                    l.chassis = MacAddr(v[1..7].try_into().unwrap());
                    chassis = true;
                }
                // Other chassis subtypes: keep something readable, never fail.
                1 if len >= 2 => chassis = true,
                2 if len >= 2 => {
                    l.port_id = String::from_utf8_lossy(&v[1..]).into_owned();
                    port = true;
                }
                3 if len == 2 => {
                    l.ttl = u16::from_be_bytes([v[0], v[1]]);
                    ttl = true;
                }
                4 => l.port_description = text(),
                5 => l.system_name = text(),
                6 => l.system_description = text(),
                7 if len == 4 => {
                    l.capabilities = u16::from_be_bytes([v[0], v[1]]);
                    l.enabled = u16::from_be_bytes([v[2], v[3]]);
                }
                8 if len >= 7 && v[0] >= 5 && v[1] == 1 => {
                    l.management = Some(Ipv4Addr(v[2..6].try_into().unwrap()));
                }
                _ => {}
            }
        }
        if !(chassis && port && ttl) {
            return Err(ParseError::Truncated);
        }
        Ok((MacAddr(frame[6..12].try_into().unwrap()), l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cdp() -> Cdp {
        Cdp {
            device_id: "sw1".into(),
            port_id: "FastEthernet0/1".into(),
            capabilities: cap::SWITCH,
            platform: "wirebench L2 switch".into(),
            software: "wirebench 0".into(),
            addresses: vec![Ipv4Addr([10, 0, 0, 1])],
            holdtime: 180,
        }
    }

    fn lldp() -> Lldp {
        Lldp {
            chassis: MacAddr([2, 0xbb, 0xff, 1, 0, 0]),
            port_id: "Fa0/1".into(),
            ttl: 120,
            port_description: "FastEthernet0/1".into(),
            system_name: "sw1".into(),
            system_description: "wirebench L2 switch".into(),
            capabilities: lldp_cap::BRIDGE | lldp_cap::ROUTER,
            enabled: lldp_cap::BRIDGE,
            management: Some(Ipv4Addr([10, 0, 0, 1])),
        }
    }

    #[test]
    fn cdp_round_trips() {
        let src = MacAddr([2, 0, 0, 0, 0, 9]);
        let f = cdp().emit(src);
        assert!(Cdp::is_cdp(&f));
        assert_eq!(Cdp::parse(&f), Ok((src, cdp())));
    }

    #[test]
    fn cdp_checksum_is_checked() {
        let mut f = cdp().emit(MacAddr([2, 0, 0, 0, 0, 9]));
        let n = f.len();
        f[n - 3] ^= 0x55;
        assert_eq!(Cdp::parse(&f), Err(ParseError::BadChecksum));
    }

    #[test]
    fn lldp_round_trips() {
        let src = MacAddr([2, 0, 0, 0, 0, 9]);
        let f = lldp().emit(src);
        assert_eq!(Lldp::parse(&f), Ok((src, lldp())));
    }

    #[test]
    fn truncation_and_noise_never_panic() {
        let src = MacAddr([2, 0, 0, 0, 0, 9]);
        for f in [cdp().emit(src), lldp().emit(src)] {
            for n in 0..f.len() {
                let _ = Cdp::parse(&f[..n]);
                let _ = Lldp::parse(&f[..n]);
            }
            for i in 14..f.len() {
                let mut g = f.clone();
                g[i] = g[i].wrapping_add(0x41);
                let _ = Cdp::parse(&g);
                let _ = Lldp::parse(&g);
            }
        }
    }

    #[test]
    fn dtp_is_not_cdp() {
        let mut f = vec![0u8; 40];
        f[0..6].copy_from_slice(&CDP_MULTICAST.0);
        f[14..22].copy_from_slice(&[0xaa, 0xaa, 0x03, 0x00, 0x00, 0x0c, 0x20, 0x04]);
        assert!(!Cdp::is_cdp(&f));
    }
}
