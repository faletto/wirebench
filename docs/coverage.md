# Coverage matrix

Revised 2026-11-15 against the published exam topics. This is the M3b exit artifact: every in-scope
item with at least one scenario test.

**Source of the topic list:** Cisco's [200-301 CCNA v2.0 exam topics](https://learningcontent.cisco.com/documents/marketing/exam-topics/200-301_CCNA_v2.0_Exam_Topics_PDF.pdf)
(3-page PDF dated May 2026; Cisco's results also list v1.1). Numbering below follows v2.0. The
first draft of this file used the v1.1 domains from memory and missed what v2.0 added: IPv6 routing
and OSPFv3, root/loop/BPDU guard, floating statics, DHCP snooping, DAI, storm control, RA guard,
SFTP/SCP, DNS record types, AAA, SNMP and syslog severity, and the AI domain. Only public exam-topic
text was used (see `PROVENANCE.md`).

**Status:** `covered` = a named test drives it end to end; `partial` = works, gap named;
`missing` = not built; `non-goal` = excluded in the plan; `concept` = a "describe" topic with nothing
to simulate.

**How the tests were checked.** All `engine`, `netdev` and `wire` tests were run on 2026-11-15 and
pass. For each `covered` row I read the test function names in the named file and the golden
scripts' names, and grepped the source for the commands. **I did not read test bodies.** A row says
`covered` when a test name plainly matches the item, so a name that overstates a test would slip
through. Goldens are `testkit/scenarios/<name>.{script,out}`, run by
`crates/engine/tests/vlan_scenario.rs`.

## 1.0 Network infrastructure and connectivity (25%)

| Topic | Status | Test | Notes |
|---|---|---|---|
| 1.1 Interface and cable issues (collisions, errors, duplex, speed) | missing | | Links are ideal. No duplex/speed mismatch, error counters or cable types |
| 1.2 Hypervisors, VMs, containers | concept | | |
| 1.3 IPv4 addressing, assignment, subnetting | covered | `router.rs`, `hosts.rs`, `m2_dhcp` | Addressing is tested; subnetting as a skill is for the lab author |
| 1.4 IPv6 addressing, prefix sizing, EUI-64 | **missing** | | No IPv6 anywhere in `wire`, `netdev` or `engine` |
| 1.5 Wireless principles | non-goal | | Say so in the user docs |
| 1.6 Troubleshoot client connectivity | partial | `hosts.rs`, `pc_prompt.rs` | Wired Windows-style PCs only; no macOS/Linux wording, no wireless |
| 1.7 Troubleshoot DHCPv4 client, server, relay | covered | `dhcp.rs`, `dhcp_shell.rs`, `m2_dhcp` | `relayed` rig covers relay |

## 2.0 Switching and network access (25%)

| Topic | Status | Test | Notes |
|---|---|---|---|
| 2.1.a L2/L3 physical interfaces | covered | `router.rs`, `switch_shell.rs` | |
| 2.1.b 802.1Q trunks | covered | `vlans.rs`, `m2_vlans`, `dtp.rs`, `m2_dtp` | DTP is emulated, not the Cisco wire format |
| 2.1.c LACP port-channel | covered | `etherchannel.rs`, `m3b_etherchannel` | PAgP not simulated |
| 2.1.d SVI | covered | `svi.rs`, `m2_svi` | |
| 2.2 Edge-host port attributes: VLAN, port channel, LACP | covered | `vlans.rs`, `etherchannel.rs` | |
| 2.2 PoE | missing | | No power model |
| 2.2 Wireless APs, VoIP phones, IoT as endpoints | non-goal | | Plan non-goals |
| 2.3 CDP and LLDP | covered | `discovery.rs`, `m3b_discovery` | Off by default; `cdp run` |
| 2.4 show commands, show logs, ping, extended ping, traceroute, packet capture | partial | `traceroute.rs`, `m2_traceroute`, `syslog.rs`, `stepping.rs` | Ping and traceroute covered. **Extended ping missing** (`ping <dst>` is the only form). Packet capture is the inspector (partial, see below) |
| 2.5 Rapid PVST+: per-VLAN trees, root and port roles, states | covered | `stp_world.rs` (`each_vlan_has_its_own_tree`), `stp_net.rs`, `stp_oracle.rs`, `m2_stp` | mstpd oracle |
| 2.5.c PortFast | covered | `stp_net.rs` (`an_edge_port_forwards_at_once...`, `portfast_set_on_a_port_already_waiting...`) | |
| 2.5.d **Root guard, loop guard, BPDU guard** | **missing** | | `netdev/src/stp.rs` header lists them as "not modelled" |

## 3.0 IP routing (20%)

| Topic | Status | Test | Notes |
|---|---|---|---|
| 3.1 Read a routing table: protocol, prefix, AD, metric, default | covered | `router.rs`, `ospf_shell.rs` | `fib.rs` carries per-kind administrative distance |
| 3.2.a-c IPv4 default, network, host routes | covered | `router.rs`, `m1_static_routes` | |
| 3.2.d **Floating static** | **missing** | | `ip route <net> <mask> <via>` has no distance argument (`engine/src/shell.rs:393`) |
| 3.2 IPv6 static routing | **missing** | | See 1.4 |
| 3.3 Single-area OSPFv2: adjacencies, point to point, DR/BDR, router ID | covered | `ospf_net.rs`, `ospf_shell.rs`, `m3_ospf` | FRR oracle on routes and packets |
| 3.3 OSPFv3 for IPv6 | **missing** | | See 1.4 |
| 3.3 Point-to-point network type | covered | `ospf_net.rs` (random networks mix p2p and broadcast links), `ospf_shell.rs` (config replay) | No test named for p2p alone; the proptest sets `point_to_point` per link |
| Multi-area OSPF, externals (beyond blueprint) | covered | `ospf_net.rs`, `m3_ospf_areas` | |
| 3.4 HSRP and VRRP status | covered | `fhrp.rs`, `m3b_fhrp` | Emulated |

## 4.0 Network services and security (20%)

| Topic | Status | Test | Notes |
|---|---|---|---|
| 4.1 Local usernames | covered | `remote.rs` (`ssh_logs_in_with_a_local_user...`), `mgmt.rs` | |
| 4.1 AAA client (TACACS+, RADIUS) | missing | | No `aaa` commands |
| 4.2 SFTP / SCP file transfer | missing | | TFTP only (`tftp.rs`, `m3b_tftp`). SSH framing exists but is unencrypted; SCP would build on it |
| 4.3 NAT and PAT | covered | `nat.rs`, `nat_switch.rs`, `m3_exit_demo` | |
| 4.4 DNS records A, AAAA, CNAME, MX, NS, PTR | partial | `dns.rs`, `m3b_dns` | A only. No reverse lookup, caching or recursion |
| 4.5 IPsec VPNs | concept | | Describe-only topic |
| 4.6 IPv4 ACLs: standard, extended, numbered, named | covered | `acl.rs`, `m3_acl` | |
| 4.7.a **DHCP snooping** | **missing** | | |
| 4.7.b **Dynamic ARP inspection** | **missing** | | |
| 4.7.c **Storm control** | **missing** | | |
| 4.7.d **RA guard** | **missing** | | Needs IPv6 first |
| 4.7.e Port security | covered | `psec.rs`, `m3b_psec` | |

## 5.0 AI, and network operations and management (10%)

| Topic | Status | Test | Notes |
|---|---|---|---|
| 5.1-5.2 Agentic AI, prompt design | concept | | Nothing to simulate |
| 5.3 Management approaches (device, cloud, controller, automation, IaC) | concept | | |
| 5.4 SNMP function | missing | | Describe-only, but a simulator may want a minimal agent for completeness. Decide in or out |
| 5.5 Ansible / config management | concept | | Out of scope in the plan |
| 5.6 Syslog content, severity, facilities | covered | `syslog.rs`, `m3b_syslog` | Messages for link, config, OSPF, HSRP/VRRP, port security. Severity levels and `logging trap` filter covered |

## Our own scope (PROMPT.md, not on the blueprint)

| Item | Status | Test | Notes |
|---|---|---|---|
| NTP, `clock` | covered | `ntp.rs`, `m3b_ntp` | |
| Management plane: secrets, banners, lines, `copy run start`, `reload` | covered | `mgmt.rs`, `m2_mgmt` | |
| Telnet and SSH to VTY lines | covered | `remote.rs` | SSH unencrypted |
| TCP/UDP sockets | covered | `engine/tests/tcp.rs` | |
| PC tools: ping, tracert, ipconfig, netsh | covered | `pc_prompt.rs` | |
| Packet inspector, simulation stepping | partial | `stepping.rs`, `snapshot.rs` | Per-hop stepping works. Summaries missing for LACPDU, CDP, LLDP, NTP, HSRP/VRRP, syslog, TFTP, DNS |
| Lab file format, save and load | covered | `lab.rs`, `lab_commands.rs` | |
| HTTP | **missing** | | In PROMPT.md scope; not a v2.0 topic |
| FTP | missing | | In PROMPT.md scope; not a v2.0 topic. NAT ALGs wait on it |
| Server device | missing | | Services live on PCs by design (2026-11-11) |
| Accessibility pass | missing | | Plan item |
| XML importer (`import-xml`) | missing | | Plan: off by default |

## Reading the gaps

**Blockers under the plan's exit criterion** (items on the v2.0 blueprint that students will test,
with nothing built): IPv6 (1.4, 3.2, OSPFv3, 4.7.d), the three spanning-tree guards (2.5.d),
floating statics (3.2.d), the Layer 2 security features DHCP snooping, DAI and storm control (4.7.a-c),
AAA (4.1), SFTP/SCP (4.2), DNS record types (4.4), and interface error and cable diagnosis (1.1).

**Cheap wins** (a day or two each): floating static, BPDU/root/loop guard, storm control, extended
ping, DNS records beyond A. **Big pieces:** IPv6 (with OSPFv3 and RA guard), DHCP snooping plus DAI.

**Decisions for the project owner:** whether 1.0 means "all of v2.0" or "v1.1-era scope plus IPv6";
PoE and cable diagnosis (1.1, 2.2); SNMP; AAA as a configuration-only feature.

## Next

1. Read test bodies for `covered` rows, starting with the ones that rest on a golden only.
2. CI check: every `covered` row names a test file that exists.
3. Decide the open items above and turn the missing rows into issues.
