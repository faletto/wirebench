# Coverage matrix

Draft, started 2026-11-15. This is the M3b exit artifact: every in-scope item with at least one
scenario test. It is a **first pass written by reading the tree**, not generated; nothing in CI checks
it yet (see "Next").

The topic list is the CCNA 200-301 blueprint as the plan doc remembers it, grouped in our own words.
**`[verify]`: it has not been compared against the current published exam topics.** Do that before
publishing. Only public exam-topic text is needed; no Packet Tracer material (see `PROVENANCE.md`).

**Status:** `covered` = at least one scenario or integration test drives it end to end;
`partial` = works, with the gap named; `missing` = not built; `non-goal` = excluded in the plan.

**Test** names a golden (`testkit/scenarios/<name>.{script,out}`) or an integration test file
(`crates/<crate>/tests/<name>.rs`). Tests listed here were matched by file name and the scope notes in
`docs/m3-status.md`; **they have not been run or opened one by one for this draft.**

## Network access

| Item | Status | Test | Notes |
|---|---|---|---|
| Ethernet, MAC learning | covered | `m2_vlans`, `switch_shell.rs` | |
| VLANs (access ports, VLAN database) | covered | `m2_vlans`, `vlans.rs` | |
| 802.1Q trunks, native VLAN, allowed list | covered | `m2_vlans`, `vlan_scenario.rs` | |
| DTP | covered | `m2_dtp`, `dtp.rs` | Emulated, not the Cisco wire format |
| Inter-VLAN: router-on-a-stick | covered | `m2_inter_vlan`, `inter_vlan.rs` | |
| Inter-VLAN: switch SVIs | covered | `m2_svi`, `svi.rs` | |
| STP (802.1D) and RSTP | covered | `m2_stp`, `stp_world.rs`, `stp_net.rs`, `stp_oracle.rs` | mstpd oracle |
| PortFast | covered | `stp_net.rs`, `stp_world.rs` | |
| BPDU guard | missing | | No match in `crates/netdev`; on the blueprint `[verify]` |
| EtherChannel: static and LACP | covered | `m3b_etherchannel`, `etherchannel.rs` | PAgP not simulated |
| CDP and LLDP | covered | `m3b_discovery`, `discovery.rs` | Off by default, `cdp run` |
| Port security | covered | `m3b_psec`, `psec.rs` | |
| Wireless (APs, WLC, WLANs) | non-goal | | Say so in the user docs |

## IP connectivity

| Item | Status | Test | Notes |
|---|---|---|---|
| ARP, ICMP, ping, traceroute | covered | `m2_traceroute`, `traceroute.rs`, `hosts.rs` | |
| IPv4 static and default routes | covered | `m1_static_routes`, `router.rs` | |
| OSPFv2, single area | covered | `m3_ospf`, `ospf_shell.rs`, `ospf_net.rs` | FRR oracle for routes and packets |
| OSPFv2, multi-area, externals | covered | `m3_ospf_areas` | Beyond the blueprint |
| OSPF: LSDB and next hops against FRR | missing | | `docs/m3-status.md` "FRR oracle, what is left" |
| OSPF: stub/NSSA, virtual links, authentication | missing | | Beyond the blueprint, not needed for 1.0 |
| Route selection (longest prefix, administrative distance) | partial | `router.rs` | `fib.rs` has per-kind AD; a configurable static distance (floating static) `[verify]` |
| First-hop redundancy (HSRP, VRRP) | covered | `m3b_fhrp`, `fhrp.rs` | Emulated |
| **IPv6**: addressing, EUI-64, link-local, static routes, NDP | **missing** | | Plan puts "basic IPv6" in M3b. Nothing exists: no `wire` module, no host or router support |

## IP services

| Item | Status | Test | Notes |
|---|---|---|---|
| DHCP server | covered | `m2_dhcp`, `dhcp.rs`, `dhcp_shell.rs` | |
| DHCP relay | covered | `dhcp.rs` (`relayed`) | |
| DNS (client, server) | covered | `m3b_dns`, `dns.rs` | No caching, recursion or reverse lookup |
| NAT and PAT | covered | `m3_exit_demo`, `nat.rs`, `nat_switch.rs` | |
| NTP | covered | `m3b_ntp`, `ntp.rs` | |
| Syslog | covered | `m3b_syslog`, `syslog.rs` | |
| TFTP (config backup/restore) | covered | `m3b_tftp`, `tftp.rs` | No flash or image copy |
| HTTP (client and server on PCs) | **missing** | | In the original scope. Needs a decision: web browser app on PCs or `curl`-style command |
| FTP | missing | | "TFTP/FTP-style basics" in scope; NAT ALGs wait on this |
| SNMP | missing | | On the blueprint `[verify]`. Not mentioned in the plan; decide in or out |
| QoS (per-hop behaviours, markings) | missing | | Blueprint concept `[verify]`. Decide in or out |
| Server device in the palette | missing | | Services live on PCs by design (2026-11-11); revisit only if HTTP needs it |

## Security fundamentals

| Item | Status | Test | Notes |
|---|---|---|---|
| Standard, extended, named ACLs | covered | `m3_acl`, `acl.rs` | |
| ACLs on VTY lines (`access-class`) | covered | `m3_exit_demo`, `remote.rs` | |
| Local passwords, `enable secret`, banners, lines | covered | `m2_mgmt`, `mgmt.rs` | Own hash format |
| Telnet and SSH to VTY lines | covered | `remote.rs` | SSH framing real, unencrypted |
| DHCP snooping, dynamic ARP inspection | missing | | Blueprint `[verify]`; not in the plan's scope list |
| AAA, 802.1X, VPN concepts | non-goal | | Concept topics; decide whether to list under "not planned" |
| Wireless security | non-goal | | |

## Management and tooling (our own scope, from PROMPT.md)

| Item | Status | Test | Notes |
|---|---|---|---|
| `show running-config`, `copy run start`, `reload` | covered | `m2_mgmt`, `mgmt.rs` | |
| Hostnames, banners, console and VTY lines | covered | `m2_mgmt` | |
| PC tools: `ping`, `tracert`, `ipconfig`, `netsh` | covered | `pc_prompt.rs` | |
| Packet inspector, simulation mode stepping | partial | `stepping.rs`, `snapshot.rs` | Summaries missing for LACPDU, CDP, LLDP, NTP, HSRP/VRRP, syslog, TFTP, DNS |
| Lab file format, save and load | covered | `lab.rs`, `lab_commands.rs` | |
| Accessibility pass | missing | | Plan item for M3b |
| XML importer (`import-xml`) | missing | | Plan: off by default, M3b at the earliest |

## Not blueprint, outside the plan

Automation and programmability (REST APIs, JSON, controllers, config management tools), network
architecture diagrams, cloud and virtualization concepts. They are concept topics with nothing to
simulate. State in the user docs that they are out.

## Totals (first pass)

Of the 48 rows above, the rows marked **missing** that block the exit criterion as the plan words it
are IPv6, HTTP, the accessibility pass and the importer. FTP, SNMP, QoS, DHCP snooping and DAI are
blueprint or scope items nobody has decided on yet.

## Next

1. Compare the topic list against the current published exam topics and fix the `[verify]` rows.
2. Open each test named here and confirm it checks that item, not just that it runs.
3. Decide on the undecided rows: FTP, SNMP, QoS, DHCP snooping, DAI.
4. Make CI check the table: every `covered` row names a test file that exists.
