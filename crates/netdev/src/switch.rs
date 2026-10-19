use crate::{Ctx, Device, DeviceEvent};
use kernel::{PortId, SimTime};
use std::any::Any;
use std::collections::BTreeMap;
use wire::MacAddr;

/// Transparent learning bridge (no VLANs or STP yet).
#[derive(Debug)]
pub struct L2Switch {
    n_ports: u16,
    table: BTreeMap<MacAddr, (PortId, SimTime)>,
    /// Entries unseen for this long are ignored (and replaced on next sight).
    pub aging: SimTime,
}

impl L2Switch {
    pub fn new(n_ports: u16) -> Self {
        L2Switch {
            n_ports,
            table: BTreeMap::new(),
            aging: SimTime::from_secs(300),
        }
    }

    /// Live MAC table entries at `now`, in address order.
    pub fn mac_table(&self, now: SimTime) -> Vec<(MacAddr, PortId)> {
        self.table
            .iter()
            .filter(|(_, (_, seen))| *seen + self.aging > now)
            .map(|(m, (p, _))| (*m, *p))
            .collect()
    }
}

impl Device for L2Switch {
    fn handle(&mut self, ctx: &mut Ctx, ev: DeviceEvent) {
        let DeviceEvent::FrameRx { port, frame } = ev else {
            return;
        };
        if frame.len() < 14 {
            return;
        }
        let dst = MacAddr(frame[0..6].try_into().unwrap());
        let src = MacAddr(frame[6..12].try_into().unwrap());
        let now = ctx.now();
        if !src.is_multicast() {
            self.table.insert(src, (port, now));
        }
        let known = self
            .table
            .get(&dst)
            .filter(|(_, seen)| *seen + self.aging > now)
            .map(|(p, _)| *p);
        match known {
            Some(out) if !dst.is_multicast() => {
                // Destination is behind the ingress port: filter.
                if out != port {
                    ctx.transmit(out, frame);
                }
            }
            _ => {
                for p in (0..self.n_ports).map(PortId).filter(|p| *p != port) {
                    ctx.transmit(p, frame.clone());
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
