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
- Only TCP, UDP and ICMP echo are translated. ICMP errors are translated on their way in (so
  traceroute through PAT works, with the quoted packet showing the inside address) but not on the way
  out. Addresses inside the data (FTP and the like) are not touched. Other protocols and later
  fragments pass untranslated.
- The router does not answer ARP for global addresses it translates; the neighbouring router needs a
  route to them (the demo uses a static route for the public block), as with a routed block from an
  ISP.
- `ip nat outside source`, `ip nat inside destination`, `match-in-vrf`, `ip nat translation timeout`
  and `extendable` are not implemented. Switches do not translate.
