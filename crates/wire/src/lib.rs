//! Wire formats. Pure data, no I/O, no clocks, no randomness.

mod addr;
mod arp;
mod checksum;
mod ethernet;
mod icmp;
mod ipv4;

pub use addr::{Ipv4Addr, MacAddr};
pub use arp::{ArpOp, ArpPacket};
pub use checksum::internet_checksum;
pub use ethernet::{EtherType, EthernetFrame, VlanTag};
pub use icmp::IcmpEcho;
pub use ipv4::{IpProto, Ipv4Packet};

/// Parse failure. Parsers never panic on arbitrary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadLength,
    BadChecksum,
    Unsupported,
}
