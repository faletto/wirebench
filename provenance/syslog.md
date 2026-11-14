# Syslog and device logging

**Date:** 2026-11-11
**Source category:** RFC (RFC 3164 for the message format and priority, UDP port 514); public vendor
documentation recalled from memory (command names, message texts such as `%LINK-3-UPDOWN`, the
layout of `show logging`); own test (`crates/wire/src/syslog.rs`, `crates/netdev/src/syslog.rs`,
`crates/engine/tests/syslog.rs`, golden `testkit/scenarios/m3b_syslog.*`, fuzz target
`wire_syslog`). No Packet Tracer material was consulted. [verify] marks choices not checked against
a real device.

## Behaviour

- **Messages** are `<PRI>` plus text, PRI = facility * 8 + severity. Facility is local7 (23) [verify:
  the usual device default]. The text is `*Mar  1 00:00:05.123: %LINK-3-UPDOWN: ...`: a timestamp
  from the device clock with the flag `show clock` shows (`*` clock never set or synchronised, `.`
  time source lost), then the tag. There is no sequence number [verify: off unless
  `service sequence-numbers`].
- **Configuration** (routers and switches): `logging host <ip>`, `logging trap <level>` (default
  informational), `logging buffered [size [level]]` (default 4096 bytes, debugging),
  `service timestamps log datetime msec` and its `no` form (on by default; off drops the timestamp
  [verify]), `no` forms of each, `show logging`, `clear logging`. Levels by name or number.
- **Hosts** get every message at the trap level or more severe; the buffer keeps those at its own
  level or more severe and drops its oldest lines when full. Messages sent while a router has no route
  to the host are lost, as on a device.
- **What is logged**: interface line state, found by a one-second watcher that runs only while a host
  or the buffer is configured: `%LINK-3-UPDOWN` (cable or far end, severity 3), `%LINK-5-CHANGED ...
  administratively down` (shutdown, severity 5), `%LINEPROTO-5-UPDOWN` for every interface including
  SVIs (an SVI has no `%LINK` message); and `%SYS-5-CONFIG_I: Configured from console by console` when
  a session leaves configuration mode (always "console"; vty sessions are not told apart). The first
  look at an interface logs nothing. Switch ports are watched by the switch, SVIs by its router half.
- **Protocol messages**: `%OSPF-5-ADJCHG` when an adjacency reaches FULL (`from LOADING to FULL,
  Loading Done`, or `EXCHANGE ... Exchange Done`) and when it leaves it (`from FULL to DOWN, Neighbor
  Down: Dead timer expired` or `Interface down or detached`), as `log-adjacency-changes` does by
  default [verify]; the neighbour is named by router ID. `%STANDBY-6-STATECHANGE` (HSRP) and
  `%VRRP-6-STATECHANGE`: `<interface> Grp <n> state <old> -> <new>`. `%PORT_SECURITY-2-PSECURE_VIOLATION`
  for a violation in restrict or shutdown mode (protect is silent; every violating frame is logged,
  a real device rate-limits), `%PM-4-ERR_DISABLE` when a port is shut by one and `%PM-4-ERR_RECOVER`
  when errdisable recovery brings it back. The router looks at these after every event, so the old
  state is the last one it saw, which can be a step before the true previous one. On a switch the
  router half is looked at once a second.
- **Protocol messages**: `%OSPF-5-ADJCHG` when an adjacency reaches FULL (`from LOADING to FULL,
  Loading Done`, or `EXCHANGE ... Exchange Done`) and when it leaves it (`from FULL to DOWN, Neighbor
  Down: Dead timer expired` or `Interface down or detached`), as `log-adjacency-changes` does by
  default [verify]; the neighbour is named by router ID. `%STANDBY-6-STATECHANGE` (HSRP) and
  `%VRRP-6-STATECHANGE`: `<interface> Grp <n> state <old> -> <new>`. `%PORT_SECURITY-2-PSECURE_VIOLATION`
  for a violation in restrict or shutdown mode (protect is silent; every violating frame is logged,
  a real device rate-limits), `%PM-4-ERR_DISABLE` when a port is shut by one and `%PM-4-ERR_RECOVER`
  when errdisable recovery brings it back. The router looks at these after every event, so "old
  state" is the last state it saw, which can be one step before the true previous one. On a switch
  the router half is looked at once a second.
- **PC collector**: `syslogd start | stop | show | clear` on a PC. It keeps up to 1000 messages with
  the sender and `facility.severity`. This is our own invention, not a Windows or IOS command: a PC
  command prompt has no syslog server, so the lab needs something to read what devices send.
- Logging is back to its defaults on `reload` and comes back from the saved configuration.

## Left out

`logging console`, `logging monitor`, `terminal monitor` (there is no console to print to),
`logging source-interface`, `logging facility`, `logging on`, `service sequence-numbers`, rate
limiting, TCP syslog, structured data (RFC 5424). Messages for spanning tree, EtherChannel, DHCP, NAT,
ACL `log` and NTP are not generated. `show logging` follows the documented
layout approximately; the counters other than the message counts are placeholders.
