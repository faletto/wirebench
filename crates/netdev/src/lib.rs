//! Device model. Devices are plain state machines driven by [`DeviceEvent`]s and
//! acting on the world only through [`Ctx`].

mod device;
mod host;
mod switch;

pub use device::{Ctx, Device, DeviceEvent};
pub use host::{Host, PingResult};
pub use switch::L2Switch;
