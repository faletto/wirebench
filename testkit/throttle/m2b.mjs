// What m2.mjs left out, in a real browser: the DTP port-mode control, the inspector's filter box, and
// Save then Open of a lab that has been changed. Screenshots are the evidence for what is drawn;
// the saved file is checked by the script itself.
//   CHROMIUM=... node m2b.mjs [rate=1] [url=http://localhost:8765/] [outdir=out-m2b]
import { mkdirSync, readFileSync } from 'fs';
import { launch, tap } from './lib.mjs';

const rate = Number(process.argv[2] ?? 1);
const outdir = process.argv[4] ?? 'out-m2b';
mkdirSync(outdir, { recursive: true });
const { browser, page, url } = await launch({ rate, url: process.argv[3] ?? 'http://localhost:8765/' });
const settle = ms => page.waitForTimeout(ms);
const shot = async n => { await page.screenshot({ path: `${outdir}/${n}.png` }); console.log('        shot', n); };
const say = async text => { await page.keyboard.type(text); await page.keyboard.press('Enter'); await settle(250); };
const failures = [];
const check = (ok, what) => { console.log(ok ? '  ok  ' : '  FAIL', what); if (!ok) failures.push(what); };

// Layout of a 1366x768 window with m2-lab.toml (see m2.mjs).
const DEV = { sw1: [683, 87], sw2: [483, 214], sw3: [883, 214], pc1: [362, 211], pc2: [1003, 211] };
const open = async name => { await tap(page, ...DEV[name]); await settle(250); };
const MODE = n => [104, 432 + 23 * (n - 1)];                 // the port-mode drop-down for Fa0/n
const CHOICE = { access: 27, trunk: 50, auto: 73, desirable: 96 };  // y offset of each choice below it
const setMode = async (port, mode) => {
  const [x, y] = MODE(port);
  await tap(page, x, y); await settle(300);
  await tap(page, x + 10, y + CHOICE[mode]); await settle(400);
};
const SIM = { dst: [1230, 290], queue: [1117, 314], run: [1284, 314], filter: [1235, 392] };
const typeInto = async ([x, y], text) => {
  await tap(page, x, y); await page.keyboard.press('End');
  for (let i = 0; i < 20; i++) await page.keyboard.press('Backspace');
  await page.keyboard.type(text); await settle(300);
};
const openLab = async path => {
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), tap(page, 258, 12)]);
  await chooser.setFiles(path);
  await settle(600);
};
// The Save button is further right while "unsaved changes" is showing.
const save = async (to, x) => {
  const [download] = await Promise.all([page.waitForEvent('download'), tap(page, x, 12)]);
  await download.saveAs(to);
  return readFileSync(to, 'utf8');
};

await page.goto(url);
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(500);
await openLab(new URL('./m2-lab.toml', import.meta.url).pathname);
await settle(3000);

// --- DTP: one end "dynamic desirable" turns a link between two "dynamic auto" ends into a trunk.
await open('sw2'); await say('show interfaces trunk'); await shot('01-before-no-trunk');
await open('sw1'); await setMode(1, 'desirable'); await shot('02-sw1-desirable');
await settle(3000 * Math.max(1, rate / 2));
await open('sw2'); await say('show interfaces trunk'); await shot('03-sw2-sees-trunk');
await open('sw2'); await say('show interfaces fa0/1 switchport'); await shot('04-sw2-switchport');
await open('sw1'); await setMode(1, 'access'); await settle(3000 * Math.max(1, rate / 2));
await open('sw2'); await say('show interfaces trunk'); await shot('05-back-to-access');

// --- The inspector's filter: queue a ping, run it, then narrow the hop log.
await typeInto(SIM.dst, '10.0.0.2');
await tap(page, ...SIM.queue); await settle(300);
await tap(page, ...SIM.run); await settle(1500);
await shot('06-hop-log');
await typeInto(SIM.filter, 'icmp'); await shot('07-filter-icmp');
await typeInto(SIM.filter, 'arp'); await shot('08-filter-arp');
await typeInto(SIM.filter, 'dev:nosuchdevice'); await shot('09-filter-matches-nothing');
await typeInto(SIM.filter, ''); await shot('10-filter-cleared');

// --- Save and Open of a changed lab: a trunk set from the panel is in the file, and the file opens
// to the same lab (saving it again gives the same text).
await open('sw1'); await setMode(1, 'trunk'); await shot('11-changed-unsaved');
const first = await save(`${outdir}/saved.lab.toml`, 289);
await shot('12-saved');
check(/switchport mode trunk/.test(first), 'the saved file has the trunk setting');
await page.reload();
await page.waitForFunction(() => !document.getElementById('loading'), null, { timeout: 120000 });
await settle(800);
await shot('13-reloaded');
await openLab(`${outdir}/saved.lab.toml`);
await open('sw1'); await say('show running-config'); await shot('14-reopened');
const second = await save(`${outdir}/saved-again.lab.toml`, 168);
check(second === first, 'opening the saved file and saving it again gives the same text');

// --- Delete and undo happen in place: another device's console keeps its session and its place.
const TOOLBAR = { select: [33, 37], del: [348, 37], undo: [421, 37] };
await open('sw2'); await say('enable'); await say('configure terminal'); await shot('15-sw2-in-config-mode');
await tap(page, ...TOOLBAR.del); await settle(150);
await tap(page, ...DEV.pc2); await settle(500);
await tap(page, ...TOOLBAR.select); await settle(150);
await shot('16-pc2-deleted');
await tap(page, ...TOOLBAR.undo); await settle(800);
await shot('17-delete-undone');
await open('sw2'); await say('hostname core'); await shot('18-sw2-same-session');

await browser.close();
if (failures.length) { console.error(`FAILED: ${failures.length}`); process.exit(1); }
