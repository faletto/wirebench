//! Wire formats. Pure data, no I/O, no clocks, no randomness.

mod addr;
mod arp;
mod bpdu;
mod checksum;
mod dhcp;
mod discovery;
mod dissect;
mod dns;
mod dtp;
mod ethernet;
mod fhrp;
mod icmp;
mod ipv4;
mod lacp;
mod ntp;
mod ospf;
mod ssh;
mod summary;
mod syslog;
mod tcp;
mod telnet;
mod tftp;
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
pub use dns::{
    DnsMessage, Question as DnsQuestion, RData as DnsData, Record as DnsRecord,
    CLASS_IN as DNS_CLASS_IN, DNS_PORT, RCODE_FORMERR as DNS_FORMERR, RCODE_NOTIMP as DNS_NOTIMP,
    RCODE_NXDOMAIN as DNS_NXDOMAIN, RCODE_OK as DNS_OK, RCODE_REFUSED as DNS_REFUSED,
    RCODE_SERVFAIL as DNS_SERVFAIL, TYPE_A as DNS_TYPE_A, TYPE_CNAME as DNS_TYPE_CNAME,
};
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
pub use ssh::{
    ident as ssh_ident, parse_ident as ssh_parse_ident, Decoder as SshDecoder, Ident as SshIdent,
    Packet as SshPacket, MSG_CHANNEL_DATA as SSH_MSG_CHANNEL_DATA,
    MSG_DISCONNECT as SSH_MSG_DISCONNECT, MSG_USERAUTH_FAILURE as SSH_MSG_USERAUTH_FAILURE,
    MSG_USERAUTH_REQUEST as SSH_MSG_USERAUTH_REQUEST,
    MSG_USERAUTH_SUCCESS as SSH_MSG_USERAUTH_SUCCESS, SOFTWARE as SSH_SOFTWARE,
};
pub use summary::summarize;
pub use syslog::{
    SyslogMessage, MAX_LEN as SYSLOG_MAX_LEN, SEVERITIES as SYSLOG_SEVERITIES, SYSLOG_PORT,
};
pub use tcp::TcpSegment;
pub use telnet::{
    escape as telnet_escape, Decoder as TelnetDecoder, Negotiation as TelnetNegotiation,
    Options as TelnetOptions, DO as TELNET_DO, OPT_ECHO as TELNET_ECHO, OPT_SGA as TELNET_SGA,
};
pub use tftp::{
    Mode as TftpMode, TftpPacket, BLOCK_SIZE as TFTP_BLOCK_SIZE, ERR_ACCESS as TFTP_ERR_ACCESS,
    ERR_DISK_FULL as TFTP_ERR_DISK_FULL, ERR_EXISTS as TFTP_ERR_EXISTS,
    ERR_FILE_NOT_FOUND as TFTP_ERR_FILE_NOT_FOUND, ERR_ILLEGAL_OP as TFTP_ERR_ILLEGAL_OP,
    ERR_UNDEFINED as TFTP_ERR_UNDEFINED, ERR_UNKNOWN_ID as TFTP_ERR_UNKNOWN_ID, TFTP_PORT,
};
pub use udp::UdpDatagram;

/// Parse failure. Parsers never panic on arbitrary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadLength,
    BadChecksum,
    Unsupported,
}
