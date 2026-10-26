use crate::{
    ArpOp, ArpPacket, DhcpMessage, DhcpOp, EtherType, EthernetFrame, IcmpEcho, IcmpError,
    IcmpErrorKind, IpProto, Ipv4Packet, UdpDatagram,
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
                                IcmpErrorKind::DestUnreachable(2) => {
                                    "ICMP protocol unreachable".into()
                                }
                                IcmpErrorKind::DestUnreachable(3) => "ICMP port unreachable".into(),
                                IcmpErrorKind::DestUnreachable(c) => {
                                    format!("ICMP unreachable (code {c})")
                                }
                                IcmpErrorKind::TimeExceeded => "ICMP time exceeded".into(),
                            }
                        } else {
                            "ICMP".into()
                        }
                    }
                    IpProto::UDP => match UdpDatagram::parse(&p.payload) {
                        Ok(u) => match DhcpMessage::parse(&u.payload) {
                            Ok(d) if matches!((u.src_port, u.dst_port), (67 | 68, 67 | 68)) => {
                                dhcp_summary(&d)
                            }
                            _ => format!("UDP {} > {}", u.src_port, u.dst_port),
                        },
                        Err(_) => "UDP (malformed)".into(),
                    },
                    IpProto(n) => format!("IP protocol {n}"),
                };
                format!("{what}: {} > {} ttl={}", p.src, p.dst, p.ttl)
            }
            Err(_) => "IPv4 (malformed)".into(),
        },
        EtherType(t) => format!("ethertype {t:#06x}"),
    }
}

fn dhcp_summary(d: &DhcpMessage) -> String {
    let kind = d.msg_type.map_or("message", |t| t.name());
    let mut o = format!("DHCP {kind}");
    if d.op == DhcpOp::Reply && d.yiaddr != crate::Ipv4Addr::UNSPECIFIED {
        o += &format!(" {}", d.yiaddr);
    } else if let Some(r) = d.requested_ip {
        o += &format!(" for {r}");
    }
    o += &format!(" xid={:#010x}", d.xid);
    if d.giaddr != crate::Ipv4Addr::UNSPECIFIED {
        o += &format!(" relay {}", d.giaddr);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DhcpType, Ipv4Addr, MacAddr};

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

    fn dhcp_frame(d: &DhcpMessage, sp: u16, dp: u16) -> Vec<u8> {
        let (src, dst) = (Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST);
        let udp = UdpDatagram {
            src_port: sp,
            dst_port: dp,
            payload: d.emit(),
        };
        let ip = Ipv4Packet {
            dscp_ecn: 0,
            ident: 1,
            flags_frag: 0,
            ttl: 64,
            proto: IpProto::UDP,
            src,
            dst,
            payload: udp.emit(src, dst),
        };
        EthernetFrame {
            dst: MacAddr::BROADCAST,
            src: MacAddr([2, 0, 0, 0, 0, 1]),
            vlan: None,
            ethertype: EtherType::IPV4,
            payload: ip.emit(),
        }
        .emit()
    }

    #[test]
    fn names_the_common_icmp_unreachables() {
        let frame = |kind| {
            let e = IcmpError {
                kind,
                original: vec![0; 28],
            };
            let ip = Ipv4Packet {
                dscp_ecn: 0,
                ident: 1,
                flags_frag: 0,
                ttl: 64,
                proto: IpProto::ICMP,
                src: Ipv4Addr::new(10, 0, 0, 1),
                dst: Ipv4Addr::new(10, 0, 0, 2),
                payload: e.emit(),
            };
            EthernetFrame {
                dst: MacAddr::BROADCAST,
                src: MacAddr([2, 0, 0, 0, 0, 1]),
                vlan: None,
                ethertype: EtherType::IPV4,
                payload: ip.emit(),
            }
            .emit()
        };
        assert!(summarize(&frame(IcmpErrorKind::DestUnreachable(3)))
            .starts_with("ICMP port unreachable:"));
        assert!(summarize(&frame(IcmpErrorKind::DestUnreachable(1)))
            .starts_with("ICMP host unreachable:"));
        assert!(summarize(&frame(IcmpErrorKind::TimeExceeded)).starts_with("ICMP time exceeded:"));
    }

    #[test]
    fn describes_dhcp_and_other_udp() {
        let mut d = DhcpMessage {
            xid: 0xabcd,
            msg_type: Some(DhcpType::Discover),
            ..DhcpMessage::default()
        };
        let s = summarize(&dhcp_frame(&d, 68, 67));
        assert!(s.starts_with("DHCP discover xid=0x0000abcd:"), "{s}");
        d.op = DhcpOp::Reply;
        d.msg_type = Some(DhcpType::Offer);
        d.yiaddr = Ipv4Addr::new(10, 0, 0, 5);
        d.giaddr = Ipv4Addr::new(10, 0, 1, 1);
        let s = summarize(&dhcp_frame(&d, 67, 68));
        assert!(
            s.starts_with("DHCP offer 10.0.0.5 xid=0x0000abcd relay 10.0.1.1:"),
            "{s}"
        );
        // The same bytes on other ports are plain UDP.
        assert!(summarize(&dhcp_frame(&d, 5000, 6000)).starts_with("UDP 5000 > 6000:"));
    }
}
