//! Copying a whole simulation to rewind to later.

#[test]
fn a_snapshot_resumes_exactly_where_it_was_taken() {
    // Run a ping part of the way, copy the world, finish both, and compare the whole trace.
    use engine::World;
    use kernel::{LinkConfig, Trace};
    use netdev::Host;
    use wire::{Ipv4Addr, MacAddr};
    let mk = || {
        let mut w = World::new(Trace::new());
        let a = w.add_device(Host::new(
            "a",
            MacAddr([2, 0, 0, 0, 0, 1]),
            Ipv4Addr::new(10, 0, 0, 1),
            24,
        ));
        let b = w.add_device(Host::new(
            "b",
            MacAddr([2, 0, 0, 0, 0, 2]),
            Ipv4Addr::new(10, 0, 0, 2),
            24,
        ));
        w.connect((a, 0), (b, 0), LinkConfig::FAST_ETHERNET);
        (w, a)
    };
    let (mut w, a) = mk();
    w.with_device::<Host, _>(a, |h, c| h.ping(c, Ipv4Addr::new(10, 0, 0, 2)));
    for _ in 0..2 {
        w.step();
    }
    let mut copy = w.snapshot().expect("hosts can be copied");
    w.run_to_idle();
    copy.run_to_idle();
    assert_eq!(w.trace_hash(), copy.trace_hash());
    assert_eq!(
        w.device::<Host>(a).ping_results.len(),
        copy.device::<Host>(a).ping_results.len()
    );
}
