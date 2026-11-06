// Offline support. Keeps the page and the bundle it names, so wirebench opens without a network
// once it has been opened with one.
//
// The bundle's files carry a content hash in their names (trunk does that), so the page is the
// only list of them: the same <link rel="modulepreload"> and <link rel="preload"> the engine
// worker reads. The page is fetched from the network first, so a new version is used as soon as
// it is published; the cache is refreshed from it, and only falls in when the network fails.

const CACHE = 'wirebench';
const EXTRAS = ['manifest.webmanifest', 'icon.svg', 'icon-192.png', 'icon-512.png'];

const scope = () => self.registration.scope;
const pageUrl = () => new URL('./', scope()).href;

// The files a page needs besides itself, as absolute URLs.
function bundleOf(html) {
  const urls = [];
  for (const [tag] of html.matchAll(/<link\b[^>]*>/g)) {
    if (!/\brel="(modulepreload|preload)"/.test(tag)) continue;
    const href = /\bhref="([^"]+)"/.exec(tag);
    if (href) urls.push(new URL(href[1], pageUrl()).href);
  }
  return urls;
}

// Make the cache hold `page` (a fresh network response for the page) and everything it names,
// and nothing else. The page goes in last, so the cached page never names a file that is not.
async function refresh(page) {
  const cache = await caches.open(CACHE);
  const html = await page.clone().text();
  const files = [...bundleOf(html), ...EXTRAS.map((f) => new URL(f, scope()).href)];
  for (const url of files) {
    if (!(await cache.match(url))) {
      const res = await fetch(url, { cache: 'no-cache' });
      if (!res.ok) throw new Error(`${url}: ${res.status}`);
      await cache.put(url, res);
    }
  }
  await cache.put(pageUrl(), page);
  const keep = new Set([pageUrl(), ...files]);
  for (const req of await cache.keys()) {
    if (!keep.has(req.url)) await cache.delete(req);
  }
}

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const page = await fetch(pageUrl(), { cache: 'no-cache' });
      if (!page.ok) throw new Error(`${pageUrl()}: ${page.status}`);
      await refresh(page);
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (name !== CACHE) await caches.delete(name);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET' || !req.url.startsWith(scope())) return;
  if (req.mode === 'navigate') {
    // Any page in scope (with any query, such as ?engine=inline) is the one page.
    event.respondWith(
      (async () => {
        try {
          const res = await fetch(req);
          const url = new URL(req.url);
          url.search = '';
          if (res.ok && (url.href === pageUrl() || url.href === pageUrl() + 'index.html')) {
            event.waitUntil(refresh(res.clone()).catch(() => {}));
          }
          return res;
        } catch (err) {
          const cached = await caches.match(pageUrl());
          if (cached) return cached;
          throw err;
        }
      })(),
    );
    return;
  }
  event.respondWith(
    (async () => (await caches.match(req, { ignoreSearch: true })) ?? fetch(req))(),
  );
});
