# Router subinterfaces (router on a stick)

**Date:** 2026-10-25
**Source category:** standard (IEEE 802.1Q, RFC 1812 forwarding) plus public doc for the vendor
specifics; own test (`crates/engine/tests/inter_vlan.rs`, golden `testkit/scenarios/m2_inter_vlan.*`).
[verify] marks choices not checked against a real router.

Behaviour implemented in `crates/netdev/src/router.rs` and `crates/engine/src/shell.rs`:

- `interface g0/0.N` creates a subinterface; `N` is a label and has nothing to do with the VLAN.
  `no interface g0/0.N` deletes it. Physical interfaces cannot be deleted.
- `encapsulation dot1q <vlan>` binds the subinterface to a VLAN; `... native` makes it the
  subinterface for untagged frames. `no encapsulation dot1q` unbinds it.
- A subinterface shares its parent port's MAC address.
- Tagged frames go to the subinterface whose VLAN matches; with none, they are dropped.
  Untagged frames go to the native subinterface if there is one, else to the physical interface.
  A tagged frame carrying the native VLAN's id does not reach the native subinterface [verify].
- Replies leave the parent port tagged with the subinterface's VLAN, untagged for a native one.
- A subinterface is up only if it and its parent port are both not shut down. A new subinterface
  is not shut down; the parent port starts shut down like every router port here.
- Two subinterfaces of one port cannot use the same VLAN. The same VLAN on different ports is fine.
- An address cannot be set on a subinterface that has no encapsulation yet. The real router
  refuses this too, with its own message; ours is our own wording.
- `show running-config` prints a subinterface's `encapsulation` line before its address, and
  prints `shutdown` only when set (physical ports always print `shutdown` or `no shutdown`).
- Routing between subinterfaces is ordinary connected-route forwarding: TTL is decremented once
  per router hop.

Not implemented: SVIs and `ip routing` on a layer 3 switch, ISL, `encapsulation` on physical
interfaces, `ip helper-address`, subinterface bandwidth/MTU settings.
