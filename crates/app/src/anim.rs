//! Packet animation for simulation mode. Purely presentational: the engine reports each
//! hop (sender, receiver, frame) and knows nothing about animation. Positions are
//! interpolated on wall-clock time, so the simulation stays deterministic and pausable.

/// Seconds a frame takes to cross one cable at 1x speed.
pub const HOP_SECS: f64 = 0.45;

pub const SPEEDS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];

/// One frame crossing one cable, drawn as a dot from `from` to `to` (indices into the nodes).
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub from: usize,
    pub to: usize,
    /// Wall-clock start and length, in seconds.
    pub t0: f64,
    pub dur: f64,
    /// Short caption drawn beside the dot, e.g. `ARP request`.
    pub label: String,
}

impl Flight {
    /// How far along the cable the dot is, 0..=1, or `None` once it has landed.
    pub fn progress(&self, now: f64) -> Option<f32> {
        let t = (now - self.t0) / self.dur;
        (t < 1.0).then_some(t.clamp(0.0, 1.0) as f32)
    }
}

/// Playback state for the stepper.
#[derive(Debug)]
pub struct Anim {
    pub enabled: bool,
    pub flights: Vec<Flight>,
    /// Auto-step on a wall-clock schedule until the simulation goes idle.
    pub playing: bool,
    /// Wall time of the next automatic step while playing.
    pub next_step: f64,
    /// Index into [`SPEEDS`].
    pub speed: usize,
}

impl Default for Anim {
    fn default() -> Self {
        Anim {
            enabled: true,
            flights: Vec::new(),
            playing: false,
            next_step: 0.0,
            speed: 1,
        }
    }
}

impl Anim {
    pub fn hop_secs(&self) -> f64 {
        HOP_SECS / f64::from(SPEEDS[self.speed])
    }

    pub fn start(&mut self, from: usize, to: usize, now: f64, label: String) {
        let dur = self.hop_secs();
        self.flights.push(Flight {
            from,
            to,
            t0: now,
            dur,
            label,
        });
    }

    /// Forget flights that have landed. Returns whether any are still in the air.
    pub fn retire(&mut self, now: f64) -> bool {
        self.flights.retain(|f| f.progress(now).is_some());
        !self.flights.is_empty()
    }
}

/// The part of a summary before the first `: `, e.g. `ICMP echo request seq=1`.
pub fn caption(summary: &str) -> String {
    summary
        .split_once(": ")
        .map_or(summary, |(a, _)| a)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight() -> Flight {
        Flight {
            from: 0,
            to: 1,
            t0: 10.0,
            dur: 0.5,
            label: String::new(),
        }
    }

    #[test]
    fn progress_runs_from_zero_to_one_then_lands() {
        let f = flight();
        assert_eq!(
            f.progress(9.0),
            Some(0.0),
            "not started yet is clamped to the start"
        );
        assert_eq!(f.progress(10.0), Some(0.0));
        assert!((f.progress(10.25).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(f.progress(10.5), None);
        assert_eq!(f.progress(99.0), None);
    }

    #[test]
    fn speed_scales_the_duration() {
        let mut a = Anim::default();
        assert_eq!(a.hop_secs(), HOP_SECS);
        a.speed = 3;
        assert_eq!(a.hop_secs(), HOP_SECS / 4.0);
        a.start(0, 1, 5.0, "x".into());
        assert_eq!(a.flights[0].dur, HOP_SECS / 4.0);
    }

    #[test]
    fn retire_drops_only_landed_flights() {
        let mut a = Anim::default();
        a.start(0, 1, 0.0, "a".into());
        a.start(1, 2, 0.3, "b".into());
        assert!(a.retire(0.46), "second flight is still in the air");
        assert_eq!(a.flights.len(), 1);
        assert!(!a.retire(5.0));
        assert!(a.flights.is_empty());
    }

    #[test]
    fn caption_strips_addresses() {
        assert_eq!(
            caption("ICMP echo request seq=1: 10.0.1.2 > 10.0.1.1 ttl=64"),
            "ICMP echo request seq=1"
        );
        assert_eq!(
            caption("ARP request: who has 10.0.1.1? tell 10.0.1.2"),
            "ARP request"
        );
        assert_eq!(caption("ethertype 0x86dd"), "ethertype 0x86dd");
    }
}
