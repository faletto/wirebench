# Throttled-browser exit demo

Drives the deployed (or a local) web build in headless Chromium with CPU throttling and runs the
M1 exit demo: build pc - switch - router - pc from scratch, configure every device from its
console, ping across, save, reload, open the saved file, and step the ping in simulation mode.
Screenshots land in `out/` (or the directory you pass) so a person can check each stage by eye.

```sh
npm install
npx playwright install chromium      # if this hangs after the download, see below
node demo.mjs [rate=4] [url=https://faletto.com/wirebench/] [outdir=out]
```

`rate` is CDP `Emulation.setCPUThrottlingRate` (4 = four times slower than this machine).

If `playwright install` finishes downloading and then never exits, unzip the archive it left in
the temp directory into `~/Library/Caches/ms-playwright/chromium-1148/`, touch
`INSTALLATION_COMPLETE` there, and run with
`CHROMIUM=~/Library/Caches/ms-playwright/chromium-1148/chrome-mac/Chromium.app/Contents/MacOS/Chromium`.

Notes on what this can and cannot say:

- Throttling is a proxy for a low-end laptop, not a measurement of one. GPU work is software
  (SwiftShader), which is slower than a Chromebook's GPU in some ways and different in others.
- The stage times printed are dominated by the script's own waits. The numbers worth reading are
  load time, `ready in` and `frame` in the status bar of each screenshot, and the frame pacing lines.
- The page is driven by pixel coordinates (egui draws everything into one canvas), so a toolbar
  layout change means updating `TOOL` and `DEVICE_X`.
- `--force-device-scale-factor=1` and `viewport: null` are deliberate; a fixed-viewport context
  doubled the canvas buffer and skewed pointer positions.

## The M2 run (`m2.mjs`)

`m2.mjs` opens `m2-lab.toml` (three switches in a triangle, rapid spanning tree, a PC on each
side) from the file chooser and exercises what M2 added, in a real browser:

- simulated time running by itself while spanning tree is on, and the tree settling;
- `show spanning-tree` at two switches, and a ping across;
- the settings panel: tick "shut" on a port, watch the tree reroute (cost 38), ping again, untick
  it and watch it heal;
- simulation mode with the live clock running: queue a ping, Step four times, Back twice, open a
  frame to see its field tree;
- idle frame pacing with the live clock repainting.

```sh
(cd ../../crates/app && trunk build --release --dist /tmp/wb-dist)
(cd /tmp/wb-dist && python3 -m http.server 8765 &)
CHROMIUM=... node m2.mjs [rate=1] [url=http://localhost:8765/] [outdir=out-m2]
```

There are no assertions: the evidence is the screenshots (`01-opened` to `11-frame-open`) and the
last line of output. Read them. Coordinates are for a 1366x768 window and the M2 layout (settings
panel on the left, simulation panel on the right). A click on the settings panel takes keyboard
focus from the console, so the script clicks the device again before typing.
