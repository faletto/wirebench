//! EtherChannel: several switch ports acting as one logical link.
//!
//! Static bundles (`mode on`) and LACP (`active` / `passive`, IEEE 802.1AX). Cisco's PAgP is
//! proprietary and not simulated. This file holds the plain data and the frame distribution
//! hash; the bundle logic lives on the switch.

use kernel::SimTime;
use wire::LacpInfo;

/// How a port joins its channel group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelMode {
    /// Bundled without negotiation.
    On,
    /// Starts LACP.
    Active,
    /// Answers LACP but never starts it.
    Passive,
}

impl ChannelMode {
    pub fn word(self) -> &'static str {
        match self {
            ChannelMode::On => "on",
            ChannelMode::Active => "active",
            ChannelMode::Passive => "passive",
        }
    }

    pub fn is_lacp(self) -> bool {
        self != ChannelMode::On
    }
}

/// A port's membership of a channel group (`channel-group N mode ...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub group: u8,
    pub mode: ChannelMode,
}

/// The highest channel group number, and the most ports in one group.
pub const MAX_GROUP: u8 = 6;
pub const MAX_MEMBERS: usize = 8;

/// Why a port could not join a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinError {
    /// Other ports in the group use a different protocol.
    ProtocolMismatch,
    TooMany,
}

/// What a port sends with: how a frame picks a member of the bundle (`port-channel load-balance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoadBalance {
    #[default]
    SrcMac,
    DstMac,
    SrcDstMac,
    SrcIp,
    DstIp,
    SrcDstIp,
}

impl LoadBalance {
    pub const ALL: [LoadBalance; 6] = [
        LoadBalance::SrcMac,
        LoadBalance::DstMac,
        LoadBalance::SrcDstMac,
        LoadBalance::SrcIp,
        LoadBalance::DstIp,
        LoadBalance::SrcDstIp,
    ];

    pub fn word(self) -> &'static str {
        match self {
            LoadBalance::SrcMac => "src-mac",
            LoadBalance::DstMac => "dst-mac",
            LoadBalance::SrcDstMac => "src-dst-mac",
            LoadBalance::SrcIp => "src-ip",
            LoadBalance::DstIp => "dst-ip",
            LoadBalance::SrcDstIp => "src-dst-ip",
        }
    }

    /// Which of `n` members carries this frame. The same flow always gets the same member.
    pub fn pick(self, frame: &[u8], n: usize) -> usize {
        if n <= 1 || frame.len() < 14 {
            return 0;
        }
        let byte = |i: usize| frame.get(i).copied().unwrap_or(0) as usize;
        let (dst, src) = (byte(5), byte(11));
        let off = if frame[12..14] == [0x81, 0x00] { 4 } else { 0 };
        let ip = (frame.len() >= 34 + off && frame[12 + off..14 + off] == [0x08, 0x00])
            .then(|| (byte(14 + off + 15), byte(14 + off + 19)));
        let h = match (self, ip) {
            (LoadBalance::SrcIp, Some((s, _))) => s,
            (LoadBalance::DstIp, Some((_, d))) => d,
            (LoadBalance::SrcDstIp, Some((s, d))) => s ^ d,
            (LoadBalance::DstMac, _) | (LoadBalance::DstIp, None) => dst,
            (LoadBalance::SrcDstMac, _) | (LoadBalance::SrcDstIp, None) => src ^ dst,
            _ => src,
        };
        h % n
    }
}

/// What the switch knows about one port's part in a bundle.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChannelPort {
    /// The last LACPDU heard from the neighbour, and when.
    pub peer: Option<(LacpInfo, SimTime)>,
    /// Currently carrying traffic as part of its group's bundle.
    pub bundled: bool,
}

/// STP path cost of a bundle of `n` 100 Mb/s links [verify against a real switch].
pub fn bundle_cost(n: usize) -> u32 {
    match n {
        0 | 1 => 19,
        2 => 12,
        3 => 9,
        4 => 8,
        5 => 7,
        6 => 6,
        7 => 6,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(src: u8, dst: u8, ip: Option<([u8; 4], [u8; 4])>) -> Vec<u8> {
        let mut f = vec![2, 0, 0, 0, 0, dst, 2, 0, 0, 0, 0, src];
        match ip {
            Some((s, d)) => {
                f.extend_from_slice(&[0x08, 0x00]);
                let mut h = vec![0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0];
                h.extend_from_slice(&s);
                h.extend_from_slice(&d);
                f.extend(h);
            }
            None => f.extend_from_slice(&[0x08, 0x06]),
        }
        f.resize(60, 0);
        f
    }

    #[test]
    fn a_flow_always_takes_the_same_member() {
        for lb in LoadBalance::ALL {
            for n in 1..=8 {
                let f = frame(3, 9, Some(([10, 0, 0, 3], [10, 0, 0, 9])));
                let first = lb.pick(&f, n);
                assert!(first < n);
                assert_eq!(first, lb.pick(&f, n));
            }
        }
    }

    #[test]
    fn different_sources_spread_over_the_members() {
        let hit: std::collections::BTreeSet<usize> = (0..8)
            .map(|s| LoadBalance::SrcMac.pick(&frame(s, 1, None), 2))
            .collect();
        assert_eq!(hit.len(), 2);
        let hit: std::collections::BTreeSet<usize> = (0..8)
            .map(|d| LoadBalance::DstIp.pick(&frame(1, 1, Some(([10, 0, 0, 1], [10, 0, 0, d]))), 4))
            .collect();
        assert_eq!(hit.len(), 4);
    }

    #[test]
    fn ip_methods_fall_back_to_macs_for_other_traffic_and_tolerate_runts() {
        let arp = frame(5, 6, None);
        assert_eq!(
            LoadBalance::SrcDstIp.pick(&arp, 4),
            LoadBalance::SrcDstMac.pick(&arp, 4)
        );
        for n in 0..60 {
            let _ = LoadBalance::SrcDstIp.pick(&arp[..n], 3);
        }
    }

    #[test]
    fn a_tagged_frame_hashes_like_an_untagged_one() {
        let plain = frame(3, 9, Some(([10, 0, 0, 3], [10, 0, 0, 9])));
        let mut tagged = plain[..12].to_vec();
        tagged.extend_from_slice(&[0x81, 0x00, 0, 10]);
        tagged.extend_from_slice(&plain[12..]);
        for lb in LoadBalance::ALL {
            assert_eq!(lb.pick(&plain, 4), lb.pick(&tagged, 4), "{lb:?}");
        }
    }
}
