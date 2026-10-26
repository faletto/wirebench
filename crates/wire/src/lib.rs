//! Wire formats. Pure data, no I/O, no clocks, no randomness.

mod addr;
mod arp;
mod checksum;
mod dhcp;
mod ethernet;
mod icmp;
mod ipv4;
mod summary;
mod udp;

pub use addr::{Ipv4Addr, MacAddr};
pub use arp::{ArpOp, ArpPacket};
pub use checksum::internet_checksum;
pub use dhcp::{DhcpMessage, DhcpOp, DhcpType};
pub use ethernet::{EtherType, EthernetFrame, VlanTag};
pub use icmp::{IcmpEcho, IcmpError, IcmpErrorKind};
pub use ipv4::{IpProto, Ipv4Packet};
pub use summary::summarize;
pub use udp::UdpDatagram;

/// Parse failure. Parsers never panic on arbitrary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadLength,
    BadChecksum,
    Unsupported,
}
