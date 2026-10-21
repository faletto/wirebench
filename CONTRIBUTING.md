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

Every commit needs a DCO sign-off (`git commit -s`). A CI check fails pull requests that
contain commits without a `Signed-off-by:` line.

## Before you open a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p cli -- det | diff - testkit/determinism.txt   # trace hashes must not change
```

If a change legitimately alters simulation behavior, the hashes in `testkit/determinism.txt`
change too: say why in the pull request. Golden transcripts are rewritten with `BLESS=1` and
the diff reviewed by eye. Fuzzing is described in `fuzz/README.md`.

## Licence

Code is Apache-2.0 OR MIT. Docs, labs and the message catalog are CC BY 4.0.
