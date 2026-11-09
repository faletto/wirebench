# Device clock and NTP

**Date:** 2026-11-08
**Source category:** RFC (RFC 5905 NTPv4 packet format and the offset/delay calculation), public
vendor documentation recalled from memory (command names, `show` layouts), and own tests
(`crates/wire/src/ntp.rs`, `crates/netdev/src/ntp.rs`, `crates/engine/tests/ntp.rs`, golden
`testkit/scenarios/m3b_ntp.*`). No Packet Tracer material was consulted. [verify] marks choices
not checked against a real device.

## Behaviour

- A device clock is the simulation time plus an offset. A new device reads 00:00:00 UTC on
  Mon 1 Mar 1993 [verify: the usual factory default]. `clock set hh:mm:ss day Mon year` (exec mode)
  sets it, interpreted in the configured zone; impossible dates (30 Feb) are refused.
  `clock timezone NAME HOURS` only affects display and `clock set`; there is no summer time.
  The clock is hardware and survives `reload`; it is not part of the running config.
- `show clock` prefixes `*` (not authoritative: never set and never synchronised), `.` (was set or
  synchronised, but NTP is configured and not synchronised now) or nothing [verify].
- **NTP client**: `ntp server <ip>` polls every 64 s (first poll one second after configuring).
  After 3 good answers the best server (lowest stratum, then delay) is trusted and the clock is
  **stepped** to it; there is no slewing, filtering, dispersion model or clustering, and no
  authentication, `prefer`, `iburst` or broadcast mode. A server that stays silent for 8 polls
  (the reach register empties) is dropped and the clock is unsynchronised again. Only answers
  matching the latest request (origin timestamp) from a stratum 1..15, non-alarm server count.
- **NTP server**: a device answers client requests while it is synchronised (own stratum = its
  server's + 1) or is an `ntp master [stratum]` (default 8, reference id `LOCL`). A device that is
  not synchronised does not answer, so an unsynchronised router is never a source. Replies come
  from the address that was asked. Switches do this through their VLAN interfaces.
- Packets are real NTPv4 (48 bytes, UDP 123). Timestamps cover era 0 only (until 2036).
- `show ntp status` and `show ntp associations` follow the documented layouts approximately;
  frequency, precision, dispersion and the `ntp uptime` line are placeholder values.

## Left out

`ntp source`, `ntp access-group`, `ntp authenticate`, `ntp peer`, `ntp broadcast`, `ntp
update-calendar`, `clock summer-time`, `show ntp associations detail`, a calendar separate from the
software clock, drift and the clock-discipline algorithm.
