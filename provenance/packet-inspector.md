# Packet inspector

**Date:** 2026-10-26
**Source category:** own design; field names follow the protocol standards (RFC 791, 792, 768,
826, 2131, IEEE 802.1D/Q), and the layout follows what packet analysers such as Wireshark show
(recalled, not copied). Tests: `crates/wire/src/dissect.rs`, `crates/engine/tests/stepping.rs`,
`crates/app/tests/inspector.rs`.

## What it shows

- **Hop log** (simulation panel): every frame delivered, now selectable, with a **filter** box.
  Terms are ANDed; `-term` negates. A bare layer name (`arp`, `ip`, `icmp`, `udp`, `dhcp`, `stp`,
  `vlan`, `ethernet`) or `proto:x` means the frame has that layer; `dev:r1` (or `at:r1`) means
  delivered to that device; `port:2` that port; any other word is searched for in the device name
  and the one-line summary, ignoring case. Half-typed terms never error.
- **Field tree**: the layers of the selected frame (Ethernet with its 802.1Q tag, ARP, IPv4,
  ICMP, UDP, DHCP with its options, spanning tree BPDUs, DTP), each field with its decoded value.
  The dissector reads the bytes itself rather than through the parsers so every field has an
  offset. A frame cut short is shown as far as it goes and marked; it never fails. DNS, TCP and
  other protocols we do not generate are not decoded.
- **Hex view**: sixteen bytes a row with the printable characters. Picking a field (or a layer)
  highlights its bytes.
- **Back**: undoes the latest Step or queued ping exactly. The engine keeps a copy of the whole
  world before each of the last 64 steps and restores it, so ARP caches, timers, queued frames
  and the trace hash come back as they were. Typing a console command, Run to idle, advancing
  time, or the app's own live clock (spanning tree) forgets the copies, so Back cannot undo a
  configuration change by accident.

## Not done

- Forward again after Back: stepping forward repeats the same frames (the simulation is
  deterministic), which is tested, but there is no separate Redo.
- Saving a capture (pcap), and filters on field values (`ip.ttl == 1`).
- Not tried in a real browser; the panel is as wide as the existing simulation panel.
