# Port security and error-disabled ports

**Date:** 2026-11-08
**Source category:** public doc (general knowledge of vendor documentation, recalled rather than
re-read) and own test (`crates/engine/tests/psec.rs`, golden `testkit/scenarios/m3b_psec.*`).
No Packet Tracer material was consulted. [verify] marks choices not checked against a real device.

## Behaviour implemented

- `switchport port-security` is refused on a dynamic port (`switchport mode access` or `trunk`
  first), which is how the real thing behaves and a common stumbling block. The port count limit is
  `maximum` (1 by default, 1..132 here).
- Addresses come from three places: typed in (`mac-address <mac>`), learned and kept in the
  configuration (`mac-address sticky`, listed as `mac-address sticky <mac>`), or learned and
  forgotten (dynamic, the default). All three count against the maximum.
- `violation shutdown` (default): the frame is dropped, the violation is counted and the whole
  port becomes error-disabled (`err-disabled` in `show interfaces status`, `Secure-shutdown` in
  `show port-security interface`). `restrict` drops and counts. `protect` drops and counts
  nothing [verify: the real device also logs, we have no syslog].
- Leaving error-disabled: `shutdown` then `no shutdown`, or `errdisable recovery cause
  psecure-violation` with `errdisable recovery interval <secs>` (default 300).
- Dynamic addresses are forgotten when the port goes down, is shut, or the maximum changes.
  Turning `sticky` on converts the dynamic addresses to sticky and turning it off converts sticky
  ones to dynamic [verify].
- Lowering the maximum below the addresses in use, and typing in an address beyond the maximum,
  are refused.
- Only frames with a unicast source address are checked; spanning tree and DTP frames are handled
  before the check.

## Left out

- Aging (`switchport port-security aging ...`), per-VLAN limits on trunks, `violation` logging
  and SNMP traps, secure addresses seen on a second port, `show port-security` totals beyond
  the first line. The "Max Addresses limit in System" line of the real summary is omitted.
- Output layouts are written from memory of the documented format and are not byte-checked.
