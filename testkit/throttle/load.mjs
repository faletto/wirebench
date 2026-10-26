// A big lab (12 switches on rapid spanning tree, 72 PCs) opened from a file: does the page stay
// responsive while the engine works, with the engine in the worker and in the page?
//   CHROMIUM=... node load.mjs [rate=4] [url=http://localhost:8765/] [outdir=out-load] [lab=./big-lab.toml]
import { mkdirSync } from 'fs';
import { launch, tap, frameStats } from './lib.mjs';
const rate = Number(process.argv[2] ?? 4);
const outdir = process.argv[4] ?? 'out-load';
mkdirSync(outdir, { recursive: true });
const { browser, page, url } = await launch({ rate, url: process.argv[3] ?? 'http://localhost:8765/' });
let inWorker = false;
page.on('console', m => { if (m.text().includes('engine in a Web Worker')) inWorker = true; });
const settle = ms => page.waitForTimeout(ms);
await page.goto(url);
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(500);
const t0 = Date.now();
const [chooser] = await Promise.all([page.waitForEvent('filechooser'), tap(page, 258, 12)]);
await chooser.setFiles(new URL(process.argv[5] ?? './big-lab.toml', import.meta.url).pathname);
// How long until the page shows the lab (a tab per device in the top bar)?
await settle(1500);
console.log(`  open + 1.5 s: ${Date.now() - t0} ms wall`);
await page.screenshot({ path: `${outdir}/1-opened.png` });
const live = await frameStats(page, 4000);
console.log('  frames with the lab running live:', JSON.stringify(live));
// Typing while the engine is busy: how long until the console answers?
await tap(page, 700, 90); await settle(300);
const typed = Date.now();
await page.keyboard.type('show spanning-tree'); await page.keyboard.press('Enter');
await settle(2500);
await page.screenshot({ path: `${outdir}/2-console.png` });
console.log('  typed a command at', Date.now() - typed, 'ms; see 2-console.png');
const after = await frameStats(page, 3000);
console.log('  frames after:', JSON.stringify(after));
await browser.close();
// Last line, for check.mjs.
console.log('RESULT ' + JSON.stringify({ inWorker, live, after }));
