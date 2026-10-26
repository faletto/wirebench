//! Instantiate a [`labfile::LabFile`] as a running [`World`]: create devices from templates,
//! cable the links, and apply each device's saved config through the real CLI.

use crate::shell::{parse_ifname, Shell};
use crate::World;
use kernel::{DeviceId, LinkConfig, LinkId, Trace};
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
    pub ports: u16,
    /// The number its MAC addresses are built from: unique among the devices of a lab, and kept
    /// when others are removed.
    pub unit: u8,
}

/// A cable between two entries of [`BuiltLab::devices`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabLink {
    pub a: usize,
    pub b: usize,
    /// Port index on each end (0-based, as in the engine; names differ per device kind).
    pub pa: u16,
    pub pb: u16,
    /// The cable in the kernel.
    pub id: LinkId,
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
    /// The file this was built from; the static half of [`Topology`].
    pub lab: LabFile,
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

/// Create the device a lab entry describes inside `world`.
fn instantiate(
    world: &mut World,
    d: &labfile::Device,
    unit: u8,
) -> Result<(DeviceId, LabKind, u16), BuildError> {
    let t = template(&d.template).ok_or_else(|| BuildError::UnknownTemplate {
        device: d.name.clone(),
        template: d.template.clone(),
    })?;
    Ok(match t {
        Template::Router(n) => (
            world.add_device(Router::new(&d.name, unit, n)),
            LabKind::Router,
            n,
        ),
        Template::Switch(n) => {
            let mut sw = L2Switch::new(n);
            sw.name = d.name.clone();
            sw.set_unit(unit);
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
                if cfg.ipv4.trim().eq_ignore_ascii_case("dhcp") {
                    // The address comes from a server; the client starts when the lab first runs.
                    h.dhcp.enabled = true;
                } else {
                    let (ip, len) = parse_cidr(&cfg.ipv4).ok_or_else(|| bad(&cfg.ipv4))?;
                    h.ip = ip;
                    h.prefix_len = len;
                    if let Some(g) = &cfg.gateway {
                        h.gateway = Some(parse_cidr(&format!("{g}/32")).ok_or_else(|| bad(g))?.0);
                    }
                }
            }
            (world.add_device(h), LabKind::Host, 1)
        }
    })
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
        let (id, kind, ports) = instantiate(&mut world, d, i as u8)?;
        devices.push(LabDevice {
            name: d.name.clone(),
            id,
            kind,
            pos: d.pos,
            ports,
            unit: i as u8,
        });
        port_counts.push(ports);
    }

    let mut used: BTreeMap<(usize, u16), ()> = BTreeMap::new();
    let mut links = Vec::new();
    for l in &lab.links {
        let mut ends = [(0usize, 0u16); 2];
        for (slot, raw) in [&l.a, &l.b].into_iter().enumerate() {
            let (idx, p) = resolve_endpoint(&devices, raw)?;
            if used.insert((idx, p), ()).is_some() {
                return Err(BuildError::PortInUse {
                    endpoint: raw.clone(),
                });
            }
            ends[slot] = (idx, p);
        }
        let id = world.connect(
            (devices[ends[0].0].id, ends[0].1),
            (devices[ends[1].0].id, ends[1].1),
            LinkConfig::FAST_ETHERNET,
        );
        links.push(LabLink {
            a: ends[0].0,
            b: ends[1].0,
            pa: ends[0].1,
            pb: ends[1].1,
            id,
        });
    }

    let mut warnings = Vec::new();
    for (d, dev) in lab.devices.iter().zip(&devices) {
        configure_device(&mut world, dev, d, &mut warnings);
    }

    // Hosts that take their address from DHCP start asking once everything is configured.
    for dev in devices.iter().filter(|d| d.kind == LabKind::Host) {
        if world.device::<Host>(dev.id).dhcp.enabled {
            world.with_device::<Host, _>(dev.id, |h, c| h.dhcp_start(c));
        }
    }

    Ok(BuiltLab {
        world,
        title: lab.meta.title.clone(),
        devices,
        links,
        annotations: lab.annotations.clone(),
        warnings,
        lab: lab.clone(),
    })
}

