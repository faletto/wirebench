//! Device model. Devices are plain state machines driven by [`DeviceEvent`]s and
//! acting on the world only through [`Ctx`].

mod acl;
mod arp;
mod channel;
mod device;
mod dhcp_server;
mod discovery;
mod fhrp;
mod fib;
mod host;
mod mgmt;
mod nat;
mod ntp;
mod ospf;
mod probe;
mod psec;
mod router;
mod stp;
mod switch;
mod syslog;
mod tcp;
mod tftp;
mod vlan;

pub use acl::{
    config_text as acl_config_text, kind_of_number as acl_kind_of_number,
    parse_rule as parse_acl_rule, show as acl_show, Acl, AclKind, AclSet, AddError as AclAddError,
    AddrSpec, Body as AclBody, Entry as AclEntry, IcmpSpec, Packet as AclPacket, PortSpec,
    Rule as AclRule,
};
pub use arp::Resolver;
pub use channel::{
    bundle_cost, ChannelMode, ChannelPort, JoinError, LoadBalance, Membership, MAX_GROUP,
    MAX_MEMBERS,
};
pub use device::{Ctx, Device, DeviceEvent};
pub use dhcp_server::{client_key, Binding, BindingState, DhcpServer, Pool};
pub use discovery::{
    Discovery, Identity as DiscoveryIdentity, Neighbour, PortInfo as DiscoveryPort,
    DEFAULT_CDP_HOLD, DEFAULT_CDP_TIMER, DEFAULT_LLDP_HOLD, DEFAULT_LLDP_TIMER,
};
pub use fhrp::{
    Group as FhrpGroup, Kind as FhrpKind, Msg as FhrpMsg, Proto as FhrpProto, State as FhrpState,
    DEFAULT_PRIORITY as FHRP_DEFAULT_PRIORITY,
};
pub use fib::{mask, Fib, Route, RouteKind};
pub use host::{DhcpClient, DhcpState, Host, PingResult};
pub use mgmt::{
    banner_delimiter, check_secret, hash_secret, is_hashed, obscure, password_text, plain_password,
    reveal, stored_secret, Line, Mgmt, Transport, User, SSH_PORT, TELNET_PORT,
};
pub use nat::{
    DestRule as NatDestRule, Entry as NatEntry, Flow as NatFlow, Nat, Pool as NatPool,
    Rule as NatRule, Side as NatSide, Static as NatStatic, Stats as NatStats, Target as NatTarget,
    Timeouts as NatTimeouts,
};
pub use ntp::{
    civil_from_unix, unix_from_civil, Assoc as NtpAssoc, Clock, Ntp, SyncState as NtpSync,
    DEFAULT_MASTER_STRATUM, MONTHS, POLL_SECS as NTP_POLL_SECS,
};
pub use ospf::{
    DefaultInfo as OspfDefaultInfo, IfCfg as OspfIfCfg, IfState as OspfIfState,
    IfView as OspfIfView, NbrState as OspfNbrState, NetStmt as OspfNetStmt, Ospf, OspfIf,
    OspfRoute, PathKind as OspfPathKind, Source as OspfSource, Tx as OspfTx,
    DEFAULT_HELLO as OSPF_DEFAULT_HELLO, DEFAULT_REF_BW as OSPF_DEFAULT_REF_BW,
};
pub use probe::{record as record_probe, ProbeBody, ProbeReply, PROBE_LOG_LIMIT};
pub use psec::{
    parse_mac, PortSecurity, PsecState, Verdict as PsecVerdict, Violation as PsecViolation,
    DEFAULT_RECOVERY_SECS,
};
pub use router::{EncapError, FhrpEntry, Interface, Outgoing, Router, StaticRoute, SubInterface};
pub use stp::{
    port_id, Action as StpAction, Mode as StpMode, PortView, Role as StpRole, State as StpState,
    Stp, Vector,
};
pub use switch::{
    L2Switch, MacEntry, PortConfig, PortMode, SpanningConfig, DEFAULT_BRIDGE_PRIORITY,
};
pub use syslog::{
    level_name as log_level_name, parse_level as parse_log_level, Collector, LineSeen, Logging,
    Received as SyslogReceived, BUFFER_DEFAULT, DEBUGGING as LOG_DEBUGGING,
    DEFAULT_FACILITY as LOG_DEFAULT_FACILITY, ERRORS as LOG_ERRORS,
    INFORMATIONAL as LOG_INFORMATIONAL, NOTIFICATIONS as LOG_NOTIFICATIONS,
    WARNINGS as LOG_WARNINGS,
};
pub use tcp::{
    Conn as TcpConn, Failure as TcpFailure, State as TcpState, Tcp, Tx as TcpTx, UdpSocket,
    EPHEMERAL_START as TCP_EPHEMERAL_START, MAX_RETRIES as TCP_MAX_RETRIES,
};
pub use tftp::{Out as TftpOut, TftpServer, MAX_RETRIES as TFTP_MAX_RETRIES, RETRY as TFTP_RETRY};
pub use vlan::{all_vlans, format_vlan_list, parse_vlan_list, DEFAULT_VLAN, MAX_VLAN};
