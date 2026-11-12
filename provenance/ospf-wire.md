# OSPFv2 packet formats

**Date:** 2026-11-06
**Source category:** standard (RFC 2328 appendix A.3 packets and A.4 LSAs, section 12.1.7 and the
ISO 8473 annex C Fletcher checksum it points to; RFC 1071 for the packet checksum); own test
(`crates/wire/src/ospf.rs` unit and property tests, fuzz target `wire_ospf`). Nothing from a vendor.
This is the wire layer only: no neighbours, flooding or SPF exist yet.

## What is read and written

- Packet header, Hello, Database Description, Link State Request, Update and Acknowledgment.
- LSA header; router LSAs (links of any type, metrics, the B and E flags), network LSAs, summary
  LSAs (types 3 and 4: mask and a 24-bit metric) and AS-external LSAs (type 5: mask, E bit,
  metric, forwarding address, tag) in structure; every other LSA type is carried as its bytes.
- Authentication: null and simple password are accepted (the 8 data bytes are kept and not
  summed). Cryptographic authentication (type 2) is `Unsupported`.
- Packet checksum: RFC 1071 over the packet without the authentication data; wrong sums are
  `BadChecksum`. LSA checksum: Fletcher mod 255 over the LSA without its age, so the age can change
  in transit; wrong sums are `BadChecksum`. One bad LSA rejects the whole Update (a real router
  discards only that LSA) [verify whether it matters].
- TOS metrics (in router links, summary and external LSAs) are skipped when reading and never
  written (RFC 2328 appendix F).

## How it was checked

- Round trips of random packets and LSAs of every kind; a flipped bit anywhere but the age always
  breaks the LSA checksum; the age may change freely; arbitrary bytes never panic the parsers.
- Hand-checked byte layouts of a Hello and a router LSA against the appendix.
- **The LSA checksum is checked only against its defining property** (the sum over the whole LSA
  is zero), not against a published vector or another implementation. **Checked against FRR on
  2026-11-11:** `crates/wire/tests/frr_capture.rs` parses 1,581 packets recorded from FRR 10.2.1's
  ospfd (`testkit/frr-oracle/captures/`); all packet and LSA checksums verify and every packet emits
  again byte for byte.
