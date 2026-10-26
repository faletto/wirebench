# wirebench

A network simulator for learning Cisco-style networking: routers, switches and PCs with an
IOS-style command line, a topology canvas, and a packet-by-packet view of what happens. Rust; the
same code runs natively and in the browser (egui). Everything is deterministic: the same lab and
the same commands give the same packets.

**Try it:** the web build is published at <https://faletto.com/wirebench/>. Natively:
`cargo run -p app --release` (add a lab file path to open it).

**What it simulates today:** Ethernet, ARP, IPv4 routing (static), ICMP, UDP, DHCP (server, relay,
client), VLANs and trunks with DTP, per-VLAN STP and RSTP, router-on-a-stick and layer 3 switches,
traceroute, passwords, banners and saved configurations. No dynamic routing (RIP, OSPF, EIGRP) or NAT yet.

**Where to read next**

- `docs/architecture.md`: the crates and the line between the UI and the engine.
- `docs/m2-status.md`: what the last milestone delivered and what is open.
- `provenance/`: one file per behaviour that is not pure RFC, with where it came from.
- `CONTRIBUTING.md`: the clean-room source policy, sign-off, checks before a pull request.
- `fuzz/README.md`, `testkit/throttle/README.md`: fuzzing, and driving the web build in a browser.

Licence: code Apache-2.0 OR MIT; docs, labs and messages CC BY 4.0.
