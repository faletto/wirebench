use engine::lab::{build, load_and_build, EditError, LabKind, Topology};
use engine::shell::Shell;
use engine::World;
use kernel::Trace;
use netdev::Host;
use wire::Ipv4Addr;

const EMPTY: &str = "format = \"labsim-lab\"\nversion = 1\n[meta]\ntitle = \"scratch\"\n";

fn scratch() -> (World, Topology) {
    let (w, t, warn) = load_and_build(EMPTY, Trace::new()).unwrap().into_parts();
    assert!(warn.is_empty());
    (w, t)
}

fn text(t: &Topology, w: &World) -> String {
    labfile::save(&t.snapshot(w))
}

/// pc1 - sw1 - r1 - pc2 built by edits only, addressed via the real shells.
fn four_node() -> (World, Topology) {
    let (mut w, mut t) = scratch();
    let pc1 = t.add_device(&mut w, LabKind::Host, [0, 0]).unwrap();
    let sw = t.add_device(&mut w, LabKind::Switch, [100, 0]).unwrap();
    let r1 = t.add_device(&mut w, LabKind::Router, [200, 0]).unwrap();
    let pc2 = t.add_device(&mut w, LabKind::Host, [300, 0]).unwrap();
    for (a, b) in [(pc1, sw), (sw, r1), (r1, pc2)] {
        t.add_link(&mut w, a, b).unwrap();
    }
    let mut sh = Shell::router(t.devices[r1].id);
    for l in [
        "enable",
        "configure terminal",
        "interface g0/0",
        "ip address 10.0.1.1 255.255.255.0",
        "no shutdown",
        "interface g0/1",
        "ip address 10.0.2.1 255.255.255.0",
        "no shutdown",
        "end",
    ] {
        sh.exec(&mut w, l).unwrap();
    }
    for (i, ip, gw) in [(pc1, "10.0.1.2", "10.0.1.1"), (pc2, "10.0.2.2", "10.0.2.1")] {
        let mut h = Shell::host(t.devices[i].id);
        h.exec(&mut w, &format!("ipconfig set {ip} 255.255.255.0"))
            .unwrap();
        h.exec(&mut w, &format!("ipconfig gateway {gw}")).unwrap();
    }
    (w, t)
}

fn ping_twice(w: &mut World, t: &Topology, from: usize, dst: [u8; 4]) -> usize {
    let id = t.devices[from].id;
    for _ in 0..2 {
        w.with_device::<Host, _>(id, |h, c| h.ping(c, Ipv4Addr(dst)));
        w.run_to_idle();
    }
    w.device::<Host>(id).ping_results.len()
}

#[test]
fn names_are_generated_and_reused() {
    let (mut w, mut t) = scratch();
    for _ in 0..2 {
        t.add_device(&mut w, LabKind::Host, [0, 0]).unwrap();
    }
    t.add_device(&mut w, LabKind::Router, [0, 0]).unwrap();
    t.add_device(&mut w, LabKind::Switch, [0, 0]).unwrap();
    let names: Vec<&str> = t.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["pc1", "pc2", "r1", "sw1"]);
    let lab = t.without_device(&w, 0).unwrap(); // drop pc1
    let (mut w2, mut t2, _) = build(&lab, Trace::new()).unwrap().into_parts();
    t2.add_device(&mut w2, LabKind::Host, [0, 0]).unwrap();
    assert_eq!(
        t2.devices.last().unwrap().name,
        "pc1",
        "lowest free number is reused"
    );
}

#[test]
fn link_rules() {
    let (mut w, mut t) = scratch();
    let a = t.add_device(&mut w, LabKind::Host, [0, 0]).unwrap();
    let b = t.add_device(&mut w, LabKind::Host, [1, 0]).unwrap();
    let sw = t.add_device(&mut w, LabKind::Switch, [2, 0]).unwrap();
    assert_eq!(t.add_link(&mut w, a, a), Err(EditError::SameDevice));
    assert_eq!(t.add_link(&mut w, a, 99), Err(EditError::NoSuchDevice));
    let k = t.add_link(&mut w, a, sw).unwrap();
    assert_eq!((t.links[k].pa, t.links[k].pb), (0, 0));
    // A PC has one NIC.
    assert_eq!(
        t.add_link(&mut w, a, sw),
        Err(EditError::NoFreePort {
            device: "pc1".into()
        })
    );
    let k = t.add_link(&mut w, b, sw).unwrap();
    assert_eq!(
        t.links[k].pb, 1,
        "second cable takes the next free switch port"
    );
    // Names in the saved file: switch ports are 1-based Fa0/N.
    let saved = text(&t, &w);
    assert!(
        saved.contains("sw1:Fa0/1") && saved.contains("sw1:Fa0/2"),
        "{saved}"
    );
}

