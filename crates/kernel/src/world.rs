use crate::{SimTime, Trace, TraceKind};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DeviceId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PortId(pub u16);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LinkId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TimerId(pub u64);

/// An event delivered to a device.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    FrameRx {
        device: DeviceId,
        port: PortId,
        frame: Vec<u8>,
        /// The device that sent it, and when its first bit left (for animating the hop).
        /// Not part of the trace, so adding these changed no golden hash.
        from: DeviceId,
        sent: SimTime,
    },
    Timer {
        device: DeviceId,
        id: TimerId,
        token: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct LinkConfig {
    pub bits_per_sec: u64,
    pub prop_delay: SimTime,
}

impl LinkConfig {
    /// 100 Mb/s, 1 us propagation. A plausible default for a short copper run.
    pub const FAST_ETHERNET: LinkConfig = LinkConfig {
        bits_per_sec: 100_000_000,
        prop_delay: SimTime(1_000),
    };
}

#[derive(Debug)]
struct Link {
    ends: [(DeviceId, PortId); 2],
    cfg: LinkConfig,
    up: bool,
    /// Bumped on every down transition so in-flight frames are recognisably stale.
    epoch: u32,
    /// Per-direction transmitter busy-until, for back-to-back serialization.
    busy_until: [SimTime; 2],
}

#[derive(Debug)]
enum Payload {
    Frame {
        link: LinkId,
        epoch: u32,
        from: DeviceId,
        sent: SimTime,
        to: (DeviceId, PortId),
        frame: Vec<u8>,
    },
    Timer {
        device: DeviceId,
        id: TimerId,
        token: u64,
    },
}

#[derive(Debug)]
struct Scheduled {
    at: SimTime,
    seq: u64,
    payload: Payload,
}

impl PartialEq for Scheduled {
    fn eq(&self, o: &Self) -> bool {
        (self.at, self.seq) == (o.at, o.seq)
    }
}
impl Eq for Scheduled {}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Scheduled {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.at, self.seq).cmp(&(o.at, o.seq))
    }
}

/// The event loop state. Devices live outside the kernel and are driven by whoever calls
/// [`Kernel::next_event`] (the engine), which keeps this crate free of device types.
#[derive(Debug)]
pub struct Kernel {
    now: SimTime,
    next_seq: u64,
    next_timer: u64,
    heap: BinaryHeap<Reverse<Scheduled>>,
    cancelled: BTreeSet<TimerId>,
    links: Vec<Link>,
    delivered: u64,
    pub trace: Trace,
}

impl Kernel {
    pub fn new(trace: Trace) -> Self {
        Kernel {
            now: SimTime::ZERO,
            next_seq: 0,
            next_timer: 0,
            heap: BinaryHeap::new(),
            cancelled: BTreeSet::new(),
            links: Vec::new(),
            delivered: 0,
            trace,
        }
    }

    /// Events handed to devices so far (dropped frames and cancelled timers excluded).
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    pub fn now(&self) -> SimTime {
        self.now
    }

    pub fn add_link(
        &mut self,
        a: (DeviceId, PortId),
        b: (DeviceId, PortId),
        cfg: LinkConfig,
    ) -> LinkId {
        self.links.push(Link {
            ends: [a, b],
            cfg,
            up: true,
            epoch: 0,
            busy_until: [SimTime::ZERO; 2],
        });
        LinkId(self.links.len() as u32 - 1)
    }

    pub fn set_link_up(&mut self, id: LinkId, up: bool) {
        let l = &mut self.links[id.0 as usize];
        if l.up && !up {
            l.epoch += 1;
        }
        l.up = up;
    }

    fn push(&mut self, at: SimTime, payload: Payload) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(Reverse(Scheduled { at, seq, payload }));
    }

    /// Transmit `frame` out of `(device, port)`. Dropped silently (but traced) if the port
    /// has no link or the link is down.
    pub fn transmit(&mut self, device: DeviceId, port: PortId, frame: Vec<u8>) {
        let now = self.now;
        let Some(idx) = self
            .links
            .iter()
            .position(|l| l.ends.contains(&(device, port)))
        else {
            self.trace
                .record(now, TraceKind::FrameDrop, device.0, port.0, &frame);
            return;
        };
        let l = &mut self.links[idx];
        if !l.up {
            self.trace
                .record(now, TraceKind::FrameDrop, device.0, port.0, &frame);
            return;
        }
        let dir = usize::from(l.ends[1] == (device, port));
        let to = l.ends[1 - dir];
        let start = now.max_with(l.busy_until[dir]);
        // ceil(bits * 1e9 / bps) ns
        let bits = frame.len() as u128 * 8;
        let ser = (bits * 1_000_000_000).div_ceil(l.cfg.bits_per_sec as u128) as u64;
        let done = SimTime(start.0 + ser);
        l.busy_until[dir] = done;
        let arrive = done + l.cfg.prop_delay;
        let epoch = l.epoch;
        self.trace
            .record(now, TraceKind::FrameTx, device.0, port.0, &frame);
        self.push(
            arrive,
            Payload::Frame {
                link: LinkId(idx as u32),
                epoch,
                from: device,
                sent: start,
                to,
                frame,
            },
        );
    }

    pub fn schedule_timer(&mut self, device: DeviceId, after: SimTime, token: u64) -> TimerId {
        let id = TimerId(self.next_timer);
        self.next_timer += 1;
        self.push(self.now + after, Payload::Timer { device, id, token });
        id
    }

    /// Lazy cancel: the entry stays in the heap and is skipped when popped.
    pub fn cancel_timer(&mut self, id: TimerId) {
        self.cancelled.insert(id);
    }

    /// Pop the next live event, advancing the clock. `None` when the queue is empty.
    pub fn next_event(&mut self) -> Option<Event> {
        self.next_event_until(SimTime(u64::MAX))
    }

    /// Like [`next_event`](Self::next_event) but leaves events after `limit` queued and
    /// advances the clock to `limit` when none remain at or before it.
    pub fn next_event_until(&mut self, limit: SimTime) -> Option<Event> {
        loop {
            match self.heap.peek() {
                Some(Reverse(s)) if s.at <= limit => {}
                _ => {
                    if limit.0 != u64::MAX && limit > self.now {
                        self.now = limit;
                    }
                    return None;
                }
            }
            let Reverse(s) = self.heap.pop().unwrap();
            self.now = s.at;
            match s.payload {
                Payload::Timer { device, id, token } => {
                    if self.cancelled.remove(&id) {
                        continue;
                    }
                    self.trace.record(
                        self.now,
                        TraceKind::Timer,
                        device.0,
                        0,
                        &token.to_be_bytes(),
                    );
                    self.delivered += 1;
                    return Some(Event::Timer { device, id, token });
                }
                Payload::Frame {
                    link,
                    epoch,
                    from,
                    sent,
                    to,
                    frame,
                } => {
                    let l = &self.links[link.0 as usize];
                    if !l.up || l.epoch != epoch {
                        self.trace
                            .record(self.now, TraceKind::FrameDrop, to.0 .0, to.1 .0, &frame);
                        continue;
                    }
                    self.trace
                        .record(self.now, TraceKind::FrameRx, to.0 .0, to.1 .0, &frame);
                    self.delivered += 1;
                    return Some(Event::FrameRx {
                        device: to.0,
                        port: to.1,
                        frame,
                        from,
                        sent,
                    });
                }
            }
        }
    }
}

