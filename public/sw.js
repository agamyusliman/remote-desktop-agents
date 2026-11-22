/* remot-agent service worker: cache app shell; never cache API/WS/screen. */
const VERSION = "ra-v10";
const SHELL = [
  "/",
  "/style.css",
  "/app.js",
  "/vendor/xterm.js",
  "/vendor/xterm.css",
  "/vendor/addon-fit.js",
  "/vendor/marked.umd.js",
  "/vendor/purify.min.js",
  "/vendor/highlight.min.js",
  "/vendor/highlight.css",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
  "/manifest.webmanifest",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(VERSION).then((cache) => cache.addAll(SHELL)).then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((key) => key !== VERSION).map((key) => caches.delete(key)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") self.skipWaiting();
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET") return;
  if (url.origin !== self.location.origin) return;

  // Never cache API, auth, websockets, or stream endpoints.
  if (
    url.pathname.startsWith("/api/") ||
    url.pathname.startsWith("/ws/") ||
    url.pathname.startsWith("/auth") ||
    url.pathname === "/sw.js" ||
    url.pathname === "/manifest.webmanifest"
  ) {
    return;
  }

  // Network-first for navigation so a redeploy is picked up; fall back to shell offline.
  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request)
        .then((response) => {
          const copy = response.clone();
          caches.open(VERSION).then((cache) => cache.put("/", copy)).catch(() => {});
          return response;
        })
        .catch(() => caches.match("/"))
    );
    return;
  }

  // Network-first for the app shell so layout fixes land immediately.
  const isShell = /\/(app\.js|style\.css|manifest\.webmanifest)$/.test(url.pathname);
  if (isShell) {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (response.ok) {
            const copy = response.clone();
            caches.open(VERSION).then((cache) => cache.put(request, copy)).catch(() => {});
          }
          return response;
        })
        .catch(() => caches.match(request))
    );
    return;
  }

  // Stale-while-revalidate for the rest (vendor/icons).
  event.respondWith(
    caches.match(request).then((cached) => {
      const network = fetch(request)
        .then((response) => {
          if (response.ok) {
            const copy = response.clone();
            caches.open(VERSION).then((cache) => cache.put(request, copy)).catch(() => {});
          }
          return response;
        })
        .catch(() => cached);
      return cached || network;
    })
  );
});
