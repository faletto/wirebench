//! VLAN ids and the list syntax used by `switchport trunk allowed vlan`.

use std::collections::BTreeSet;

pub const DEFAULT_VLAN: u16 = 1;
pub const MAX_VLAN: u16 = 4094;

/// Every VLAN id a trunk can carry.
pub fn all_vlans() -> BTreeSet<u16> {
    (1..=MAX_VLAN).collect()
}

/// Parse `1,5,10-20`. Ids are 1 to 4094; a range runs low to high. `None` for anything else.
pub fn parse_vlan_list(s: &str) -> Option<BTreeSet<u16>> {
    let mut out = BTreeSet::new();
    for part in s.split(',') {
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (a.parse::<u16>().ok()?, b.parse::<u16>().ok()?),
            None => {
                let v = part.parse::<u16>().ok()?;
                (v, v)
            }
        };
        if lo < 1 || hi > MAX_VLAN || lo > hi {
            return None;
        }
        out.extend(lo..=hi);
    }
    Some(out)
}

/// The inverse of [`parse_vlan_list`], with runs collapsed (`1-9,11-4094`). Empty gives `none`.
pub fn format_vlan_list(set: &BTreeSet<u16>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut it = set.iter().copied().peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while it.peek() == Some(&(end + 1)) {
            end = it.next().unwrap();
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_parse_and_format_back() {
        let s = parse_vlan_list("1,5,10-12").unwrap();
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), [1, 5, 10, 11, 12]);
        assert_eq!(format_vlan_list(&s), "1,5,10-12");
        assert_eq!(format_vlan_list(&all_vlans()), "1-4094");
        assert_eq!(format_vlan_list(&BTreeSet::new()), "none");
    }

    #[test]
    fn bad_lists_are_rejected() {
        for bad in ["", "0", "4095", "5-3", "a", "1,,2", "1-", "-3", "1-2-3"] {
            assert_eq!(parse_vlan_list(bad), None, "{bad:?}");
        }
    }
}
