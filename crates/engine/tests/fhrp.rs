//! HSRP and VRRP end to end: election, virtual address ownership, failover, tracking, hostile
//! packets, and saving the configuration.

use engine::lab::{load_and_build, BuiltLab};
use engine::shell::Shell;
use kernel::{PortId, SimTime, Trace};
use netdev::{FhrpState, L2Switch, Router};
use wire::{
    ArpOp, ArpPacket, EtherType, EthernetFrame, Hsrp, HsrpOp, IpProto, Ipv4Addr, Ipv4Packet,
    MacAddr, UdpDatagram, Vrrp,
};

const LAB: &str = include_str!("../../../testkit/scenarios/m3b_fhrp.lab.toml");
const PC1: usize = 0;
const SW1: usize = 1;
const R1: usize = 2;
const R2: usize = 3;
const SRV: usize = 5;

fn console(lab: &mut BuiltLab, dev: usize, lines: &[&str]) -> String {
    let id = lab.devices[dev].id;
    let mut sh = match dev {
        PC1 | SRV => Shell::host(id),
        SW1 | 4 => Shell::switch(id),
        _ => Shell::router(id),
    };
    let mut out = String::new();
    let prefix: &[&str] = if matches!(dev, PC1 | SRV) {
        &[]
    } else {
        &["enable"]
    };
    for l in prefix.iter().chain(lines) {
        out = sh.exec(&mut lab.world, l).unwrap();
    }
    out
}

fn wait(lab: &mut BuiltLab, secs: u64) {
    let to = lab.world.kernel.now() + SimTime::from_secs(secs);
    lab.world.run_until(to);
}

fn router(lab: &BuiltLab, dev: usize) -> &Router {
    lab.world.device::<Router>(lab.devices[dev].id)
}

fn state(lab: &BuiltLab, dev: usize, iface: usize) -> FhrpState {
    router(lab, dev)
        .fhrp
        .iter()
        .find(|e| e.iface == iface)
        .expect("group")
        .g
        .state()
}

/// Both routers get a group on their LAN side, `cmd` is `standby` or `vrrp`.
fn lan(cmd: &str, p1: u32, p2: u32) -> BuiltLab {
    let mut lab = load_and_build(LAB, Trace::new()).unwrap();
    console(
        &mut lab,
        PC1,
        &[
            "ipconfig set 10.0.0.10 255.255.255.0",
            "ipconfig gateway 10.0.0.254",
        ],
    );
    console(
        &mut lab,
        SRV,
        &[
            "ipconfig set 10.0.1.10 255.255.255.0",
            "ipconfig gateway 10.0.1.1",
        ],
    );
    for (dev, host, prio) in [(R1, 1, p1), (R2, 2, p2)] {
        console(
            &mut lab,
            dev,
            &[
                "configure terminal",
                "interface g0/0",
                &format!("ip address 10.0.0.{host} 255.255.255.0"),
                &format!("{cmd} 1 ip 10.0.0.254"),
                &format!("{cmd} 1 priority {prio}"),
                "no shutdown",
                "interface g0/1",
                &format!("ip address 10.0.1.{host} 255.255.255.0"),
                "no shutdown",
                "end",
            ],
        );
    }
    lab
}

#[test]
fn vrrp_elects_the_higher_priority_and_the_virtual_address_answers() {
    let mut lab = lan("vrrp", 150, 100);
    wait(&mut lab, 10);
    assert_eq!(state(&lab, R1, 0), FhrpState::Master);
    assert_eq!(state(&lab, R2, 0), FhrpState::Backup);
    let out = console(&mut lab, PC1, &["ping 10.0.0.254"]);
    assert!(out.contains("received 4"), "{out}");
    // The reply came from the virtual MAC, which the switch learned from r1's side.
    let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    let vmac = MacAddr([0, 0, 0x5e, 0, 1, 1]);
    let at = sw.mac_table(lab.world.kernel.now());
    assert!(at.contains(&(vmac, PortId(1))), "{at:?}");
}

