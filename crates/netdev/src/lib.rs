//! Device model. Devices are plain state machines driven by [`DeviceEvent`]s and
//! acting on the world only through [`Ctx`].

mod arp;
mod device;
mod dhcp_server;
mod fib;
mod host;
mod mgmt;
mod router;
mod switch;
mod vlan;

pub use arp::Resolver;
pub use device::{Ctx, Device, DeviceEvent};
pub use dhcp_server::{client_key, Binding, BindingState, DhcpServer, Pool};
pub use fib::{mask, Fib, Route, RouteKind};
pub use host::{DhcpClient, DhcpState, Host, PingResult};
pub use mgmt::{
    banner_delimiter, check_secret, hash_secret, is_hashed, obscure, password_text, plain_password,
    reveal, stored_secret, Line, Mgmt,
};
pub use router::{EncapError, Interface, Outgoing, Router, StaticRoute, SubInterface};
pub use switch::{L2Switch, MacEntry, PortConfig, PortMode};
pub use vlan::{all_vlans, format_vlan_list, parse_vlan_list, DEFAULT_VLAN, MAX_VLAN};
