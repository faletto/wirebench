use core::ops::Add;

/// Virtual time in nanoseconds since simulation start.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct SimTime(pub u64);

impl SimTime {
    pub const ZERO: SimTime = SimTime(0);

    pub const fn from_micros(us: u64) -> Self {
        SimTime(us * 1_000)
    }

    pub const fn from_millis(ms: u64) -> Self {
        SimTime(ms * 1_000_000)
    }

    pub const fn from_secs(s: u64) -> Self {
        SimTime(s * 1_000_000_000)
    }
}

impl Add for SimTime {
    type Output = SimTime;
    fn add(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.saturating_add(rhs.0))
    }
}
