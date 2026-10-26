use crate::{
    ArpOp, ArpPacket, Bpdu, BpduKind, DhcpMessage, DhcpOp, Dtp, DtpMode, EtherType, EthernetFrame,
    IcmpEcho, IcmpError, IcmpErrorKind, IpProto, Ipv4Packet, UdpDatagram,
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
        // A length instead of a type: an LLC frame, which here means a spanning tree BPDU.
        EtherType(t) if t < 0x0600 => match (Bpdu::parse(&f.payload), Dtp::parse(&f.payload)) {
            (Ok(b), _) => bpdu_summary(&b),
            (_, Ok(d)) => format!(
                "DTP {}{}",
                match d.mode {
                    DtpMode::On => "on",
                    DtpMode::Off => "off",
                    DtpMode::Desirable => "desirable",
                    DtpMode::Auto => "auto",
                },
                if d.trunking { " (trunking)" } else { "" }
            ),
            _ => format!("LLC frame ({t} bytes)"),
        },
        EtherType(t) => format!("ethertype {t:#06x}"),
    }
}

fn bpdu_summary(b: &Bpdu) -> String {
    match b.kind {
        BpduKind::Tcn => "STP topology change notification".into(),
        kind => {
            let name = if kind == BpduKind::Rapid {
                "RSTP"
            } else {
                "STP"
            };
            let mut o = format!(
                "{name} BPDU root {}/{} cost {} bridge {}/{}",
                b.root.priority, b.root.mac, b.root_cost, b.bridge.priority, b.bridge.mac
            );
            for (on, flag) in [
                (b.proposal, "proposal"),
                (b.agreement, "agreement"),
                (b.topology_change, "topology change"),
                (b.topology_change_ack, "topology change ack"),
            ] {
                if on {
                    o += &format!(", {flag}");
                }
            }
            o
        }
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

    #[test]
    fn describes_bpdus() {
        use crate::{BpduRole, BridgeId, PVST_MULTICAST};
        let b = Bpdu {
            kind: BpduKind::Rapid,
            topology_change: false,
            topology_change_ack: false,
            proposal: true,
            agreement: false,
            learning: false,
            forwarding: false,
            role: BpduRole::Designated,
            root: BridgeId {
                priority: 4097,
                mac: MacAddr([2, 0xbb, 0xff, 0, 0, 0]),
            },
            root_cost: 19,
            bridge: BridgeId {
                priority: 32769,
                mac: MacAddr([2, 0xbb, 0xff, 1, 0, 0]),
            },
            port: 0x8001,
            message_age: 0,
            max_age: Bpdu::seconds(20),
            hello_time: Bpdu::seconds(2),
            forward_delay: Bpdu::seconds(15),
        };
        let payload = b.emit();
        let frame = EthernetFrame {
            dst: PVST_MULTICAST,
            src: MacAddr([2, 0xbb, 0xff, 1, 0, 1]),
            vlan: None,
            ethertype: EtherType(payload.len() as u16),
            payload,
        }
        .emit();
        assert_eq!(
            summarize(&frame),
            "RSTP BPDU root 4097/02bb.ff00.0000 cost 19 bridge 32769/02bb.ff01.0000, proposal"
        );
    }
}
