//! pc1 -- r1 -- r2 -- pc2 with static routes both ways.

use crate::World;
use kernel::{DeviceId, LinkConfig, PortId, Trace};
use netdev::{Host, Router, StaticRoute};
use wire::{Ipv4Addr, MacAddr};

#[derive(Debug)]
pub struct Chain {
    pub world: World,
    pub pc1: DeviceId,
    pub r1: DeviceId,
    pub r2: DeviceId,
    pub pc2: DeviceId,
}

/// pc1 10.0.1.2/24 gw .1 | r1 10.0.1.1/24, 10.0.12.1/30 | r2 10.0.12.2/30, 10.0.2.1/24 |
/// pc2 10.0.2.2/24 gw .1. Not yet run: callers start pings.
pub fn static_chain() -> Chain {
    let mut w = World::new(Trace::keeping());
    let mut pc1 = Host::new(
        "pc1",
        MacAddr([2, 0, 0, 0, 1, 1]),
        Ipv4Addr::new(10, 0, 1, 2),
        24,
    );
    pc1.gateway = Some(Ipv4Addr::new(10, 0, 1, 1));
    let mut pc2 = Host::new(
        "pc2",
        MacAddr([2, 0, 0, 0, 2, 1]),
        Ipv4Addr::new(10, 0, 2, 2),
        24,
    );
    pc2.gateway = Some(Ipv4Addr::new(10, 0, 2, 1));
    let mut r1 = Router::new("r1", 1, 2);
    let mut r2 = Router::new("r2", 2, 2);
    let p = PortId;
    r1.set_address(p(0), Some((Ipv4Addr::new(10, 0, 1, 1), 24)));
    r1.set_address(p(1), Some((Ipv4Addr::new(10, 0, 12, 1), 30)));
    r2.set_address(p(0), Some((Ipv4Addr::new(10, 0, 12, 2), 30)));
    r2.set_address(p(1), Some((Ipv4Addr::new(10, 0, 2, 1), 24)));
    for r in [&mut r1, &mut r2] {
        r.set_shutdown(p(0), false);
        r.set_shutdown(p(1), false);
    }
    r1.add_static(StaticRoute {
        net: Ipv4Addr::new(10, 0, 2, 0),
        len: 24,
        via: Ipv4Addr::new(10, 0, 12, 2),
    });
    r2.add_static(StaticRoute {
        net: Ipv4Addr::new(10, 0, 1, 0),
        len: 24,
        via: Ipv4Addr::new(10, 0, 12, 1),
    });
    let pc1 = w.add_device(pc1);
    let r1 = w.add_device(r1);
    let r2 = w.add_device(r2);
    let pc2 = w.add_device(pc2);
    w.connect((pc1, 0), (r1, 0), LinkConfig::FAST_ETHERNET);
    w.connect((r1, 1), (r2, 0), LinkConfig::FAST_ETHERNET);
    w.connect((r2, 1), (pc2, 0), LinkConfig::FAST_ETHERNET);
    Chain {
        world: w,
        pc1,
        r1,
        r2,
        pc2,
    }
}

/// `static_chain` with one ping pc1 -> pc2, run to idle.
pub fn static_chain_ping() -> Chain {
    let mut c = static_chain();
    c.world
        .with_device::<Host, _>(c.pc1, |h, ctx| h.ping(ctx, Ipv4Addr::new(10, 0, 2, 2)));
    c.world.run_to_idle();
    c
}
