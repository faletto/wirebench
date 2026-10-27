// CI check of offline use. First an older version is served (the same build with its bundle
// renamed, as a new hash would), then the real one, which must replace it in the service worker's
// cache. Then the server stops, the browser goes offline and the page is opened again: it must
// come back from the cache and its engine (in the worker, whose bundle also has to come from the
// cache) must answer.
//   CHROMIUM=... node offline.mjs <dist dir> [port=8766] [path=/]
// `path` is where the build expects to be served from (trunk's --public-url).
import http from 'http';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { launch } from './lib.mjs';

const dist = path.resolve(process.argv[2] ?? '../../crates/app/dist');
const port = +(process.argv[3] ?? 8766);
const prefix = process.argv[4] ?? '/';
const url = `http://localhost:${port}${prefix}`;

const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json',
  '.webmanifest': 'application/manifest+json', '.svg': 'image/svg+xml', '.png': 'image/png' };
// The older version: every bundle file renamed, and the page pointing at the new names.
const old = fs.mkdtempSync(path.join(os.tmpdir(), 'wirebench-old-'));
let page0 = fs.readFileSync(path.join(dist, 'index.html'), 'utf8');
for (const f of fs.readdirSync(dist)) {
  const renamed = f.replace(/^(wirebench-app-)/, '$1old');
  fs.copyFileSync(path.join(dist, f), path.join(old, renamed));
  if (renamed !== f) page0 = page0.replaceAll(f, renamed);
}
fs.writeFileSync(path.join(old, 'index.html'), page0);
let root = old;
fs.mkdirSync('out-offline', { recursive: true });

const server = http.createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url, url).pathname);
  if (!p.startsWith(prefix)) { res.writeHead(404).end(); return; }
  p = p.slice(prefix.length);
  if (p === '' || p.endsWith('/')) p += 'index.html';
  const file = path.join(root, p);
  if (!file.startsWith(root) || !fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[path.extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-cache' });
  fs.createReadStream(file).pipe(res);
});
await new Promise(r => server.listen(port, r));

const { browser, context, page } = await launch({ rate: 1, url });
const errors = [];
page.on('pageerror', e => errors.push(e.message));
page.on('console', m => { if (m.type() === 'error') errors.push(m.text().slice(0, 300)); });
// Count what workers send back: the engine's answers.
await context.addInitScript(() => {
  window.__workerReplies = 0;
  const Real = window.Worker;
  window.Worker = class extends Real {
    constructor(...a) { super(...a); this.addEventListener('message', () => window.__workerReplies++); }
  };
});

const fail = [];
const replies = () => page.evaluate(() => window.__workerReplies);
const waitFor = async (what, fn, ms = 60000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    // An error page (or one being replaced) has no app to ask: that counts as not yet.
    if (await fn().catch(() => false)) return true;
    await page.waitForTimeout(250);
  }
  fail.push(`timed out waiting for ${what}`);
  return false;
};

// The bundle files in the cache, by name.
const bundle = () => page.evaluate(async () => (await (await caches.open('wirebench')).keys())
  .map(k => new URL(k.url).pathname.split('/').pop()).filter(n => n.startsWith('wirebench-app-')).sort());
const want = f => fs.readdirSync(f).filter(n => n.startsWith('wirebench-app-')).sort().join(' ');

await page.goto(url);
await waitFor('the engine to answer online (older version)', async () => (await replies()) > 0);
await waitFor('the older version\'s offline copy', async () =>
  (await page.evaluate(() => !!navigator.serviceWorker.controller)) && (await bundle()).join(' ') === want(old));

root = dist;
await page.goto(url);
await waitFor('the engine to answer online', async () => (await replies()) > 0);
await waitFor('the new version to replace the old in the offline copy', async () => (await bundle()).join(' ') === want(dist));
const cached = await page.evaluate(async () => (await (await caches.open('wirebench')).keys()).map(k => new URL(k.url).pathname)).catch(() => []);
console.log('cached:', cached.join(' '));

// No network at all from here: the server is gone and the browser is offline.
await new Promise(r => { server.close(r); server.closeAllConnections(); });
await context.setOffline(true);
for (const query of ['', '?engine=inline']) {
  await page.goto(url + query).catch(e => fail.push(`offline load ${query}: ${e.message}`));
  if (query === '') {
    await waitFor('the engine in the worker to answer offline', async () => (await replies()) > 0, 30000);
  }
  // The page's wasm ran: it removes the loading text once eframe has started (or failed to, which
  // logs an error, caught above).
  await waitFor(`the app to start offline ${query}`, () => page.evaluate(() => {
    const l = document.getElementById('loading');
    return !l || l.style.display === 'none' || !l.isConnected || getComputedStyle(l).display === 'none';
  }), 30000);
  await page.screenshot({ path: `out-offline/${query ? 'inline' : 'worker'}.png` });
}
for (const e of errors) fail.push(`page error: ${e}`);
await browser.close();
fs.rmSync(old, { recursive: true, force: true });
if (fail.length) { console.error('FAIL\n' + fail.join('\n')); process.exit(1); }
console.log('ok: opened offline from the cache, engine answering');
