//! Device model. Devices are plain state machines driven by [`DeviceEvent`]s and
//! acting on the world only through [`Ctx`].

mod arp;
mod device;
mod fib;
mod host;
mod router;
mod switch;

pub use arp::Resolver;
pub use device::{Ctx, Device, DeviceEvent};
pub use fib::{mask, Fib, Route, RouteKind};
pub use host::{Host, PingResult};
pub use router::{Interface, Router, StaticRoute};
pub use switch::L2Switch;
