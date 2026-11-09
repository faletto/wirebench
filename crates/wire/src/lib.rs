//! Wire formats. Pure data, no I/O, no clocks, no randomness.

mod addr;
mod arp;
mod bpdu;
mod checksum;
mod dhcp;
mod discovery;
mod dissect;
mod dtp;
mod ethernet;
mod fhrp;
mod icmp;
mod ipv4;
mod lacp;
mod ntp;
mod ospf;
mod summary;
mod tcp;
mod udp;

pub use addr::{Ipv4Addr, MacAddr};
pub use arp::{ArpOp, ArpPacket};
pub use bpdu::{Bpdu, BpduKind, BpduRole, BridgeId, PVST_MULTICAST};
pub use checksum::internet_checksum;
pub use dhcp::{DhcpMessage, DhcpOp, DhcpType};
pub use discovery::{
    cap as cdp_cap, lldp_cap, Cdp, Lldp, CDP_MULTICAST, LLDP_MULTICAST, LLDP_TYPE,
};
pub use dissect::{dissect, hexdump, protocols, Node};
pub use dtp::{Dtp, DtpMode, DTP_MULTICAST};
pub use ethernet::{EtherType, EthernetFrame, VlanTag};
pub use fhrp::{
    hsrp_state, hsrp_vmac, vrrp_vmac, Hsrp, HsrpOp, Vrrp, HSRP_GROUP, HSRP_MAC, HSRP_PORT,
    VRRP_GROUP, VRRP_MAC, VRRP_PROTO,
};
pub use icmp::{IcmpEcho, IcmpError, IcmpErrorKind};
pub use ipv4::{IpProto, Ipv4Packet};
pub use lacp::{
    LacpInfo, Lacpdu, SLOW_PROTOCOLS, SLOW_PROTOCOLS_TYPE, STATE_ACTIVITY, STATE_AGGREGATION,
    STATE_COLLECTING, STATE_DEFAULTED, STATE_DISTRIBUTING, STATE_EXPIRED, STATE_SYNC,
    STATE_TIMEOUT,
};
pub use ntp::{
    from_timestamp as ntp_from_timestamp, to_timestamp as ntp_to_timestamp, NtpPacket, Timestamp,
    MODE_CLIENT as NTP_MODE_CLIENT, MODE_SERVER as NTP_MODE_SERVER, NTP_PORT,
};
pub use ospf::{
    Dbd, Hello, LsRequest, Lsa, LsaBody, LsaHeader, OspfBody, OspfPacket, RouterLink,
    ALL_DR_ROUTERS, ALL_SPF_ROUTERS, DBD_INIT, DBD_MASTER, DBD_MORE, INITIAL_SEQ, LINK_P2P,
    LINK_STUB, LINK_TRANSIT, LSA_ASBR_SUMMARY, LSA_EXTERNAL, LSA_NETWORK, LSA_ROUTER, LSA_SUMMARY,
    LS_INFINITY, MAX_AGE, MAX_SEQ, OPT_E, ROUTER_B, ROUTER_E,
};
pub use summary::summarize;
pub use tcp::TcpSegment;
pub use udp::UdpDatagram;

/// Parse failure. Parsers never panic on arbitrary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadLength,
    BadChecksum,
    Unsupported,
}
