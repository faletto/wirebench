//! Device model. Devices are plain state machines driven by [`DeviceEvent`]s and
//! acting on the world only through [`Ctx`].

mod arp;
mod device;
mod fib;
mod host;
mod router;
mod switch;
mod vlan;

pub use arp::Resolver;
pub use device::{Ctx, Device, DeviceEvent};
pub use fib::{mask, Fib, Route, RouteKind};
pub use host::{Host, PingResult};
pub use router::{EncapError, Interface, Outgoing, Router, StaticRoute, SubInterface};
pub use switch::{L2Switch, MacEntry, PortConfig, PortMode};
pub use vlan::{all_vlans, format_vlan_list, parse_vlan_list, DEFAULT_VLAN, MAX_VLAN};
