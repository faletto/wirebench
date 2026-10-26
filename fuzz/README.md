# Fuzzing

Targets live in `fuzz_targets/`. This directory is not part of the main workspace because
fuzzing needs a nightly compiler and sanitizer flags.

| Target | What it feeds | Property checked |
|---|---|---|
| `wire_ethernet` | arbitrary bytes to the Ethernet/802.1Q parser | never panics; parse -> emit -> parse is identical |
| `wire_arp`, `wire_ipv4`, `wire_icmp`, `wire_udp`, `wire_dhcp`, `wire_bpdu`, `wire_dtp` | arbitrary bytes to those parsers | same |
| `wire_dissect` | arbitrary bytes to the packet inspector's dissector, its protocol names and hex dump | never panics; every field's byte range lies inside the frame and inside its parent's |
| `wire_summary` | arbitrary bytes to the one-line frame summary | never panics |
| `lab_edit` | arbitrary lab text, then edits to it (add, cable, remove) | never panics; snapshots stay loadable |
| `shell_exec` | arbitrary text typed into a router or PC shell, plus `?` and Tab on every line | never panics |
| `lab_load` | arbitrary text as a lab file | never panics; a loaded lab saves and reloads equal; building it never panics |
| `device_frames` | arbitrary frames delivered to a host, a router and a switch | never panics or loops (run bounded in virtual time) |

```sh
cargo install cargo-fuzz --locked
rustup toolchain install nightly --profile minimal
cargo +nightly fuzz run wire_ipv4 -- -max_total_time=60
```

CI runs each target for 20 seconds on every push. A crash input is saved under
`fuzz/artifacts/<target>/`; copy it into a regular test before fixing the bug so it stays fixed.
The `corpus/` and `artifacts/` directories are ignored by git.