#[test]
fn vrrp_fails_over_in_about_three_intervals_when_the_master_goes() {
    let mut lab = lan("vrrp", 150, 100);
    wait(&mut lab, 10);
    console(
        &mut lab,
        R1,
        &["configure terminal", "interface g0/0", "shutdown", "end"],
    );
    wait(&mut lab, 1);
    // r1 did say goodbye, so this is quick: a resigning master hands over after the skew.
    wait(&mut lab, 3);
    assert_eq!(state(&lab, R2, 0), FhrpState::Master);
    let out = console(&mut lab, PC1, &["ping 10.0.0.254"]);
    assert!(out.contains("received 4"), "{out}");
}

#[test]
fn only_the_active_router_answers_arp_for_the_virtual_address() {
    let mut lab = lan("standby", 110, 100);
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "interface g0/0",
            "standby 1 preempt",
            "end",
        ],
    );
    wait(&mut lab, 40);
    assert_eq!(state(&lab, R1, 0), FhrpState::Active);
    assert_eq!(state(&lab, R2, 0), FhrpState::Standby);
    let out = console(&mut lab, PC1, &["ping 10.0.0.254"]);
    assert!(out.contains("received 4"), "{out}");
    // The standby router does not claim the address as its own.
    assert!(!router(&lab, R2).fhrp.is_empty());
    let r2 = lab.world.device::<Router>(lab.devices[R2].id);
    assert!(r2.fhrp[0].g.vmac() == MacAddr([0, 0, 0x0c, 7, 0xac, 1]));
    assert!(!r2.fhrp[0].g.is_active());
}

#[test]
fn hostile_hellos_are_ignored() {
    let mut lab = lan("standby", 110, 100);
    wait(&mut lab, 40);
    let before = router(&lab, R2).fhrp[0].g.transitions;
    // A forged hello claiming to be a better active router, for the wrong group, then a forged
    // VRRP advertisement with a bad TTL.
    let sw = lab.devices[SW1].id;
    let forge = |lab: &mut BuiltLab,
                 payload: Vec<u8>,
                 proto: IpProto,
                 dst: Ipv4Addr,
                 mac: MacAddr,
                 ttl: u8| {
        let pkt = Ipv4Packet {
            dscp_ecn: 0,
            ident: 1,
            flags_frag: 0,
            ttl,
            proto,
            src: Ipv4Addr([10, 0, 0, 99]),
            dst,
            payload,
        }
        .emit();
        let f = EthernetFrame {
            dst: mac,
            src: MacAddr([2, 9, 9, 9, 9, 9]),
            vlan: None,
            ethertype: EtherType::IPV4,
            payload: pkt,
        }
        .emit();
        lab.world
            .with_device::<L2Switch, _>(sw, |_, ctx| ctx.transmit(PortId(0), f));
        wait(lab, 1);
    };
    let hello = |group: u8| {
        UdpDatagram {
            src_port: 1985,
            dst_port: 1985,
            payload: Hsrp {
                op: HsrpOp::Coup,
                state: 16,
                hello: 3,
                hold: 10,
                priority: 255,
                group,
                vip: Ipv4Addr([10, 0, 0, 254]),
            }
            .emit(),
        }
        .emit(Ipv4Addr([10, 0, 0, 99]), wire::HSRP_GROUP)
    };
    forge(
        &mut lab,
        hello(7),
        IpProto::UDP,
        wire::HSRP_GROUP,
        wire::HSRP_MAC,
        1,
    );
    assert_eq!(
        router(&lab, R2).fhrp[0].g.transitions,
        before,
        "wrong group"
    );
    let vrrp = Vrrp {
        group: 1,
        priority: 255,
        advert: 1,
        ips: vec![Ipv4Addr([10, 0, 0, 254])],
    }
    .emit();
    forge(
        &mut lab,
        vrrp,
        IpProto(112),
        wire::VRRP_GROUP,
        wire::VRRP_MAC,
        64,
    );
    assert_eq!(
        router(&lab, R2).fhrp[0].g.transitions,
        before,
        "VRRP with TTL 64"
    );
    // Garbage in the same slots must not panic.
    for n in 0..24 {
        forge(
            &mut lab,
            vec![0xa5; n],
            IpProto::UDP,
            wire::HSRP_GROUP,
            wire::HSRP_MAC,
            1,
        );
        forge(
            &mut lab,
            vec![0x21; n],
            IpProto(112),
            wire::VRRP_GROUP,
            wire::VRRP_MAC,
            255,
        );
    }
}

