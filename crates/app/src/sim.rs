//! Simulation mode state: a hop log fed by `EngineEvent::Hop` and the controls' inputs.

/// One delivered frame, as listed in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopRow {
    pub ns: u64,
    pub device: String,
    pub port: u16,
    pub summary: String,
    /// The frame as delivered, and its layers, for the field tree and the hex view.
    pub frame: Vec<u8>,
    pub layers: Vec<wire::Node>,
    /// Layer names (`icmp`, `arp`, ...) for the filter.
    pub protocols: Vec<&'static str>,
}

#[derive(Debug, Default)]
pub struct Sim {
    /// Index into `App::nodes` of the PC that sends the queued ping.
    pub src: Option<usize>,
    pub dst: String,
    pub hops: Vec<HopRow>,
    /// Text typed in the filter box.
    pub filter: String,
    /// The row open in the detail view (an index into `hops`), and the bytes of the field picked.
    pub selected: Option<usize>,
    pub field: Option<std::ops::Range<usize>>,
    /// `hops.len()` before each step or queued ping the engine can still undo, oldest first.
    pub marks: Vec<usize>,
    /// A step is out with the engine; no other is started until it is answered.
    pub in_flight: bool,
    /// Steps left before a stepping run gives up (so a runaway lab cannot hang the UI).
    pub budget: u32,
    /// How the latest stepping run ended: `Some(true)` something happened (or the budget ran
    /// out), `Some(false)` the simulation is idle.
    pub outcome: Option<bool>,
    /// Whether the latest `StepBack` was done.
    pub rewound: bool,
}

/// `a.b.c.d`, nothing else.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut it = s.trim().split('.');
    for b in &mut out {
        *b = it.next()?.parse().ok()?;
    }
    it.next().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotted_quads_only() {
        assert_eq!(parse_ipv4(" 10.0.1.1 "), Some([10, 0, 1, 1]));
        for bad in ["", "1.2.3", "1.2.3.4.5", "1.2.3.256", "a.b.c.d", "1..2.3"] {
            assert_eq!(parse_ipv4(bad), None, "{bad}");
        }
    }
}
