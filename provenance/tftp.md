# TFTP: configuration backup and restore

**Date:** 2026-11-11
**Source category:** RFC (RFC 1350 for the protocol; RFC 1123 section 4.2.3.1 for not resending on a
duplicate acknowledgement); public vendor documentation recalled from memory (the `copy` forms and
the layout of their output); own test (`crates/wire/src/tftp.rs`, `crates/netdev/src/tftp.rs`,
`crates/engine/tests/tftp.rs`, golden `testkit/scenarios/m3b_tftp.*`, fuzz target `wire_tftp`).
No Packet Tracer material was consulted. [verify] marks choices not checked against a real device.

## Behaviour

- **Protocol**: read and write requests to UDP 69, data, ack and error packets (codes 0 to 6 are
  named in `wire`), blocks of 512 bytes, a short or empty final block ends a transfer, each
  transfer served from its own port (its TID). Octet and netascii are accepted and treated alike
  (no line-ending conversion). No options (RFC 2347): block size, transfer size and timeout
  negotiation are left out, so a request carrying options simply gets plain TFTP back. Block numbers
  do not wrap, so a file is at most 65535 blocks (about 32 MB).
- **Client** (routers, and switches through their VLAN interfaces): `copy running-config
  tftp://<ip>/<file>`, `copy startup-config tftp://...`, `copy tftp://... running-config` (merged into
  the running configuration by feeding the lines through the CLI, as a paste would be; lines the CLI
  rejects are listed) and `copy tftp://... startup-config` (replaces the saved configuration). The
  address must be an IPv4 address; there is no resolver. The command runs the transfer to its end,
  letting simulated time pass, so it returns when the transfer is over. After two seconds of silence
  the last packet is sent again, up to five times, then `%Error opening ... (Timed out)`. An error
  from the server is shown in the same form: `(File not found)`.
- **Output** follows the documented shape: `Writing <file> !!`, `Accessing ...`, `Loading <file>
  from <ip>: !`, `[OK - N bytes]`, `N bytes copied in S secs (R bytes/sec)`. **The interactive
  prompts** (`Address or name of remote host []?`, `Destination filename []?`) **are not asked**: the
  URL carries both answers and the command never waits for input [verify]. One `!` per block, at
  most 60. The seconds are simulation time, so a transfer on a lab link takes almost none.
- **Server** (on a PC): `tftpd start | stop | list | log | show <file> | delete <file>`. It holds files
  in memory, accepts both reads and writes and overwrites without asking. A request for a missing
  file gets error 1. A packet on a transfer's port from anyone but its peer gets error 5 and the
  transfer goes on. Silence resends the last packet after 3 seconds, five times, then the transfer is
  dropped; a repeated request for a running transfer is ignored. This is our own invention, not a
  Windows command: the lab needs a server to back up to. The files live as long as the PC does and are
  not part of the lab file.

## Left out

`tftp-server` on a router (serving its own files), flash and other file systems (`copy flash:`),
`copy tftp: flash:`, image upgrade, `ip tftp source-interface`, `boot` and `archive` commands, IPv6, the
interactive prompts, preloading files into a PC from the lab file, and the packet inspector's own
summary of TFTP and syslog packets (they show as plain UDP).
