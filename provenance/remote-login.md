# Telnet, SSH and vty login

Sources: RFC 854 and 855 (Telnet and its options), RFC 4253, 4252 and 4254 (SSH transport,
authentication, connection), RFC 4251 for the string encoding. Router and switch behaviour follows
the publicly documented IOS command set (`line vty`, `login local`, `transport input`,
`access-class`, `username`, `crypto key generate rsa`); wording of messages is ours where we could
not check it against a device. No Packet Tracer material.

## What is on the wire

Telnet: a real TCP connection to port 23. The server offers ECHO and SUPPRESS-GO-AHEAD, the client
accepts, other options are refused (`wire/src/telnet.rs`). Lines end in CR LF.

SSH: a real TCP connection to port 22 with the real identification string exchange
(`SSH-2.0-wirebench_1.0`) and the real message numbers for user authentication and channel data.
**It is not a secure channel.** Key exchange, encryption and MACs are not simulated: packets are
framed as length, message number, payload, without padding or a MAC, and the password travels in
the clear. A capture shows the same sequence of events as a real session, minus the encrypted bytes.
Only password authentication, one attempt per `ssh` command, protocol 2 only.

## Behaviour we chose

- A session needs the far device to be running something that accepts it: port 23 listens when
  `transport input` allows telnet; port 22 listens when it allows ssh **and** the device has an
  `ip domain-name` and an RSA key (as IOS requires). Otherwise the SYN is reset and the client says
  `% Connection refused by remote host`.
- `access-class N in` on the vty lines is checked when the connection opens, against the caller's
  address; a caller it denies is reset like a closed port.
- Telnet login: `login` with a line password asks `Password:`; `login local` asks `Username:` then
  `Password:` against `username` entries; `login` with no password prints `Password required, but
  none set` and closes; no `login` lets the caller straight in. Three wrong passwords print
  `% Bad passwords` and close. `username ... privilege 15` lands in privileged mode.
- The RSA key is not part of the configuration. It survives `reload` and `erase startup-config`, as
  keys do on a real device. `crypto key zeroize rsa` removes it without asking for confirmation.
- The far terminal's prompt is read directly from that device's shell, not sent as text over the
  connection. The far device only processes input while a client is typing at it.
- `username ... secret` is stored hashed (like `enable secret`); `password` is stored as typed (shown
  as `wb0$` under `service password-encryption`).
- Not done: `show users`, `show ssh`, `exec-timeout`, `ip ssh version 1`, public-key
  authentication, `ssh` with options other than `-l`, host names (addresses only), Ctrl-Shift-6.
