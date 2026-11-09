# CDP and LLDP

**Date:** 2026-11-08
**Source category:** RFC/IEEE public standard (IEEE 802.1AB-2016 for LLDP), public documentation
and packet dissectors for CDP (its TLV layout is long published, for instance by the Wireshark
dissector; recalled rather than re-read), and own tests (`crates/wire/src/discovery.rs`,
`crates/netdev/src/discovery.rs`, `crates/engine/tests/discovery.rs`, golden
`testkit/scenarios/m3b_discovery.*`). No Packet Tracer material was consulted.
[verify] marks choices not checked against a real device.

## Wire formats

- **CDP**: 802.3 frame to `01:00:0c:cc:cc:cc`, LLC/SNAP `aa aa 03 00 00 0c 20 00`, then version 2,
  holdtime, Internet checksum, and TLVs: device id (1), addresses (2, NLPID `0xcc` for IPv4), port
  id (3), capabilities (4: router 0x01, switch 0x08, host 0x10), software version (5), platform
  (6). DTP shares the destination address and is told apart by the SNAP protocol id (0x2004).
- **LLDP**: Ethernet type `0x88cc` to `01:80:c2:00:00:0e`; TLVs chassis id (subtype 4, MAC), port
  id (subtype 5, interface name), TTL, port description, system name, system description,
  system capabilities, management address (IPv4), end. A TTL of 0 withdraws the entry.
- Text fields name this simulator ("wirebench router"); no vendor platform strings are used.

## Behaviour

- **CDP is off by default (deliberate deviation).** A real device runs it from the start. Here a
  device with CDP running has a permanent one-second background timer, which would stop every
  lab from ever going idle (the app stops repainting when nothing is happening) and put neighbour
  frames in every golden. `cdp run` turns it on; `lldp run` likewise (LLDP is also off by default
  on real devices). Both appear in the running config, so a saved lab keeps them.
- Advertise on link up and every `cdp timer` (60 s) / `lldp timer` (30 s); entries live for the
  advertised holdtime (180 s / 120 s) and vanish with the link. Per-interface `cdp enable`,
  `lldp transmit`, `lldp receive` (all on by default).
- A switch running CDP consumes CDP frames; one not running it floods them like any unknown
  multicast (the old "CDP sees through a switch" quirk). LLDP is link-local and is never bridged
  whether or not the switch runs it.
- Switches send and listen on every physical port, so a bundle shows one neighbour per member
  cable. Frames go untagged.
- Output layouts (`show cdp neighbors [detail]`, `show cdp entry`, `show cdp interface`, `show
  lldp neighbors [detail]`) follow the documented formats from memory, with simplifications: the
  capability legend lists only R, S, H, and there is no `show cdp traffic`.

## Left out

CDP version 1, VTP/native VLAN/duplex/power TLVs, LLDP-MED and 802.1 / 802.3 organisational TLVs,
LLDP reinitialisation delay and fast start, `cdp log mismatch`, SNMP. The packet inspector does not
yet decode these frames.
