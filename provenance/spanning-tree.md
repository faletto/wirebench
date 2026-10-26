# Spanning tree: STP and RSTP, one tree per VLAN

**Date:** 2026-10-25
**Source category:** standard (IEEE 802.1D-1998 and 802.1D-2004 for the protocol, BPDU layout, timers
and the proposal/agreement handshake, recalled rather than re-read) and public doc (general
knowledge of Cisco's PVST+ and Rapid PVST+ behaviour and of `show spanning-tree`); own tests
(`crates/netdev/tests/stp_net.rs`, `crates/engine/tests/stp_world.rs`, `stp_shell.rs`,
`crates/app/tests/stp_live.rs`, golden `testkit/scenarios/m2_stp.*`). [verify] marks choices not
checked against a real device or the standard's text.

No oracle ran. The plan names `mstpd` as one; it is Linux-only and was not available. The checks
are instead properties (below) and the invariants the standard states.

## What is modelled

- **Election.** Lowest bridge identifier is root. Each bridge's root port is the one with the best
  priority vector (root, cost, sender's bridge, sender's port), ties broken by the receiving port
  number. Designated, alternate and backup roles follow from comparing a port's own vector with
  what it has heard. The bridge identifier is a priority in steps of 4096 plus the VLAN number (the
  extended system id), then a MAC address.
- **Timers.** Hello 2 s, max age 20 s, forward delay 15 s, not configurable. Message age grows by
  one second per hop. In rapid mode, information is dropped after three missed hellos.
- **Classic (`spanning-tree mode pvst`).** Root and designated ports pass through listening and
  learning, 15 s each, before forwarding. Only the root originates configuration BPDUs; others
  relay what their root port hears. Topology changes go up as TCNs (once per hello until
  acknowledged); the root then flags changes for max age plus forward delay.
- **Rapid (`spanning-tree mode rapid-pvst`).** A designated port offers to forward with a
  *proposal*. The neighbour's root port, after putting its own non-edge designated ports back to
  discarding (the *sync*), answers with an *agreement* and forwards at once; an alternate port
  agrees immediately. A port whose neighbour never agrees falls back to the timers. A new root
  port forwards at once, after the sync. A failed root port is replaced by an alternate without
  waiting. Topology changes set a flag on the BPDUs of the other ports for 3 s and flush learned
  addresses, except those learned on the port the news came from.
- **Edge ports** (`spanning-tree portfast`) forward at once and send no proposals; a BPDU arriving
  makes the port an ordinary one until its link goes down and up.
- **Mixed mode.** A rapid port that hears a classic BPDU treats that neighbour as classic (no
  handshakes, classic BPDUs sent), shown as `P2p Peer(STP)`.
- **Per VLAN.** There is one tree for each VLAN that exists; a trunk takes part in the trees of the
  VLANs it carries. `no spanning-tree vlan N` turns one off, and a switch with no tree for a VLAN
  floods that VLAN's BPDUs like any multicast.
- **Data plane.** A port that is not learning drops everything; one that is learning learns
  source addresses but forwards nothing; only forwarding ports send or accept data. BPDUs are
  accepted in any state.
- **On the wire.** BPDUs go to `01:00:0c:cc:cc:cd` (the PVST+ address) inside the VLAN they belong
  to, tagged on trunks except for the native VLAN. The payload is the plain 802.1D BPDU with an
  LLC header, with no Cisco SNAP header or VLAN TLV [verify: a real PVST+ BPDU has both]. Class of
  service is 6.
- **Shutdown.** `shutdown` on a switch port takes it out of every VLAN and tree.

## Commands

`spanning-tree mode pvst | rapid-pvst`, `no spanning-tree mode`, `spanning-tree vlan <list>`,
`no spanning-tree vlan <list>`, `... priority <n>` (multiples of 4096, with the real error text for
others [verify the wording]), `no ... priority`, `... root primary | secondary`, and on ports
`spanning-tree portfast | cost <n> | port-priority <n>` with their `no` forms. `show
spanning-tree [vlan <n>]` follows the real layout: root block, bridge block, then
interface, role, state, cost, priority.number and type. `root primary` sets 24576, or one step
below a lower current root priority; `root secondary` sets 28672 (the macros' documented
behaviour [verify the exact rule]).

## Choices that are ours

- **Spanning tree starts off.** A new switch runs none, where a real one starts in `pvst`. This
  keeps every lab that has no redundant links forwarding at once, as before. It means a loop of
  cables is a storm until someone types `spanning-tree mode ...`. The saved configuration
  carries the mode, so labs for this topic include it.
- **A storm cannot hang the simulation.** A link's transmit queue holds 10 ms of frames and drops
  the rest, and `run_to_idle` gives up after 500,000 events and sets `World::storming`. Neither
  changes any run that does not storm.
- **Link state is polled once a second** (and at every configuration change), not signalled.
  A cable pulled is noticed within a second, not instantly; real RSTP notices at once.
- **Every link is point to point and costs 19** unless set (the short-path-cost value for 100
  Mb/s). The simulated links' speeds are not read.
- **Time moves on its own only while spanning tree is on.** The tick is a *background* timer: it
  never makes the simulation busy, so `run_to_idle` and Play still finish. In the app, simulated
  time follows real time while any such timer is pending (a lab with spanning tree), at 1x. A
  console command that needs the tree to settle should be followed by waiting, as on a real
  switch; the script format has `%wait <seconds>`.
- **Not implemented:** MST, UplinkFast, BackboneFast, root guard, BPDU guard, loop guard, bridge
  assurance, port-channel interaction, configurable timers (`hello-time`, `forward-time`,
  `max-age`), `show spanning-tree summary`, `detail`, `blockedports` and `root`, per-port
  `spanning-tree vlan N cost`, `link-type`, and storm control. A port-level `portfast` is also
  not turned on automatically for access ports.

## How it was checked

- Unit and integration: BPDU codec round trips and refusals (and a fuzz target `wire_bpdu`);
  bridges wired together directly (`stp_net.rs`); real switches with frames over links
  (`stp_world.rs`); the console (`stp_shell.rs`); the app advancing time (`stp_live.rs`).
- **Property tests** build random connected networks of up to six bridges with parallel links and
  random priorities, in both modes. They check, when each event's exchange of BPDUs has finished
  and again every second, that the live links (both ends forwarding) never contain a cycle; and,
  once settled, that there is one root (the best identifier in each connected piece), every other
  bridge has a root port, the live links form a spanning tree, every root and designated port
  forwards and every other port discards, and each bridge's cost to the root is the shortest-path
  cost. A second property pulls and restores links at random and requires the same once the
  network is left alone. They ran 4,000 cases each once, by hand, besides the 64 in the suite.
- **What they found:** BPDUs that never stop (an agreement taken as a reason to reply with a
  proposal); an agreement from an alternate port being ignored; topology change notifications
  passed on at once chasing each other round a loop of stale information (they now wait for the
  next hello); and, from the same flapping, information a neighbour gave on one link outliving its
  newer word on another (now dropped).
- **Mutation checks**, each against a passing suite: skipping the sync step, keeping the stale
  information, sending TCNs at once, ignoring alternate-port agreements, and letting information
  never age each fail a specific test. (Two further breaks were tried and found not to change any
  behaviour that can be observed: dropping the receiving-port tie-break, which only matters for a
  cable looped back to a single port, and removing the "agreement must answer a proposal" test.)
- **Known to be weaker than it looks:** messages are delivered instantly and in a fixed order, so
  races between messages in flight are not explored. A check made when an exchange has finished
  does not see states that exist for a few BPDU hops in the middle of it. A stricter version of
  the loop check, made after every single BPDU, does find such states (for instance two
  designated ports facing each other, both forwarding, while stale root information still
  circulates after a root is cut off from a loop), so a real network could briefly loop in
  those races. The sync step has its own test because the property cannot see it.
