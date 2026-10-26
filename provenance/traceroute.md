# Traceroute, tracert and router ping

**Date:** 2026-10-25
**Source category:** standard (RFC 792 ICMP, RFC 1122 and RFC 1812 for who answers and from which
address) and public doc (general knowledge of the vendors' commands and output, recalled rather
than re-read); own test (`crates/engine/tests/traceroute.rs`, `trace_shell.rs`, golden
`testkit/scenarios/m2_traceroute.*`). [verify] marks choices not checked against a real device.

## What the network does

- A probe with a TTL that runs out at a router makes that router send ICMP time exceeded from the
  address of the **interface the probe arrived on**, quoting the probe's IP header and first
  eight bytes. The sender matches the reply to a probe by what is quoted (a UDP destination port,
  or an echo's identifier and sequence number).
- A UDP packet for a closed port makes a host or router send ICMP **port unreachable** (type 3
  code 3). A router sends it **from the address that was probed**, not from the interface the
  probe arrived on (RFC 1122 asks for this); that is why tracing to a router's far interface
  address ends on that address.
- A router with no route for a forwarded packet sends net unreachable (type 3 code 0).
- **Rate limit [verify].** A router sends at most one forwarded-traffic unreachable every 500 ms
  (the default of the real `ip icmp rate-limit unreachable`). Time exceeded is not limited. The
  port unreachable a router sends for traffic addressed to itself is **not** limited here, so
  tracing to a router's own address shows three answers; whether the real limit applies to that
  case is not settled.

## Commands

- PCs: `tracert <dst>` and `tracert -h <hops> <dst>`: ICMP echo requests, three per hop, the
  default maximum of 30 hops, Windows-style layout, `Request timed out.` for a silent hop, and
  `reports: Destination net unreachable.` when a router cannot deliver.
- Routers and layer 3 switches: `traceroute <dst>` and `traceroute <dst> ttl <min> <max>`: UDP
  probes to ports from 33434 up, three per hop, IOS-style layout with `*` for silence and `!N`,
  `!H`, `!P`, `!A` where a router says it cannot deliver. A hop that answers "unreachable" ends
  the trace at once. `ping <dst>`: five echo requests as `!` (reply), `.` (nothing) and `U`
  (unreachable), with a success rate line.
- Per-probe timeouts are three seconds for traces and two for ping (IOS defaults [verify]).

## Choices that are ours

- Times are printed in **microseconds** (`56 usec`, `13 us`) rather than whole milliseconds,
  because everything here is faster than a millisecond; a real device would show `0 msec`.
- The first probe through a path includes ARP resolution and so takes longer. A real router
  drops the first packet while it resolves a neighbour (the classic `.!!!!`) [verify]; here the
  packet waits, so the first ping succeeds.
- A router with no route for a probe it originates prints `% No route to <dst>` (our wording);
  a PC with nowhere to send it prints `General failure.` [verify both].
- Traces to a host that does not exist show silence from the router that cannot resolve it: no
  host unreachable is sent after an ARP failure yet.
- "Idle" for the simulation means nothing due within 30 virtual seconds, so a long trace of
  silent hops runs but does not hold up Run or Play afterwards.
- Not implemented: extended traceroute dialogs (`source`, `probe`, `timeout`, `port` options),
  `tracert -d` and other switches, IPv6, MPLS and ICMP extensions in replies.
