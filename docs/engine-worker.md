# Moving the engine into a Web Worker

**Status 2026-10-26:** steps 1 and 2 of 5 are done (step 2 is described under "Done"). The app talks to the engine only through
`EngineLink` (`crates/app/src/link.rs`), and every command and answer crosses it as postcard
bytes, so everything that must cross a worker boundary is already serialisable and exercised by
the whole app test suite (462 tests). The engine still runs in the page's thread; nothing in the
browser has changed, and nothing about the worker has been tried in a browser.

## Done

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

## To do, in order

2. **The worker entry.** A second wasm target that owns an `Engine` and loops
   `onmessage -> handle_bytes -> postMessage`, built next to the page's by trunk. Transfer
   `ArrayBuffer`s rather than copying.
3. **Pacing.** Run-to-idle and live time then happen off the page's thread, so a storm or a
   long Run cannot stall drawing. `Advance` can be driven by the worker's own timer instead of
   the page's frames.
4. **Prove it in a browser.** The throttled-browser harness (`testkit/throttle`: `demo.mjs` and
   `m2.mjs`) is the way, with the engine in the worker, at 1x and 8x. The plan's per-frame budget is easiest to meet once this is true.

## Why this order

Steps 1 and 2 could be finished and checked without a browser, and were (step 2 against a
simulated slow engine, and both harness runs unchanged in Chromium). What is left changes where
the engine runs, which only a browser can check.
