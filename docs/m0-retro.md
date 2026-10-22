# M0 retrospective

**M0's exit criterion is met except for one measurement: web cold start on a Chromebook.** The
core is deterministic across three targets, the CLI and the egui front end work, the app is
deployed, and there is a measured baseline. Two plan targets are *not* met as written
(kernel throughput, idle CPU); both are explained below and neither blocks M1.

Elapsed: three calendar days over several working sessions (2026-10-19 to 2026-10-21), against
a two-week plan for one developer. The pace came from working with an AI assistant and should
not be used to size M1.

## Exit criterion

| Criterion (from the plan) | Result |
|---|---|
| Two-host ping gives a byte-identical trace hash on x86_64, aarch64 and wasm32 in CI | **Met**, for four scenarios (two-host ping, two-router chain, the CLI-driven script, the TOML lab). CI diffs the hashes on Ubuntu, macOS (arm64) and wasm32 under wasmtime |
| WASM bundle size measured and written down | **Met**: 1.71 MB gzip |
| Web cold start measured and written down | **Partly**: 399 ms to first paint on an M3 laptop. **Chromebook not measured** |
| Kernel event throughput measured and written down | **Met**: ~1.6 M events/s native, 1.58 M wasm (wasmtime proxy) |
| egui text input on web does not feel wrong | **Typing, yes** (tester report). Paste and IME not verified in a browser |

## What shipped, against the day-by-day plan

| Days | Plan | Status |
|---|---|---|
| 1 | Workspace, CI, licences, CONTRIBUTING, DCO, cargo-deny | Done. LICENSE files, NOTICE and a DCO check landed last, at the end of M0 |
| 2-3 | `wire`: Ethernet, 802.1Q, ARP, IPv4, ICMP, fuzz targets | Done. Fuzz targets landed at the end: seven targets, ~108 M executions, no crashes, and they run in CI |
| 4-5 | `kernel`: virtual time, event queue, links, trace hash, RNG | Done |
| 6-7 | `netdev`: hosts, ARP, L2 switch (learning, aging), world dispatch | Done |
| 8-9 | `clish`: grammar, prefix matching, caret errors, `?`, Tab, modes | Done, with the three named transcript tests |
| 10 | Router: LPM, TTL, ICMP errors, show commands | Done |
| 11 | Engine protocol (postcard) and the CLI runner | Done |
| 12 | egui terminal widget | Done, with headless UI tests instead of Playwright |
| 13 | Canvas from `lab.toml`, click to open a terminal, deploy to Pages | Done. Playwright smoke test **not done** |
| 14 | Measure, determinism job, retro | Done (this document, `docs/perf-baseline.md`) |

Beyond the plan: the `labfile` crate and a lab builder (the plan had them in M1), a 100-node
benchmark generator, and a public deploy at `faletto.com/wirebench/`.

Partial: "every user-visible string goes into the message catalog". `clish` has a catalog; the
shell, `show` output and UI strings are still inline in `engine::shell` and `app`.

## What went wrong or surprised us

1. **Throughput is ~20% under target** (1.6 M vs 2 M events/s). Not profiled yet.
2. **Idle is not ~0%.** eframe's web loop polls at the display refresh rate forever
   (120 callbacks/s here). It is cheap (~1.2% of a core) but constant.
3. **A real bug in lab loading**, found by a test written to probe out-of-range ports: a switch
   port named `Fa0/0` panicked on an eager `n - 1`. Fixed; a lab file must never panic the app.
4. **Wrong expectations in my own tests** (the first golden RTT) were caught only by running them.
   Golden values were derived from the code's behaviour and then reasoned about by hand;
   that review step is what made them trustworthy.
5. **CRLF checkouts on Windows broke every golden-file test**, visible only in CI. Fixed with a
   `.gitattributes` that pins LF.
6. **Build output was committed** (`crates/app/dist`) by a blanket `git add -A`. Untracked and
   ignored now; it remains in the branch's history.
7. **egui/eframe 0.37 differs from what we knew** (renamed types, changed input model). Several
   compile-fix cycles. Pin the version and upgrade on purpose.
8. **`cargo-deny` failed when eframe arrived**: three permissive licences (clipboard crate and
   bundled fonts) and three "unmaintained" notices in accesskit's Linux D-Bus stack. Handled with
   an explicit allow list and ignore list in `deny.toml`; worth a second look.
9. **Two usability bugs found only by hands-on testing**: on macOS, Option+digit typed a
   special character when switching devices, and there was no word-wise cursor movement. Both
   fixed.
10. **The web build was never loaded in a browser by the assistant that wrote it** (no browser
    tooling in the session). Everything browser-side was checked by the project owner. That is
    a process gap: M1 needs an automated browser smoke test.

## The plan's risks, now

| Risk (plan) | Now |
|---|---|
| egui text input on web feels wrong | Typing is fine. Paste and IME unverified |
| `clish` eats the schedule | Did not happen; built in about one session |
| Accessibility of a canvas UI | Partly addressed (focusable, labelled device buttons; keyboard device switching). The terminal is custom-painted and has not been tried with a screen reader. **Still open** |
| Wasm too big | 1.71 MB gzip of a 4 MB budget; propose a 2.5 MB CI budget |
| Determinism | Strongest area: hashes pinned and CI-checked on three targets |
| IP and naming | Name chosen (wirebench); domains, trademarks and the lawyer review still to do |

## Decisions for the project owner

1. **Call M0 done?** I would, with the Chromebook cold-start number as the first M1 task.
2. **Copyright holder.** The licence files say "wirebench contributors"; put a legal name or
   entity there if you want one.
3. **CC BY 4.0 for docs, labs and the message catalog** is promised in `CONTRIBUTING.md`, but no
   licence text file exists for it yet.
4. **`deny.toml` exceptions** (BSL-1.0, OFL-1.1, Ubuntu-font-1.0; three ignored advisories): accept or
   replace the dependencies.
5. **Message wording and `show` formats** are our own text but follow IOS idiom closely (route
   codes, interface names). Decide how close is close enough, and have the IP lawyer look.
6. **The deployed site is public.** It exposes the app and the bundled demo lab only.

## Suggested start of M1

1. Run the app on a Chromebook; read "ready in N ms" with the cache disabled.
2. Add an automated browser smoke test (page loads, canvas present, terminal accepts `enable`).
3. Yield to the browser before the setup task so the loading text paints; measure again.
4. Profile the kernel and recover the last 20%.
5. M1 proper: switch CLI and `show mac address-table`, simulation-mode stepping with a basic PDU
   view, device dragging and link drawing, and save/load of labs from the UI.
6. A UI bench mode, to measure frame time at 100 nodes.
