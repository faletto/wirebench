# Default switch port mode

**Update 2026-10-26:** DTP is now modelled (see `dtp.md`) and the default is `dynamic auto`. The
text below is the earlier simplification, kept for the record.

**Date:** 2026-10-25
**Source category:** own decision (a deliberate simplification).

On the devices we imitate, an unconfigured switch port is `dynamic auto` and becomes a trunk if
the neighbour asks. We do not model DTP yet, so every port defaults to a **static access port in
VLAN 1** with all VLANs allowed and native VLAN 1. When DTP is added the default should change to
`dynamic auto`, and saved labs that rely on the old default will need a migration (the lab format
version bumps).

`show running-config` on a switch lists only ports that differ from this default, and only the
lines that differ. A real device prints every interface; we print fewer lines to keep the console
readable on 8 to 24 port switches.
