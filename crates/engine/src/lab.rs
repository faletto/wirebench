//! Instantiate a [`labfile::LabFile`] as a running [`World`]: create devices from templates,
//! cable the links, and apply each device's saved config through the real CLI.

use crate::shell::{parse_ifname, Shell};
use crate::World;
use kernel::{DeviceId, LinkConfig, Trace};
use labfile::{Annotation, LabFile};
use netdev::{Host, L2Switch, Router};
use std::collections::BTreeMap;
use wire::{Ipv4Addr, MacAddr};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LabKind {
    Router,
    Host,
    Switch,
}

#[derive(Clone, Debug)]
pub struct LabDevice {
    pub name: String,
    pub id: DeviceId,
    pub kind: LabKind,
    pub pos: [i32; 2],
}

/// A cable between two entries of [`BuiltLab::devices`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabLink {
    pub a: usize,
    pub b: usize,
}

#[derive(Debug)]
pub struct BuiltLab {
    pub world: World,
    pub title: String,
    pub devices: Vec<LabDevice>,
    pub links: Vec<LabLink>,
    pub annotations: Vec<Annotation>,
    /// Non-fatal problems, e.g. a config line the CLI rejected.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    UnknownTemplate { device: String, template: String },
    DuplicateDevice(String),
    TooManyDevices,
    BadHostAddress { device: String, value: String },
    BadEndpoint(String),
    UnknownDevice { link_end: String },
    UnknownPort { endpoint: String },
    PortInUse { endpoint: String },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::UnknownTemplate { device, template } => {
                write!(f, "device {device}: unknown template `{template}`")
            }
            BuildError::DuplicateDevice(d) => write!(f, "device name `{d}` is used twice"),
            BuildError::TooManyDevices => write!(f, "a lab holds at most 255 devices"),
            BuildError::BadHostAddress { device, value } => {
                write!(f, "device {device}: `{value}` is not a.b.c.d/len")
            }
            BuildError::BadEndpoint(e) => write!(f, "link endpoint `{e}` is not Device:port"),
            BuildError::UnknownDevice { link_end } => {
                write!(f, "link endpoint `{link_end}`: no such device")
            }
            BuildError::UnknownPort { endpoint } => {
                write!(f, "link endpoint `{endpoint}`: no such port")
            }
            BuildError::PortInUse { endpoint } => {
                write!(f, "link endpoint `{endpoint}` is already cabled")
            }
        }
    }
}

impl std::error::Error for BuildError {}

enum Template {
    Router(u16),
    Host,
    Switch(u16),
}

fn template(t: &str) -> Option<Template> {
    if t == "host-pc" {
        return Some(Template::Host);
    }
    let ports = |rest: &str, suffix: &str| -> Option<u16> {
        let n: u16 = rest.strip_suffix(suffix).unwrap_or(rest).parse().ok()?;
        (1..=48).contains(&n).then_some(n)
    };
    if let Some(rest) = t.strip_prefix("router-") {
        return rest
            .strip_suffix('g')
            .and_then(|n| ports(n, ""))
            .map(Template::Router);
    }
    t.strip_prefix("switch-l2-")
        .and_then(|n| ports(n, ""))
        .map(Template::Switch)
}

/// Switch ports are `Fa0/1..` (1-based), after FastEthernet.
fn switch_port(name: &str, n_ports: u16) -> Option<u16> {
    let s = name.to_ascii_lowercase();
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, rest) = s.split_at(split);
    if letters.is_empty() || !"fastethernet".starts_with(letters) {
        return None;
    }
    let n: u16 = rest.strip_prefix("0/")?.parse().ok()?;
    // Lazy: `n - 1` would underflow for `Fa0/0` if it were evaluated before the range check.
    (1..=n_ports).contains(&n).then(|| n - 1)
}

fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let (ip, len) = s.split_once('/')?;
    let mut o = [0u8; 4];
    let mut it = ip.split('.');
    for b in &mut o {
        *b = it.next()?.parse().ok()?;
    }
    if it.next().is_some() {
        return None;
    }
    let len: u8 = len.parse().ok().filter(|l| *l <= 32)?;
    Some((Ipv4Addr(o), len))
}

