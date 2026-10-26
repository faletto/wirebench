//! The world: kernel plus devices, and the dispatch loop between them.

pub mod chain;
pub mod lab;
pub mod protocol;
pub mod scenarios;
pub mod script;
pub mod shell;
mod trace;

use kernel::{DeviceId, Event, Kernel, LinkConfig, LinkId, PortId, SimTime, Trace};
use netdev::{Ctx, Device, DeviceEvent};

/// One event as seen by the stepper, for simulation mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivered {
    Frame {
        at: SimTime,
        device: DeviceId,
        port: PortId,
        frame: Vec<u8>,
        /// Sender and departure time, so a UI can animate the hop.
        from: DeviceId,
        sent: SimTime,
    },
    Timer {
        at: SimTime,
        device: DeviceId,
    },
}

/// How far ahead the simulation looks to decide it is idle. Anything scheduled later (a DHCP lease
/// renewal) stays queued without keeping `run_to_idle` or Play going.
pub const IDLE_HORIZON: SimTime = SimTime::from_secs(30);

pub struct World {
    pub kernel: Kernel,
    devices: Vec<Box<dyn Device>>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("devices", &self.devices.len())
            .finish()
    }
}

impl World {
    pub fn new(trace: Trace) -> Self {
        World {
            kernel: Kernel::new(trace),
            devices: Vec::new(),
        }
    }

    pub fn add_device(&mut self, d: impl Device) -> DeviceId {
        self.devices.push(Box::new(d));
        DeviceId(self.devices.len() as u32 - 1)
    }

    pub fn connect(&mut self, a: (DeviceId, u16), b: (DeviceId, u16), cfg: LinkConfig) -> LinkId {
        self.kernel
            .add_link((a.0, PortId(a.1)), (b.0, PortId(b.1)), cfg)
    }

    /// Run `f` on device `id` (downcast to `T`) with a context, e.g. to start a ping.
    pub fn with_device<T: Device, R>(
        &mut self,
        id: DeviceId,
        f: impl FnOnce(&mut T, &mut Ctx) -> R,
    ) -> R {
        let dev = self.devices[id.0 as usize]
            .as_any_mut()
            .downcast_mut::<T>()
            .expect("device type mismatch");
        let mut ctx = Ctx::new(&mut self.kernel, id);
        f(dev, &mut ctx)
    }

    pub fn device<T: Device>(&self, id: DeviceId) -> &T {
        self.devices[id.0 as usize]
            .as_any()
            .downcast_ref::<T>()
            .expect("device type mismatch")
    }

    fn dispatch(&mut self, ev: Event) {
        let (id, dev_ev) = match ev {
            Event::FrameRx {
                device,
                port,
                frame,
                ..
            } => (device, DeviceEvent::FrameRx { port, frame }),
            Event::Timer { device, id, token } => (device, DeviceEvent::Timer { id, token }),
        };
        let mut ctx = Ctx::new(&mut self.kernel, id);
        self.devices[id.0 as usize].handle(&mut ctx, dev_ev);
    }

    pub fn is_device<T: Device>(&self, id: DeviceId) -> bool {
        self.devices[id.0 as usize].as_any().is::<T>()
    }

    /// Dispatch exactly one event and report what was delivered, or `None` when idle (see
    /// [`IDLE_HORIZON`]).
    pub fn step(&mut self) -> Option<Delivered> {
        let ev = self.kernel.next_event_within(IDLE_HORIZON)?;
        let at = self.kernel.now();
        let d = match &ev {
            Event::FrameRx {
                device,
                port,
                frame,
                from,
                sent,
            } => Delivered::Frame {
                at,
                device: *device,
                port: *port,
                frame: frame.clone(),
                from: *from,
                sent: *sent,
            },
            Event::Timer { device, .. } => Delivered::Timer {
                at,
                device: *device,
            },
        };
        self.dispatch(ev);
        Some(d)
    }

    /// Run all events up to and including `t`, leaving the clock at `t`.
    pub fn run_until(&mut self, t: SimTime) {
        while let Some(ev) = self.kernel.next_event_until(t) {
            self.dispatch(ev);
        }
    }

    /// Run until nothing is due within [`IDLE_HORIZON`]. Packets in flight and short timers keep
    /// it going; a timer further away (a lease renewal hours from now) is left queued and fires
    /// when time is advanced past it.
    pub fn run_to_idle(&mut self) {
        while let Some(ev) = self.kernel.next_event_within(IDLE_HORIZON) {
            self.dispatch(ev);
        }
    }

    /// Run events up to `t`, stopping early as soon as `done` holds. If `t` is reached the clock
    /// is left at `t`; if `done` ends it, the clock is at the last event.
    pub fn run_until_or(&mut self, t: SimTime, mut done: impl FnMut(&World) -> bool) {
        while !done(self) {
            match self.kernel.next_event_until(t) {
                Some(ev) => self.dispatch(ev),
                None => break,
            }
        }
    }

    pub fn trace_hash(&self) -> u64 {
        self.kernel.trace.hash()
    }
}