#[test]
fn in_place_edits_equal_a_rebuild_from_the_saved_file() {
    let (mut w, t) = four_node();
    let saved = text(&t, &w);
    // Saving is stable: load -> build -> snapshot -> save gives the same text.
    let (w2, t2, warn) = load_and_build(&saved, Trace::new()).unwrap().into_parts();
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(text(&t2, &w2), saved);
    // And the rebuilt lab behaves the same: both deliver both pings.
    let mut w2 = w2;
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 2);
    assert_eq!(ping_twice(&mut w2, &t2, 0, [10, 0, 2, 2]), 2);
    assert_eq!(w.trace_hash(), w2.trace_hash(), "same events, same trace");
}

#[test]
fn removing_a_device_keeps_everyone_elses_live_state() {
    let (mut w, mut t) = four_node();
    let extra = t.add_device(&mut w, LabKind::Host, [0, 100]).unwrap();
    let lab = t.without_device(&w, extra).unwrap();
    let (mut w2, t2, _) = build(&lab, Trace::new()).unwrap().into_parts();
    assert_eq!(t2.devices.len(), 4);
    // Router config and host addressing survived the rebuild.
    assert_eq!(ping_twice(&mut w2, &t2, 0, [10, 0, 2, 2]), 2);
}

#[test]
fn removing_a_device_removes_its_cables_and_reindexes() {
    let (w, t) = four_node();
    let lab = t.without_device(&w, 1).unwrap(); // the switch
    assert_eq!(lab.devices.len(), 3);
    assert_eq!(lab.links.len(), 1, "only r1-pc2 is left: {:?}", lab.links);
    let (_, t2, _) = build(&lab, Trace::new()).unwrap().into_parts();
    assert_eq!(
        t2.devices
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["pc1", "r1", "pc2"]
    );
    assert_eq!((t2.links[0].a, t2.links[0].b), (1, 2));
}

#[test]
fn removing_a_link_and_bad_indices() {
    let (w, t) = four_node();
    let lab = t.without_link(&w, 1).unwrap(); // sw-r1
    assert_eq!(lab.links.len(), 2);
    assert_eq!(t.without_link(&w, 9).unwrap_err(), EditError::NoSuchLink);
    assert_eq!(
        t.without_device(&w, 9).unwrap_err(),
        EditError::NoSuchDevice
    );
    // Without the cable the ping cannot reach the far side.
    let (mut w2, t2, _) = build(&lab, Trace::new()).unwrap().into_parts();
    assert_eq!(ping_twice(&mut w2, &t2, 0, [10, 0, 2, 2]), 0);
}

#[test]
fn moves_update_positions_without_touching_the_world() {
    let (mut w, mut t) = four_node();
    let before = w.trace_hash();
    t.move_device(0, [-5, 42]);
    t.move_device(99, [1, 1]); // ignored
    assert_eq!(w.trace_hash(), before);
    assert!(text(&t, &w).contains("pos = [-5, 42]"));
    let _ = &mut w;
}

#[test]
fn adding_many_devices_hits_the_limit_cleanly() {
    let (mut w, mut t) = scratch();
    for _ in 0..255 {
        t.add_device(&mut w, LabKind::Host, [0, 0]).unwrap();
    }
    assert_eq!(
        t.add_device(&mut w, LabKind::Host, [0, 0]),
        Err(EditError::TooManyDevices)
    );
}

