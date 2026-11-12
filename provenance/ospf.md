# OSPFv2: areas, border routers, external routes

**Date:** 2026-11-06
**Source category:** standard (RFC 2328 for the protocol: neighbours section 10, DR election 9.4,
database exchange 10.6-10.8, flooding 13, origination 12.4, shortest path 16.1-16.4; RFC 1071 for the
packet checksum; packet formats in `provenance/ospf-wire.md`); public doc (general knowledge of how
IOS configures OSPF and prints `show ip ospf ...`, recalled rather than re-read; no Packet Tracer
material); own test (`crates/netdev/tests/ospf_net.rs`, `crates/engine/tests/ospf_shell.rs`, golden
`testkit/scenarios/m3_ospf.*`). [verify] marks choices not checked against a real device. **Compared
with FRR ospfd 10.2.1 on 2026-11-11** (`testkit/frr-oracle`): the routes (prefix, kind, cost) for
three networks (broadcast and point to point, two leaf areas, an E2 default route) are the same, and
FRR's packets parse here. Only those cases were compared: not the database, timing, equal-cost
paths, stub areas or NSSA.

## What is modelled

- **Neighbours:** Hello with the checks of 10.5 (area, mask, hello and dead intervals, E bit), the
  state machine Down, Init, 2-Way, ExStart, Exchange, Loading, Full, inactivity timeout, and
  tear-down to Init when a neighbour stops listing us. Broadcast and point-to-point network types
  (`ip ospf network`); no NBMA, point-to-multipoint or virtual links.
- **DR and BDR** on broadcast networks as in 9.4, including the second pass when this router's own
  role changes, the wait timer (the dead interval) and the "backup seen" shortcut. Election does not
  pre-empt: a late router with a better priority joins as DROther. Priority 0 never stands.
  Adjacency only with the DR and BDR (everyone, on point to point).
- **Database exchange:** master/slave by router ID, sequence numbers, repeats, summary lists,
  request lists, a restart on sequence mismatch or a request for something we do not have. An
  Update larger than the budget goes in several packets. A Database Description with a larger MTU
  than ours (1500) is ignored, which leaves the neighbour stuck in ExStart as the RFC says.
- **Flooding** per 13.3: out of every interface with a neighbour in Exchange or later, not back to
  the sender, not out of the receiving interface if the sender was the DR or BDR (or we are the
  BDR), to 224.0.0.5 (224.0.0.6 when we are not DR/BDR), with retransmission to each neighbour
  every 5 s until acknowledged. Comparison of instances (13.1: sequence, checksum, MaxAge, age
  difference 15 min), handling of duplicates and of an older copy, and self-originated LSAs that
  come back newer (13.4: re-originate above the received number, or flush if no longer wanted).
- **Aging:** LSAs age in the database; ours are re-originated after 30 minutes; at one hour they are
  flooded as MaxAge and removed once acknowledged. Premature aging (14.1) withdraws a network LSA
  when we stop being DR, and a router LSA's links when an interface goes away.
- **Origination:** one router LSA **per area** (stub, point-to-point and transit links, as 12.4.1.1
  and 12.4.1.2 say; the B flag when we are an area border router, the E flag when we originate
  external routes) and, on a broadcast network where we are DR with a full neighbour, a network LSA.
  A new instance is only made when the contents changed.
- **Areas:** each interface belongs to the area of the first `network` statement that covers its
  address; a Hello or other packet from a different area is dropped. Every LSA of types 1 to 4 lives
  in one area's database and is flooded only there; type 5 (AS-external) LSAs belong to the whole
  AS and are flooded in every area. Database exchange with a neighbour covers its area's LSAs and
  the AS-external ones.
- **Area border routers (12.4.3, 16.2):** a router with interfaces in more than one area **and in
  the backbone (area 0)** originates, into each area it is in, summary LSAs (type 3) for the
  intra-area routes of its other areas and for inter-area routes it learned through the backbone
  (never back into the area they came from, never an inter-area route learned from a non-backbone
  area), and type 4 summaries for AS boundary routers likewise. Routers use these to compute
  inter-area routes: cost to the border router plus the summary's metric, always less preferred
  than an intra-area route. A border router looks only at the backbone's summaries. Without a
  working attachment to area 0 a router stops acting as a border router [verify; the RFC 3509
  discussion of "Cisco behaviour" is the reference].
- **External routes (16.4):** type 5 LSAs install `O E1` (metric added to the cost to the boundary
  router) and `O E2` (the metric alone; the cost to the boundary router only breaks ties) routes
  when the boundary router is reachable, by an intra-area path or through a type 4 summary. A
  nonzero forwarding address is honoured if a route to it exists. Intra-area, then inter-area,
  then E1, then E2 routes win for the same prefix, whatever their cost.
- **`default-information originate [always] [metric n] [metric-type 1|2]`:** originates a
  `0.0.0.0/0` type 5 LSA (metric 1, type 2 by default) while the router has a default route that
  does not come from OSPF, or always with `always`, and sets the E flag in its router LSAs. It is
  withdrawn (premature aging) when the condition ends [verify the IOS defaults].
- **Shortest path (16.1):** Dijkstra over router and network LSAs with the two-way check, run once
  per area, next hops derived as 16.1.1 describes, routes to stub and transit networks. Installed
  in the forwarding table as `O`, `O IA`, `O E1` and `O E2` routes at distance 110 (connected and
  static win).
