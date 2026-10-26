//! The packet inspector's non-drawing parts: the filter language for the hop log, and the
//! selection state. Drawing lives in `app.rs` beside the rest of the simulation panel.

use crate::sim::HopRow;

/// Names a filter term may use on its own to mean "has this layer".
const PROTOCOLS: [&str; 8] = [
    "ethernet", "vlan", "arp", "ip", "icmp", "udp", "dhcp", "stp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    /// Case-insensitive text found in the device name or the one-line summary.
    Text(String),
    /// The frame has this layer (`arp`, `icmp`, `proto:udp`, ...).
    Proto(String),
    /// Delivered to this device (`dev:r1`).
    Device(String),
    /// Delivered on this port number (`port:2`).
    Port(u16),
}

/// A list of terms, all of which must hold. `-term` negates one. Typing something half-finished
/// never errors: anything not understood is just text to look for.
///
/// ```text
/// icmp                  ICMP frames
/// dev:sw1 -arp          frames delivered to sw1 that are not ARP
/// port:1 reply          arrived on port 1, with "reply" in the summary
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    terms: Vec<(bool, Term)>,
}

impl Filter {
    pub fn parse(text: &str) -> Filter {
        let mut terms = vec![];
        for word in text.split_whitespace() {
            let (negated, w) = match word.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, word),
            };
            let lower = w.to_ascii_lowercase();
            let term = match lower.split_once(':') {
                Some(("proto", p)) if !p.is_empty() => Term::Proto(p.to_string()),
                Some(("dev" | "at", d)) if !d.is_empty() => Term::Device(d.to_string()),
                Some(("port", n)) => match n.parse() {
                    Ok(n) => Term::Port(n),
                    Err(_) => Term::Text(lower),
                },
                _ if PROTOCOLS.contains(&lower.as_str()) => Term::Proto(lower),
                _ => Term::Text(lower),
            };
            terms.push((negated, term));
        }
        Filter { terms }
    }

    pub fn matches(&self, row: &HopRow) -> bool {
        self.terms.iter().all(|(negated, t)| {
            let hit = match t {
                Term::Text(s) => {
                    row.device.to_ascii_lowercase().contains(s)
                        || row.summary.to_ascii_lowercase().contains(s)
                }
                Term::Proto(p) => row.protocols.contains(&p.as_str()),
                Term::Device(d) => row.device.to_ascii_lowercase() == *d,
                Term::Port(n) => row.port == *n,
            };
            hit != *negated
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(device: &str, port: u16, summary: &str, protocols: Vec<&'static str>) -> HopRow {
        HopRow {
            ns: 0,
            device: device.into(),
            port,
            summary: summary.into(),
            frame: vec![],
            layers: vec![],
            protocols,
        }
    }

    fn rows() -> Vec<HopRow> {
        vec![
            row(
                "sw1",
                1,
                "ARP request: who has 10.0.0.2? tell 10.0.0.1",
                vec!["ethernet", "arp"],
            ),
            row(
                "r1",
                0,
                "ICMP echo request seq=1: 10.0.0.1 > 10.0.1.2 ttl=64",
                vec!["ethernet", "ip", "icmp"],
            ),
            row(
                "sw1",
                2,
                "ICMP echo reply seq=1: 10.0.1.2 > 10.0.0.1 ttl=63",
                vec!["ethernet", "vlan", "ip", "icmp"],
            ),
        ]
    }

    fn pick(f: &str) -> Vec<usize> {
        let f = Filter::parse(f);
        rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| f.matches(r))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn an_empty_filter_keeps_everything() {
        assert_eq!(Filter::parse("  "), Filter::default());
        assert_eq!(pick(""), [0, 1, 2]);
    }

    #[test]
    fn layer_names_device_and_port_terms() {
        assert_eq!(pick("icmp"), [1, 2]);
        assert_eq!(pick("proto:arp"), [0]);
        assert_eq!(pick("vlan"), [2]);
        assert_eq!(pick("dev:sw1"), [0, 2]);
        assert_eq!(pick("at:R1"), [1]);
        assert_eq!(pick("port:2"), [2]);
    }

    #[test]
    fn terms_are_anded_and_can_be_negated() {
        assert_eq!(pick("dev:sw1 icmp"), [2]);
        assert_eq!(pick("dev:sw1 -arp"), [2]);
        assert_eq!(pick("-icmp"), [0]);
        assert_eq!(pick("-dev:sw1 -arp"), [1]);
    }

    #[test]
    fn plain_words_search_the_summary_and_the_device_ignoring_case() {
        assert_eq!(pick("REPLY"), [2]);
        assert_eq!(pick("10.0.1.2"), [1, 2]);
        assert_eq!(pick("r1"), [1]);
        assert_eq!(pick("nothing like this"), Vec::<usize>::new());
    }

    #[test]
    fn half_typed_terms_never_error_or_match_by_accident() {
        // A lone dash, an empty value and a bad port are all just text.
        assert_eq!(pick("-"), Vec::<usize>::new());
        assert_eq!(pick("dev:"), Vec::<usize>::new());
        assert_eq!(pick("port:x"), Vec::<usize>::new());
        assert_eq!(pick("ar"), [0]);
    }
}
