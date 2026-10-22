#![no_main]
use engine::chain::static_chain;
use engine::scenarios::host;
use engine::World;
use kernel::{LinkConfig, PortId, SimTime, Trace};
use libfuzzer_sys::fuzz_target;
use netdev::{Device, DeviceEvent, Host, L2Switch, Router};

// Arbitrary bytes delivered to a host, a router and a switch as if they had arrived on a port.
// Whatever they answer is run forward for a bounded time. Nothing may panic or loop.
// Byte 0 picks the device, byte 1 the port (clamped to the device's real ports).
fuzz_target!(|data: &[u8]| {
    let [kind, port, frame @ ..] = data else { return };
    let frame = frame.to_vec();
    match kind % 3 {
        0 => {
            let mut c = static_chain();
            let id = c.pc1;
            c.world.with_device::<Host, _>(id, |h, ctx| {
                h.handle(ctx, DeviceEvent::FrameRx { port: PortId(0), frame })
            });
            c.world.run_until(SimTime::from_secs(5));
        }
        1 => {
            let mut c = static_chain();
            let id = c.r1;
            let p = PortId(u16::from(*port) % 2);
            c.world.with_device::<Router, _>(id, |r, ctx| {
                r.handle(ctx, DeviceEvent::FrameRx { port: p, frame })
            });
            c.world.run_until(SimTime::from_secs(5));
        }
        _ => {
            let mut w = World::new(Trace::new());
            let sw = w.add_device(L2Switch::new(4));
            for i in 0..3u8 {
                let h = w.add_device(host(i + 1));
                w.connect((h, 0), (sw, u16::from(i)), LinkConfig::FAST_ETHERNET);
            }
            let p = PortId(u16::from(*port) % 4);
            w.with_device::<L2Switch, _>(sw, |s, ctx| {
                s.handle(ctx, DeviceEvent::FrameRx { port: p, frame })
            });
            w.run_until(SimTime::from_secs(5));
        }
    }
});
