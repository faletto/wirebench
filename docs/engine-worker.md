# Moving the engine into a Web Worker

**Status 2026-10-26:** step 1 of 5 is done. The app talks to the engine only through
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

## To do, in order

2. **An asynchronous link.** `EngineLink::send` returns its answers at once. Across a worker it
   cannot. The calls that need an answer before they can go on are the ones to redesign:
   - opening a terminal (`new_tab`) and every console line (`apply`): the tab shows the output
     when it arrives; the prompt and history wait for it;
   - edits (`edit`, `add_device`, `connect`, delete, undo/redo, load): the canvas can draw
     optimistically and be corrected by the `Lab` event, or wait for it;
   - `snapshot_lab` / `export_lab(&self)`, used by undo, drag, dirty tracking, autosave, save:
     the engine should push a `LabText` after every command that can change it (edits, console
     lines) so the app always holds a recent copy to read synchronously;
   - stepping and Run (`sim_step_hop` returns whether something happened): becomes "ask, then
     the hop arrives".
   Give each command an id and have the worker echo it, so late answers find their request.
3. **The worker entry.** A second wasm target that owns an `Engine` and loops
   `onmessage -> handle_bytes -> postMessage`, built next to the page's by trunk. Transfer
   `ArrayBuffer`s rather than copying.
4. **Pacing.** Run-to-idle and live time then happen off the page's thread, so a storm or a
   long Run cannot stall drawing. `Advance` can be driven by the worker's own timer instead of
   the page's frames.
5. **Prove it in a browser.** The throttled-browser harness (`testkit/throttle`, removed from
   the machine after M1) is the way: load, build a lab, configure, ping, save, with the engine
   in the worker, at 1x and 8x. The plan's per-frame budget is easiest to meet once this is true.

## Why this order

Step 1 was the part that could be finished and checked without a browser. Steps 2 and 3 change
how the app behaves (answers arrive later), which is where bugs will be, and need the harness to
be believed.
