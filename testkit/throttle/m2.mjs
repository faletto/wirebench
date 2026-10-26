// The M2 features in a real browser, under CPU throttling: a lab of three switches in a triangle
// (rapid spanning tree) with a PC on each side is opened from a file, left to settle in live time,
// read from the console and the settings panel, cut and healed from the panel, then stepped,
// stepped back and inspected in simulation mode. Screenshots are the evidence; read them.
//   CHROMIUM=... node m2.mjs [rate=1] [url=http://localhost:8765/] [outdir=out-m2]
import { mkdirSync } from 'fs';
import { launch, tap, frameStats } from './lib.mjs';

const rate = Number(process.argv[2] ?? 1);
const outdir = process.argv[4] ?? 'out-m2';
mkdirSync(outdir, { recursive: true });
const { browser, page, url } = await launch({ rate, url: process.argv[3] ?? 'http://localhost:8765/' });
const settle = ms => page.waitForTimeout(ms);
const shot = async n => { await page.screenshot({ path: `${outdir}/${n}.png` }); console.log('        shot', n); };
const say = async text => { await page.keyboard.type(text); await page.keyboard.press('Enter'); await settle(250); };

// A click on the settings panel takes keyboard focus; clicking the device again gives it back to
// the console, so every console command after a panel click is preceded by `open`.
// Pixel positions of the lab in a 1366x768 window (see m2-lab.toml); panels as laid out in M2.
const DEV = { sw1: [683, 87], sw2: [483, 214], sw3: [883, 214], pc1: [362, 211], pc2: [1003, 211] };
const SHUT = n => [233, 432 + 23 * (n - 1)];    // the settings panel's "shut" box for port Fa0/n
const SIM = { dst: [1230, 290], queue: [1117, 314], step: [1187, 314], back: [1329, 314], row: [1200, 418] };
const open = async name => { await tap(page, ...DEV[name]); await settle(250); };

await page.goto(url);
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(500);
const [chooser] = await Promise.all([page.waitForEvent('filechooser'), tap(page, 258, 12)]);
await chooser.setFiles(new URL('./m2-lab.toml', import.meta.url).pathname);
await settle(500);
console.log('  opened the lab; letting simulated time run (spanning tree converges in real time)');
await settle(12000 * Math.max(1, rate / 2));
await shot('01-opened');

await open('sw2'); await say('show spanning-tree'); await shot('02-sw2-tree');
await open('sw3'); await say('show spanning-tree'); await shot('03-sw3-tree');
await open('pc1'); await say('ping 10.0.0.2'); await settle(3500); await shot('04-ping-across');

// Cut sw2's link to the root from the settings panel (tick "shut" on Fa0/1) and watch it reroute.
await open('sw2'); await tap(page, ...SHUT(1)); await settle(500); await shot('05-fa01-shut');
await settle(9000 * Math.max(1, rate / 2));
await open('sw2'); await say('show spanning-tree'); await shot('06-after-cut');
await open('pc1'); await say('ping 10.0.0.2'); await settle(3500); await shot('07-ping-after-cut');
await open('sw2'); await tap(page, ...SHUT(1)); await settle(9000 * Math.max(1, rate / 2));
await open('sw2'); await say('show spanning-tree'); await shot('08-healed');

// Simulation mode: aim the panel at pc2 (it starts at 10.0.2.2), queue a ping, step it, step back,
// and open a frame.
await tap(page, ...SIM.dst); await page.keyboard.press('End');
for (let i = 0; i < 15; i++) await page.keyboard.press('Backspace');
await page.keyboard.type('10.0.0.2'); await settle(200);
await tap(page, ...SIM.queue); await settle(200);
for (let i = 0; i < 4; i++) { await tap(page, ...SIM.step); await settle(300); }
await shot('09-stepped');
await tap(page, ...SIM.back); await settle(300);
await tap(page, ...SIM.back); await settle(300);
await shot('10-stepped-back-twice');
await tap(page, ...SIM.row); await settle(400);
await shot('11-frame-open');

// With spanning tree on the page repaints about ten times a second; see what that costs.
const idle = await frameStats(page, 3000);
console.log('        idle with spanning tree running:', JSON.stringify(idle));
await browser.close();
