# IP access lists

**Date:** 2026-11-06
**Source category:** public doc (general knowledge of how IOS access lists are configured and shown,
recalled rather than re-read; no Packet Tracer material) for the command line and its output; standard
(RFC 791, RFC 792, RFC 793/9293 and RFC 768 for the header fields matched; RFC 1122 for when an ICMP
error may be sent); own test (`crates/netdev/src/acl.rs` unit and property tests,
`crates/engine/tests/acl.rs`, golden `testkit/scenarios/m3_acl.*`). [verify] marks choices not checked
against a real device.

## What is modelled

- **Standard** lists (source address only) and **extended** lists (protocol, source and destination
  address, TCP/UDP port conditions `eq neq lt gt range`, `established`, ICMP type and code), numbered
  (1-99 and 1300-1999 standard; 100-199 and 2000-2699 extended) and named. Addresses are an address
  plus wildcard bits; the wildcard need not be contiguous, and the address is stored with the ignored
  bits cleared. `any` and `host` are the usual shorthands.
- First matching line decides; the end of a list denies (the implicit deny is not counted in
  `show access-lists`).
- Lines have sequence numbers (10, 20, ...). In a named list (or a numbered one entered with
  `ip access-list extended 101`) a line can be inserted with an explicit number and removed with
  `no <number>` or by retyping it after `no`. `remark` lines take a number but are not listed by
  `show access-lists`, only in the configuration [verify].
- `ip access-group <list> in|out` on physical interfaces, subinterfaces and SVIs. A name with no list
  behind it **permits everything** (and takes effect when the list is created) [verify].
- Order of operations in the router: the inbound list sees every packet that arrives, including those
  addressed to the router; then the routing decision (no route and TTL expiry are decided first for
  forwarded packets); then the outbound list, which judges **forwarded packets only**. Packets the
  router originates (its own pings, ICMP errors, DHCP) are never filtered by an outbound list.
  NAT is not modelled yet; its place in this order is left for the NAT work [verify].
- A refused packet is dropped and the sender is told with ICMP destination unreachable code 13
  (administratively prohibited), from the address of the interface it arrived on, at most one per
  half second like the other unreachables, and not for broadcasts, packets from 0.0.0.0, or later
  fragments (RFC 1122 3.2.2) [verify the source address for an outbound refusal].
- Hit counters per line, shown as `(N matches)` (`(1 match)` for one), reset by
  `clear access-list counters [list]`.
- Output and configuration forms: `show access-lists [list]` and `show ip access-lists [list]`;
  in a standard list a wildcard prints as `permit 10.0.0.0, wildcard bits 0.0.0.255`; known port
  numbers print as names (`eq www`). `running-config` prints numbered lists as `access-list N ...`
  lines, and as an `ip access-list` block (with the numbers that differ from 10, 20, ...) once their
  numbering was edited. The printed text replays to the same configuration.

## Packets that have no ports

A later fragment has no TCP/UDP header, so port, `established` and ICMP conditions cannot be
evaluated. A **permit** line with such a condition lets it through and a **deny** line with one does
not catch it; lines with address and protocol only match as usual [verify]. The `fragments` keyword
is not supported.

## Not done

- `access-class` on a `line` is accepted, stored and printed, but **nothing enforces it**: there is no
  Telnet or SSH to a router yet (M3b). It starts to matter when those exist.
- `log` and `log-input` are refused with a message; matches are counted but not logged.
- No `reflexive`, `time-range`, `dynamic`, `object-group`, `ipv6 access-list`, `show ip interface`
  access-list lines, or `ip access-list resequence`.
- Port and ICMP names are a short list (the common ones), not all of IOS's.
- Error messages are simplified: a bad line in an ACL command names the token it stopped at instead of
  pointing a `^` marker at it.
- The packet inspector has no TCP dissector yet (`wire::TcpSegment` exists for the next work and is
  fuzzed, but the ACLs read ports and flags straight from the header bytes so that a truncated first
  fragment still matches).

## How it was checked

- A property test compares the evaluator with an independent bit-by-bit reference on random lists
  and packets (also checking the hit counters), another that every rule prints as text that parses
  back to the same rule, and one that parsing arbitrary words never panics.
- Console tests cover inbound lists seeing traffic to the router, outbound lists skipping the router's
  own packets, a list created or deleted after it was applied, counters, sequence editing, kind and
  range errors, replaying the printed configuration, and a save and reload.
- Golden transcript `testkit/scenarios/m3_acl.*`, written by the assistant: no one who knows IOS has
  compared it with a real device.
