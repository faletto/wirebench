# M3 status

**Started 2026-11-06.** Written by working with an AI assistant; the pace is not a guide to sizing the rest.

## Done so far

| M3 item | Where | Notes |
|---|---|---|
| ACLs | `netdev/src/acl.rs`, `provenance/acls.md` | Standard and extended, numbered and named, in/out on interfaces; `access-class` is enforced on the vty lines (Telnet/SSH) |
| TCP header | `wire/src/tcp.rs` | Header parse and emit with checksum; the stack is below |
| NAT and PAT | `netdev/src/nat.rs`, `engine/src/shell/nat.rs`, `provenance/nat.md` | `ip nat inside/outside`, static (one-to-one and port forwards), `ip nat outside source static`, pools, `overload` (PAT, also through an interface address), `extendable` statics, rotary `ip nat inside destination` (TCP load balancing over a pool), `max-entries`, configurable timeouts including `syn-timeout`, ARP answered for translated addresses, ICMP errors translated both ways so traceroute and port-unreachable work, `show ip nat translations/statistics`, `clear ip nat translation *`; on routers and on layer 3 switches (VLAN interfaces). Example lab "NAT: two PCs, one public address" |
| M3a exit demo | `testkit/scenarios/m3_exit_demo.{lab.toml,script,out}` | Five routers: four on one OSPF segment (r2 originates a default route), NAT at the edge towards an "internet" router, a static public address for one PC (answered by proxy ARP, no route on the internet router), `access-class` on the VTY lines keeping the internet out. Golden transcript plus assertions in `engine/tests/vlan_scenario.rs`. FRR comparison: routes only, see the oracle row |
| OSPFv2 | `netdev/src/ospf.rs`, `wire/src/ospf.rs`, `provenance/ospf.md`, `provenance/ospf-wire.md` | Neighbours, DR/BDR, database exchange, flooding, aging, multiple areas with border routers and summary LSAs, E1/E2 external routes, `default-information originate`; goldens `m3_ospf`, `m3_ospf_areas` |
| FRR oracle for OSPF | `testkit/frr-oracle/`, `ospf_net.rs` (`routes_match_the_ones_frr_recorded`), `wire/tests/frr_capture.rs` | Done 2026-11-11 on the home PC (PR #41). FRR 10.2.1 `ospfd` in Docker over three networks (broadcast LAN plus point to point with unequal costs, two leaf areas with two border routers, an E2 default route): our routes match in prefix, kind and cost for every router, and all 1,581 packets FRR sent (574 LSAs) parse here with valid packet and LSA checksums and re-emit byte for byte. Tests run from the recorded files; no Docker in CI. Re-record with `testkit/frr-oracle/run.py` |
| Syslog | `wire/src/syslog.rs`, `netdev/src/syslog.rs`, `engine/src/shell/logging.rs`, `provenance/syslog.md` | `logging host/trap/buffered`, `service timestamps`, `show/clear logging` on routers and switches; messages for link and line-protocol changes and for leaving config mode; a `syslogd` collector on PCs. Golden `m3b_syslog`. Also `%OSPF-5-ADJCHG`, `%STANDBY`/`%VRRP-6-STATECHANGE` and port security violation messages. PCs run the collector, the lab has no Server device |
| TFTP | `wire/src/tftp.rs`, `netdev/src/tftp.rs`, `engine/src/shell/tftp.rs`, `provenance/tftp.md` | `copy running-config / startup-config tftp://ip/file` and back on routers and switches, with retransmission and the usual errors; a `tftpd` server on PCs (list, show, delete, log). Backup and restore of a configuration works end to end. Golden `m3b_tftp`. Not yet: flash and image copies, serving files from a router, preloaded files, the interactive prompts |
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

- **FRR oracle for OSPF, what is left.** The first comparison is done (see the table). Not yet
  compared: the LSDB (LSA sets by type, id, advertising router and body, from
  `show ip ospf database`), next hops, timing (SPF delay, MinLSArrival), the ABR rule without a
  backbone attachment, equal-cost paths, and the plan's 20-router random topology (`run.py` reads
  hand-written cases; a generator from the `ospf_net.rs` strategies is the missing piece). A CI job
  that re-runs FRR with the pinned image, as `stp-oracle` does for mstpd, is not set up; today CI
  only checks against the recorded answers. Each difference found is either a bug here or a choice
  to write down in `provenance/ospf.md`.
- OSPF: redistribution, stub and NSSA areas, virtual links, `area range`, equal-cost paths,
  authentication.
- NAT leftovers: application-level gateways (FTP's `PORT`, SIP and the like). They only matter once
  the simulator has an application that puts addresses in its data, so they wait for an FTP client
  and server to demonstrate on. Also per-host and per-list `max-entries`, `match-in-vrf`.
- The M3a exit demo is built and runs; the FRR comparison it asks for is done for routes and packets, and open for the LSDB and the 20-router topology (above).
- Packet inspector support for the new frame types (LACPDU, CDP, LLDP, NTP, HSRP/VRRP, syslog, TFTP).
- Still open from the M3b scope: DNS and HTTP (the application layer beyond syslog and TFTP), IPv6 basics, a Server device, the accessibility pass and the XML importer.
- PAgP, if ever wanted (Cisco-only; not planned).
- SSH/Telnet leftovers: `exec-timeout`, public-key auth, host names (no resolver yet). `show users` and `show ssh` exist; telnet users are not named.
