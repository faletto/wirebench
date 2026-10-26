//! The device settings panel's non-drawing parts: turning what was typed into CLI lines. The
//! lines go to the engine's `Configure`, which runs them through the device's own shell, so the
//! panel can only do what the console can.

use crate::sim::parse_ipv4;

/// `a.b.c.d/len`, with `len` 0 to 32.
pub fn parse_cidr(text: &str) -> Option<([u8; 4], u8)> {
    let (ip, len) = text.trim().split_once('/')?;
    let len: u8 = len.trim().parse().ok().filter(|l| *l <= 32)?;
    Some((parse_ipv4(ip)?, len))
}

/// The dotted mask for a prefix length: 24 gives `255.255.255.0`.
pub fn dotted_mask(len: u8) -> String {
    let m = if len == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(len.min(32)))
    };
    let b = m.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn dotted(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// A PC's address, gateway and DHCP setting. `addr` is `a.b.c.d/len`; empty leaves it alone.
pub fn host_lines(addr: &str, gateway: &str, dhcp: bool) -> Result<Vec<String>, String> {
    if dhcp {
        return Ok(vec!["ipconfig dhcp".into()]);
    }
    let mut out = vec![];
    if !addr.trim().is_empty() {
        let (ip, len) = parse_cidr(addr).ok_or("address must look like 10.0.0.5/24")?;
        out.push(format!("ipconfig set {} {}", dotted(ip), dotted_mask(len)));
    }
    if !gateway.trim().is_empty() {
        let gw = parse_ipv4(gateway).ok_or("gateway must look like 10.0.0.1")?;
        out.push(format!("ipconfig gateway {}", dotted(gw)));
    }
    Ok(out)
}

/// A router interface: its address (empty removes it) and whether it is shut down.
pub fn interface_lines(cli: &str, addr: &str, shutdown: bool) -> Result<Vec<String>, String> {
    let mut out = vec![format!("interface {cli}")];
    if addr.trim().is_empty() {
        out.push("no ip address".into());
    } else {
        let (ip, len) = parse_cidr(addr).ok_or("address must look like 10.0.0.1/24")?;
        out.push(format!("ip address {} {}", dotted(ip), dotted_mask(len)));
    }
    out.push(if shutdown { "shutdown" } else { "no shutdown" }.into());
    Ok(out)
}

/// A static route to add (`add`) or remove: `net` is `a.b.c.d/len`.
pub fn route_lines(net: &str, via: &str, add: bool) -> Result<Vec<String>, String> {
    let (n, len) = parse_cidr(net).ok_or("network must look like 10.0.2.0/24")?;
    let v = parse_ipv4(via).ok_or("next hop must look like 10.0.12.2")?;
    Ok(vec![format!(
        "{}ip route {} {} {}",
        if add { "" } else { "no " },
        dotted(n),
        dotted_mask(len),
        dotted(v)
    )])
}

/// A switch port's mode, VLAN, shutdown and portfast, as lines for `interface <cli>`.
pub fn port_lines(
    cli: &str,
    mode: &str,
    access_vlan: u16,
    shutdown: bool,
    portfast: bool,
) -> Vec<String> {
    let mut out = vec![format!("interface {cli}")];
    out.push(
        match mode {
            "access" => "switchport mode access",
            "trunk" => "switchport mode trunk",
            "dynamic desirable" => "switchport mode dynamic desirable",
            _ => "switchport mode dynamic auto",
        }
        .into(),
    );
    out.push(format!("switchport access vlan {access_vlan}"));
    out.push(if shutdown { "shutdown" } else { "no shutdown" }.into());
    out.push(
        if portfast {
            "spanning-tree portfast"
        } else {
            "no spanning-tree portfast"
        }
        .into(),
    );
    out
}

/// A VLAN to create (with a name) or delete.
pub fn vlan_lines(id: u16, name: &str, add: bool) -> Vec<String> {
    if !add {
        return vec![format!("no vlan {id}")];
    }
    let mut out = vec![format!("vlan {id}")];
    let name = name.trim();
    if !name.is_empty() {
        out.push(format!("name {}", name.replace(' ', "_")));
    }
    out.push("exit".into());
    out
}

/// Spanning tree mode: `off`, `pvst` or `rapid-pvst`.
pub fn spanning_lines(mode: &str) -> Vec<String> {
    vec![match mode {
        "pvst" => "spanning-tree mode pvst".to_string(),
        "rapid-pvst" => "spanning-tree mode rapid-pvst".to_string(),
        _ => "no spanning-tree mode".to_string(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_and_masks() {
        assert_eq!(parse_cidr(" 10.0.1.9/24 "), Some(([10, 0, 1, 9], 24)));
        for bad in [
            "10.0.1.9",
            "10.0.1.9/33",
            "10.0.1/24",
            "x/24",
            "10.0.1.9/",
            "10.0.1.9/-1",
        ] {
            assert_eq!(parse_cidr(bad), None, "{bad}");
        }
        assert_eq!(dotted_mask(24), "255.255.255.0");
        assert_eq!(dotted_mask(30), "255.255.255.252");
        assert_eq!(dotted_mask(0), "0.0.0.0");
        assert_eq!(dotted_mask(32), "255.255.255.255");
    }

    #[test]
    fn host_settings_become_ipconfig_lines() {
        assert_eq!(
            host_lines("10.0.1.9/24", "10.0.1.1", false).unwrap(),
            [
                "ipconfig set 10.0.1.9 255.255.255.0",
                "ipconfig gateway 10.0.1.1"
            ]
        );
        assert_eq!(host_lines("", "", false).unwrap(), Vec::<String>::new());
        assert_eq!(host_lines("garbage", "", true).unwrap(), ["ipconfig dhcp"]);
        assert!(host_lines("10.0.1.9", "", false).is_err());
        assert!(host_lines("10.0.1.9/24", "gw", false).is_err());
    }

    #[test]
    fn interface_and_route_lines() {
        assert_eq!(
            interface_lines("g0/1", "10.0.12.1/30", false).unwrap(),
            [
                "interface g0/1",
                "ip address 10.0.12.1 255.255.255.252",
                "no shutdown"
            ]
        );
        assert_eq!(
            interface_lines("g0/1.10", "", true).unwrap(),
            ["interface g0/1.10", "no ip address", "shutdown"]
        );
        assert!(interface_lines("g0/1", "10.0.12.1", false).is_err());
        assert_eq!(
            route_lines("10.0.2.0/24", "10.0.12.2", true).unwrap(),
            ["ip route 10.0.2.0 255.255.255.0 10.0.12.2"]
        );
        assert_eq!(
            route_lines("10.0.2.0/24", "10.0.12.2", false).unwrap(),
            ["no ip route 10.0.2.0 255.255.255.0 10.0.12.2"]
        );
        assert!(route_lines("10.0.2.0", "10.0.12.2", true).is_err());
    }

    #[test]
    fn switch_lines() {
        assert_eq!(
            port_lines("fa0/3", "access", 10, false, true),
            [
                "interface fa0/3",
                "switchport mode access",
                "switchport access vlan 10",
                "no shutdown",
                "spanning-tree portfast"
            ]
        );
        assert_eq!(
            port_lines("fa0/1", "trunk", 1, true, false)[1],
            "switchport mode trunk"
        );
        assert_eq!(
            port_lines("fa0/1", "whatever", 1, true, false)[1],
            "switchport mode dynamic auto"
        );
        assert_eq!(
            vlan_lines(10, "sales team", true),
            ["vlan 10", "name sales_team", "exit"]
        );
        assert_eq!(vlan_lines(10, " ", true), ["vlan 10", "exit"]);
        assert_eq!(vlan_lines(10, "", false), ["no vlan 10"]);
        assert_eq!(spanning_lines("off"), ["no spanning-tree mode"]);
        assert_eq!(
            spanning_lines("rapid-pvst"),
            ["spanning-tree mode rapid-pvst"]
        );
    }
}

use engine::panel::DeviceState;
use std::collections::BTreeMap;

/// What the settings panel holds between frames: the device it shows, the state the engine last
/// reported, what the person has typed but not applied, and what the CLI said about the last change.
#[derive(Debug, Default)]
pub struct Panel {
    pub device: Option<u32>,
    pub state: Option<DeviceState>,
    /// The device may have changed behind the panel's back (a console command): ask again.
    pub stale: bool,
    pub said: Vec<String>,
    pub host_addr: String,
    pub host_gw: String,
    pub if_addr: BTreeMap<String, String>,
    pub route_net: String,
    pub route_via: String,
    pub vlan_id: String,
    pub vlan_name: String,
}

impl Panel {
    /// Fill the text boxes from a freshly read state.
    pub fn load_drafts(&mut self) {
        self.if_addr.clear();
        match &self.state {
            Some(DeviceState::Host(h)) => {
                self.host_addr = if h.ip.is_empty() {
                    String::new()
                } else {
                    format!("{}/{}", h.ip, h.prefix)
                };
                self.host_gw = h.gateway.clone();
            }
            Some(DeviceState::Router(r)) => {
                for i in &r.interfaces {
                    self.if_addr.insert(i.cli.clone(), i.addr.clone());
                }
            }
            _ => {}
        }
    }
}