/// The device (index into `devices`) and 0-based port a lab-file endpoint such as `sw1:Fa0/2`
/// names.
fn resolve_endpoint(devices: &[LabDevice], raw: &str) -> Result<(usize, u16), BuildError> {
    let (dev, port) =
        labfile::split_endpoint(raw).ok_or_else(|| BuildError::BadEndpoint(raw.to_string()))?;
    let idx =
        devices
            .iter()
            .position(|d| d.name == dev)
            .ok_or_else(|| BuildError::UnknownDevice {
                link_end: raw.to_string(),
            })?;
    let ports = devices[idx].ports;
    match devices[idx].kind {
        LabKind::Host => (port.eq_ignore_ascii_case("eth0")).then_some(0),
        LabKind::Router => parse_ifname(port, ports as usize).map(|p| p as u16),
        LabKind::Switch => switch_port(port, ports),
    }
    .map(|p| (idx, p))
    .ok_or_else(|| BuildError::UnknownPort {
        endpoint: raw.to_string(),
    })
}

/// Give a freshly made router or switch the saved and running configuration its lab entry carries.
fn configure_device(
    world: &mut World,
    dev: &LabDevice,
    d: &labfile::Device,
    warnings: &mut Vec<String>,
) {
    match dev.kind {
        LabKind::Router => {
            world.with_device::<Router, _>(dev.id, |r, _| r.startup = d.startup_config.clone())
        }
        LabKind::Switch => {
            world.with_device::<L2Switch, _>(dev.id, |s, _| s.startup = d.startup_config.clone())
        }
        LabKind::Host => return,
    }
    // The saved configuration is device state (`reload` restores it); the running one is
    // replayed over a fresh device.
    if let Some(cfg) = d.running_config.as_ref().or(d.startup_config.as_ref()) {
        apply_config(world, dev, cfg, warnings);
    }
}

/// Replay IOS-style config text through the device's CLI. Lines the CLI rejects become
/// warnings, not errors, so an old lab still opens after the grammar changes.
fn apply_config(world: &mut World, dev: &LabDevice, cfg: &str, warnings: &mut Vec<String>) {
    let mut sh = match dev.kind {
        LabKind::Switch => Shell::switch(dev.id),
        _ => Shell::router(dev.id),
    };
    for p in sh.replay(world, cfg) {
        warnings.push(format!("{} {p}", dev.name));
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

// ---- editing ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    TooManyDevices,
    SameDevice,
    NoFreePort { device: String },
    NoSuchDevice,
    NoSuchLink,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::TooManyDevices => write!(f, "a lab holds at most 255 devices"),
            EditError::SameDevice => write!(f, "cannot connect a device to itself"),
            EditError::NoFreePort { device } => write!(f, "{device} has no free port"),
            EditError::NoSuchDevice => write!(f, "no such device"),
            EditError::NoSuchLink => write!(f, "no such cable"),
        }
    }
}

impl std::error::Error for EditError {}

/// Everything about a lab except the running world: names, kinds, positions, cabling, and the
/// file it came from. Editing keeps the three in step: this struct, the `LabFile` it carries,
/// and the `World` passed to each call. Every edit happens in place: the kernel keeps the numbers of
/// removed cables and devices (nothing else is renumbered), they just stop doing anything, and
/// [`Topology::reconcile`] brings the lab to another lab's structure the same way (undo and redo).
/// [`Topology::without_device`] / [`Topology::without_link`] write what a lab would be without
/// something, for a caller that wants to build that afresh.
#[derive(Debug)]
pub struct Topology {
    pub title: String,
    pub devices: Vec<LabDevice>,
    pub links: Vec<LabLink>,
    pub annotations: Vec<Annotation>,
    lab: LabFile,
}

impl BuiltLab {
    /// Split into the running world, its topology, and load warnings.
    pub fn into_parts(self) -> (World, Topology, Vec<String>) {
        (
            self.world,
            Topology {
                title: self.title,
                devices: self.devices,
                links: self.links,
                annotations: self.annotations,
                lab: self.lab,
            },
            self.warnings,
        )
    }
}

/// Lab-file endpoint text for a port, e.g. `r1:GigabitEthernet0/1` or `sw:Fa0/2`.
fn endpoint(name: &str, kind: LabKind, port: u16) -> String {
    match kind {
        LabKind::Host => format!("{name}:eth0"),
        LabKind::Router => format!("{name}:GigabitEthernet0/{port}"),
        LabKind::Switch => format!("{name}:Fa0/{}", port + 1),
    }
}

