//! Device settings as the panels use them: structured state out, CLI lines in, over bytes.

use engine::panel::DeviceState;
use engine::protocol::{decode, encode, Engine, EngineCmd, EngineEvent, LabView};
use kernel::Trace;

const ROUTERS: &str = include_str!("../../../labs/m1_static_routes.lab.toml");
const SWITCHES: &str = include_str!("../../../testkit/scenarios/m2_stp.lab.toml");

struct Rig {
    e: Engine,
    view: LabView,
}

impl Rig {
    fn new(lab: &str) -> Rig {
        let mut e = Engine::new(engine::World::new(Trace::new()));
        let out = e.handle_bytes(&encode(&EngineCmd::Load { lab: lab.into() }));
        let view = match decode::<Vec<EngineEvent>>(&out).unwrap().remove(0) {
            EngineEvent::Lab { view } => view,
            other => panic!("{other:?}"),
        };
        Rig { e, view }
    }

    fn id(&self, name: &str) -> u32 {
        self.view
            .devices
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .device
    }

    fn send(&mut self, cmd: EngineCmd) -> EngineEvent {
        decode::<Vec<EngineEvent>>(&self.e.handle_bytes(&encode(&cmd)))
            .unwrap()
            .remove(0)
    }

    fn state(&mut self, name: &str) -> DeviceState {
        let device = self.id(name);
        match self.send(EngineCmd::DeviceState { device }) {
            EngineEvent::State { state, .. } => state,
            other => panic!("{other:?}"),
        }
    }

    fn configure(&mut self, name: &str, lines: &[&str]) -> Vec<String> {
        let device = self.id(name);
        match self.send(EngineCmd::Configure {
            device,
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }) {
            EngineEvent::Configured { said, .. } => said,
            other => panic!("{other:?}"),
        }
    }

