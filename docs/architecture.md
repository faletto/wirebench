# Architecture

A map of the workspace as of the end of M2. The plan (milestones, legal notes, risks) lives in the
project's plan document; this file says what is in the code.

## Crates, bottom up

| Crate | What it holds |
|---|---|
| `wire` | Packet formats, no I/O: Ethernet/802.1Q, ARP, IPv4, ICMP, UDP, DHCP, STP/RSTP BPDUs, DTP; `summarize` (one line per frame); `dissect` and `hexdump` (the inspector's field tree with byte ranges). Parsers never panic; every format has a fuzz target. |
| `kernel` | Deterministic discrete-event kernel: virtual clock, event queue, links (serialisation delay, 10 ms transmit queue that tail-drops), timers (foreground and *background*), and the canonical trace hash. `Kernel` is `Clone`. |
| `netdev` | Devices as plain state machines driven by events: `Host` (ARP, ICMP, DHCP client, probes), `Router` (interfaces, subinterfaces, SVIs, static routes, DHCP server and relay, management plane), `L2Switch` (VLANs, trunks, DTP, one spanning tree per VLAN, an embedded `Router` for SVIs), and the pure `Stp` machine. Devices can be cloned (`Device::clone_box`) so a simulation can be snapshotted. |
| `clish` | Data-driven IOS-style CLI engine: modes, grammar, `?`, Tab, error markers. |
| `labfile` | The versioned TOML lab format. |
| `engine` | `World` (kernel plus devices); the shells that give each device its CLI; lab building (`lab`) and editing (`Topology`); settings-panel state and `Configure` (`panel`); traceroute and friends (`trace`); and the **protocol** (`protocol`): `EngineCmd` in, `EngineEvent` out, `Request`/`Response` envelopes, `Engine::serve`. |
| `app` | egui front end, native and web from one code base. Talks to the engine only through `EngineLink` (see below). |
| `cli` | Headless runner: scenarios, determinism checks, interactive shell. |

## The line between the UI and the engine

The app never holds a `World`. Everything crosses `app::link::EngineLink` as postcard bytes:

- **Commands** (`EngineCmd`): open a console and type at it; load, edit and snapshot a lab; ask
  for a device's state or send it CLI lines (`Configure`); queue a ping, `Step`, `Run`, `StepBack`,
  `Advance` time; ask the clock.
- **Answers** (`EngineEvent`) come back numbered, in order, possibly late. The engine pushes the
  lab as text (`LabText`) after anything that could change it, and only when it did.
- **The app never waits.** `App::post` sends a command with a note of what its answer is for
  (`Awaiting`); `App::pump` hands each answer to `on_response`. Tests run the whole app against an
  engine that answers four frames late (`crates/app/tests/async_link.rs`).
- **Where the engine runs** is the transport's business: `Local` (this thread, native and tests),
  `Delayed` (tests), `WorkerTransport` (web: the same wasm bundle started again inside a module
  Web Worker; `?engine=inline` or a failure to start one falls back to the page). See
  `engine-worker.md`.

## Time

Simulated time is virtual and only moves when something moves it. Most of the simulation is
event-driven: when nothing is due within 30 virtual seconds it is *idle* (`IDLE_HORIZON`).
Spanning tree and DTP keep time with a one-second *background* timer that never counts as work;
so that waiting at a console behaves as on a switch, the app advances simulated time in real time
while any background timer is pending (the live clock), pausing it while a ping is being stepped.
A loop with spanning tree off is a storm, which cannot hang the simulation: link queues drop, and
`run_to_idle` gives up after 500,000 events (`World::storming`).

## Where behaviour is written down

- `provenance/` has one file per behaviour that is not pure RFC (what, where it came from, what
  was chosen differently, what is not implemented).
- `testkit/scenarios/` holds golden console transcripts (`BLESS=1` rewrites them);
  `testkit/determinism.txt` pins trace hashes, checked on three targets in CI.
- `testkit/throttle/` drives the real web build in Chromium (not in CI).
- `testkit/mstpd-oracle/` runs spanning tree scripts through mstpd's protocol core and ours and
  compares where they settle (CI job `stp-oracle`).