impl SimTime {
    fn max_with(self, o: SimTime) -> SimTime {
        if self >= o {
            self
        } else {
            o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: DeviceId = DeviceId(0);
    const B: DeviceId = DeviceId(1);
    const P: PortId = PortId(0);

    fn k() -> Kernel {
        Kernel::new(Trace::keeping())
    }

    #[test]
    fn same_time_events_fire_in_insertion_order() {
        let mut k = k();
        for token in 0..5 {
            k.schedule_timer(A, SimTime::from_millis(1), token);
        }
        let got: Vec<u64> = std::iter::from_fn(|| k.next_event())
            .map(|e| match e {
                Event::Timer { token, .. } => token,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn cancelled_timer_never_fires() {
        let mut k = k();
        let t = k.schedule_timer(A, SimTime::from_millis(1), 1);
        k.schedule_timer(A, SimTime::from_millis(2), 2);
        k.cancel_timer(t);
        match k.next_event() {
            Some(Event::Timer { token: 2, .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
        assert!(k.next_event().is_none());
    }

    #[test]
    fn frame_arrives_after_ser_plus_prop_delay() {
        let mut k = k();
        k.add_link((A, P), (B, P), LinkConfig::FAST_ETHERNET);
        k.transmit(A, P, vec![0; 125]); // 1000 bits @ 100Mb/s = 10_000 ns
        assert!(matches!(
            k.next_event(),
            Some(Event::FrameRx { device: B, .. })
        ));
        assert_eq!(k.now(), SimTime(10_000 + 1_000));
    }

    #[test]
    fn back_to_back_frames_serialize() {
        let mut k = k();
        k.add_link((A, P), (B, P), LinkConfig::FAST_ETHERNET);
        k.transmit(A, P, vec![0; 125]);
        k.transmit(A, P, vec![0; 125]);
        k.next_event();
        k.next_event();
        assert_eq!(k.now(), SimTime(20_000 + 1_000));
    }

    #[test]
    fn link_down_drops_in_flight() {
        let mut k = k();
        let l = k.add_link((A, P), (B, P), LinkConfig::FAST_ETHERNET);
        k.transmit(A, P, vec![1; 64]);
        k.set_link_up(l, false);
        k.set_link_up(l, true); // back up before arrival: still stale
        assert!(k.next_event().is_none());
        k.transmit(A, P, vec![2; 64]);
        assert!(matches!(k.next_event(), Some(Event::FrameRx { .. })));
    }

    #[test]
    fn next_event_until_advances_clock() {
        let mut k = k();
        k.schedule_timer(A, SimTime::from_secs(5), 0);
        assert!(k.next_event_until(SimTime::from_secs(1)).is_none());
        assert_eq!(k.now(), SimTime::from_secs(1));
        assert!(k.next_event_until(SimTime::from_secs(5)).is_some());
    }

    fn run_scenario() -> u64 {
        let mut k = k();
        k.add_link((A, P), (B, P), LinkConfig::FAST_ETHERNET);
        for i in 0..20u8 {
            k.transmit(A, P, vec![i; 64 + i as usize]);
            k.schedule_timer(B, SimTime::from_micros(i as u64 * 7), i as u64);
        }
        while k.next_event().is_some() {}
        k.trace.hash()
    }

    #[test]
    fn trace_hash_stable_across_runs() {
        assert_eq!(run_scenario(), run_scenario());
        // Blessed value: changing it means the canonical trace format or kernel semantics
        // changed. Re-bless deliberately, and the CI determinism job must agree on all targets.
        assert_eq!(run_scenario(), BLESSED);
    }

    const BLESSED: u64 = 7958038576765015734;
}