#[test]
fn removing_in_place_keeps_the_world_running_and_the_rest_of_the_lab_working() {
    let (mut w, mut t) = four_node();
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 2);
    let at = w.kernel.now();
    let extra = t.add_device(&mut w, LabKind::Host, [0, 100]).unwrap();
    t.add_link(&mut w, extra, 1).unwrap();
    t.remove_device(&mut w, extra).unwrap();
    assert_eq!((t.devices.len(), t.links.len()), (4, 3));
    assert_eq!(t.snapshot(&w).links.len(), 3);
    assert!(w.kernel.now() >= at, "the clock was not reset");
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 4);
    // And what is saved loads back as the same lab.
    let (w2, t2, _) = load_and_build(&text(&t, &w), Trace::new())
        .unwrap()
        .into_parts();
    assert_eq!(text(&t2, &w2), text(&t, &w));
}

#[test]
fn removing_a_device_in_the_middle_reindexes_the_cabling_and_frees_its_ports() {
    let (mut w, mut t) = four_node();
    t.remove_device(&mut w, 1).unwrap(); // the switch
    assert_eq!(
        t.devices
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["pc1", "r1", "pc2"]
    );
    assert_eq!(t.links.len(), 1);
    assert_eq!((t.links[0].a, t.links[0].b), (1, 2));
    // pc1 and r1 can be cabled directly: the ports the switch used are free again.
    t.add_link(&mut w, 0, 1).unwrap();
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 1, 1]), 2);
    assert_eq!(
        t.remove_device(&mut w, 9).unwrap_err(),
        EditError::NoSuchDevice
    );
}

#[test]
fn removing_a_link_in_place_cuts_the_path_and_recabling_restores_it() {
    let (mut w, mut t) = four_node();
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 2);
    t.remove_link(&mut w, 1).unwrap(); // sw-r1
    assert_eq!(
        ping_twice(&mut w, &t, 0, [10, 0, 2, 2]),
        2,
        "no new replies"
    );
    assert_eq!(t.remove_link(&mut w, 9).unwrap_err(), EditError::NoSuchLink);
    t.add_link(&mut w, 1, 2).unwrap();
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 4);
}

#[test]
fn reconciling_undoes_an_add_and_a_delete_without_starting_over() {
    let (mut w, mut t) = four_node();
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 2);
    let before = t.snapshot(&w);
    let at = w.kernel.now();
    // Delete the switch in the middle, then bring it back by reconciling to the earlier lab.
    t.remove_device(&mut w, 1).unwrap();
    let removed = t.reconcile(&mut w, &before).unwrap();
    assert!(removed.is_empty());
    assert_eq!(
        t.devices
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["pc1", "sw1", "r1", "pc2"],
        "order restored"
    );
    assert_eq!(
        (t.links[0].a, t.links[0].b, t.links[1].a, t.links[1].b),
        (0, 1, 1, 2),
        "cabling restored in its order"
    );
    assert!(w.kernel.now() >= at);
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 4);
    assert_eq!(text(&t, &w), labfile::save(&before));
    // Undo an add: the added device and its cable go; the router keeps the config typed into it.
    let before_add = t.snapshot(&w);
    let extra = t.add_device(&mut w, LabKind::Host, [0, 100]).unwrap();
    t.add_link(&mut w, extra, 1).unwrap();
    let removed = t.reconcile(&mut w, &before_add).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!((t.devices.len(), t.links.len()), (4, 3));
    assert_eq!(ping_twice(&mut w, &t, 0, [10, 0, 2, 2]), 6);
}

#[test]
fn reconciling_to_a_lab_that_does_not_build_changes_nothing() {
    let (mut w, mut t) = four_node();
    let was = text(&t, &w);
    let mut bad = t.snapshot(&w);
    bad.devices[0].template = "no-such-box".into();
    assert!(t.reconcile(&mut w, &bad).is_err());
    assert_eq!(text(&t, &w), was);
}

#[test]
fn devices_added_after_a_removal_never_share_a_mac_unit() {
    let (mut w, mut t) = four_node();
    t.remove_device(&mut w, 0).unwrap();
    let new = t.add_device(&mut w, LabKind::Host, [0, 0]).unwrap();
    let mut units: Vec<u8> = t.devices.iter().map(|d| d.unit).collect();
    units.sort();
    units.dedup();
    assert_eq!(units.len(), t.devices.len());
    assert_eq!(t.devices[new].name, "pc1", "the freed name is reused");
}
