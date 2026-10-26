# Management plane: passwords, banner, lines, saving, reload

**Date:** 2026-10-25
**Source category:** public doc (general knowledge of vendor documentation, recalled rather than
re-read) and own test (`crates/engine/tests/mgmt.rs`, golden `testkit/scenarios/m2_mgmt.*`).
[verify] marks choices not checked against a real device. [lawyer] marks the secret formats,
which the plan says must never imitate a vendor's.

## Secrets and passwords

- `enable secret <word>` stores a salted digest; `enable password <word>` stores the word.
  Both ask for the password at `enable`; **the secret wins** when both are set. Three wrong
  tries print `% Bad secrets` and leave the session unprivileged.
- **Own formats [lawyer].** A secret prints as `enable secret wb1$<salt>$<digest>`; with
  `service password-encryption` a plain password prints as `wb0$<hex>`. Neither resembles the
  vendor's numbered types or their output. The digest is a few rounds of a non-cryptographic
  hash (FNV) salted from the device name, and `wb0$` is a reversible xor. **They are teaching aids
  and give no security.**
- The encoded forms are recognised **by shape**, not by a keyword, so no ordinary password is
  ever mistaken for one (a keyword made `w` and `wb` impossible to use as a password; found by
  the property test). A password that looks exactly like an encoded form is read as that form.
- `service password-encryption` is a display switch: turning it off prints plain passwords
  again. Real devices leave already-encrypted passwords encrypted until retyped [verify].
- The salt is derived from the device name rather than random, so the same input always gives
  the same output (simulations must be reproducible).

## Lines, login and banner

- `line console 0` and `line vty <first> <last>` enter line configuration; `password` and
  `login` / `no login` apply. Only the **console** is enforced: a new console session shows the
  banner and `User Access Verification`, then asks for the password. Three wrong tries print
  `% Bad passwords` and ask again; **the console never locks out** [deliberate deviation: the
  real device refuses a `login` line with no password set; here the login is simply not asked,
  so a lab can never lock itself out]. The vty lines are stored and printed but nothing connects
  to them yet (no telnet or SSH).
- `banner motd <delimiter>text<delimiter>`, or a delimiter alone followed by text lines ending at
  the delimiter. Shown when a console session opens. Printed back in the multi-line form with a
  delimiter that does not appear in the text. The delimiter is not restricted [verify].
- Typed passwords are not echoed, not kept in the command history and not drawn; `?` and Tab are
  ordinary characters at a password prompt.

## Saving and reloading

- `copy running-config startup-config`, `write` and `write memory` save the running
  configuration as the device's **startup config**, which is now device state and travels with
  the lab file (`startup_config`). `show startup-config` prints it, or
  `startup-config is not present`. `erase startup-config` asks to confirm.
- `reload` asks `System configuration has been modified. Save? [yes/no]:` when the running config
  differs from the saved one, then `Proceed with reload? [confirm]`. It resets the device to its
  factory state (default hostname, no addresses, ports shut down, no passwords) and replays the
  saved configuration. ARP caches and learned MAC addresses are lost, virtual time continues.
  The console session returns to user mode and shows the banner and login again.
- A device that was never saved reloads to a blank one. The wording of the questions is our own.

Not implemented: `username`/`login local`, AAA, `exec-timeout`, `logging synchronous`, enable
levels, `secret 0/5/..` forms, telnet/SSH, `show version`, `boot` settings.
