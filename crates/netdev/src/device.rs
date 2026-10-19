use kernel::{DeviceId, Kernel, PortId, SimTime, TimerId, TraceKind};
use std::any::Any;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceEvent {
    FrameRx { port: PortId, frame: Vec<u8> },
    Timer { id: TimerId, token: u64 },
}

/// A device's view of the world during one event.
#[derive(Debug)]
pub struct Ctx<'a> {
    kernel: &'a mut Kernel,
    device: DeviceId,
}

impl<'a> Ctx<'a> {
    pub fn new(kernel: &'a mut Kernel, device: DeviceId) -> Self {
        Ctx { kernel, device }
    }

    pub fn now(&self) -> SimTime {
        self.kernel.now()
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    pub fn transmit(&mut self, port: PortId, frame: Vec<u8>) {
        self.kernel.transmit(self.device, port, frame);
    }

    pub fn schedule(&mut self, after: SimTime, token: u64) -> TimerId {
        self.kernel.schedule_timer(self.device, after, token)
    }

    pub fn cancel(&mut self, id: TimerId) {
        self.kernel.cancel_timer(id);
    }

    /// Free-form trace record, for golden tests and the packet inspector.
    pub fn note(&mut self, text: &str) {
        let (t, d) = (self.kernel.now(), self.device.0);
        self.kernel
            .trace
            .record(t, TraceKind::Note, d, 0, text.as_bytes());
    }
}

pub trait Device: Any {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