impl Topology {
    fn free_port(&self, dev: usize) -> Option<u16> {
        (0..self.devices[dev].ports).find(|p| {
            !self
                .links
                .iter()
                .any(|l| (l.a == dev && l.pa == *p) || (l.b == dev && l.pb == *p))
        })
    }

    /// Add a device with the default template for its kind and the first unused name
    /// (`pc1`, `r1`, `sw1`, ...). Returns its index.
    pub fn add_device(
        &mut self,
        world: &mut World,
        kind: LabKind,
        pos: [i32; 2],
    ) -> Result<usize, EditError> {
        if self.devices.len() >= 255 {
            return Err(EditError::TooManyDevices);
        }
        let (prefix, template) = match kind {
            LabKind::Host => ("pc", "host-pc"),
            LabKind::Router => ("r", "router-3g"),
            LabKind::Switch => ("sw", "switch-l2-8"),
        };
        let n = (1..)
            .find(|n| {
                let name = format!("{prefix}{n}");
                !self.devices.iter().any(|d| d.name == name)
            })
            .expect("unbounded range");
        let d = labfile::Device {
            name: format!("{prefix}{n}"),
            template: template.to_string(),
            pos,
            startup_config: None,
            running_config: None,
            host: None,
        };
        // The first unit nobody has, so a removed device's number can be reused but never shared.
        let unit = (0..=255u8)
            .find(|u| !self.devices.iter().any(|d| d.unit == *u))
            .expect("fewer than 256 devices");
        let (id, kind, ports) = instantiate(world, &d, unit).expect("default templates are valid");
        self.devices.push(LabDevice {
            name: d.name.clone(),
            id,
            kind,
            pos,
            ports,
            unit,
        });
        self.lab.devices.push(d);
        Ok(self.devices.len() - 1)
    }

    pub fn move_device(&mut self, dev: usize, pos: [i32; 2]) {
        if let Some(d) = self.devices.get_mut(dev) {
            d.pos = pos;
            self.lab.devices[dev].pos = pos;
        }
    }

    /// Cable the first free port on each device. Returns the new link's index.
    pub fn add_link(&mut self, world: &mut World, a: usize, b: usize) -> Result<usize, EditError> {
        if a >= self.devices.len() || b >= self.devices.len() {
            return Err(EditError::NoSuchDevice);
        }
        if a == b {
            return Err(EditError::SameDevice);
        }
        let no_port = |i: usize| EditError::NoFreePort {
            device: self.devices[i].name.clone(),
        };
        let pa = self.free_port(a).ok_or_else(|| no_port(a))?;
        let pb = self.free_port(b).ok_or_else(|| no_port(b))?;
        let id = world.connect(
            (self.devices[a].id, pa),
            (self.devices[b].id, pb),
            LinkConfig::FAST_ETHERNET,
        );
        self.lab.links.push(labfile::Link {
            a: endpoint(&self.devices[a].name, self.devices[a].kind, pa),
            b: endpoint(&self.devices[b].name, self.devices[b].kind, pb),
            medium: None,
        });
        self.links.push(LabLink { a, b, pa, pb, id });
        Ok(self.links.len() - 1)
    }

    /// The lab as it stands: structure and positions from here, live state from `world`
    /// (router and switch configs, host addressing). Saving this and loading it again gives
    /// the same lab.
    pub fn snapshot(&self, world: &World) -> LabFile {
        let mut lab = self.lab.clone();
        for (d, entry) in self.devices.iter().zip(&mut lab.devices) {
            entry.pos = d.pos;
            let config = match d.kind {
                LabKind::Router => Some(crate::shell::running_config(world.device::<Router>(d.id))),
                LabKind::Switch => {
                    Some(crate::shell::switch_config(world.device::<L2Switch>(d.id)))
                }
                LabKind::Host => None,
            };
            // What `copy running-config startup-config` saved travels with the lab.
            match d.kind {
                LabKind::Router => {
                    entry.startup_config = world.device::<Router>(d.id).startup.clone()
                }
                LabKind::Switch => {
                    entry.startup_config = world.device::<L2Switch>(d.id).startup.clone()
                }
                LabKind::Host => {}
            }
            if let Some(cfg) = config {
                // Only record a running config when it differs from the startup config.
                let same = entry
                    .startup_config
                    .as_deref()
                    .is_some_and(|s| s.trim() == cfg.trim());
                entry.running_config = (!same).then_some(cfg);
            }
            if d.kind == LabKind::Host {
                let h = world.device::<Host>(d.id);
                if h.dhcp.enabled {
                    // The lease is not saved, only the fact that this PC asks for one.
                    entry.host = Some(labfile::HostConfig {
                        ipv4: "dhcp".to_string(),
                        gateway: None,
                        dns: Vec::new(),
                    });
                } else if h.ip != Ipv4Addr::UNSPECIFIED {
                    let dns = entry
                        .host
                        .as_ref()
                        .map(|c| c.dns.clone())
                        .unwrap_or_default();
                    entry.host = Some(labfile::HostConfig {
                        ipv4: format!("{}/{}", h.ip, h.prefix_len),
                        gateway: h.gateway.map(|g| g.to_string()),
                        dns,
                    });
                }
            }
        }
        lab
    }

