# Dynamic Trunking Protocol

**Date:** 2026-10-26
**Source category:** public doc (general knowledge of Cisco's DTP: the mode table, the
`dynamic auto`/`desirable` and `nonegotiate` commands, `show interfaces ... switchport` and
`show interfaces trunk`), recalled rather than re-read; own tests (`crates/engine/tests/dtp.rs`,
golden `testkit/scenarios/m2_dtp.*`). DTP is proprietary and not documented by a standard, so the
packet layout below is from memory of captures. [verify] marks what was not checked.

## What is modelled

- **Port modes.** `access`, `trunk`, `dynamic auto` and `dynamic desirable`. **The default is
  `dynamic auto`**, as on the real thing; this replaces the old simplification (see
  `vlan-port-defaults.md`). A dynamic port is an access port until negotiation says otherwise.
- **The decision table.** A port trunks when its neighbour is `trunk` or `desirable` and it is
  `auto` or `desirable`; or when it is `desirable` and the neighbour is `auto`. `auto` against
  `auto` stays access. `access` never trunks and a static `trunk` always is one, whatever the
  other end says (so a trunk against an access port is one-sided and drops the tagged frames).
  This is tested for all sixteen pairs.
- **Messages.** A DTP frame goes to `01:00:0c:cc:cc:cc`, untagged, LLC/SNAP (OUI `00:00:0c`,
  protocol `0x2004`), version 1, with TLVs for domain, trunk status, trunk type (always 802.1Q)
  and the sender's address. The status byte is the admin mode in the low nibble (1 on, 2 off,
  3 desirable, 4 auto) and `0x80` when the sender is trunking now [verify the values].
- **When it speaks.** Ports set to `trunk`, `desirable` or `access` send when configured or when
  their link comes up and then every 30 s; **a `dynamic auto` port never starts a conversation
  but answers any DTP that is news to it.** That is what lets a switch nobody typed at become
  a trunk when its neighbour asks. A neighbour not heard for 90 s is forgotten. A cable lost or a
  neighbour changing to `access` ends the negotiated trunk at once.
- **`switchport nonegotiate`** sends nothing and ignores everything. It is refused on a dynamic
  port with the real message (`Command rejected: Conflict between 'nonegotiate' and 'dynamic'
  status.`) [verify the wording], and so is setting a dynamic mode while it is on.
- **Commands.** `switchport mode dynamic auto|desirable`, `switchport nonegotiate` and its `no`,
  `show interfaces <name> switchport` (the common lines, not all of them), and `show interfaces
  trunk` now lists negotiated trunks with the mode as `on`, `auto` or `desirable`.
  `show vlan brief` lists ports that are access in practice, including dynamic ones.

## Choices that are ours

- **A quiet switch stays quiet.** A switch with every port at the default sends no DTP and
  schedules no timer, so labs that never touch trunking are unchanged apart from the trace of
  those that do (the golden for the two-VLAN lab moved by 3 us per ping because a static trunk
  now sends DTP).
- **Ticking happens only while something is negotiating or spanning tree runs**, on the same
  background timer as spanning tree. Link changes are noticed at the next command or tick.
- **One domain.** There is no VTP, so the domain TLV is empty and always matches.
- **802.1Q only.** No ISL, `switchport trunk encapsulation`, or `show dtp`.
- **Saved labs** that had no mode line now load as `dynamic auto` where they used to load as
  `access`; the two differ only next to a neighbour that asks for a trunk.
