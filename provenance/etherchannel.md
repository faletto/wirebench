# EtherChannel and LACP

**Date:** 2026-11-08
**Source category:** RFC/IEEE public standard (IEEE 802.1AX / 802.3ad clause 43, recalled from the
published standard), public vendor documentation recalled from memory (command names and `show`
layouts), and own tests (`crates/engine/tests/etherchannel.rs`, `crates/wire/src/lacp.rs`, golden
`testkit/scenarios/m3b_etherchannel.*`). No Packet Tracer material was consulted.
[verify] marks choices not checked against a real device.

## Wire format (`wire::Lacpdu`)

Destination `01:80:c2:00:00:02`, Ethernet type `0x8809`, subtype 1 (LACP), version 1. Actor TLV
(type 1, length 20) and Partner TLV (type 2, length 20): system priority, system id, key, port
priority, port number, state byte (activity, timeout, aggregation, synchronization, collecting,
distributing, defaulted, expired), 3 reserved bytes. Collector TLV (type 3, length 16) and a
terminator, padded to 110 bytes. Bridges never forward the destination address.

## Behaviour

- `channel-group N mode on|active|passive`. Mixing `on` with a LACP mode in one group is refused.
  Up to 8 ports per group, groups 1..6.
- **PAgP (`desirable`/`auto`) is not simulated.** It is proprietary and its wire format is not
  published; the commands are absent rather than faked.
- Static (`on`) members bundle as soon as their link is up, whatever the neighbour does. This
  reproduces the real-world hazard of a one-sided static channel.
- LACP members bundle once a LACPDU has been heard from the neighbour, and the neighbour is active
  or this end is. Passive ends only transmit after hearing an active neighbour. PDUs are sent on
  link up, on change, and every 30 s (slow rate); a silent neighbour is dropped after 90 s. The fast
  rate is not implemented. Members must agree on the neighbour's system id.
- A bundle is one port for spanning tree and the MAC table. It is represented by its
  lowest-numbered member (whatever the state of that member), so losing a cable changes the path
  cost, not the port identity. The cost for N Fast Ethernet members is 19, 12, 9, 8, 7, 6, 6, 5 [verify].
- A frame leaves by one member chosen by hashing (`port-channel load-balance`: src-mac default,
  dst-mac, src-dst-mac, src-ip, dst-ip, src-dst-ip). A member whose link is down is skipped
  at once, without waiting for LACP to notice.
- Settings typed on `interface port-channel N` go to every member, and settings typed on a member
  go to the group and the other members, except `shutdown` and port security, which stay per port.
  A port joining an existing group takes the group's settings [verify: a real switch refuses a
  mismatch instead].
- `interface range` is added alongside (`fa0/1 - 4`, `fa0/1-4`, `fa0/1,fa0/3`); the prompt stays
  `(config-if)#` rather than `(config-if-range)#`.
- **Administrative shutdown now drops the link at both ends** (new `Kernel::set_carrier`). Before,
  a shut switch port left its neighbour believing the link was up. Error-disabled ports do the same.

## Left out

Hot-standby members and `lacp max-bundle`, `lacp port-priority`/`system-priority`, fast rate,
minimum links, Layer 3 port-channels, `show etherchannel detail`, load balancing by L4 port.