    /// Take cable `link` away in place: the world keeps running, the two ports go down.
    pub fn remove_link(&mut self, world: &mut World, link: usize) -> Result<(), EditError> {
        if link >= self.links.len() {
            return Err(EditError::NoSuchLink);
        }
        let l = self.links.remove(link);
        self.lab.links.remove(link);
        world.kernel.remove_link(l.id);
        Ok(())
    }

    /// Take device `dev` and its cables away in place. Later devices move up one place; the
    /// device ids everything else uses do not change.
    pub fn remove_device(&mut self, world: &mut World, dev: usize) -> Result<(), EditError> {
        if dev >= self.devices.len() {
            return Err(EditError::NoSuchDevice);
        }
        while let Some(i) = self.links.iter().position(|l| l.a == dev || l.b == dev) {
            self.remove_link(world, i)?;
        }
        for l in &mut self.links {
            if l.a > dev {
                l.a -= 1;
            }
            if l.b > dev {
                l.b -= 1;
            }
        }
        let gone = self.devices.remove(dev);
        self.lab.devices.remove(dev);
        world.kernel.retire_device(gone.id);
        Ok(())
    }

    /// Bring the running lab to the structure of `target` without starting over: devices and cables
    /// that are not in it are removed, ones it has that are missing are added (with the
    /// configuration it carries), positions follow it, and the order of devices and cables becomes
    /// its order. Devices in both keep everything they have learned and been configured with, and the
    /// clock does not move. This is what undo and redo do. A device with the same name but a
    /// different template counts as a different device. Returns the ids of the devices removed. If
    /// `target` would not build, nothing changes.
    pub fn reconcile(
        &mut self,
        world: &mut World,
        target: &LabFile,
    ) -> Result<Vec<DeviceId>, String> {
        build(target, Trace::new()).map_err(|e| e.to_string())?;
        // The target's devices and cabling, by name and port, which is what identifies them.
        let tdevs: Vec<LabDevice> = target
            .devices
            .iter()
            .map(|d| {
                let (kind, ports) = match template(&d.template) {
                    Some(Template::Host) => (LabKind::Host, 1),
                    Some(Template::Router(n)) => (LabKind::Router, n),
                    Some(Template::Switch(n)) => (LabKind::Switch, n),
                    None => unreachable!("the target built"),
                };
                LabDevice {
                    name: d.name.clone(),
                    id: DeviceId(0),
                    kind,
                    pos: d.pos,
                    ports,
                    unit: 0,
                }
            })
            .collect();
        let key = |na: &str, pa: u16, nb: &str, pb: u16| {
            let (x, y) = ((na.to_string(), pa), (nb.to_string(), pb));
            if x <= y {
                (x, y)
            } else {
                (y, x)
            }
        };
        let mut tkeys = Vec::new();
        for l in &target.links {
            let (a, pa) = resolve_endpoint(&tdevs, &l.a).map_err(|e| e.to_string())?;
            let (b, pb) = resolve_endpoint(&tdevs, &l.b).map_err(|e| e.to_string())?;
            tkeys.push(key(&tdevs[a].name, pa, &tdevs[b].name, pb));
        }
        let same_device = |i: usize, this: &Topology| {
            target.devices.iter().any(|t| {
                t.name == this.devices[i].name && t.template == this.lab.devices[i].template
            })
        };
        let ckey = |this: &Topology, l: &LabLink| {
            key(&this.devices[l.a].name, l.pa, &this.devices[l.b].name, l.pb)
        };
        // Out with the cables and devices the target does not have.
        for i in (0..self.links.len()).rev() {
            let k = ckey(self, &self.links[i]);
            if !tkeys.contains(&k) {
                self.remove_link(world, i).map_err(|e| e.to_string())?;
            }
        }
        let mut removed = Vec::new();
        for i in (0..self.devices.len()).rev() {
            if !same_device(i, self) {
                removed.push(self.devices[i].id);
                self.remove_device(world, i).map_err(|e| e.to_string())?;
            }
        }
        // In with the ones it adds.
        let mut warnings = Vec::new();
        for d in &target.devices {
            if self.devices.iter().any(|x| x.name == d.name) {
                continue;
            }
            let unit = (0..=255u8)
                .find(|u| !self.devices.iter().any(|d| d.unit == *u))
                .ok_or("a lab holds at most 255 devices")?;
            let (id, kind, ports) = instantiate(world, d, unit).map_err(|e| e.to_string())?;
            let dev = LabDevice {
                name: d.name.clone(),
                id,
                kind,
                pos: d.pos,
                ports,
                unit,
            };
            configure_device(world, &dev, d, &mut warnings);
            if kind == LabKind::Host && world.device::<Host>(id).dhcp.enabled {
                world.with_device::<Host, _>(id, |h, c| h.dhcp_start(c));
            }
            self.devices.push(dev);
            self.lab.devices.push(d.clone());
        }
        let have: Vec<_> = self.links.iter().map(|l| ckey(self, l)).collect();
        for (l, k) in target.links.iter().zip(&tkeys) {
            if have.contains(k) {
                continue;
            }
            let (a, pa) = resolve_endpoint(&self.devices, &l.a).map_err(|e| e.to_string())?;
            let (b, pb) = resolve_endpoint(&self.devices, &l.b).map_err(|e| e.to_string())?;
            let id = world.connect(
                (self.devices[a].id, pa),
                (self.devices[b].id, pb),
                LinkConfig::FAST_ETHERNET,
            );
            self.links.push(LabLink { a, b, pa, pb, id });
            self.lab.links.push(l.clone());
        }
        // The target's order, and its positions.
        let order: Vec<usize> = target
            .devices
            .iter()
            .map(|t| self.devices.iter().position(|d| d.name == t.name))
            .collect::<Option<_>>()
            .ok_or("device lists disagree")?;
        let mut place = vec![0; order.len()];
        for (new, old) in order.iter().enumerate() {
            place[*old] = new;
        }
        let mut devices: Vec<LabDevice> = order.iter().map(|&i| self.devices[i].clone()).collect();
        for (d, t) in devices.iter_mut().zip(&target.devices) {
            d.pos = t.pos;
        }
        for l in &mut self.links {
            l.a = place[l.a];
            l.b = place[l.b];
        }
        self.devices = devices;
        let mut links = Vec::new();
        for k in &tkeys {
            let i = self
                .links
                .iter()
                .position(|l| ckey(self, l) == *k)
                .ok_or("cable lists disagree")?;
            links.push(self.links[i]);
        }
        self.links = links;
        self.title = target.meta.title.clone();
        self.annotations = target.annotations.clone();
        self.lab = target.clone();
        Ok(removed)
    }

    /// [`Topology::snapshot`] without device `dev` and every cable attached to it.
    pub fn without_device(&self, world: &World, dev: usize) -> Result<LabFile, EditError> {
        let name = &self.devices.get(dev).ok_or(EditError::NoSuchDevice)?.name;
        let mut lab = self.snapshot(world);
        lab.devices.remove(dev);
        lab.links.retain(|l| {
            let on = |raw: &str| labfile::split_endpoint(raw).is_some_and(|(d, _)| d == name);
            !on(&l.a) && !on(&l.b)
        });
        Ok(lab)
    }

    /// [`Topology::snapshot`] without cable `link`.
    pub fn without_link(&self, world: &World, link: usize) -> Result<LabFile, EditError> {
        if link >= self.links.len() {
            return Err(EditError::NoSuchLink);
        }
        let mut lab = self.snapshot(world);
        lab.links.remove(link);
        Ok(lab)
    }
}
