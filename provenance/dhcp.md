# DHCPv4: server, relay and client

**Date:** 2026-10-25
**Source category:** standard (RFC 2131, RFC 2132, RFC 1542, RFC 768) for the protocol; public doc
(general knowledge of vendor documentation, recalled rather than re-read) for the command line;
own test (`crates/netdev/src/dhcp_server.rs` unit tests, `crates/engine/tests/dhcp.rs`,
`dhcp_shell.rs`, golden `testkit/scenarios/m2_dhcp.*`). [verify] marks choices not checked
against a real device. The plan's oracle (ISC Kea on a captured exchange) has **not** been run;
it would be a separate process only, never copied code.

## Protocol, as the RFCs say

- Server replies are addressed per RFC 2131 section 4.1: to the relay (`giaddr`) when there is
  one; a NAK is broadcast; a renewing client (`ciaddr` set) is answered at that address; a client
  that set the BROADCAST flag gets a broadcast; anyone else gets the offered address by unicast
  to its hardware address (it cannot ARP before it has an address).
- An offer is held for 120 seconds and then released. A client that chooses another server
  withdraws our offer. SELECTING, INIT-REBOOT and RENEWING/REBINDING requests are told apart by
  `server-id`, `ciaddr` and the requested-address option. A request for an address we have no
  record of is **ignored**, not NAKed, unless it is in the wrong network (INIT-REBOOT) or taken by
  someone else.
- The client renews at T1 (half the lease unless the server says otherwise) by unicast, rebinds at
  T2 (7/8) by broadcast, and gives up the address when the lease ends. Retries wait half the time
  left, at least a minute. DISCOVER and REQUEST are retried at 2, 4, 8, 16 seconds.
- A relay (`ip helper-address`) stamps `giaddr`, counts hops (dropping at 16), forwards by unicast
  to each helper from the receiving interface's address, and delivers the server's reply to the
  client on the interface that owns `giaddr`. The server picks the pool from `giaddr`.

## Choices that are ours

- **Finite retries.** A client with no server sends five DISCOVERs (the first plus four retries
  at 2, 6, 14 and 30 seconds) and then waits until told to try again (`ipconfig renew`). Real
  clients keep trying; a simulation that never goes idle would make Run and Play useless.
- **Idle means "nothing due within 30 virtual seconds".** Timers further away (a lease renewal
  hours ahead) stay queued and fire when time is advanced past them.
- **No address conflict checks.** A real server pings an address before offering it [verify] and
  a real client ARP-probes before using it; neither is done, so a statically configured duplicate
  is not noticed. `decline` and the conflict list exist and work if a client sends one.
- The server never hands out its own interface addresses, the network or the broadcast address,
  or anything excluded. Allocation is lowest free address first [verify the real order].
- Leases are not stored in saved labs; only `ipv4 = "dhcp"` is. A loaded lab's DHCP clients start
  when the simulation first runs.
- Lease times in `show ip dhcp binding` are time remaining (`HH:MM:SS`), not a calendar date;
  the layout of `show ip dhcp pool` is our own.
- Option 82 (relay agent information), option 61 (we send none; the hardware address identifies a
  client), `domain-name`, `option` and `host` (manual) pools, `ip dhcp relay` variants, DHCPv6.

## Commands

`service dhcp`, `ip dhcp excluded-address <low> [<high>]`, `ip dhcp pool <name>` with `network`,
`default-router`, `dns-server` (up to three), `lease <days> [<hours> [<minutes>]]`,
`ip helper-address <server>` on an interface or SVI, `show ip dhcp binding | pool | conflict`,
`clear ip dhcp binding <addr>|*`. On PCs: `ipconfig dhcp | release | renew`; typing an address
turns DHCP off. Everything prints in the running config and replays.
