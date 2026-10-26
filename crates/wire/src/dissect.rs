//! The packet inspector's view of a frame: a tree of layers and fields, each knowing which bytes
//! it covers, and a hex dump. This reads the bytes directly rather than through the parsers, so
//! that every field has an offset; it never fails, and a layer it cannot finish is cut short and
//! marked.

use crate::{DtpMode, Ipv4Addr, MacAddr};
use std::ops::Range;

/// One line of the tree. `range` is the bytes of the frame it describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub range: Range<usize>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(label: impl Into<String>, range: Range<usize>) -> Node {
        Node {
            label: label.into(),
            range,
            children: vec![],
        }
    }

    fn with(mut self, children: Vec<Node>) -> Node {
        self.children = children;
        self
    }

    /// The innermost node whose range holds byte `at`, with the path of labels down to it.
    pub fn find(&self, at: usize) -> Option<Vec<&Node>> {
        if !self.range.contains(&at) {
            return None;
        }
        let mut path = vec![self];
        if let Some(c) = self.children.iter().find_map(|c| c.find(at)) {
            path.extend(c);
        }
        Some(path)
    }
}

/// Cuts fields out of a frame, never reading past its end.
struct Cursor<'a> {
    b: &'a [u8],
}

impl Cursor<'_> {
    fn get(&self, r: Range<usize>) -> Option<&[u8]> {
        self.b.get(r)
    }

    fn u8(&self, at: usize) -> Option<u8> {
        self.b.get(at).copied()
    }

    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes(self.get(at..at + 2)?.try_into().ok()?))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes(self.get(at..at + 4)?.try_into().ok()?))
    }

    fn mac(&self, at: usize) -> Option<MacAddr> {
        Some(MacAddr(self.get(at..at + 6)?.try_into().ok()?))
    }

    fn ip(&self, at: usize) -> Option<Ipv4Addr> {
        Some(Ipv4Addr(self.get(at..at + 4)?.try_into().ok()?))
    }

    /// A field with a label made from its value, or nothing if the frame ends first.
    fn field(
        &self,
        name: &str,
        at: usize,
        len: usize,
        show: impl FnOnce(&Self) -> Option<String>,
    ) -> Option<Node> {
        self.get(at..at + len)?;
        Some(Node::new(format!("{name}: {}", show(self)?), at..at + len))
    }

    fn mac_field(&self, name: &str, at: usize) -> Option<Node> {
        self.field(name, at, 6, |c| Some(c.mac(at)?.to_string()))
    }

    fn ip_field(&self, name: &str, at: usize) -> Option<Node> {
        self.field(name, at, 4, |c| Some(c.ip(at)?.to_string()))
    }

    fn u8_field(&self, name: &str, at: usize, show: impl FnOnce(u8) -> String) -> Option<Node> {
        self.field(name, at, 1, |c| Some(show(c.u8(at)?)))
    }

    fn u16_field(&self, name: &str, at: usize, show: impl FnOnce(u16) -> String) -> Option<Node> {
        self.field(name, at, 2, |c| Some(show(c.u16(at)?)))
    }

    fn u32_field(&self, name: &str, at: usize, show: impl FnOnce(u32) -> String) -> Option<Node> {
        self.field(name, at, 4, |c| Some(show(c.u32(at)?)))
    }
}

/// Fields in order, stopping at the first that does not fit; if one did not, a final line says so.
fn collect(fields: Vec<Option<Node>>, end: usize) -> Vec<Node> {
    let mut out = vec![];
    for f in fields {
        match f {
            Some(n) => out.push(n),
            None => {
                out.push(Node::new("(frame ends here)", end..end));
                break;
            }
        }
    }
    out
}

/// The layers of `frame`, outermost first: Ethernet, then whatever it carries.
pub fn dissect(frame: &[u8]) -> Vec<Node> {
    let mut layers = dissect_layers(frame);
    // A header can claim more than the frame holds; whatever it says, no range leaves the frame.
    fn clamp(n: &mut Node, len: usize) {
        n.range.start = n.range.start.min(len);
        n.range.end = n.range.end.min(len).max(n.range.start);
        n.children.iter_mut().for_each(|c| clamp(c, len));
    }
    layers.iter_mut().for_each(|l| clamp(l, frame.len()));
    layers
}

