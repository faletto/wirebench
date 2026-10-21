# Performance baseline (M0)

Numbers measured during M0. They are a floor to re-baseline against, not targets.
Machine: Apple Silicon Mac (aarch64-apple-darwin), 2026-10-19.

## Measured

| What | Value | How |
|---|---|---|
| Kernel event throughput, native | **~1.57 M delivered events/s** (~261 k cross-router pings/s) | `cargo run --release -p cli -- bench`: pc1 pings pc2 across two routers 20 000 times, hash-only trace, so trace hashing is included |
| Web bundle, wasm | **3.9 MB raw, 1.58 MB gzip -9** | `cd crates/app && trunk build --release` (opt-level default, `wasm-opt -Oz` via trunk) |
| Web bundle, JS glue | 74 KB raw, 11.6 KB gzip | same |

The bench uses tiny frames and a three-device path, so treat events/s as an upper-ish bound
for simple topologies. It will drop as protocol modules (OSPF, STP) add work per event.

## Not measured yet (needs a browser or other hardware)

- **Web cold start** (laptop Chrome, Chromebook). The bundle builds, but nothing in this
  session could run a browser, so it has not been loaded even once. Measure time from
  navigation to first interactive frame, and record whether it is a cold or warm cache.
- **Kernel throughput on wasm32** (run the `bench` command under wasmtime, then in a browser).
- **Native cold start** (process launch to first frame).
- **UI frame time** at 100 nodes. The M0 UI has no canvas yet.
- **egui text input feel on web**: IME, paste, key repeat. This is the M0 risk item; the
  widget logic is unit and headless tested, but real-browser behavior is unverified.

## Reproduce

```sh
cargo run --release -p cli -- bench
(cd crates/app && trunk build --release && ls -l dist && gzip -9 -c dist/*_bg.wasm | wc -c)
```
