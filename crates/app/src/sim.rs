//! Simulation mode state: a hop log fed by `EngineEvent::Hop` and the controls' inputs.

/// One delivered frame, as listed in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopRow {
    pub ns: u64,
    pub device: String,
    pub port: u16,
    pub summary: String,
}

#[derive(Debug, Default)]
pub struct Sim {
    /// Index into `App::nodes` of the PC that sends the queued ping.
    pub src: Option<usize>,
    pub dst: String,
    pub hops: Vec<HopRow>,
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
