use kernel::PortId;
use std::collections::BTreeMap;
use wire::Ipv4Addr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteKind {
    Connected,
    Static,
}

impl RouteKind {
    /// Administrative distance: lower wins among routes to the same prefix.
    pub fn distance(self) -> u8 {
        match self {
            RouteKind::Connected => 0,
            RouteKind::Static => 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Route {
    pub net: Ipv4Addr,
    pub len: u8,
    /// `None` for connected routes: the next hop is the destination itself.
    pub via: Option<Ipv4Addr>,
    pub port: PortId,
    pub kind: RouteKind,
}

pub fn mask(len: u8) -> u32 {
    if len == 0 {
        0
    } else {
        u32::MAX << (32 - len as u32)
    }
}

/// Forwarding table with longest-prefix-match lookup: one exact-match probe per prefix
/// length, longest first.
#[derive(Debug, Default, Clone)]
pub struct Fib {
    routes: BTreeMap<(u8, u32), Route>,
}

impl Fib {
    pub fn insert(&mut self, r: Route) {
        let key = (r.len, r.net.to_u32() & mask(r.len));
        let r = Route {
            net: Ipv4Addr::from_u32(key.1),
            ..r
        };
        match self.routes.get(&key) {
            Some(old) if old.kind.distance() <= r.kind.distance() => {}
            _ => {
                self.routes.insert(key, r);
            }
        }
    }

    pub fn lookup(&self, dst: Ipv4Addr) -> Option<&Route> {
        (0..=32u8)
            .rev()
            .find_map(|len| self.routes.get(&(len, dst.to_u32() & mask(len))))
    }

    /// Routes ordered by prefix then length, for display.
    pub fn routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn r(net: u32, len: u8, port: u16) -> Route {
        Route {
            net: Ipv4Addr::from_u32(net),
            len,
            via: None,
            port: PortId(port),
            kind: RouteKind::Static,
        }
    }

    #[test]
    fn longest_prefix_wins_and_default_route_works() {
        let mut f = Fib::default();
        f.insert(r(0, 0, 0));
        f.insert(r(0x0a00_0000, 8, 1));
        f.insert(r(0x0a01_0000, 16, 2));
        assert_eq!(
            f.lookup(Ipv4Addr::new(10, 1, 2, 3)).unwrap().port,
            PortId(2)
        );
        assert_eq!(
            f.lookup(Ipv4Addr::new(10, 9, 2, 3)).unwrap().port,
            PortId(1)
        );
        assert_eq!(f.lookup(Ipv4Addr::new(8, 8, 8, 8)).unwrap().port, PortId(0));
    }

    #[test]
    fn connected_beats_static_for_same_prefix() {
        let mut f = Fib::default();
        f.insert(r(0x0a00_0000, 24, 1));
        f.insert(Route {
            kind: RouteKind::Connected,
            ..r(0x0a00_0000, 24, 2)
        });
        f.insert(r(0x0a00_0000, 24, 3));
        assert_eq!(
            f.lookup(Ipv4Addr::new(10, 0, 0, 5)).unwrap().port,
            PortId(2)
        );
    }

    proptest! {
        #[test]
        fn lpm_matches_bruteforce(
            entries in proptest::collection::vec((any::<u32>(), 0u8..=32), 0..20),
            dst in any::<u32>(),
        ) {
            let mut f = Fib::default();
            for (i, (net, len)) in entries.iter().enumerate() {
                f.insert(r(*net, *len, i as u16));
            }
            // Brute force over the canonicalised prefixes; ties (same prefix) keep the first
            // inserted, matching Fib's equal-distance rule.
            let mut best: Option<(u8, u16)> = None;
            for (i, (net, len)) in entries.iter().enumerate() {
                let m = mask(*len);
                if dst & m == net & m && best.is_none_or(|(bl, _)| *len > bl) {
                    best = Some((*len, i as u16));
                }
            }
            let got = f.lookup(Ipv4Addr::from_u32(dst)).map(|r| (r.len, r.port.0));
            prop_assert_eq!(got, best);
        }
    }
}
