use crate::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, IcmpEcho, IcmpError, IcmpErrorKind, IpProto,
    Ipv4Packet,
};

/// One-line description of a frame for the packet inspector. Never fails: unparseable
/// layers are reported as such.
pub fn summarize(frame: &[u8]) -> String {
    let Ok(f) = EthernetFrame::parse(frame) else {
        return format!("malformed frame ({} bytes)", frame.len());
    };
    match f.ethertype {
        EtherType::ARP => match ArpPacket::parse(&f.payload) {
            Ok(a) if a.op == ArpOp::Request => {
                format!("ARP request: who has {}? tell {}", a.target_ip, a.sender_ip)
            }
            Ok(a) => format!("ARP reply: {} is at {}", a.sender_ip, a.sender_mac),
            Err(_) => "ARP (malformed)".into(),
        },
        EtherType::IPV4 => match Ipv4Packet::parse(&f.payload) {
            Ok(p) => {
                let what = match p.proto {
                    IpProto::ICMP => {
                        if let Ok(e) = IcmpEcho::parse(&p.payload) {
                            format!(
                                "ICMP echo {} seq={}",
                                if e.reply { "reply" } else { "request" },
                                e.seq
                            )
                        } else if let Ok(e) = IcmpError::parse(&p.payload) {
                            match e.kind {
                                IcmpErrorKind::DestUnreachable(0) => {
                                    "ICMP network unreachable".into()
                                }
                                IcmpErrorKind::DestUnreachable(1) => "ICMP host unreachable".into(),
                                IcmpErrorKind::DestUnreachable(c) => {
                                    format!("ICMP unreachable (code {c})")
                                }
                                IcmpErrorKind::TimeExceeded => "ICMP time exceeded".into(),
                            }
                        } else {
                            "ICMP".into()
                        }
                    }
                    IpProto(n) => format!("IP protocol {n}"),
                };
                format!("{what}: {} > {} ttl={}", p.src, p.dst, p.ttl)
            }
            Err(_) => "IPv4 (malformed)".into(),
        },
        EtherType(t) => format!("ethertype {t:#06x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ipv4Addr, MacAddr};

    #[test]
    fn describes_arp_and_icmp() {
        let req = ArpPacket {
            op: ArpOp::Request,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::default(),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let f = EthernetFrame {
            dst: MacAddr::BROADCAST,
            src: req.sender_mac,
            vlan: None,
            ethertype: EtherType::ARP,
            payload: req.emit(),
        };
        assert_eq!(
            summarize(&f.emit()),
            "ARP request: who has 10.0.0.2? tell 10.0.0.1"
        );
        assert!(summarize(&[1, 2, 3]).starts_with("malformed"));
    }
}
