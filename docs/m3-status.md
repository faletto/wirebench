# M3 status

**Started 2026-11-06.** Written by working with an AI assistant; the pace is not a guide to sizing the rest.

## Done so far

| M3 item | Where | Notes |
|---|---|---|
| ACLs | `netdev/src/acl.rs`, `provenance/acls.md` | Standard and extended, numbered and named, in/out on interfaces; `access-class` is enforced on the vty lines (Telnet/SSH) |
| TCP header | `wire/src/tcp.rs` | Header parse and emit with checksum; the stack is below |
| NAT and PAT | `netdev/src/nat.rs`, `engine/src/shell/nat.rs`, `provenance/nat.md` | `ip nat inside/outside`, static (one-to-one and port forwards), `ip nat outside source static`, pools, `overload` (PAT, also through an interface address), `extendable` statics, rotary `ip nat inside destination` (TCP load balancing over a pool), `max-entries`, configurable timeouts including `syn-timeout`, ARP answered for translated addresses, ICMP errors translated both ways so traceroute and port-unreachable work, `show ip nat translations/statistics`, `clear ip nat translation *`; on routers and on layer 3 switches (VLAN interfaces). Example lab "NAT: two PCs, one public address" |
| M3a exit demo | `testkit/scenarios/m3_exit_demo.{lab.toml,script,out}` | Five routers: four on one OSPF segment (r2 originates a default route), NAT at the edge towards an "internet" router, a static public address for one PC (answered by proxy ARP, no route on the internet router), `access-class` on the VTY lines keeping the internet out. Golden transcript plus assertions in `engine/tests/vlan_scenario.rs`. Missing: the FRR comparison below |
| OSPFv2 | `netdev/src/ospf.rs`, `wire/src/ospf.rs`, `provenance/ospf.md`, `provenance/ospf-wire.md` | Neighbours, DR/BDR, database exchange, flooding, aging, multiple areas with border routers and summary LSAs, E1/E2 external routes, `default-information originate`; goldens `m3_ospf`, `m3_ospf_areas` |
| TCP and UDP sockets | `netdev/src/tcp.rs`, `provenance/tcp-udp.md` | Handshake, streams, retransmission, close, resets on hosts, routers and switches; UDP port table. No options, congestion control or out-of-order buffering |
| Telnet and SSH | `engine/src/shell/remote.rs`, `wire/src/{telnet,ssh}.rs`, `provenance/remote-login.md` | `telnet` / `ssh -l` clients on every device; vty login (`login`, `login local`, `username`, `transport input`, `access-class`, `crypto key generate rsa`). SSH framing is real but unencrypted |
| `do` in config modes | `clish/src/session.rs` | Runs exec commands from config modes; help and Tab now work after `do` |
| PC command prompt | `engine/src/shell/pc.rs` | `C:\>` prompt, no `?` help (the key types a `?`), `help` command, Windows wording for errors, `ipconfig /all /release /renew`, `netsh interface ip set address <name> static <ip> <mask> [gw]` / `dhcp`, `netsh interface ip show config`. The older `ipconfig set/gateway/dhcp` forms still work. `cls` clears the screen; commands ignore case (user names after `-l` keep theirs) |
| Port security | `provenance/port-security.md` | Limits, sticky addresses, protect/restrict/shutdown, err-disabled and recovery, `show interfaces status` |
| EtherChannel | `wire` LACPDU, `provenance/etherchannel.md` | Static and LACP bundles, `interface port-channel`, `interface range`, load-balance methods; a bundle counts as one port for spanning tree. PAgP (Cisco-only) is not simulated |
| CDP / LLDP | `provenance/cdp-lldp.md` | Neighbour discovery on switches and routers. **Off by default** (a real device runs CDP from the start); enable with `cdp run`. On by default, its constant timer stopped the app ever going idle and ~25 tests failed |
| NTP and device clock | `provenance/ntp-clock.md` | `clock set`, `clock timezone`, `ntp server`, `ntp master`, `show ntp ...`; a client trusts a server after 3 good replies and steps its clock |
| HSRP / VRRP | `provenance/fhrp.md` | Election, preempt, timers, interface tracking, virtual MAC, `show standby` / `show vrrp`, including on switch VLAN interfaces |

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
- NAT leftovers: application-level gateways (FTP's `PORT`, SIP and the like). They only matter once
  the simulator has an application that puts addresses in its data, so they wait for an FTP client
  and server to demonstrate on. Also per-host and per-list `max-entries`, `match-in-vrf`.
- The M3a exit demo is built and runs; the one part of it still open is the FRR comparison above.
- Packet inspector support for the new frame types (LACPDU, CDP, LLDP, NTP, HSRP/VRRP).
- PAgP, if ever wanted (Cisco-only; not planned).
- SSH/Telnet leftovers: `exec-timeout`, public-key auth, host names (no resolver yet). `show users` and `show ssh` exist; telnet users are not named.
