# Layout of `show vlan brief`, `show interfaces trunk`, `show mac address-table`

**Date:** 2026-10-25
**Source category:** public doc (the general shape of the vendor's command output, recalled, not
copied) and own test (golden transcript `testkit/scenarios/m2_vlans.out`). [verify] against a
real device before treating column widths as faithful.

What we print:

- `show vlan brief`: columns `VLAN Name Status Ports`; one row per VLAN; the ports column lists
  access ports in that VLAN, comma separated, **without line wrapping**. Reserved VLANs 1002 to
  1005 are not listed. Status is always `active`.
- `show interfaces trunk`: three sections (trunking ports with mode, encapsulation, status and
  native VLAN; VLANs allowed on the trunk; VLANs allowed and active). The fourth section (VLANs in
  spanning-tree forwarding state) is left out until spanning tree exists. Mode is always `on` and
  status `trunking`, because there is no negotiation or link state at the switch yet. Prints
  nothing when there are no trunks.
- `show mac address-table`: columns `Vlan Mac Address Port`. The real command also has a Type
  column and a header banner; ours does not.

Port names are `Fa0/N`, numbered from 1.
