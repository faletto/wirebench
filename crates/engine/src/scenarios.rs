//! Canned scenarios shared by tests, the CLI and CI so they cannot drift apart.

use crate::World;
use kernel::{DeviceId, LinkConfig, Trace};
use netdev::Host;
use wire::{Ipv4Addr, MacAddr};

pub fn host(n: u8) -> Host {
    Host::new(
        &format!("pc{n}"),
        MacAddr([2, 0, 0, 0, 0, n]),
        Ipv4Addr::new(10, 0, 0, n),
        24,
    )
}

/// Two hosts on one cable; pc1 pings pc2 once; the world is run to idle.
pub fn two_hosts_ping() -> (World, DeviceId, DeviceId) {
    let mut w = World::new(Trace::keeping());
    let a = w.add_device(host(1));
    let b = w.add_device(host(2));
    w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
    w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    w.run_to_idle();
    (w, a, b)
}
