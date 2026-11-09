# M3 status

**Started 2026-11-06.** Written by working with an AI assistant; the pace is not a guide to sizing the rest.

## Done so far

| M3 item | Where | Notes |
|---|---|---|
| ACLs | `netdev/src/acl.rs`, `provenance/acls.md` | Standard and extended, numbered and named, in/out on interfaces; `access-class` is stored but not enforced until there is Telnet/SSH |
| TCP header | `wire/src/tcp.rs` | Parser only; no TCP stack yet |
| OSPFv2 | `netdev/src/ospf.rs`, `wire/src/ospf.rs`, `provenance/ospf.md`, `provenance/ospf-wire.md` | Neighbours, DR/BDR, database exchange, flooding, aging, multiple areas with border routers and summary LSAs, E1/E2 external routes, `default-information originate`; goldens `m3_ospf`, `m3_ospf_areas` |
| Port security | `provenance/` note in the commit | Limits, sticky addresses, protect/restrict/shutdown, err-disabled and recovery, `show interfaces status` |
| EtherChannel | `wire` LACPDU format, provenance note | Static and LACP bundles, `interface port-channel`, `interface range`, load-balance methods; a bundle counts as one port for spanning tree. PAgP (Cisco-only) is not simulated |
| CDP / LLDP | provenance note | Neighbour discovery on switches and routers. **Off by default** (a real device runs CDP from the start); enable with `cdp run`. On by default, its constant timer stopped the app ever going idle and ~25 tests failed |
| NTP and device clock | provenance note | `clock set`, `clock timezone`, `ntp server`, `ntp master`, `show ntp ...`; a client trusts a server after 3 good replies and steps its clock |
| HSRP / VRRP | provenance note | Election, preempt, timers, interface tracking, virtual MAC, `show standby` / `show vrrp`, including on switch VLAN interfaces |

Shared-core changes made along the way: `Kernel::set_carrier` and `Device::start`. Shutting a
switch port now drops the link at both ends (before, the neighbour still saw it up).

## Todo

- **FRR oracle for OSPF (needs Docker; planned for the home PC).** The plan's M3a exit demo asks
  for an FRR `ospfd` comparison of LSDB and routes on a 20-router random topology. Nothing in OSPF
  has been compared with another implementation yet: the LSA checksum, the timing, the border
  router rules and all the show output are our reading of RFC 2328. Sketch, in the style of
  `testkit/mstpd-oracle`:
  1. A script generates a random topology (the generators in `netdev/tests/ospf_net.rs` already do
     this, single and multi-area) and writes it as FRR containers joined by Docker networks, one
     per segment, configured with the same costs, priorities and areas.
  2. Wait for convergence, then dump each router's `show ip ospf database` and `show ip route ospf`.
  3. Run the same topology in our engine and compare LSA sets (type, id, advertising router,
     body; not ages or sequence numbers) and routes (prefix, cost, kind, next hop).
  4. Run it in CI as a separate job, with a pinned FRR image tag, as `stp-oracle` does for mstpd.
  Expect to find differences; each one is either a bug here or a choice to write down in
  `provenance/ospf.md` (SPF delay, ECMP, ABR rule without a backbone attachment, MinLSArrival).
- OSPF: redistribution, stub and NSSA areas, virtual links, `area range`, equal-cost paths,
  authentication.
- NAT/PAT, minimal UDP/TCP sockets (the rest of M3a), and the M3a exit demo (five-router OSPF lab
  with NAT to an "internet" router and ACLs on the VTY lines).
- **SSH/Telnet.** Needs a TCP stack on routers and switches. Once it exists, enforce
  `access-class` on the VTY lines (currently stored only).
- Packet inspector support for the new frame types (LACPDU, CDP, LLDP, NTP, HSRP/VRRP).
- PAgP, if ever wanted (Cisco-only; not planned).
