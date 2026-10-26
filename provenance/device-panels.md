# Device settings panels

**Date:** 2026-10-26
**Source category:** own design. Tests: `crates/engine/tests/panel.rs`,
`crates/app/tests/settings.rs`, unit tests in `crates/app/src/settings.rs`.

A left-hand panel shows the settings of the device whose tab is open.

- **PC:** address (`a.b.c.d/len`), gateway, and "get the address from DHCP".
- **Router:** each interface's address and shutdown (with up/down shown), and the static routes
  (add, remove). Subinterfaces are listed; SVIs are not.
- **Switch:** spanning tree mode (off, pvst, rapid-pvst); VLANs (add with a name, delete);
  and for each port its mode (access, trunk, dynamic auto, dynamic desirable), access VLAN,
  shutdown and portfast, with "(trunk)" shown on a dynamic port that negotiated one.

## How it works

The panel never changes a device itself. It reads a structured `DeviceState` from the engine
(`EngineCmd::DeviceState`) and turns what was typed into **CLI lines**, which the engine runs
through the device's own shell (`EngineCmd::Configure`) in configuration mode, without asking for
a password. So a panel can do only what the console can, a change made in a panel is in
`show running-config` and the saved lab, and anything the CLI refuses comes back against the line
that was refused (shown under the panel) while the lines after it still run. The panel reads the
device again after each change and after any console command.

## Not done

- Not tried in a browser; layout is a fixed-width left panel.
- No panel for DHCP pools, helper addresses, SVIs, passwords or banners, per-VLAN spanning tree
  priority, port cost, or trunk allowed/native VLANs: the console still covers these.
- Edits are applied one control at a time; there is no "revert" and no undo entry for them (the
  lab-level undo records structure and positions, and keeps configuration typed since).
