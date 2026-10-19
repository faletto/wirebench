# Contributing

## Source policy (clean-room)

Allowed sources for behavior: public standards (IEEE/RFC), public vendor documentation,
behavior observed through legitimate use, and our own testing.

Not allowed: decompiled/disassembled vendor software, leaked or proprietary material,
vendor assets (icons, help text, output strings copied wholesale), and any code or
algorithms for decrypting or decoding vendor file containers. The importer accepts
already-decoded XML only.

Any behavior that is not pure RFC needs an entry in `provenance/`.

## Sign-off

Every commit needs a DCO sign-off (`git commit -s`).

## Licence

Code is Apache-2.0 OR MIT. Docs, labs and the message catalog are CC BY 4.0.
