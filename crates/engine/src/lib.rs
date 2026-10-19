//! The world: kernel plus devices, and the dispatch loop between them.

use kernel::{DeviceId, Event, Kernel, LinkConfig, LinkId, PortId, SimTime, Trace};
use netdev::{Ctx, Device, DeviceEvent};

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
            } => (device, DeviceEvent::FrameRx { port, frame }),
            Event::Timer { device, id, token } => (device, DeviceEvent::Timer { id, token }),
        };
        let mut ctx = Ctx::new(&mut self.kernel, id);
        self.devices[id.0 as usize].handle(&mut ctx, dev_ev);
    }

    /// Run all events up to and including `t`, leaving the clock at `t`.
    pub fn run_until(&mut self, t: SimTime) {
        while let Some(ev) = self.kernel.next_event_until(t) {
            self.dispatch(ev);
        }
    }

    /// Run until the queue is empty.
    pub fn run_to_idle(&mut self) {
        while let Some(ev) = self.kernel.next_event() {
            self.dispatch(ev);
        }
    }

    pub fn trace_hash(&self) -> u64 {
        self.kernel.trace.hash()
    }
}