- **Configuration and show:** `router ospf N` (one process), `router-id`, `network ... area`,
  `passive-interface`, `auto-cost reference-bandwidth`, `default-information originate`; per
  interface `ip ospf cost | priority | hello-interval | dead-interval | network
  point-to-point|broadcast | mtu-ignore` and `ip mtu`; `show ip ospf`, `... neighbor`, `...
  interface`, `... database` (router, network, summary net, summary ASB and AS-external sections),
  `show ip protocols`, `clear ip ospf process`; `show ip route` prints `O`, `O IA`, `O E1`, `O E2`
  routes as `[110/cost]`. The configuration prints in `running-config` and replays to the same
  thing. It works on SVIs of a layer 3 switch.
- **MTU:** `ip mtu` sets the size an interface advertises in Database Description packets. The
  simulator has no fragmentation, so that is all it affects. A neighbour advertising a larger MTU
  than ours is ignored (the exchange sticks in ExStart) unless `ip ospf mtu-ignore` is set.

## Choices that are ours or simplified

- **Areas** can be numbers or dotted addresses on input and are printed as numbers. A non-backbone
  area must reach area 0 through a border router that is directly attached to both; there are no
  virtual links, so a partitioned backbone cannot be repaired through another area. Stub, totally
  stubby and NSSA areas, and `area ... range` summarisation, do not exist: every prefix is
  summarised as it is.
- **Cost:** every interface counts as gigabit, so the default cost is `max(1, reference / 1000)`
  with the reference at 100 Mb/s, i.e. 1 [verify]. `ip ospf cost` overrides.
- **Router ID:** `router-id` if set, else the highest address of any usable interface when the
  process starts (the lab has no loopbacks). Changing it restarts the process at once, where IOS
  asks for `clear ip ospf process` [verify]. It does not change when addresses change later.
- **Dead interval** is four hellos unless set; 10 s / 40 s on both network types [verify].
- **SPF** runs one second after the database changes, with none of IOS's throttling [verify]. Only
  **one next hop** per route: equal-cost paths are not shared (IOS installs up to four), so which of
  two equal paths is chosen is arbitrary but stable [verify].
- **Acknowledgments** are sent at once for each Update; the delayed, combined acks of 13.5 are not
  used. MinLSArrival (1 s) is not enforced. Retransmission is unicast to each neighbour.
- **Timers** run from a one-second tick, so everything happens on whole seconds. Hellos start at the
  first tick after an interface is enabled and are not jittered.
- **Passive interfaces** advertise their network as a stub and neither send nor read OSPF.
- **Authentication:** only the null type; a packet with any other type is dropped.
- **Multicast:** a router now drops all IPv4 multicast (224.0.0.0/4) it receives instead of trying to
  route it (there is no multicast routing). OSPF packets go out with TTL 1 and DSCP CS6, to the
  multicast MAC for the group; a switch's SVI hears the two OSPF groups.
- **Access lists:** an inbound list sees OSPF packets like any other, so it can break adjacencies
  [verify that IOS does the same]; an outbound list never filters the router's own OSPF.
- Show output follows the shape of IOS's, abbreviated (no SPF counters, timers or area details) [verify].

## Not done

- Redistribution (`redistribute static|connected`), so external routes come only from
  `default-information originate`. External LSAs from other sources are understood but nothing in
  the simulator originates them.
- Stub and NSSA areas, virtual links, `area range`, ABR/ASBR show commands (`show ip ospf
  border-routers`), type 7 LSAs.
- Equal-cost paths (`maximum-paths`): one next hop per route. Authentication of any kind: a packet
  with an authentication type other than none is dropped. `ip ospf <pid> area`; `network` matching
  secondary addresses; `distance`; `passive-interface default`; `debug ip ospf`; the confirmation
  question of `clear ip ospf process`; the `show ip protocols` list of routing sources.
- Interface MTU affects nothing but OSPF.

## How it was checked

- **Several areas** (`ospf_net.rs`): random backbones with leaf areas, each reaching the backbone
  through one border router. Within an area every router that can still reach the others holds the
  same LSAs for it, no router holds LSAs of an area it is not in, and route costs and kinds (intra
  or inter-area) match an independent calculation that goes out of the router's leaf area through
  its border router, across the backbone and in through the target area's. After a cut, the same.
  Default routes originated by a random router (type 1 or 2, random metric) reach every router in
  every area at the cost the configuration and the distance say, and following next hops arrives at
  the originator. Deliberate breaks (flooding that ignores area scope; inter-area cost without the
  distance to the border router; inter-area routes summarised back into the backbone) were all
  caught.
- **Random networks** (`ospf_net.rs`, 48 per run in CI, 2000 tried by hand): routers as pure state
  machines joined by segments (broadcast and point to point, with priorities and costs), packets
  delivered at once. After 150 simulated seconds every router has the same database and its route
  costs equal those of an independent Dijkstra over the same graph; after one segment is cut, the
  reachable set and costs match again. Two deliberate breaks (wrong path costs; flooding that never
  retransmits to neighbours) were both caught by these tests.
- DR election rules (highest priority then ID; priority 0 never; no pre-emption), and dead-neighbour
  handling have their own tests; random hostile packets in random order (1000 sequences) never
  panic the state machine, and everything it sends parses back.
- Console tests: errors, printing and replaying the configuration, a save and reload, passive
  interfaces, removing the process, and a layer 3 switch forming an adjacency over an SVI.
- Golden transcripts: `m3_ospf.*` (three routers on a switch: DR/BDR/DROther, routes, database,
  pings, a cost change and a withdrawn network) and `m3_ospf_areas.*` (two areas joined by a border
  router, a default route from a boundary router, its withdrawal, and `always metric 5 metric-type
  1`). Written by the assistant: no one who knows IOS has
  compared it with a real device. In particular the LSA checksum is only checked by its defining
  property, and the timing of everything is our reading of the RFC.