    fn running(&mut self) -> String {
        match self.send(EngineCmd::Snapshot) {
            EngineEvent::LabText { text } => text,
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_pc_panel_sets_address_gateway_and_dhcp() {
    let mut r = Rig::new(ROUTERS);
    assert!(r
        .configure(
            "pc1",
            &[
                "ipconfig set 10.0.1.9 255.255.255.0",
                "ipconfig gateway 10.0.1.1"
            ]
        )
        .is_empty());
    let DeviceState::Host(h) = r.state("pc1") else {
        panic!()
    };
    assert_eq!(
        (h.ip.as_str(), h.prefix, h.gateway.as_str(), h.dhcp),
        ("10.0.1.9", 24, "10.0.1.1", false)
    );
    // Starting DHCP prints what it is doing, which the panel passes on as a note.
    let said = r.configure("pc1", &["ipconfig dhcp"]);
    assert!(said.iter().all(|l| l.starts_with("ipconfig dhcp: ")), "{said:?}");
    let DeviceState::Host(h) = r.state("pc1") else {
        panic!()
    };
    assert!(h.dhcp);
    // A bad address is reported against its line and the PC keeps what it had.
    let said = r.configure("pc1", &["ipconfig set 10.0.1.999 255.255.255.0"]);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("ipconfig set 10.0.1.999 255.255.255.0: "),
        "{said:?}"
    );
}

#[test]
fn a_router_panel_sets_interfaces_and_routes() {
    let mut r = Rig::new(ROUTERS);
    let said = r.configure(
        "r1",
        &[
            "interface g0/0",
            "ip address 10.0.1.1 255.255.255.0",
            "no shutdown",
            "exit",
            "ip route 10.9.0.0 255.255.0.0 10.0.12.2",
        ],
    );
    assert!(said.is_empty(), "{said:?}");
    let DeviceState::Router(s) = r.state("r1") else {
        panic!()
    };
    let g0 = s.interfaces.iter().find(|i| i.cli == "g0/0").unwrap();
    assert_eq!((g0.addr.as_str(), g0.shutdown), ("10.0.1.1/24", false));
    assert!(s
        .routes
        .iter()
        .any(|x| x.net == "10.9.0.0/16" && x.via == "10.0.12.2"));
    r.configure("r1", &["interface g0/0", "shutdown"]);
    let DeviceState::Router(s) = r.state("r1") else {
        panic!()
    };
    let g0 = s.interfaces.iter().find(|i| i.cli == "g0/0").unwrap();
    assert!(g0.shutdown && !g0.up);
    r.configure("r1", &["no ip route 10.9.0.0 255.255.0.0 10.0.12.2"]);
    let DeviceState::Router(s) = r.state("r1") else {
        panic!()
    };
    assert!(s.routes.iter().all(|x| x.net != "10.9.0.0/16"));
}

#[test]
fn a_refused_line_is_reported_and_the_rest_still_run() {
    let mut r = Rig::new(ROUTERS);
    let said = r.configure(
        "r1",
        &[
            "interface g0/0",
            "ip address 300.1.1.1 255.255.255.0",
            "ip address 10.5.5.1 255.255.255.0",
        ],
    );
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("ip address 300.1.1.1 255.255.255.0: "));
    let DeviceState::Router(s) = r.state("r1") else {
        panic!()
    };
    assert_eq!(
        s.interfaces.iter().find(|i| i.cli == "g0/0").unwrap().addr,
        "10.5.5.1/24"
    );
}

#[test]
fn a_switch_panel_sets_vlans_ports_and_spanning_tree() {
    let mut r = Rig::new(SWITCHES);
    let said = r.configure(
        "sw1",
        &[
            "vlan 10",
            "name sales",
            "exit",
            "interface fa0/3",
            "switchport mode access",
            "switchport access vlan 10",
            "spanning-tree portfast",
            "exit",
            "interface fa0/1",
            "switchport mode trunk",
            "exit",
            "spanning-tree mode rapid-pvst",
        ],
    );
    assert!(said.is_empty(), "{said:?}");
    let DeviceState::Switch(s) = r.state("sw1") else {
        panic!()
    };
    assert!(s.vlans.contains(&(10, "sales".to_string())));
    let p3 = &s.ports[2];
    assert_eq!(
        (p3.mode.as_str(), p3.access_vlan, p3.portfast, p3.trunking),
        ("access", 10, true, false)
    );
    assert_eq!(s.ports[0].mode, "trunk");
    assert!(s.ports[0].trunking);
    assert_eq!(s.ports[3].mode, "dynamic auto");
    assert_eq!(s.spanning_tree.as_deref(), Some("rapid-pvst"));
    r.configure(
        "sw1",
        &["no spanning-tree mode", "interface fa0/3", "shutdown"],
    );
    let DeviceState::Switch(s) = r.state("sw1") else {
        panic!()
    };
    assert!(s.spanning_tree.is_none() && s.ports[2].shutdown);
    // Everything done in the panel is in the saved lab like any console change.
    let text = r.running();
    assert!(text.contains("switchport access vlan 10"), "{text}");
}

#[test]
fn a_device_with_console_passwords_can_still_be_set_up_from_its_panel() {
    let mut r = Rig::new(SWITCHES);
    assert!(r
        .configure(
            "sw1",
            &[
                "enable secret hunter2",
                "line console 0",
                "password lab",
                "login"
            ]
        )
        .is_empty());
    // The console would now ask for both; the panel is not a login.
    let said = r.configure("sw1", &["vlan 20", "name eng"]);
    assert!(said.is_empty(), "{said:?}");
    let DeviceState::Switch(s) = r.state("sw1") else {
        panic!()
    };
    assert!(s.vlans.iter().any(|(v, n)| *v == 20 && n == "eng"));
}

#[test]
fn unknown_devices_are_refused() {
    let mut r = Rig::new(ROUTERS);
    assert!(matches!(
        r.send(EngineCmd::DeviceState { device: 999 }),
        EngineEvent::Error { .. }
    ));
    assert!(matches!(
        r.send(EngineCmd::Configure {
            device: 999,
            lines: vec![]
        }),
        EngineEvent::Error { .. }
    ));
}
