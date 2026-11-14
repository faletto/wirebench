# DNS: name servers, host tables and name lookup

**Date:** 2026-11-11
**Source category:** RFC (RFC 1035 for the message format, name compression and response codes; RFC 1034
for the lookup model); public vendor documentation recalled from memory (command names, the
layouts of `show hosts`, the `Translating ...` lines and Windows `nslookup`/`ping` wording); own test
(`crates/wire/src/dns.rs`, `crates/netdev/src/dns.rs`, `crates/engine/tests/dns.rs`, golden
`testkit/scenarios/m3b_dns.*`, fuzz target `wire_dns`). No Packet Tracer material was consulted.
[verify] marks choices not checked against a real device.

## Behaviour

- **Messages**: UDP port 53, standard queries for A records, answers with A and CNAME records (any
  other type keeps its data as bytes), response codes 0 to 5, compression pointers read (backwards only,
  so they cannot loop) and never written. Authority and additional records are skipped. No TCP, no
  truncation, no EDNS, no AAAA or PTR (a reverse lookup is not supported).
- **Client** (routers, switches through their VLAN interfaces, PCs): the name is looked up before the
  command runs, letting simulated time pass, and replaced by the address in `ping`, `traceroute`, `tracert`,
  `telnet` and `ssh -l <user>`. Nothing else uses names (not `copy tftp://`, which needs an address,
  and not a bare name typed at the prompt, which a real router would try to telnet to [verify]).
  A name without a dot gets the `ip domain-name` first and is then tried as typed. Servers are asked
  in order, each twice with two seconds' wait; a server that answers "no such name" ends the
  search for that name, one that is silent passes it to the next.
- **Router and switch**: `ip name-server <ip>` (up to six, repeated lines add), `ip host <name>
  <ip>` (names kept in lower case; the table is checked before any server), `ip domain-lookup` (on by
  default), `no` forms, `show hosts` (domain, servers, the static table; there is no cache, so it
  holds only permanent entries), `ip dns server` (answer queries from the `ip host` table). Output on
  success is `Translating "name"...domain server (a.b.c.d) [OK]`; on failure the same line without `[OK]`
  and `% Unrecognized host or address, or protocol not running.`, which is also what a name with no
  server or with lookup off gets.
- **PC**: `nslookup <name> [server]` in the Windows layout (server shown as `UnKnown`, as it is without
  reverse lookup), `netsh interface ip set dns <name> static <ip>` (DHCP already supplies servers),
  `ping` and `tracert` with a name print `Pinging <name> [<ip>]:` and `Tracing route to <name> [<ip>]`,
  and Windows' own wording when the name does not resolve. `dnsd start | stop | add <name> <ip> |
  delete <name> | list` runs a name server holding names in memory. `dnsd` is our own invention, like
  `syslogd` and `tftpd`: a PC command prompt has no DNS server.
- Settings come back from the saved configuration and are cleared by `reload` like the rest.

## Left out

Caching and `clear host`, `ip domain-list`, `ip domain-name` search lists beyond the one name, `ip
domain-lookup source-interface`, `ip name-server vrf`, recursion (a name server only answers from its own
table), zones and other record types, IPv6, DNS over TCP, `nslookup` interactive mode and `set type`,
`ipconfig /displaydns` and `/flushdns`, a hosts file on PCs, and names in `copy tftp://`.
