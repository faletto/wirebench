# M2 status

**Written 2026-10-26, at the end of the M2 work.** Not a retrospective yet: the project owner has
not closed M2, and two things below are open. As with M0 and M1, the pace came from working with an
AI assistant and should not be used to size M3.

## What shipped

| M2 item | Where it is | Notes |
|---|---|---|
| Undo and redo | `app/src/history.rs` | Snapshots of the lab file; one entry per drag; typed configuration is carried forward |
| VLANs, trunks | `netdev/src/switch.rs`, `provenance/vlan-*.md` | Access/trunk, native and allowed VLANs, per-VLAN MAC tables |
| Router on a stick | `provenance/router-subinterfaces.md` | `encapsulation dot1q [native]` |
| SVIs, layer 3 switch | `provenance/switch-svi-and-ip-routing.md` | A `Router` embedded in the switch; routing off until `ip routing` |
| Management plane | `provenance/management-plane.md` | Enable secret/password, line passwords, banner, `write`/`reload`/`erase`; our own secret formats |
| DHCP | `provenance/dhcp.md` | Server, relay (`ip helper-address`), client state machine |
| Traceroute, `ping` on routers | `provenance/traceroute.md` | `tracert`, `traceroute`, ICMP port unreachable, rate limit |
| STP and RSTP, per VLAN | `provenance/spanning-tree.md` | Pure state machine plus switch integration; property-tested; **off by default on new switches** |
| DTP | `provenance/dtp.md` | Dynamic auto/desirable, `nonegotiate`; **default port mode is now dynamic auto** |
| Packet inspector | `provenance/packet-inspector.md` | Filter, field tree, hex view, exact step back |
| Device settings panels | `provenance/device-panels.md` | PC, router, switch; run through the real CLI |
| Engine in a Web Worker | `docs/engine-worker.md` | All 5 steps done; load-tested at 8x; browser check in CI |

About 500 tests (unit, integration, property, golden transcripts, a late-answering engine) pass on
the three CI targets; there are fuzz targets for every new parser.

## Against the plan's M2 scope and exit demo

The plan (M2, "MVP, public alpha") lists more than the table above. Not done:

- **mstpd oracle for spanning tree**, which the plan calls "not optional". It is Linux-only and was
  not available; properties and mutation checks stand in for it, which is weaker.
- **PWA offline.**
- **"100-node benchmark green."** Measured instead: an 84-device lab in a throttled browser (frame
  gaps, not the plan's per-frame budget) and, in M1, engine cold start at 100 nodes.
- **Exit demo:** "ten switching and addressing labs ... each with a scripted golden test". There
  are eight M2 golden scenarios (VLANs, router on a stick, SVIs, management plane, DHCP,
  traceroute, spanning tree, DTP), written by the assistant, not ten, and **no tester** who knows
  Packet Tracer has tried any of them.
- **Public alpha** under the new name, and the legal read the plan puts before it.
- Also from the plan's scope: Kea/other oracles were never run (DHCP), and device config panels
  cover PCs, routers and switches but not every setting.

## Deviations from the plan and from the devices imitated

- **Spanning tree starts off** on a new switch (IOS starts in `pvst`), so labs without redundant
  links forward at once. A loop of cables is a broadcast storm until `spanning-tree mode ...`.
- **Link state is polled**, once a second while spanning tree or DTP is running, and at each
  command; the kernel has no link-change event. Every link costs 19 for spanning tree.
- **The simulation moves in real time while spanning tree runs.** Simulated time otherwise moves
  only when something happens. Time a hidden tab missed is made up for on return, up to an hour.
- **No mstpd oracle** for spanning tree and no Kea oracle for DHCP. Checks are properties over
  random networks, golden transcripts and mutation checks (see each provenance note).
- **Transient loops are possible** in races between messages (the property tests check each
  exchange's settled result, not every BPDU hop).
- Not implemented from the plan's M2 list or noted along the way: MST, BPDU/root/loop guard,
  configurable spanning tree timers, port channels, VTP, host-unreachable after an ARP failure, a
  router dropping the first packet while it resolves ARP, DHCP conflict probes, a canvas UI for
  VLANs and SVIs (the settings panel and console cover them), a Redo for simulation Back.

## What was checked in a real browser

`testkit/throttle` (Chromium, CPU throttled, run by hand): the M1 demo, an M2 run (spanning-tree
reroute, panel, Step/Back, field tree), `m2b.mjs` (the DTP mode control turning a link between two
"dynamic auto" ends into a trunk and back, the inspector's filter, a changed lab saved, reopened
and saved again to the same text, and delete and undo leaving another console's session alone), and
load runs with the engine in the page and in the worker (84 and 100 devices). The browser found
three bugs unit tests could not: the live clock running a stepped ping to the end, the worker's
clock loop doubling the page's work, and the dissector's out-of-range byte ranges (found by
fuzzing, not the browser). See `testkit/throttle/README.md` and `perf-baseline.md`.

## Open

- **Hidden tab:** time that passes while the tab draws no frames is made up for on the next frame
  (up to an hour), not run in the background (`engine-worker.md`). Not tried in a real hidden tab.
- **UI scale:** past 12 devices the tab strip is one picker (previous, next, a list with a filter
  box). A 100-device lab measures p99 0.47 ms native (CPU side) and about 16 ms browser frame gaps
  at 4x throttle. The browser number is a pacing measure, not paint cost, and there is no
  low-end machine to try it on. A lab of 100 devices is also hard to *read* on one canvas; there is
  no grouping, minimap or search on the canvas itself.
- **Consoles and tables now survive delete and undo.** Delete, cable removal, undo and redo change
  the running lab in place (`Topology::reconcile`); a device that comes back through undo is new
  (empty ARP and MAC tables, its saved configuration). Removed devices stay in the engine's memory
  for the session. Opening a lab still starts everything over, as it should.
- `m2b.mjs` is a pixel-driven harness with screenshots, run by hand, not in CI; only
  `check.mjs` (frame pacing, worker against page) runs there.
