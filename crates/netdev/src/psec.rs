//! Port security: limit which source addresses a switch port will accept.
//!
//! The configuration lives in [`PortSecurity`] (part of the port's settings); the addresses
//! learned while running, the violation count and the error-disabled state live in
//! [`PsecState`] on the switch.

use kernel::SimTime;
use std::collections::{BTreeMap, BTreeSet};
use wire::MacAddr;

/// What happens to a frame from an address the port may not accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Violation {
    /// Drop the frame silently; nothing is counted.
    Protect,
    /// Drop the frame and count it.
    Restrict,
    /// Count it and put the whole port into the error-disabled state.
    #[default]
    Shutdown,
}

impl Violation {
    pub fn word(self) -> &'static str {
        match self {
            Violation::Protect => "protect",
            Violation::Restrict => "restrict",
            Violation::Shutdown => "shutdown",
        }
    }
}

/// Port security settings for one port. `None` in the port's config means it is off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSecurity {
    /// How many addresses may be in use at once.
    pub max: u16,
    pub violation: Violation,
    /// Learned addresses are kept in the configuration.
    pub sticky: bool,
    /// Addresses typed in by hand.
    pub statics: BTreeSet<MacAddr>,
    /// Addresses learned while `sticky` was on.
    pub stuck: BTreeSet<MacAddr>,
}

impl Default for PortSecurity {
    fn default() -> Self {
        PortSecurity {
            max: 1,
            violation: Violation::Shutdown,
            sticky: false,
            statics: BTreeSet::new(),
            stuck: BTreeSet::new(),
        }
    }
}

impl PortSecurity {
    /// Whether `mac` is already known to the port (configured or sticky).
    pub fn knows(&self, mac: &MacAddr) -> bool {
        self.statics.contains(mac) || self.stuck.contains(mac)
    }
}

/// What the switch decided about one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Drop the frame.
    Drop,
    /// Drop the frame and error-disable the port.
    Disable,
}

/// Runtime port security state of a whole switch, indexed by port.
#[derive(Clone, Debug, Default)]
pub struct PsecState {
    /// Addresses learned dynamically (not kept in the config), with the VLAN they arrived in.
    pub dynamic: Vec<BTreeMap<MacAddr, u16>>,
    pub violations: Vec<u64>,
    pub last: Vec<Option<(MacAddr, u16)>>,
    /// Set while the port is error-disabled: when that began.
    pub disabled: Vec<Option<SimTime>>,
    /// `errdisable recovery cause psecure-violation`.
    pub recover: bool,
    /// Seconds before a recovering port is enabled again.
    pub interval: u32,
}

pub const DEFAULT_RECOVERY_SECS: u32 = 300;

impl PsecState {
    pub fn new(n: usize) -> Self {
        PsecState {
            dynamic: vec![BTreeMap::new(); n],
            violations: vec![0; n],
            last: vec![None; n],
            disabled: vec![None; n],
            recover: false,
            interval: DEFAULT_RECOVERY_SECS,
        }
    }

    /// Decide on a frame from `src` in `vlan` arriving on `port`, learning the address if the
    /// port has room (sticky learning writes to `cfg`).
    pub fn admit(
        &mut self,
        port: usize,
        cfg: &mut PortSecurity,
        src: MacAddr,
        vlan: u16,
        now: SimTime,
    ) -> Verdict {
        if cfg.knows(&src) || self.dynamic[port].contains_key(&src) {
            return Verdict::Allow;
        }
        if self.count(port, cfg) < usize::from(cfg.max) {
            if cfg.sticky {
                cfg.stuck.insert(src);
            } else {
                self.dynamic[port].insert(src, vlan);
            }
            return Verdict::Allow;
        }
        self.last[port] = Some((src, vlan));
        match cfg.violation {
            Violation::Protect => Verdict::Drop,
            Violation::Restrict => {
                self.violations[port] += 1;
                Verdict::Drop
            }
            Violation::Shutdown => {
                self.violations[port] += 1;
                self.disabled[port] = Some(now);
                Verdict::Disable
            }
        }
    }

    /// Addresses in use on `port`: configured, sticky and dynamic.
    pub fn count(&self, port: usize, cfg: &PortSecurity) -> usize {
        cfg.statics.len() + cfg.stuck.len() + self.dynamic[port].len()
    }
}

/// `aaaa.bbbb.cccc`, or six colon- or dash-separated bytes.
pub fn parse_mac(s: &str) -> Option<MacAddr> {
    let hex: String = if s.contains('.') {
        let groups: Vec<&str> = s.split('.').collect();
        if groups.len() != 3 || groups.iter().any(|g| g.len() != 4) {
            return None;
        }
        groups.concat()
    } else {
        let groups: Vec<&str> = s.split([':', '-']).collect();
        if groups.len() != 6 || groups.iter().any(|g| g.len() != 2) {
            return None;
        }
        groups.concat()
    };
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(MacAddr(out))
}
