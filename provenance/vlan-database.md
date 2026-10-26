# VLAN database: VLAN 1, creation, naming

**Date:** 2026-10-25
**Source category:** public doc (general knowledge of vendor documentation, not re-read for this
entry) and own test (`crates/engine/tests/vlans.rs`, `switch_shell.rs`). Items marked [verify]
have not been checked against a real device.

Behaviour implemented:

- VLAN 1 always exists, is named `default`, and cannot be deleted (`% Default VLAN 1 may not be
  deleted.`).
- `vlan N` creates VLAN N if it does not exist and enters VLAN configuration mode. A VLAN created
  without a name is called `VLANnnnn` (zero-padded to four digits).
- `switchport access vlan N` for a VLAN that does not exist creates it and prints `% Access VLAN
  does not exist. Creating vlan N` [verify the exact wording].
- Deleting a VLAN leaves ports assigned to it configured but carrying no traffic; they carry
  traffic again if the VLAN is re-created. Addresses learned in that VLAN are forgotten.
- VLAN names are at most 32 characters.

Not implemented, deliberately: the reserved VLANs 1002 to 1005, extended-range VLAN rules, VTP.
The text of messages is our own wording where marked; it follows IOS idiom closely and should be
reviewed with the rest of the message catalog (see the plan's legal notes).
