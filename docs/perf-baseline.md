# Performance baseline (M0)

What was measured during M0, against the targets in the engineering plan (section 7), and what
the targets should become. The plan called its own numbers estimates to be re-baselined here.

Machines: kernel and lab benchmarks ran on an Apple Silicon Mac (release build, otherwise idle
machine). The browser trace is from a **MacBook Pro M3 in low-power mode, running Vivaldi**
(Chromium), loading the deployed site with a cold cache over HTTP/2 on a fast link. No
Chromebook measurement exists yet.

## Results against the original targets

| Metric | Plan target (native / wasm) | M0 measured | Verdict |
|---|---|---|---|
| Web cold start, empty lab | wasm: < 3 s first visit on a Chromebook | **First paint at 399 ms** (M3 laptop, see timeline below) | Passes on a laptop by 7x; **Chromebook unmeasured** |
| Native cold start | < 300 ms | not measured (the app now shows a "ready in N ms" readout to make this easy) | Open |
| Cold start, 100-node lab (engine only) | native < 800 ms | **parse 1.7 ms + build 9.9 ms**, whole process 10 ms, replaying about 3,100 config lines (2,450 static routes) through the CLI | Passes by ~80x. The UI side (canvas with 100 nodes) is unmeasured |
| Wasm download size | < 4 MB compressed | **1.71 MB gzip** (4.40 MB raw) + 12 KB JS | Passes |
| UI frame time, 100 nodes | native p99 < 8 ms, wasm p99 < 16 ms | not measured (no 100-node UI test, no `--bench-ui` mode yet) | Open |
| Idle CPU | ~0%, no repaint when nothing changes | **~1.2% of a core** from a 120 Hz animation-frame poll, see below | **Not met** as written |
| Keystroke to echo | < 1 frame | not instrumented; typing "feels good" (tester report) | Open |
| Kernel throughput | native >= 2.0 M events/s, wasm >= 0.7 M | **native 1.54 / 1.67 / 1.65 M**; **wasm (wasmtime) 1.58 M** | **Native is ~20% below target**; wasm passes |
| OSPF / RSTP convergence | various | n/a, not implemented | n/a |
| Memory, 100-node lab | native < 150 MB, wasm tab < 250 MB | engine alone: **3.9 MB** peak RSS. UI and wasm tab unmeasured | Engine passes |

## Browser trace (M3 MacBook Pro, low-power mode, Vivaldi, cold cache)

Times are from navigation start.

| Milestone | Time |
|---|---|
| HTML request sent (browser-side setup before it) | 73 ms |
| JS received | 105 ms |
| Wasm download complete | 148 ms (1.71 MB sent, 4.40 MB decoded) |
| Streaming wasm compile finished | 143 ms; main-thread cost only 2.9 ms (V8 compiles on background threads) |
| Main thread busy after the download | 161 ms (instantiate + eframe and app setup), then a 64 ms task at 309 ms |
| **First paint (FCP)** | **399 ms** |

- Compile is not a bottleneck. The 161 ms setup block cannot be split further, because the
  release wasm has no function names. Native lab load + build is about 11 ms, so app logic is
  not where the time goes; instantiation and eframe/WebGL/font setup are the likely cost.
- The "Loading..." text does not paint before 399 ms, because the setup task blocks the main
  thread right after the download. Yielding to the browser before starting would fix that.
- Steady state: the page ran **120 animation frames per second even in seconds with zero
  input**. That is eframe's web loop: it requests a frame at the display refresh rate forever
  and only paints when needed (eframe source, `web/events.rs`). The callbacks are cheap (p50
  0.05 ms, max 1.7 ms), main-thread busy time was 11-13 ms per second (about 1.2%), and GPU
  tasks 2.5-4.4 ms per second. The compositor still committed a frame on nearly every tick.
- Frame cost under interaction is **inconclusive**: no animation-frame callback cost more
  than 0.3 ms, but the trace cannot say which frames repainted.
- Caveats: one trace; DevTools recording adds overhead; low-power mode should slow the CPU,
  while the fast link flatters the download.

## Fuzzing (not a performance number, but a baseline)

Seven targets, 60 s each on the dev machine (the Ethernet target 20 s): about 108 million
executions, **0 crashes**, and the parse -> emit -> parse round-trip held on every wire target.
Throughput ranged from ~580 k runs/s (ARP) to ~5 k/s (a full CLI session per run).

## Proposed re-baselined targets

These are proposals; the first column of numbers is what M0 achieved.

| Metric | Proposed target | Why |
|---|---|---|
| Kernel throughput, native | floor 1.5 M events/s now; regression alert at -10%; aim for 2 M | Measured 1.6 M. Not yet profiled. Per-event costs that likely matter: hashing every frame byte into the trace, cloning frame buffers, ordered-map lookups. [unverified] |
| Kernel throughput, wasm | >= 0.7 M in a browser (unchanged); wasmtime proxy 1.58 M | Browser V8 numbers still needed |
| Web first paint | < 1.5 s on a Chromebook; < 600 ms on a laptop | Laptop measured 399 ms; scale for a slower CPU and the network |
| Wasm size | budget 2.5 MB gzip, checked in CI (now 1.71) | Leaves room for protocols without a surprise |
| 100-node lab build (engine) | < 100 ms | Measured 11 ms; 10x headroom |
| Idle | < 2% of a core on a 120 Hz display, no GPU work when nothing changes | "~0%" is not reachable with eframe's polling loop without upstream changes |
| Frame time at 100 nodes | keep p99 < 16 ms wasm | Needs a UI bench mode to measure |

## Reproduce

```sh
cargo run --release -p cli -- bench                      # kernel throughput
python3 testkit/bench/gen_chain.py 50 > /tmp/chain-100.lab.toml
/usr/bin/time -l ./target/release/wirebench bench-lab /tmp/chain-100.lab.toml
cargo build --release -p cli --target wasm32-wasip1      # then run under wasmtime:
wasmtime target/wasm32-wasip1/release/wirebench.wasm bench
(cd crates/app && trunk build --release && ls -l dist)   # bundle size
```

For a cold-start number in the browser: open DevTools, Network tab, tick "Disable cache",
reload, and read the "ready in N ms" figure in the status bar (it counts from navigation
start) or record a trace in the Performance panel.
