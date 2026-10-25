// The M1 exit demo under CPU throttling: build the four-node lab from scratch, configure it from the
// consoles, ping across, save, reload, open the saved file and step the ping in simulation mode.
//   CHROMIUM=... node demo.mjs [rate=4] [url=https://faletto.com/wirebench/] [outdir=out]
import { mkdirSync } from 'fs';
import { launch, TOOL, click, tap, frameStats } from './lib.mjs';

const rate = Number(process.argv[2] ?? 4);
const outdir = process.argv[4] ?? 'out';
mkdirSync(outdir, { recursive: true });
const { browser, page, url } = await launch({ rate, url: process.argv[3] ?? 'https://faletto.com/wirebench/' });
const rows = [];
let t = Date.now();
const lap = async (what, shot) => {
  const ms = Date.now() - t;
  rows.push([what, ms]);
  console.log(String(ms).padStart(7), 'ms ', what);
  if (shot) await page.screenshot({ path: `${outdir}/${shot}.png` });
  t = Date.now();
};
const settle = ms => page.waitForTimeout(ms);
const say = async text => { await page.keyboard.type(text); await page.keyboard.press('Enter'); await settle(150); };
const run = async lines => { for (const l of lines) await say(l); };
// Clicking a device opens its console and focuses it (Alt+N did not fire in headless Chromium).
const DEVICE_X = { 1: 250, 2: 500, 3: 750, 4: 1000 };
const tab = async n => { await tap(page, DEVICE_X[n], 151); await settle(200); };

page.on('console', m => { if (m.text().includes('first frame')) console.log('        ', m.text()); });
await page.goto(url);
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(500);
await lap('load to first interactive frame', '01-loaded');

// 1. Build from scratch: clear the stock lab, then place and cable pc - switch - router - pc.
await click(page, TOOL.del);
for (const x of [248, 533, 833, 1118]) { await tap(page, x, 151); await settle(150); }
await click(page, TOOL.pc);     await tap(page, 250, 151);  await settle(150);
await click(page, TOOL.sw);     await tap(page, 500, 151);  await settle(150);
await click(page, TOOL.router); await tap(page, 750, 151);  await settle(150);
await click(page, TOOL.pc);     await tap(page, 1000, 151); await settle(150);
await click(page, TOOL.cable);
for (const [a, b] of [[250, 500], [500, 750], [750, 1000]]) {
  await tap(page, a, 151); await settle(150);
  await tap(page, b, 151); await settle(300);
}
await click(page, TOOL.select);
await lap('build topology from scratch (4 devices, 3 cables)', '02-built');

// 2. Configure entirely from the consoles.
await tab(3);
await run(['enable', 'configure terminal', 'interface g0/0', 'ip address 10.0.1.1 255.255.255.0', 'no shutdown', 'exit',
  'interface g0/1', 'ip address 10.0.2.1 255.255.255.0', 'no shutdown', 'end', 'show ip interface brief', 'copy running-config startup-config']);
await lap('configure r1 (12 commands)', '03-r1');
await tab(1); await run(['ipconfig set 10.0.1.2 255.255.255.0', 'ipconfig gateway 10.0.1.1', 'ipconfig']);
await tab(4); await run(['ipconfig set 10.0.2.2 255.255.255.0', 'ipconfig gateway 10.0.2.1', 'ipconfig']);
await lap('configure pc1 and pc2', '04-pcs');

// 3. Ping across the router (pc1 -> pc2); allow the replies to print.
await tab(1);
await say('ping 10.0.2.2');
await settle(2500);
await lap('ping pc1 -> pc2 from the console', '05-ping');

// 4. Save: the web build downloads a .lab.toml.
const [download] = await Promise.all([page.waitForEvent('download'), tap(page, 320, 12)]);
const saved = `${outdir}/saved.lab.toml`;
await download.saveAs(saved);
await lap(`save lab (${download.suggestedFilename()})`, '06-saved');

// 5. Reload the page; the autosave offers recovery, but the demo opens the saved file instead.
await page.reload();
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(500);
await lap('reload page to first interactive frame', '07-reloaded');
const [chooser] = await Promise.all([page.waitForEvent('filechooser'), tap(page, 258, 12)]);
await chooser.setFiles(saved);
await settle(800);
await lap('open saved lab', '08-opened');

// 6. Step the ping in simulation mode (the sim panel's source is pc1, destination 10.0.2.2).
await tap(page, 1117, 314); await settle(200);       // Queue ping
const steps = [];
for (let i = 0; i < 12; i++) { await tap(page, 1187, 314); await settle(250); }  // Step
await lap('queue ping and step 12 frames in simulation mode', '09-stepped');

// 7. Frame pacing while the packet animation plays, then idle.
await tap(page, 1117, 314); await settle(100);
await tap(page, 1236, 314); // Play
const anim = await frameStats(page, 4000);
await lap('animate (4 s)');
await tap(page, 1236, 314); await settle(1500);
const idle = await frameStats(page, 3000);
console.log('        animating:', JSON.stringify(anim));
console.log('        idle     :', JSON.stringify(idle));
await page.screenshot({ path: `${outdir}/10-final.png` });
await browser.close();
