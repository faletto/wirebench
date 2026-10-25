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