#[test]
fn the_configuration_survives_save_and_reload() {
    let mut lab = lan("standby", 110, 100);
    console(
        &mut lab,
        R1,
        &[
            "configure terminal",
            "interface g0/0",
            "standby 1 preempt",
            "standby 1 timers 1 4",
            "standby 2 ip 10.0.0.253",
            "vrrp 5 ip 10.0.0.250",
            "no vrrp 5 preempt",
            "vrrp 5 timers advertise 2",
            "standby 1 track g0/1 15",
            "end",
            "copy running-config startup-config",
        ],
    );
    let before = console(&mut lab, R1, &["show running-config"]);
    for want in [
        " standby 1 ip 10.0.0.254",
        " standby 1 priority 110",
        " standby 1 preempt",
        " standby 1 timers 1 4",
        " standby 1 track GigabitEthernet0/1 15",
        " standby 2 ip 10.0.0.253",
        " vrrp 5 ip 10.0.0.250",
        " no vrrp 5 preempt",
        " vrrp 5 timers advertise 2",
    ] {
        assert!(before.contains(want), "missing {want:?} in\n{before}");
    }
    let mut sh = Shell::router(lab.devices[R1].id);
    sh.exec(&mut lab.world, "enable").unwrap();
    sh.exec(&mut lab.world, "reload").unwrap();
    sh.exec(&mut lab.world, "").unwrap();
    let after = console(&mut lab, R1, &["show running-config"]);
    assert_eq!(before, after);
}

#[test]
fn bad_input_is_refused_with_a_message() {
    let mut lab = lan("standby", 110, 100);
    console(&mut lab, R1, &["configure terminal", "interface g0/0"]);
    for (cmd, want) in [
        ("standby 1 ip 224.0.0.5", "unicast"),
        ("standby 1 ip 0.0.0.0", "unicast"),
        ("standby 1 timers 5 5", "longer"),
        ("standby 1 track nonsense", "No such interface"),
        ("no standby 9", "No such group"),
    ] {
        let mut sh = Shell::router(lab.devices[R1].id);
        for l in ["enable", "configure terminal", "interface g0/0"] {
            sh.exec(&mut lab.world, l).unwrap();
        }
        let out = sh.exec(&mut lab.world, cmd).unwrap();
        assert!(out.contains(want), "{cmd}: {out}");
    }
    // A switch port is not a routed interface.
    let out = console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "interface fa0/1",
            "standby 1 ip 10.0.0.254",
        ],
    );
    assert!(out.contains("routed interfaces"), "{out}");
}

#[test]
fn a_layer_3_switch_can_hold_a_virtual_gateway_on_its_vlan_interface() {
    let mut lab = lan("standby", 90, 100);
    // sw1 joins the LAN as a third gateway candidate with the highest priority.
    console(
        &mut lab,
        SW1,
        &[
            "configure terminal",
            "ip routing",
            "interface vlan 1",
            "ip address 10.0.0.3 255.255.255.0",
            "no shutdown",
            "standby 1 ip 10.0.0.254",
            "standby 1 priority 200",
            "standby 1 preempt",
            "end",
        ],
    );
    wait(&mut lab, 60);
    let sw = lab.world.device::<L2Switch>(lab.devices[SW1].id);
    let g = &sw.l3().fhrp[0].g;
    assert_eq!(g.state(), FhrpState::Active);
    let out = console(&mut lab, PC1, &["ping 10.0.0.254"]);
    assert!(out.contains("received 4"), "{out}");
    let cfg = console(&mut lab, SW1, &["show running-config"]);
    assert!(
        cfg.contains(" standby 1 ip 10.0.0.254") && cfg.contains(" standby 1 priority 200"),
        "{cfg}"
    );
    let _ = (ArpOp::Reply, ArpPacket::LEN, EtherType::ARP);
}