fn dissect_layers(frame: &[u8]) -> Vec<Node> {
    let c = Cursor { b: frame };
    let mut layers = vec![];
    let (eth, ethertype, mut at) = ethernet(&c);
    layers.push(eth);
    let Some(et) = ethertype else {
        return layers;
    };
    if et < 0x0600 {
        layers.extend(llc(&c, at, et as usize));
        return layers;
    }
    match et {
        0x0806 => layers.push(arp(&c, at)),
        0x0800 => {
            let (ip, proto, payload_at, end) = ipv4(&c, at);
            layers.push(ip);
            at = payload_at;
            let c = Cursor {
                b: &frame[..end.min(frame.len())],
            };
            match proto {
                Some(1) => layers.push(icmp(&c, at)),
                Some(17) => {
                    let (udp, ports) = udp(&c, at);
                    layers.push(udp);
                    if matches!(ports, Some((67 | 68, 67 | 68))) {
                        layers.push(dhcp(&c, at + 8));
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    layers
}

/// The layer names, lower case, for filtering: `ethernet`, `arp`, `ip`, `icmp`, `udp`, `dhcp`,
/// `stp`, `dtp`, `vlan` (when tagged).
pub fn protocols(layers: &[Node]) -> Vec<&'static str> {
    let mut out = vec![];
    for l in layers {
        let name = l.label.split([',', ' ']).next().unwrap_or("");
        out.push(match name {
            "Ethernet" => "ethernet",
            "Address" => "arp",
            "Internet" => "ip",
            "ICMP" => "icmp",
            "UDP" => "udp",
            "DHCP" => "dhcp",
            "Spanning" => "stp",
            "Dynamic" => "dtp",
            _ => continue,
        });
        if l.children.iter().any(|c| c.label.starts_with("802.1Q")) {
            out.push("vlan");
        }
    }
    out
}

fn ethernet(c: &Cursor) -> (Node, Option<u16>, usize) {
    let mut kids = vec![c.mac_field("Destination", 0), c.mac_field("Source", 6)];
    let mut at = 12;
    let mut et = c.u16(12);
    if et == Some(0x8100) {
        let tci = c.u16(14);
        kids.push(
            c.field("802.1Q tag", 12, 4, |_| {
                let t = tci?;
                Some(format!("priority {}, VLAN {}", t >> 13, t & 0x0fff))
            })
            .map(|n| {
                let t = tci.unwrap_or(0);
                n.with(vec![
                    Node::new(format!("Priority: {}", t >> 13), 14..15),
                    Node::new(format!("Drop eligible: {}", (t >> 12) & 1), 14..15),
                    Node::new(format!("VLAN: {}", t & 0x0fff), 14..16),
                ])
            }),
        );
        et = c.u16(16);
        at = 16;
    }
    kids.push(c.u16_field(
        if et.is_some_and(|e| e < 0x0600) {
            "Length"
        } else {
            "Type"
        },
        at,
        |e| {
            if e < 0x0600 {
                e.to_string()
            } else {
                match e {
                    0x0800 => "IPv4 (0x0800)".into(),
                    0x0806 => "ARP (0x0806)".into(),
                    e => format!("{e:#06x}"),
                }
            }
        },
    ));
    let src = c.mac(6).map(|m| m.to_string()).unwrap_or_default();
    let dst = c.mac(0).map(|m| m.to_string()).unwrap_or_default();
    let end = at + 2;
    let node = Node::new(
        format!("Ethernet, Src: {src}, Dst: {dst}"),
        0..end.min(c.b.len()),
    )
    .with(collect(kids, c.b.len()));
    (node, et, end)
}

fn arp(c: &Cursor, at: usize) -> Node {
    let op = c.u16(at + 6);
    let kids = vec![
        c.u16_field("Hardware type", at, |h| {
            if h == 1 {
                "Ethernet (1)".into()
            } else {
                h.to_string()
            }
        }),
        c.u16_field("Protocol type", at + 2, |p| {
            if p == 0x0800 {
                "IPv4 (0x0800)".into()
            } else {
                format!("{p:#06x}")
            }
        }),
        c.u8_field("Hardware size", at + 4, |v| v.to_string()),
        c.u8_field("Protocol size", at + 5, |v| v.to_string()),
        c.u16_field("Opcode", at + 6, |o| match o {
            1 => "request (1)".into(),
            2 => "reply (2)".into(),
            o => o.to_string(),
        }),
        c.mac_field("Sender MAC", at + 8),
        c.ip_field("Sender IP", at + 14),
        c.mac_field("Target MAC", at + 18),
        c.ip_field("Target IP", at + 24),
    ];
    let what = match op {
        Some(1) => "request",
        Some(2) => "reply",
        _ => "message",
    };
    Node::new(
        format!("Address Resolution Protocol ({what})"),
        at..(at + 28).min(c.b.len()),
    )
    .with(collect(kids, c.b.len()))
}

/// Returns the node, the protocol number, where the payload starts, and where the packet ends
/// (the IP total length, bounded by the frame).
fn ipv4(c: &Cursor, at: usize) -> (Node, Option<u8>, usize, usize) {
    let vihl = c.u8(at);
    let ihl = vihl.map_or(20, |v| usize::from(v & 0x0f) * 4).max(20);
    let total = c.u16(at + 2).map_or(c.b.len(), |t| at + usize::from(t));
    let proto = c.u8(at + 9);
    let kids = vec![
        c.u8_field("Version", at, |v| (v >> 4).to_string()),
        c.u8_field("Header length", at, |v| format!("{} bytes", (v & 0x0f) * 4)),
        c.u8_field("Type of service", at + 1, |v| format!("{v:#04x}")),
        c.u16_field("Total length", at + 2, |v| v.to_string()),
        c.u16_field("Identification", at + 4, |v| format!("{v:#06x}")),
        c.u16_field("Flags and fragment offset", at + 6, |v| {
            format!(
                "{}{} offset {}",
                if v & 0x4000 != 0 {
                    "don't fragment "
                } else {
                    ""
                },
                if v & 0x2000 != 0 {
                    "more fragments"
                } else {
                    ""
                },
                v & 0x1fff
            )
            .replace("  ", " ")
        }),
        c.u8_field("Time to live", at + 8, |v| v.to_string()),
        c.u8_field("Protocol", at + 9, |v| match v {
            1 => "ICMP (1)".into(),
            17 => "UDP (17)".into(),
            v => v.to_string(),
        }),
        c.u16_field("Header checksum", at + 10, |v| format!("{v:#06x}")),
        c.ip_field("Source", at + 12),
        c.ip_field("Destination", at + 16),
    ];
    let src = c.ip(at + 12).map(|i| i.to_string()).unwrap_or_default();
    let dst = c.ip(at + 16).map(|i| i.to_string()).unwrap_or_default();
    let node = Node::new(
        format!("Internet Protocol, Src: {src}, Dst: {dst}"),
        at..(at + ihl).min(c.b.len()),
    )
    .with(collect(kids, c.b.len()));
    (node, proto, at + ihl, total.min(c.b.len()).max(at))
}

fn icmp(c: &Cursor, at: usize) -> Node {
    let ty = c.u8(at);
    let code = c.u8(at + 1);
    let name = match (ty, code) {
        (Some(0), _) => "echo reply",
        (Some(8), _) => "echo request",
        (Some(3), Some(0)) => "network unreachable",
        (Some(3), Some(1)) => "host unreachable",
        (Some(3), Some(2)) => "protocol unreachable",
        (Some(3), Some(3)) => "port unreachable",
        (Some(3), _) => "destination unreachable",
        (Some(11), _) => "time exceeded",
        _ => "message",
    };
    let mut kids = vec![
        c.u8_field("Type", at, |v| v.to_string()),
        c.u8_field("Code", at + 1, |v| v.to_string()),
        c.u16_field("Checksum", at + 2, |v| format!("{v:#06x}")),
    ];
    if matches!(ty, Some(0 | 8)) {
        kids.push(c.u16_field("Identifier", at + 4, |v| v.to_string()));
        kids.push(c.u16_field("Sequence", at + 6, |v| v.to_string()));
    }
    let mut kids = collect(kids, c.b.len());
    if matches!(ty, Some(3 | 11)) && c.b.len() > at + 8 {
        kids.push(Node::new(
            "Original datagram (header and first bytes)",
            at + 8..c.b.len(),
        ));
    } else if matches!(ty, Some(0 | 8)) && c.b.len() > at + 8 {
        kids.push(Node::new("Data", at + 8..c.b.len()));
    }
    Node::new(format!("ICMP {name}"), at..c.b.len()).with(kids)
}

fn udp(c: &Cursor, at: usize) -> (Node, Option<(u16, u16)>) {
    let (s, d) = (c.u16(at), c.u16(at + 2));
    let kids = vec![
        c.u16_field("Source port", at, |v| v.to_string()),
        c.u16_field("Destination port", at + 2, |v| v.to_string()),
        c.u16_field("Length", at + 4, |v| v.to_string()),
        c.u16_field("Checksum", at + 6, |v| format!("{v:#06x}")),
    ];
    let label = match (s, d) {
        (Some(s), Some(d)) => format!("UDP, Src Port: {s}, Dst Port: {d}"),
        _ => "UDP".to_string(),
    };
    (
        Node::new(label, at..(at + 8).min(c.b.len())).with(collect(kids, c.b.len())),
        s.zip(d),
    )
}

fn dhcp(c: &Cursor, at: usize) -> Node {
    let kids = vec![
        c.u8_field("Message type", at, |v| match v {
            1 => "request (1)".into(),
            2 => "reply (2)".into(),
            v => v.to_string(),
        }),
        c.u8_field("Hardware type", at + 1, |v| v.to_string()),
        c.u8_field("Hardware length", at + 2, |v| v.to_string()),
        c.u8_field("Hops", at + 3, |v| v.to_string()),
        c.u32_field("Transaction id", at + 4, |v| format!("{v:#010x}")),
        c.u16_field("Seconds", at + 8, |v| v.to_string()),
        c.u16_field("Flags", at + 10, |v| {
            format!(
                "{v:#06x}{}",
                if v & 0x8000 != 0 { " (broadcast)" } else { "" }
            )
        }),
        c.ip_field("Client address", at + 12),
        c.ip_field("Your address", at + 16),
        c.ip_field("Server address", at + 20),
        c.ip_field("Relay address", at + 24),
        c.mac_field("Client hardware address", at + 28),
    ];
    let have_all = kids.iter().all(Option::is_some);
    let mut kids = collect(kids, c.b.len());
    if have_all {
        kids.extend(dhcp_options(c, at + 236));
    }
    let kind = dhcp_type(c, at + 240).unwrap_or("message");
    Node::new(format!("DHCP {kind}"), at..c.b.len()).with(kids)
}

fn dhcp_type(c: &Cursor, mut at: usize) -> Option<&'static str> {
    while let Some(code) = c.u8(at) {
        match code {
            0 => at += 1,
            255 => return None,
            code => {
                let len = usize::from(c.u8(at + 1)?);
                if code == 53 {
                    return Some(match c.u8(at + 2)? {
                        1 => "discover",
                        2 => "offer",
                        3 => "request",
                        4 => "decline",
                        5 => "ack",
                        6 => "nak",
                        7 => "release",
                        8 => "inform",
                        _ => "message",
                    });
                }
                at += 2 + len;
            }
        }
    }
    None
}

fn dhcp_options(c: &Cursor, cookie: usize) -> Vec<Node> {
    let Some(magic) = c.get(cookie..cookie + 4) else {
        return vec![];
    };
    let mut out = vec![Node::new(
        if magic == [99, 130, 83, 99] {
            "Magic cookie: DHCP".to_string()
        } else {
            "Magic cookie: wrong".to_string()
        },
        cookie..cookie + 4,
    )];
    let mut at = cookie + 4;
    while let Some(code) = c.u8(at) {
        if code == 0 {
            at += 1;
            continue;
        }
        if code == 255 {
            out.push(Node::new("Option 255: end", at..at + 1));
            break;
        }
        let Some(len) = c.u8(at + 1).map(usize::from) else {
            break;
        };
        let Some(v) = c.get(at + 2..at + 2 + len) else {
            out.push(Node::new(
                format!("Option {code}: truncated"),
                at..c.b.len(),
            ));
            break;
        };
        let text = match (code, v) {
            (1, v) if v.len() == 4 => format!("Subnet mask: {}", Ipv4Addr(v.try_into().unwrap())),
            (3, v) if v.len() >= 4 => format!("Router: {}", Ipv4Addr(v[..4].try_into().unwrap())),
            (6, v) if v.len() >= 4 => {
                format!("DNS server: {}", Ipv4Addr(v[..4].try_into().unwrap()))
            }
            (50, v) if v.len() == 4 => {
                format!("Requested address: {}", Ipv4Addr(v.try_into().unwrap()))
            }
            (51, v) if v.len() == 4 => {
                format!(
                    "Lease time: {} s",
                    u32::from_be_bytes(v.try_into().unwrap())
                )
            }
            (53, [t]) => format!("Message type: {}", dhcp_type_name(*t)),
            (54, v) if v.len() == 4 => {
                format!("Server identifier: {}", Ipv4Addr(v.try_into().unwrap()))
            }
            (58, v) if v.len() == 4 => {
                format!(
                    "Renewal time: {} s",
                    u32::from_be_bytes(v.try_into().unwrap())
                )
            }
            (59, v) if v.len() == 4 => {
                format!(
                    "Rebinding time: {} s",
                    u32::from_be_bytes(v.try_into().unwrap())
                )
            }
            (c, _) => format!("Option {c} ({len} bytes)"),
        };
        out.push(Node::new(text, at..at + 2 + len));
        at += 2 + len;
    }
    out
}

fn dhcp_type_name(t: u8) -> &'static str {
    match t {
        1 => "discover",
        2 => "offer",
        3 => "request",
        4 => "decline",
        5 => "ack",
        6 => "nak",
        7 => "release",
        8 => "inform",
        _ => "unknown",
    }
}

/// An LLC frame: a spanning tree BPDU or a DTP message here.
fn llc(c: &Cursor, at: usize, len: usize) -> Vec<Node> {
    let mut out = vec![
        Node::new("Logical Link Control", at..(at + 3).min(c.b.len())).with(collect(
            vec![
                c.u8_field("DSAP", at, |v| format!("{v:#04x}")),
                c.u8_field("SSAP", at + 1, |v| format!("{v:#04x}")),
                c.u8_field("Control", at + 2, |v| format!("{v:#04x}")),
            ],
            c.b.len(),
        )),
    ];
    let _ = len;
    if c.get(at..at + 3) == Some(&[0x42, 0x42, 0x03]) {
        out.push(bpdu(c, at + 3));
    } else if c.get(at..at + 8) == Some(&[0xaa, 0xaa, 0x03, 0, 0, 0x0c, 0x20, 0x04]) {
        out.push(dtp(c, at + 8));
    }
    out
}

fn bridge_id(c: &Cursor, name: &str, at: usize) -> Option<Node> {
    c.field(name, at, 8, |c| {
        Some(format!("{} / {}", c.u16(at)?, c.mac(at + 2)?))
    })
}

fn bpdu(c: &Cursor, at: usize) -> Node {
    let ty = c.u8(at + 3);
    if ty == Some(0x80) {
        return Node::new(
            "Spanning Tree Protocol (topology change notification)",
            at..c.b.len(),
        )
        .with(collect(
            vec![
                c.u16_field("Protocol id", at, |v| v.to_string()),
                c.u8_field("Version", at + 2, |v| v.to_string()),
                c.u8_field("Type", at + 3, |_| "topology change notification".into()),
            ],
            c.b.len(),
        ));
    }
    let rapid = ty == Some(2);
    let flag_text = |f: u8| {
        let mut names = vec![];
        for (bit, n) in [
            (0x01, "topology change"),
            (0x02, "proposal"),
            (0x10, "learning"),
            (0x20, "forwarding"),
            (0x40, "agreement"),
            (0x80, "topology change ack"),
        ] {
            if f & bit != 0 && (rapid || bit & 0x81 != 0) {
                names.push(n);
            }
        }
        if rapid {
            names.push(match (f >> 2) & 3 {
                1 => "role alternate/backup",
                2 => "role root",
                3 => "role designated",
                _ => "role unknown",
            });
        }
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        }
    };
    let secs = |v: u16| format!("{} s", f64::from(v) / 256.0);
    let kids = vec![
        c.u16_field("Protocol id", at, |v| v.to_string()),
        c.u8_field("Version", at + 2, |v| v.to_string()),
        c.u8_field("Type", at + 3, |v| {
            if v == 2 {
                "rapid (2)".into()
            } else {
                "configuration (0)".into()
            }
        }),
        c.u8_field("Flags", at + 4, flag_text),
        bridge_id(c, "Root bridge", at + 5),
        c.u32_field("Root path cost", at + 13, |v| v.to_string()),
        bridge_id(c, "Bridge", at + 17),
        c.u16_field("Port id", at + 25, |v| format!("{}.{}", v >> 8, v & 0xff)),
        c.u16_field("Message age", at + 27, secs),
        c.u16_field("Max age", at + 29, secs),
        c.u16_field("Hello time", at + 31, secs),
        c.u16_field("Forward delay", at + 33, secs),
    ];
    Node::new(
        format!(
            "Spanning Tree Protocol ({})",
            if rapid { "rapid" } else { "classic" }
        ),
        at..c.b.len(),
    )
    .with(collect(kids, c.b.len()))
}

fn dtp(c: &Cursor, at: usize) -> Node {
    let mut kids: Vec<Node> = c
        .u8_field("Version", at, |v| v.to_string())
        .into_iter()
        .collect();
    let mut p = at + 1;
    while let (Some(ty), Some(len)) = (c.u16(p), c.u16(p + 2)) {
        let len = usize::from(len);
        if len < 4 || p + len > c.b.len() {
            break;
        }
        let v = &c.b[p + 4..p + len];
        let text = match (ty, v) {
            (1, d) => format!("Domain: {:?}", String::from_utf8_lossy(d)),
            (2, [s]) => {
                let m = match s & 0x0f {
                    1 => Some(DtpMode::On),
                    2 => Some(DtpMode::Off),
                    3 => Some(DtpMode::Desirable),
                    4 => Some(DtpMode::Auto),
                    _ => None,
                };
                format!(
                    "Trunk status: {}{}",
                    match m {
                        Some(DtpMode::On) => "on",
                        Some(DtpMode::Off) => "off",
                        Some(DtpMode::Desirable) => "desirable",
                        Some(DtpMode::Auto) => "auto",
                        None => "unknown",
                    },
                    if s & 0x80 != 0 { ", trunking" } else { "" }
                )
            }
            (3, [t]) => format!(
                "Trunk type: {}",
                if *t == 0xa5 { "802.1Q" } else { "other" }
            ),
            (4, m) if m.len() == 6 => {
                format!("Neighbour: {}", MacAddr(m.try_into().unwrap()))
            }
            (t, _) => format!("TLV {t}"),
        };
        kids.push(Node::new(text, p..p + len));
        p += len;
    }
    Node::new("Dynamic Trunking Protocol", at..c.b.len()).with(kids)
}

/// A classic hex dump: offset, up to sixteen bytes as hex, and the printable characters.
pub fn hexdump(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, row)| {
            let hex: Vec<String> = (0..16)
                .map(|j| row.get(j).map_or("  ".to_string(), |b| format!("{b:02x}")))
                .collect();
            let text: String = row
                .iter()
                .map(|b| {
                    if (0x20..0x7f).contains(b) {
                        *b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!(
                "{:04x}  {}  {}  {text}",
                i * 16,
                hex[..8].join(" "),
                hex[8..].join(" ")
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArpOp, ArpPacket, Bpdu, BpduKind, BpduRole, BridgeId, DhcpMessage, DhcpType, Dtp,
        EtherType, EthernetFrame, IcmpEcho, IpProto, Ipv4Packet, UdpDatagram, DTP_MULTICAST,
        PVST_MULTICAST,
    };

    fn eth(dst: MacAddr, et: EtherType, payload: Vec<u8>) -> Vec<u8> {
        EthernetFrame {
            dst,
            src: MacAddr([2, 0, 0, 0, 0, 1]),
            vlan: None,
            ethertype: et,
            payload,
        }
        .emit()
    }

    fn ip(proto: IpProto, payload: Vec<u8>) -> Vec<u8> {
        Ipv4Packet {
            dscp_ecn: 0,
            ident: 0,
            flags_frag: 0x4000,
            ttl: 64,
            proto,
            src: Ipv4Addr::new(10, 0, 0, 1),
            dst: Ipv4Addr::new(10, 0, 0, 2),
            payload,
        }
        .emit()
    }

    /// Every node's range must lie inside the frame, and children inside their parent.
    fn check(frame: &[u8], nodes: &[Node]) {
        fn walk(n: &Node, len: usize) {
            assert!(
                n.range.start <= n.range.end && n.range.end <= len,
                "{n:?} in {len}"
            );
            for c in &n.children {
                assert!(
                    c.range.start >= n.range.start && c.range.end <= n.range.end,
                    "{c:?} outside {n:?}"
                );
                walk(c, len);
            }
        }
        for n in nodes {
            walk(n, frame.len());
        }
    }

    fn labels(nodes: &[Node]) -> Vec<String> {
        fn flat(n: &Node, out: &mut Vec<String>) {
            out.push(n.label.clone());
            for c in &n.children {
                flat(c, out);
            }
        }
        let mut out = vec![];
        for n in nodes {
            flat(n, &mut out);
        }
        out
    }

    #[test]
    fn an_arp_request_shows_every_field_at_its_offset() {
        let a = ArpPacket {
            op: ArpOp::Request,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::default(),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let f = eth(MacAddr::BROADCAST, EtherType::ARP, a.emit());
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "arp"]);
        let all = labels(&l);
        assert!(all.contains(&"Opcode: request (1)".to_string()), "{all:?}");
        assert!(all.contains(&"Target IP: 10.0.0.2".to_string()));
        // The target IP is the last four bytes of the frame.
        let t = l[1]
            .children
            .iter()
            .find(|n| n.label.starts_with("Target IP"))
            .unwrap();
        assert_eq!(t.range, f.len() - 4..f.len());
    }

    #[test]
    fn a_ping_has_ethernet_ip_and_icmp_layers() {
        let e = IcmpEcho {
            reply: false,
            ident: 1,
            seq: 7,
            data: vec![1, 2, 3],
        };
        let f = eth(
            MacAddr([2, 0, 0, 0, 0, 2]),
            EtherType::IPV4,
            ip(IpProto::ICMP, e.emit()),
        );
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "ip", "icmp"]);
        let all = labels(&l);
        assert!(all.contains(&"Time to live: 64".to_string()));
        assert!(all.contains(&"Sequence: 7".to_string()), "{all:?}");
        assert_eq!(l[2].label, "ICMP echo request");
    }

    #[test]
    fn a_dhcp_discover_lists_its_options() {
        let d = DhcpMessage {
            xid: 0xabcd,
            msg_type: Some(DhcpType::Discover),
            ..DhcpMessage::default()
        };
        let u = UdpDatagram {
            src_port: 68,
            dst_port: 67,
            payload: d.emit(),
        }
        .emit(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let f = eth(MacAddr::BROADCAST, EtherType::IPV4, ip(IpProto::UDP, u));
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "ip", "udp", "dhcp"]);
        let all = labels(&l);
        assert!(
            all.contains(&"Message type: discover".to_string()),
            "{all:?}"
        );
        assert_eq!(l[3].label, "DHCP discover");
        assert!(all.contains(&"Transaction id: 0x0000abcd".to_string()));
    }

    #[test]
    fn a_tagged_frame_shows_the_tag_and_the_vlan() {
        let mut f = eth(
            MacAddr::BROADCAST,
            EtherType::ARP,
            ArpPacket {
                op: ArpOp::Reply,
                sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
                sender_ip: Ipv4Addr::new(10, 0, 0, 1),
                target_mac: MacAddr([2, 0, 0, 0, 0, 2]),
                target_ip: Ipv4Addr::new(10, 0, 0, 2),
            }
            .emit(),
        );
        f.splice(12..12, [0x81, 0x00, 0x20, 0x0a]);
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "vlan", "arp"]);
        assert!(labels(&l).contains(&"VLAN: 10".to_string()));
        assert!(labels(&l).contains(&"Priority: 1".to_string()));
    }

    #[test]
    fn bpdus_and_dtp_are_decoded() {
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
        let p = b.emit();
        let f = eth(PVST_MULTICAST, EtherType(p.len() as u16), p);
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "stp"]);
        let all = labels(&l);
        assert!(
            all.contains(&"Flags: proposal, role designated".to_string()),
            "{all:?}"
        );
        assert!(all.contains(&"Root bridge: 4097 / 02bb.ff00.0000".to_string()));
        assert!(all.contains(&"Hello time: 2 s".to_string()));

        let d = Dtp {
            mode: DtpMode::Desirable,
            trunking: false,
            domain: vec![],
            sender: MacAddr([2, 0xbb, 0xff, 1, 0, 1]),
        };
        let p = d.emit();
        let f = eth(DTP_MULTICAST, EtherType(p.len() as u16), p);
        let l = dissect(&f);
        check(&f, &l);
        assert_eq!(protocols(&l), ["ethernet", "dtp"]);
        assert!(labels(&l).contains(&"Trunk status: desirable".to_string()));
    }

    #[test]
    fn a_header_that_claims_more_than_the_frame_holds_stays_inside_it() {
        // Found by the fuzzer: an IPv4 header with a zero length nibble, then a frame that ends
        // before the payload it points to.
        let frame = [
            0x41, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x08, 0x00, 0x00, 0x0a, 0x90, 0x90, 0x90, 0x00,
            0x90, 0x90, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0x03,
        ];
        let l = dissect(&frame);
        check(&frame, &l);
    }

    #[test]
    fn a_truncated_frame_is_cut_short_not_refused() {
        let a = ArpPacket {
            op: ArpOp::Request,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::default(),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let full = eth(MacAddr::BROADCAST, EtherType::ARP, a.emit());
        for cut in 0..full.len() {
            let l = dissect(&full[..cut]);
            check(&full[..cut], &l);
            assert!(!l.is_empty());
        }
    }

    #[test]
    fn find_walks_down_to_the_field_holding_a_byte() {
        let a = ArpPacket {
            op: ArpOp::Request,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::default(),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let f = eth(MacAddr::BROADCAST, EtherType::ARP, a.emit());
        let l = dissect(&f);
        let path = l[1].find(f.len() - 1).unwrap();
        assert_eq!(path.last().unwrap().label, "Target IP: 10.0.0.2");
        assert!(l[0].find(f.len() - 1).is_none());
    }

    #[test]
    fn the_hex_dump_has_offsets_hex_and_text() {
        let rows = hexdump(b"hello, world!!!!abc");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            "0000  68 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 21 21 21  hello, world!!!!"
        );
        assert!(rows[1].starts_with("0010  61 62 63 "), "{}", rows[1]);
        assert!(rows[1].ends_with("  abc"));
        assert!(hexdump(&[]).is_empty());
    }

    proptest::proptest! {
        /// Whatever the bytes, no panic and every range inside the frame.
        #[test]
        fn arbitrary_bytes_never_break_the_dissector(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200)) {
            let l = dissect(&bytes);
            check(&bytes, &l);
            let _ = protocols(&l);
        }
    }
}
