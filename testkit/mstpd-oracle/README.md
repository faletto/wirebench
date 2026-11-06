# mstpd oracle for spanning tree

[mstpd](https://github.com/mstpd/mstpd) is a Linux implementation of RSTP and MSTP (802.1D-2004,
802.1Q). Here its protocol core, `mstp.c`, is driven by `harness.c` without a kernel, sockets
or clock, and its answers are compared with ours by `crates/netdev/tests/stp_oracle.rs`.

```sh
export MSTPD_ORACLE=$(testkit/mstpd-oracle/build.sh)    # Linux or macOS, needs git and a C compiler
cargo test -p netdev --test stp_oracle                  # 256 random networks and the recorded cases
MSTPD_CASES=20000 cargo test --release -p netdev --test stp_oracle
MSTPD_BLESS=1 cargo test -p netdev --test stp_oracle    # rewrite cases/*.expected from mstpd
```

Without `MSTPD_ORACLE` the random test does nothing and the recorded cases in `cases/` are checked
against the answers mstpd gave when they were written. The CI job `stp-oracle` builds the oracle
and runs 2,000 random networks.

## What is compared

Both sides wire bridges together the same way: point to point links, BPDUs delivered at once and
in order, one second at a time, bridge by bridge. A script (format at the top of `harness.c`)
builds bridges, sets costs and priorities, plugs and pulls cables and changes a bridge's
priority. A `dump` prints each bridge's root, root path cost and root port, and each port's role
and state (`discarding` covers blocking, listening and disabled). The random scripts take a
`dump` 90 seconds after the last change, so they compare where the network settles, not how fast.

Settings that keep the two comparable: classic mode is mstpd with `ForceProtocolVersion` 0 (STP);
every port is point to point and not an edge port (mstpd's automatic edge detection is off, as we
only have `portfast`); every cost is 19 unless the script sets one; priorities are in mstpd's
steps (4096 for bridges, 16 for ports). Ours runs in the same steps without the VLAN number in
the bridge priority, which does not change any comparison.

## Licence

mstpd is GPL-2.0-or-later, so `harness.c`, which is linked with it, is too. Neither is part of
wirebench: `build.sh` fetches mstpd at a pinned commit into `target/` at test time, the binary is
only run by tests as a separate process, and nothing from it is shipped or vendored. The rest of
this directory (the scripts, `cases/`, `compat/`) is under the repository's licences.
