# FRR oracle for OSPF

[FRR](https://frrouting.org)'s `ospfd` (image `quay.io/frrouting/frr:10.2.1`) computes the routes
for the networks in `cases/*.case`; `crates/netdev/tests/ospf_net.rs` (`routes_match_the_ones_frr_recorded`)
checks that ours are the same: prefix, kind (intra, inter, e1, e2) and cost, for every router.

```sh
testkit/frr-oracle/run.py [case ...]     # needs Docker; rewrites cases/<name>.expected (about 2 minutes a case)
cargo test -p netdev --test ospf_net routes_match
```

Only the recorded `.expected` files are needed to run the test, so it needs no Docker.

`run.py` also records every OSPF packet each router sent or heard (`tcpdump` in a sidecar
container, built from alpine on first use) into `captures/*.pcap`. `crates/wire/tests/frr_capture.rs`
parses them all: the packet checksum and every LSA's Fletcher checksum must verify and each packet
must emit again as the same bytes (1,581 packets, 574 LSAs when recorded).

## Case format

```
nodes 5
seg bcast 0 : 0/1/1 1/1/1 2/1/1     # broadcast or p2p, area, then node/priority/cost for each member
default 3 e2 20                      # node 3 always originates a default route: e1 or e2, metric
```

Segment `s` in area `a` is 10.a.s.0/24 and its member `k` is at .k+1. Router IDs are the highest
interface address (set explicitly in FRR, as ours chooses it that way). Default timers on both sides.

## What it does not cover

Routes are compared, not the database or timing; packets are only checked to parse and round trip. Not covered: more than one path of
equal cost, virtual links, stub areas, NSSA, authentication.

## Licence

FRR is GPL-2.0-or-later. It is only run as a separate process in a container, nothing from it is
vendored or shipped; the `.expected` files are facts about our own test networks.
