# Moving the engine into a Web Worker

**Status 2026-10-26:** the engine runs in a Web Worker on the web build, and the app talks to it
only through serialisable messages that may be answered late. Pacing (step 4) is done as far as it
can be without a worker-owned clock. What is left is the 8x load proof and a CI check of the worker
(see "To do"). Native builds still run the engine in the app's thread through the same link.

## Done in step 1: the app talks to the engine only by message

- The engine owns the world **and** the topology. A UI loads, edits and snapshots a lab with
  `Load`, `AddDevice`, `MoveDevice`, `AddLink`, `RemoveDevice`, `RemoveLink`, `Snapshot` and
  `Now`, and gets `LabView` (names, kinds, places, cabling) back. `App` holds a `LabView`, not
  a `Topology`, and never sees a `World`.
- Hops carry their frame; step back is a message (`StepBack`) and the engine keeps the copies.
- Time and "upkeep pending" travel on `Time` events, so the app can run the live clock for
  spanning tree without reading the kernel.

## Done in step 2: the link is asynchronous

- Every command goes out as a numbered `Request` and its `Response` comes back with the same
  number, in order (`engine::protocol::{Request, Response}`, `Engine::serve`). The app's
  `EngineLink` has `post` and `poll`; a `Transport` carries the bytes. `Local` runs the engine in
  this thread and answers at once; `Delayed` holds each answer back for a number of polls, which is
  what a worker will look like and what the tests use (`App::from_lab_text_delayed`).
- The app never waits. `App::post` sends a command with an `Awaiting` note saying what the answer
  is for; `App::pump` (start of every frame, and after anything that posts) hands each answer to
  `on_response`. Console open/lines/help/Tab, structural edits, undo/redo, opening labs, the
  settings panel, stepping/Run/Play/Back and the live clock all work this way, tested with a
  four-frame-late engine (`crates/app/tests/async_link.rs`).
- The engine pushes `LabText` after anything that can change what a snapshot would say, and only
  when it did; the app keeps that copy, so saving, dirty tracking, autosave and undo read it
  without asking.
- The only blocking calls left are the first snapshot when an app is built around an engine in
  this thread (`App::from_built`), and `EngineLink::send`, used only by tests of the link.

## Done in step 3: the engine runs in a Web Worker

- The page's own wasm bundle is started a second time inside a module worker
  (`crates/app/src/worker.rs`). There `main` returns at once (no page) and the worker's script
  calls the exported `worker_start`, which makes an `Engine` and hands back the function that
  answers each message with `Engine::serve`. The worker's script is built at run time as a blob
  that `import()`s the bundle trunk linked (found through the page's `modulepreload` and `preload`
  links), so trunk needed no change. Messages that arrive while the wasm is loading are queued.
- `WorkerTransport` is the page's end; it wakes the page (`request_repaint`) when an answer
  comes. `App::with_transport` builds the screen from a local copy of the bundled lab and tells
  the worker to load the same lab before anything else, so device numbers agree.
- It is the default on the web. `?engine=inline` keeps the engine in the page, and so does any
  failure to start a worker (the console says why).
- Checked in Chromium (`testkit/throttle`): `demo.mjs` (build from scratch, configure, ping,
  save, reload, open, step, animate) and `m2.mjs` (spanning tree live clock, settings panel,
  Step/Back, field tree) give the same results with the worker as without. There is no test of
  the worker in CI; the harness is run by hand.

## Done in step 4: pacing

- **Fast-forward is batched.** `Run` used to be up to 100,000 round trips (one `Step` each); it is
  now `EngineCmd::Run { max }`, which steps up to `max` times in the worker and answers with what
  each step delivered (5,000 at a time, so the page can draw between batches). Back still undoes
  one step at a time; a batch equals that many single steps (tested, trace hashes equal).
- **The live clock has flow control.** One message a round (`Advance` already forgets saved states
  and answers with the clock), one round at a time, not more often than every 0.1 s; time that
  passes while the engine is busy is asked for together when it catches up, so a slow engine is
  never queued more than it can do. Answers to the clock do not wake the page for a frame; answers
  somebody is waiting for do.
- **Measured** with `load.mjs` on an 84-device lab: with these, the page's frame gaps with the
  engine in the worker equal those with it in the page (see the harness README). Before the last
  two changes the worker made them six times worse.
- Not moved: the clock is still driven by the page's frames, so a hidden tab stops simulated time.

## To do, in order

1. **Prove it in a browser, under load.** The throttled-browser harness (`testkit/throttle`: `demo.mjs` and
   `m2.mjs`) is the way, at 8x and with a lab big enough that the engine is busy; and put a check of the worker in CI. The plan's per-frame budget is easiest to meet once this is true.

## Why this order

Steps 1 and 2 could be finished and checked without a browser, and were (step 2 against a
simulated slow engine). Steps 3 and 4 change where the engine runs and when it is asked, which
only a browser can check, and the browser found what the unit tests could not (a clock loop that
doubled the page's work). The last step is the same kind of check at a harder setting, plus keeping
it from regressing.
