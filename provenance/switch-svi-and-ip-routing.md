# Switch virtual interfaces (SVIs) and `ip routing`

**Date:** 2026-10-25
**Source category:** public doc (general knowledge of vendor documentation, recalled rather than
re-read) and own test (`crates/engine/tests/svi.rs`, golden `testkit/scenarios/m2_svi.*`).
[verify] marks choices not checked against a real switch.

Behaviour implemented in `crates/netdev/src/switch.rs`, `router.rs` and `shell.rs`:

- `interface vlan N` (or `interface Vlan10`) creates an SVI: a layer 3 interface for VLAN N with
  its own MAC address (`02:bb:00:<unit>:<vlan>`) and IP address. `no interface vlan N` deletes it.
- Frames for an SVI's MAC, and broadcasts, in that VLAN go to the layer 3 half; frames sent to
  an SVI's MAC are not bridged onward (the address belongs to the switch, not to a port).
- Without `ip routing` the switch answers for its own addresses (ping, ARP) but forwards nothing
  between VLANs: packets for other addresses are dropped silently [verify]. `ip default-gateway`
  then gives it a default route for its own traffic, and is ignored once routing is on.
- With `ip routing`, SVIs are connected routes and the switch forwards between them like a router
  (TTL decremented, ICMP errors). Static routes apply only while routing is on.
- An SVI's line state is **up only while its VLAN exists and some port carries it**. A VLAN with
  no port, or a deleted VLAN, takes the SVI down: it leaves the routing table and
  `show ip interface brief` says `down` (as opposed to `admin down` after `shutdown`). Cable and
  link state are not modelled at the switch, so "a port carries it" means configured membership,
  not a live link [verify: the real rule needs a port in the VLAN that is up].
- A new SVI is not shut down [verify: some platforms create it shut down].
- `ip address` on a switch port, and `switchport` commands on an SVI, are refused with our own
  wording.
- `show running-config` prints `ip routing`, each SVI (`interface Vlan10`), the default gateway
  and static routes. `show ip route` with routing off lists connected and default routes without
  the "IP routing not enabled" note a real switch prints.

Not implemented: HSRP/VRRP, `ip helper-address`, `ip routing` per-VRF, routed ports
(`no switchport`), SVI autostate exclude, SVI MTU, ACLs on SVIs.
