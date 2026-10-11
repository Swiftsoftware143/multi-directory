// ZaarHub Scanner Service Worker — app-shell caching for offline use.
//
// IMPORTANT: the app page ('/scanner') is served NETWORK-FIRST. A café phone that has opened the
// scanner once must always get the CURRENT page; the saved copy is only a fallback for when the
// device is offline. (This worker used to answer the page from the saved copy FIRST under a cache
// name that never changed, so returning staff kept seeing an old scanner forever — every shipped
// scanner fix was invisible to them.) Bumping CACHE below also purges the stale store on activate.
const CACHE = 'zaarhub-scanner-v3';
const ASSETS = [
  '/scanner',
  '/scanner-manifest.json',
  '/vendor/jsqr.js'
];

self.addEventListener('install', function(e) {
  e.waitUntil(
    caches.open(CACHE).then(function(c) {
      return c.addAll(ASSETS);
    })
  );
  self.skipWaiting();
});

self.addEventListener('activate', function(e) {
  e.waitUntil(
    caches.keys().then(function(keys) {
      return Promise.all(
        keys.filter(function(k) { return k !== CACHE; }).map(function(k) { return caches.delete(k); })
      );
    })
  );
  self.clients.claim();
});

self.addEventListener('fetch', function(e) {
  var req = e.request;
  // Never touch API traffic, and only ever handle plain page/asset loads.
  if (req.url.indexOf('/api/') !== -1) return;
  if (req.method !== 'GET') return;

  // The scanner page itself: try the network first so the newest page always wins; fall back to
  // the saved copy only when the device is offline.
  var isPage = req.mode === 'navigate' || req.destination === 'document' ||
               /\/scanner(\?|$|\/)/.test(req.url);
  if (isPage) {
    e.respondWith(
      fetch(req).then(function(res) {
        var copy = res.clone();
        caches.open(CACHE).then(function(c) { c.put(req, copy); }).catch(function() {});
        return res;
      }).catch(function() {
        return caches.match(req).then(function(r) { return r || caches.match('/scanner'); });
      })
    );
    return;
  }

  // Static assets (the QR library, icons, manifest): serve from the saved copy first.
  e.respondWith(
    caches.match(req).then(function(r) { return r || fetch(req); })
  );
});
