// Shared helpers: launch Chromium with CPU throttling and drive the egui canvas by pixel.
import { chromium } from 'playwright';

export async function launch({ rate = 4, url = 'https://faletto.com/wirebench/' } = {}) {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || undefined, // set CHROMIUM if Playwright's own download is missing
    args: ['--headless=new', '--force-device-scale-factor=1', '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'],
  });
  // viewport: null + setViewportSize keeps the canvas buffer at 1x; a fixed-viewport context doubled it and skewed pointer coordinates.
  const context = await browser.newContext({ viewport: null, acceptDownloads: true });
  const page = await context.newPage();
  await page.setViewportSize({ width: 1366, height: 768 });
  page.on('pageerror', e => console.log('pageerror:', e.message));
  page.on('console', m => { if (m.type() === 'error') console.log('console error:', m.text().slice(0, 600)); });
  const cdp = await context.newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate });
  return { browser, context, page, cdp, url };
}

// Toolbar buttons, measured from a 1366x768 screenshot; the topology strip is y 50-253.
export const TOOL = { select: [33, 37], pc: [87, 37], sw: [149, 37], router: [222, 37], cable: [288, 37], del: [348, 37] };

// egui only registers a click on a widget it saw hovered, so give move, press and release a frame each.
export async function tap(page, x, y) {
  await page.mouse.move(x, y); await page.waitForTimeout(50);
  await page.mouse.down(); await page.waitForTimeout(40);
  await page.mouse.up();
}
export const click = (page, [x, y]) => tap(page, x, y);

// Counts requestAnimationFrame callbacks and their gaps for `ms`, to estimate frame pacing.
// Uses performance.now() rather than the callback's timestamp, which is the frame start time.
export async function frameStats(page, ms) {
  const gaps = await page.evaluate(ms => new Promise(res => {
    const gaps = []; let last = performance.now(); const end = last + ms;
    const tick = () => { const now = performance.now(); gaps.push(now - last); last = now; now < end ? requestAnimationFrame(tick) : res(gaps); };
    requestAnimationFrame(tick);
  }), ms);
  gaps.sort((a, b) => a - b);
  const q = p => gaps[Math.min(gaps.length - 1, Math.floor(p * gaps.length))];
  return { frames: gaps.length, p50: +q(0.5).toFixed(1), p99: +q(0.99).toFixed(1), max: +gaps[gaps.length - 1].toFixed(1) };
}
