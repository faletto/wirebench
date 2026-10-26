# VLAN forwarding on access and trunk ports

**Date:** 2026-10-25
**Source category:** standard (IEEE 802.1Q tagging, bridging) plus public doc for the vendor
specifics below; own test (`crates/engine/tests/vlans.rs`). [verify] marks choices not checked
against a real switch.

Standard behaviour: a frame belongs to exactly one VLAN; trunks carry a 4-byte 802.1Q tag with
the VLAN id; MAC learning and flooding are per VLAN.

Vendor-specific choices in `crates/netdev/src/switch.rs`:

- **Native VLAN** frames leave a trunk untagged, and untagged frames arriving on a trunk belong to
  the native VLAN. A native VLAN mismatch between the two ends of a trunk therefore moves traffic
  from one VLAN into another (tested as a leak; real switches log a CDP/STP warning, which we do
  not model).
- A frame tagged with VLAN id 0 (priority tag) is treated as native-VLAN traffic.
- The **allowed list** applies to the native VLAN too: if the native VLAN is removed from it,
  untagged traffic on that trunk is dropped.
- **Access ports** accept untagged frames, and tagged frames whose id equals the access VLAN or is
  0; frames tagged for any other VLAN are dropped [verify: real behaviour differs between
  platforms].
- A frame in a VLAN that does not exist is dropped, on ingress and on egress.
- Changing any VLAN setting on a port forgets the addresses learned on that port.
- A known destination whose learned port no longer carries the VLAN is flooded like an unknown one.
- Priority bits of a tag are preserved across trunks; an untagged frame gets priority 0 when it is
  tagged.

Not implemented: pruning, private VLANs, 802.1Q tunnelling. Trunk negotiation is in `dtp.md` and
per-VLAN spanning tree in `spanning-tree.md` (both added after this note was written; a port's
mode here is its *operational* mode, which negotiation can change).
