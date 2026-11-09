# TCP and UDP sockets

Sources: RFC 9293 (TCP), RFC 768 (UDP), RFC 1122 section 4.2 (host requirements), RFC 6298 for the
doubling retransmission timer. No vendor documentation.

What is implemented (`netdev/src/tcp.rs`): the three-way handshake, a byte stream each way with
cumulative acknowledgements, a fixed 8192-byte receive window, go-back-N retransmission with a timer
that starts at 1 s and doubles, orderly close (FIN in either order, simultaneous close), a shortened
TIME-WAIT (4 s), resets (RFC 9293 3.10.7.1 for segments with no connection), and checksum checks.
UDP is a table of bound ports that queue datagrams; a datagram for an unbound port still gets ICMP
port unreachable.

Our choices, not from the RFC:
- No options, so the segment size is the default 536 bytes (RFC 9293 3.7.1). No congestion control, no
  window scaling, no out-of-order buffering (a segment ahead of the expected one is dropped and
  re-acknowledged).
- Giving up after 4 retransmissions (31 s in all). A Cisco `telnet` waits about as long.
- Initial sequence numbers come from the simulation clock (RFC 9293 3.4.1's 4 microsecond clock) plus
  a per-device salt, so runs repeat exactly.
- Listening ports are set by the owner before each segment is handled; a router sets them from its
  vty line configuration.
