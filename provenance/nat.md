# NAT and PAT

Sources: RFC 3022 (traditional NAT: basic NAT and NAPT), RFC 4787 (UDP behavioural requirements,
for the mapping terms), RFC 5382 (TCP), RFC 5508 (ICMP), RFC 1624 (checksum updates, here replaced by
recomputing). Command syntax and `show` layouts follow the publicly documented IOS `ip nat` family;
message wording is ours where we could not check it. No Packet Tracer material.

## What is implemented (`netdev/src/nat.rs`, `engine/src/shell/nat.rs`)

- `ip nat inside` / `ip nat outside` on interfaces.
- Static: `ip nat inside source static <local> <global>` (every protocol, both directions) and the
  port form with `tcp` or `udp` (port forwarding).
- Dynamic: `ip nat pool` and `ip nat inside source list <acl> pool <name>` gives each inside host one
  pool address; with `overload` hosts share an address and are told apart by port (PAT).
  `... list <acl> interface <if> overload` uses the interface's own address.
- `show ip nat translations`, `show ip nat statistics`, `clear ip nat translation *`, and the
  configuration in `show running-config`.

## Order of operations (as on IOS)

- Outside to inside: translate the destination first, before the router decides whether the packet
  is for itself or where to send it. This is what lets PAT through the router's own address work.
- Inside to outside: route first, then translate the source, then the outbound access list sees the
  translated packet. A packet from inside that leaves by an interface that is not `outside` is not
  touched, and neither is a packet no rule covers.
- Traffic the router itself originates is not translated.

## Our choices

- A mapping is keyed by the inside host's address, port and protocol only (endpoint-independent
  mapping, RFC 4787), so one inside socket keeps one global port whatever it talks to. IOS keeps a
  mapping per conversation; the visible difference is rare.
- PAT keeps the source port if it is free and otherwise takes the lowest free port in the same range
  (1-511, 512-1023, 1024-65535); an echo's identifier is used as its port.
- Timeouts: ICMP 60 s, UDP 300 s, TCP 24 h, 60 s once a FIN or RST has crossed, one-to-one dynamic
  entries 24 h. Entries are dropped when next looked at, not by a timer, so NAT never keeps the
  simulation from going idle.
- With no pool address left the packet is dropped and the sender gets ICMP host unreachable.
- Only TCP, UDP and ICMP echo are translated, plus ICMP errors about them (on the way in, so
  traceroute through PAT works with the quoted packet showing the inside address; on the way out,
  see below). Addresses inside the data (FTP and the like) are not touched. Other protocols and later
  fragments pass untranslated.
- On an `outside` interface the router answers ARP for the global addresses it translates: a static
  mapping's, and any in use by a dynamic entry (not unused pool addresses). The neighbour therefore
  needs no route to them when they sit on the outside link's subnet; for a routed block it still
  needs one, as with an ISP.
- `ip nat translation timeout | tcp-timeout | udp-timeout | icmp-timeout | finrst-timeout <secs>` set
  the idle times; `no` restores the default. `0` and `never` are not accepted. Only non-default values
  are printed. `syn-timeout` and `max-entries` are not implemented.
- `ip nat outside source static <global> <local>`: a packet arriving from outside whose source is the
  global address is given the local one; a packet from inside to the local address is given the global
  one as destination after routing (so the local address must be routable towards the outside
  interface). Only the static form exists.
- An ICMP error sent by an inside host to the outside (port unreachable from a forwarded port, say)
  leaves from the translated address, and the packet it quotes shows the global address and port
  again. If the host has no translation the error is left alone.
- Layer 3 switches translate between VLAN interfaces (`ip nat inside` on `interface vlan 10`, and so
  on) when `ip routing` is on, with the same commands and `show` output. `ip nat` on a physical
  switch port is refused.
- Not implemented: application-level gateways (FTP, SIP and the like, which rewrite addresses in the
  data and the sequence numbers after them). Nothing in the simulator speaks those protocols, so there
  is nothing to test one against. Also not: `ip nat inside destination`, `match-in-vrf`,
  `extendable`, and translation of ICMP errors that quote another ICMP error.