pub fn build(lab: &LabFile, trace: Trace) -> Result<BuiltLab, BuildError> {
    if lab.devices.len() > 255 {
        return Err(BuildError::TooManyDevices);
    }
    let mut world = World::new(trace);
    let mut devices: Vec<LabDevice> = Vec::new();
    let mut port_counts: Vec<u16> = Vec::new();
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();

    for (i, d) in lab.devices.iter().enumerate() {
        if by_name.insert(&d.name, i).is_some() {
            return Err(BuildError::DuplicateDevice(d.name.clone()));
        }
        // The index doubles as the MAC "unit", so addresses are unique and deterministic.
        let unit = i as u8;
        let t = template(&d.template).ok_or_else(|| BuildError::UnknownTemplate {
            device: d.name.clone(),
            template: d.template.clone(),
        })?;
        let (id, kind, ports) = match t {
            Template::Router(n) => (
                world.add_device(Router::new(&d.name, unit, n)),
                LabKind::Router,
                n,
            ),
            Template::Switch(n) => {
                let mut sw = L2Switch::new(n);
                sw.name = d.name.clone();
                (world.add_device(sw), LabKind::Switch, n)
            }
            Template::Host => {
                let mut h = Host::new(
                    &d.name,
                    MacAddr([2, 0, 0, 0, unit, 1]),
                    Ipv4Addr::UNSPECIFIED,
                    0,
                );
                if let Some(cfg) = &d.host {
                    let bad = |v: &str| BuildError::BadHostAddress {
                        device: d.name.clone(),
                        value: v.to_string(),
                    };
                    let (ip, len) = parse_cidr(&cfg.ipv4).ok_or_else(|| bad(&cfg.ipv4))?;
                    h.ip = ip;
                    h.prefix_len = len;
                    if let Some(g) = &cfg.gateway {
                        h.gateway = Some(parse_cidr(&format!("{g}/32")).ok_or_else(|| bad(g))?.0);
                    }
                }
                (world.add_device(h), LabKind::Host, 1)
            }
        };
        devices.push(LabDevice {
            name: d.name.clone(),
            id,
            kind,
            pos: d.pos,
        });
        port_counts.push(ports);
    }

    let mut used: BTreeMap<(usize, u16), ()> = BTreeMap::new();
    let mut links = Vec::new();
    for l in &lab.links {
        let mut ends = [(0usize, 0u16); 2];
        for (slot, raw) in [&l.a, &l.b].into_iter().enumerate() {
            let (dev, port) =
                labfile::split_endpoint(raw).ok_or_else(|| BuildError::BadEndpoint(raw.clone()))?;
            let idx = *by_name.get(dev).ok_or_else(|| BuildError::UnknownDevice {
                link_end: raw.clone(),
            })?;
            let p = match devices[idx].kind {
                LabKind::Host => (port.eq_ignore_ascii_case("eth0")).then_some(0),
                LabKind::Router => parse_ifname(port, port_counts[idx] as usize).map(|p| p as u16),
                LabKind::Switch => switch_port(port, port_counts[idx]),
            }
            .ok_or_else(|| BuildError::UnknownPort {
                endpoint: raw.clone(),
            })?;
            if used.insert((idx, p), ()).is_some() {
                return Err(BuildError::PortInUse {
                    endpoint: raw.clone(),
                });
            }
            ends[slot] = (idx, p);
        }
        world.connect(
            (devices[ends[0].0].id, ends[0].1),
            (devices[ends[1].0].id, ends[1].1),
            LinkConfig::FAST_ETHERNET,
        );
        links.push(LabLink {
            a: ends[0].0,
            b: ends[1].0,
        });
    }

    let mut warnings = Vec::new();
    for (d, dev) in lab.devices.iter().zip(&devices) {
        if dev.kind == LabKind::Host {
            continue;
        }
        if let Some(cfg) = d.running_config.as_ref().or(d.startup_config.as_ref()) {
            apply_config(&mut world, dev, cfg, &mut warnings);
        }
    }

    Ok(BuiltLab {
        world,
        title: lab.meta.title.clone(),
        devices,
        links,
        annotations: lab.annotations.clone(),
        warnings,
    })
}

/// Replay IOS-style config text through the router's CLI. Lines the CLI rejects become
/// warnings, not errors, so an old lab still opens after the grammar changes.
fn apply_config(world: &mut World, dev: &LabDevice, cfg: &str, warnings: &mut Vec<String>) {
    let mut sh = match dev.kind {
        LabKind::Switch => Shell::switch(dev.id),
        _ => Shell::router(dev.id),
    };
    for line in ["enable", "configure terminal"] {
        sh.exec(world, line);
    }
    for (n, raw) in cfg.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line == "!" || line == "end" {
            continue;
        }
        match sh.exec(world, line) {
            Some(out) if out.is_empty() => {}
            Some(out) => warnings.push(format!(
                "{} config line {}: `{line}`: {}",
                dev.name,
                n + 1,
                out.lines().last().unwrap_or("")
            )),
            None => break,
        }
    }
}

/// Load lab text and build it in one step. Load warnings are merged into the result.
pub fn load_and_build(text: &str, trace: Trace) -> Result<BuiltLab, String> {
    let loaded = labfile::load(text).map_err(|e| e.to_string())?;
    let mut built = build(&loaded.lab, trace).map_err(|e| e.to_string())?;
    let mut w = loaded.warnings;
    w.append(&mut built.warnings);
    built.warnings = w;
    Ok(built)
}
