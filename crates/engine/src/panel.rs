//! What a device's settings panel shows, and how a panel changes a device. The panel reads a
//! structured [`DeviceState`] and sends back CLI lines; the lines run through the same shell a
//! console uses, so there is one set of rules and a change made in a panel is a change made at
//! the console (it shows in `show running-config`, and is refused for the same reasons).

use crate::protocol::ShellKind;
use crate::shell::{iface_name, Shell};
use crate::World;
use kernel::{DeviceId, PortId};
use netdev::{Host, L2Switch, PortMode, Router, StpMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum DeviceState {
    Host(HostState),
    Router(RouterState),
    Switch(SwitchState),
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HostState {
    pub name: String,
    /// Empty when the PC has no address.
    pub ip: String,
    pub prefix: u8,
    pub gateway: String,
    /// The PC asks a DHCP server for its address.
    pub dhcp: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RouterState {
    pub name: String,
    pub interfaces: Vec<InterfaceState>,
    pub routes: Vec<RouteState>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct InterfaceState {
    /// `GigabitEthernet0/1` or `GigabitEthernet0/1.10`.
    pub name: String,
    /// What to type after `interface`: `g0/1`.
    pub cli: String,
    /// `10.0.0.1/24`, or empty.
    pub addr: String,
    pub shutdown: bool,
    pub up: bool,
    pub sub: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RouteState {
    /// `10.0.2.0/24`
    pub net: String,
    pub via: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SwitchState {
    pub name: String,
    pub vlans: Vec<(u16, String)>,
    pub ports: Vec<PortState>,
    /// `pvst`, `rapid-pvst`, or `None` while spanning tree is off.
    pub spanning_tree: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PortState {
    /// `Fa0/1`
    pub name: String,
    /// What to type after `interface`: `fa0/1`.
    pub cli: String,
    /// `access`, `trunk`, `dynamic auto` or `dynamic desirable`.
    pub mode: String,
    pub access_vlan: u16,
    /// Trunking right now (configured, or negotiated).
    pub trunking: bool,
    pub shutdown: bool,
    pub portfast: bool,
}

pub fn state(world: &World, kind: ShellKind, id: DeviceId) -> DeviceState {
    match kind {
        ShellKind::Host => {
            let h = world.device::<Host>(id);
            DeviceState::Host(HostState {
                name: h.name.clone(),
                ip: if h.ip == wire::Ipv4Addr::UNSPECIFIED {
                    String::new()
                } else {
                    h.ip.to_string()
                },
                prefix: h.prefix_len,
                gateway: h.gateway.map(|g| g.to_string()).unwrap_or_default(),
                dhcp: h.dhcp.enabled,
            })
        }
        ShellKind::Router => {
            let r = world.device::<Router>(id);
            let interfaces = (0..r.interfaces().len())
                .filter(|i| r.interfaces()[*i].svi.is_none())
                .map(|i| {
                    let ifc = &r.interfaces()[i];
                    let name = iface_name(r, i);
                    InterfaceState {
                        cli: name.replace("GigabitEthernet", "g"),
                        name,
                        addr: ifc
                            .addr
                            .map(|(a, l)| format!("{a}/{l}"))
                            .unwrap_or_default(),
                        shutdown: ifc.shutdown,
                        up: r.is_up(i),
                        sub: ifc.sub.is_some(),
                    }
                })
                .collect();
            let routes = r
                .statics()
                .iter()
                .map(|s| RouteState {
                    net: format!("{}/{}", s.net, s.len),
                    via: s.via.to_string(),
                })
                .collect();
            DeviceState::Router(RouterState {
                name: r.name.clone(),
                interfaces,
                routes,
            })
        }
        ShellKind::Switch => {
            let s = world.device::<L2Switch>(id);
            let ports = (0..s.n_ports())
                .map(|p| {
                    let c = s.port(PortId(p)).expect("port in range");
                    PortState {
                        name: format!("Fa0/{}", p + 1),
                        cli: format!("fa0/{}", p + 1),
                        mode: match c.mode {
                            PortMode::Access => "access",
                            PortMode::Trunk => "trunk",
                            PortMode::DynamicAuto => "dynamic auto",
                            PortMode::DynamicDesirable => "dynamic desirable",
                        }
                        .to_string(),
                        access_vlan: c.access_vlan,
                        trunking: s.is_trunk(PortId(p)),
                        shutdown: c.shutdown,
                        portfast: c.portfast,
                    }
                })
                .collect();
            DeviceState::Switch(SwitchState {
                name: s.name.clone(),
                vlans: s.vlans().map(|(id, n)| (id, n.to_string())).collect(),
                ports,
                spanning_tree: s.spanning().mode.map(|m| {
                    match m {
                        StpMode::Stp => "pvst",
                        StpMode::Rapid => "rapid-pvst",
                    }
                    .to_string()
                }),
            })
        }
    }
}

/// Run `lines` as a person would type them: for a router or switch, in configuration mode; for a
/// PC, at its prompt. Returns what the CLI said about lines it refused or commented on, each with
/// the line it was about. A refused line does not stop the ones after it.
pub fn configure(
    world: &mut World,
    kind: ShellKind,
    id: DeviceId,
    lines: &[String],
) -> Vec<String> {
    let mut sh = match kind {
        ShellKind::Host => Shell::host(id),
        ShellKind::Router => Shell::router(id),
        ShellKind::Switch => Shell::switch(id),
    };
    let mut said = vec![];
    // A device with a console password would ask for it; a panel is the owner, not a login.
    sh.enter_config();
    for line in lines {
        if let Some(out) = sh.exec(world, line) {
            let out = out.trim();
            if !out.is_empty() {
                said.push(format!("{line}: {}", out.lines().last().unwrap_or("")));
            }
        }
    }
    said
}
