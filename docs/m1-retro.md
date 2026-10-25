# M1 retrospective

**M1's exit demo works end to end, and its one unmeasured piece is replaced by a throttled-laptop
proxy.** Building two PCs, a switch and a router from scratch, configuring them from their
consoles, pinging across, saving, reloading, opening the saved lab and stepping the ping in
simulation mode all ran under 1x, 4x, 6x and 8x CPU throttling. The project owner also tried the
deployed site at 20x CPU slowdown in a real browser and reported that it felt the same. There is
no Chromebook measurement and there will not be one (see below).

Elapsed: started 2026-10-21; PRs #3 to #5 were merged by 2026-10-22; the throttled demo and this
retro are 2026-10-25. As with M0, the pace came from working with an AI assistant and should not be
used to size M2.

## Exit criterion

| Criterion (from the plan) | Result |
|---|---|
| Build the four-node topology from scratch | **Met** in the browser, scripted by `testkit/throttle/demo.mjs` (place PC, switch, router, PC; cable them) |
| Configure it entirely from the CLI | **Met**; a router and two PCs, with `copy running-config startup-config` |
| Ping across | **Met**; 4 of 4 replies from the console, also as a hop-by-hop trace |
| Save the lab, reload the page, load it | **Met**; web download, reload, file picker, then the same lab opens |
| Step the ping in simulation mode | **Met**; 12 steps show both ARP exchanges and the echo request and reply across the router, with packet animation |
| Golden test of the same scenario passes in CI | **Met**; `testkit/scenarios/m1_static_routes.{script,out}` run through the CLI |
| On a school-grade Chromebook | **Replaced.** No Chromebook is available. A throttled desktop browser stands in, with the limits below |

Note that the demo topology is pc, switch, router, pc, which is the plan's scope (two PCs, one
switch, one router). The golden script uses two routers instead; both are covered.

## What shipped, against the M1 scope

| Plan scope | Status |
|---|---|
| Router and switch run the IOS-style shell | Done. Switch config is the hostname only |
| `show running-config`, `show ip interface brief`, `show ip route`, `show mac address-table`, `copy run start` | Done. `copy run start` is not persisted into the saved lab |
| PCs have `ipconfig` and `ping` | Done |
| Canvas: place devices, draw links, click to open a terminal | Done, and the click-to-open path was exercised by the browser demo |
| Lab save and load in TOML | Done on native (path) and web (download and picker), plus dirty tracking, autosave with recovery, confirm dialogs, Ctrl/Cmd+S and +O |
| Simulation mode steps ARP and ICMP with a basic PDU view | Done: hop log panel and packet animation. The PDU view is a one-line summary per frame, not a field tree |
| Deployed to a static URL | Done: `faletto.com/wirebench/` via Pages |

Not checked in this pass: the plan's 4 ms per-frame engine budget for the in-process host. The
code was not read against it.

Tests: 204 pass across the workspace.

## What went wrong or surprised us

1. **Deleting the last device crashed the whole page.** The terminal panel and the canvas both
   indexed `self.tabs[active]`, which is empty once every device is gone. 203 tests and three
   merged PRs had not caught it; it appeared the first time something deleted everything, which
   the from-scratch demo has to do. Fixed with a regression test that fails without the fix. The
   deployed site had the bug until this change merged. The demo found it, which is the argument
   for keeping a browser-driven test around.
2. **The M0 process gap is closed locally but not in CI.** M0 listed "no automated browser smoke
   test" as a gap. `testkit/throttle` is that test, but it is not wired into CI: it needs a
   Chromium download, drives the UI by pixel coordinates, and was only run by hand.
3. **A console ping advances four seconds of virtual time inside one frame, and that frame is
   slow.** The status bar read 287 ms at 1x and 254 ms at 8x throttle, so the cost does not scale
   with CPU speed and is probably not compute. Not investigated. It sits against the plan's
   4 ms per-frame budget and is the most useful thing to look at first if typing ever feels laggy.
