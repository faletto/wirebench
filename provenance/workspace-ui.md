# Home screen and workspace layout

**Date:** 2026-11-08
**Source category:** own design. No Packet Tracer material was used: no screenshots, icons, artwork,
colours, help text or labels were consulted or copied. The broad arrangement (a bar of components
along the bottom edge that are dragged onto a canvas, and a window that opens when a device is
clicked) is a common pattern in diagramming and network-modelling tools and is described here in
our own terms. Tests: `crates/app/tests/home.rs`, `crates/app/tests/canvas.rs`,
`crates/app/tests/headless.rs`, unit tests in `crates/app/src/workspace.rs`.

## What it is

- **Home screen** (what the app opens on): New lab (with an optional name), Open a file,
  Continue (unsaved work from a session that ended without saving, else the lab that was open
  last), and example labs (`labs/examples/`, plus `labs/m1_static_routes.lab.toml`), each
  pre-configured with a note on the canvas saying what to try. "How it works" gives three steps.
- **Top bar:** Home, the lab's title and whether it has unsaved changes, Open, Save, Undo, Redo,
  Select, Delete, and a chip per device (a picker past twelve) that opens the device.
- **Component bar** (bottom): categories Routers, Switches, PCs, Cables; each lists models
  (`router-2g/3g/4g`, `switch-l2-8/24/48`, `host-pc`, a cable). A model is dragged onto the canvas,
  or clicked and then the canvas clicked. A cable dropped on a device starts there; the next device
  clicked ends it (first free port on each, as before). Live / Simulation at the right show or
  hide the simulation panel.
- **Device window:** clicking a device opens a movable, resizable window with two tabs, Console and
  Settings (the panel described in `device-panels.md`). It opens in the corner of the canvas where
  it covers the fewest devices. Esc closes it (after first stopping any tool in use). Alt+1..9
  (Option on a Mac) opens the first nine devices; the shortcut reads the modifiers carried by the
  key event, because in the browser a quick Option+digit can be let go before the frame that sees
  it, and the symbol Option+digit types on a Mac is dropped rather than typed into a console.
- **Symbols:** drawn in code (`canvas.rs`, `icons.rs`): a disc with four arrows for a router, a slab
  with two opposed arrows for a switch, a screen on a stand for a PC; toolbar icons are simple line
  drawings. Device kind is carried by shape and label as well as colour.

## Not done

- One device window at a time; clicking another device moves the window to it.
- Continue remembers one lab, not a list of recent files.
- `testkit/throttle/demo.mjs`, `m2.mjs` and `m2b.mjs` drive the page by pixel and still use the old
  layout's coordinates; `load.mjs` (used by CI) was updated.