4. **Idle still polls.** About 180 animation frames arrived in 3 s of no input (60 Hz) at every
   throttle level. This is the eframe web loop the M0 baseline described. The idle CPU target is
   still not met as written.
5. **Browser automation needed workarounds.**
   - `playwright install` finishes downloading and then never exits here; the archive had to be
     unpacked by hand.
   - A fixed-viewport Playwright context doubled the canvas buffer and skewed pointer positions
     by 2x. Using `viewport: null` plus `--force-device-scale-factor=1` fixed it. This was a
     test artifact, not an app bug, but it looked like one until the cause was found.
   - egui only registers a click on a widget it saw hovered, so every click is move, press and
     release on separate frames. Back-to-back events on a throttled page sometimes dropped the
     click.
   - `Alt+digit` tab switching did not fire in headless Chromium. The M0 fix for macOS
     (Option+digit) was not retested in a real browser here.
6. **Cosmetic:** a lab built from scratch keeps the stock name "Two routers, static routes".

## Measurements (throttled desktop browser, not a Chromebook)

Headless Chromium on an Apple Silicon Mac, software GL (SwiftShader), local release build,
1366x768, device scale factor 1.

| Throttle | Load to first interactive frame | Animation frame gap p50 / p99 / max (4 s) |
|---|---|---|
| 1x | 0.83 s | 16.7 / 17.6 / 18.0 ms |
| 4x | 1.03 s | 16.9 / 19.2 / 24.4 ms |
| 6x | 1.11 s | 17.0 / 19.2 / 20.0 ms |
| 8x | 1.21 s | 17.1 / 20.0 / 21.7 ms |

The app's own "ready in" readout ranged from 0.1 to 0.5 s in the same runs. Frame gaps are
`requestAnimationFrame` intervals, so they show pacing, not paint cost. These numbers are a proxy:
SwiftShader is slower than a real GPU in some ways and different in others, throttling scales the
main thread only, the network was local, and one machine produced all of it. They do say that
nothing in the build, configure, save, open and simulate flow collapses when the CPU is 8x slower,
and a real browser at 20x agreed.

## The plan's risks, now

| Risk (plan) | Now |
|---|---|
| The CLI engine eats the schedule | Did not happen; M1's CLI work was a small part of it |
| Chromebook performance | Unmeasured by decision; the throttled proxy shows large headroom |
| Accessibility of a canvas UI | Unchanged from M0: terminal untested with a screen reader. Still open |
| Losing work in a browser | Addressed: dirty tracking, autosave with recovery, leave-page warning. Confirmed by the project owner on the deployed site |

## Known gaps carried into M2

- No undo.
- Deleting a device or cable rebuilds the world, so consoles, ARP caches and virtual time restart.
- No port labels on cables.
- `copy run start` is not persisted into the saved lab; switch config is the hostname only.
- The management plane (`enable secret`, `banner motd`, `reload`, line and vty) is not implemented.
- Messages and `show` output are still inline strings, not the message catalog.

## Decisions for the project owner

1. **Call M1 done?** I would. The exit demo works; the Chromebook step is dropped by decision.
2. **Wire the browser demo into CI?** It would have caught the delete crash, but it needs a
   Chromium download and coordinates that break when the toolbar changes. A smaller smoke test
   (load, type `enable`, no panic) would be cheaper to keep green.
3. **Order of M2 work.** Undo before VLANs, or after. Undo gets more expensive as editing grows.

## Suggested start of M2

1. Investigate the slow frame after a console ping (item 3 above) and decide whether the engine
   should be time-sliced.
2. Undo for canvas edits, and stop resetting consoles on delete.
3. VLAN database, access and trunk ports, `show vlan brief`, `show interfaces trunk`.
4. Management plane, with our own secret-hash label rather than Cisco's formats (see the plan's
   legal notes).
5. Typed `DeviceConfig` and the message catalog, so `copy run start` can persist.
