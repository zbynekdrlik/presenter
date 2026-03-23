// Tablet PWA Service Worker - Network-first strategy for auto-updates
const CACHE_NAME = "tablet-pwa-v1";
const TABLET_URL = "/ui/tablet";

// Install - don't cache anything initially, let network-first handle it
self.addEventListener("install", (event) => {
  // Skip waiting to activate immediately
  self.skipWaiting();
});

// Activate - clean up old caches
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((cacheNames) => {
        return Promise.all(
          cacheNames
            .filter(
              (name) => name.startsWith("tablet-pwa-") && name !== CACHE_NAME,
            )
            .map((name) => caches.delete(name)),
        );
      })
      .then(() => {
        // Take control of all pages immediately
        return self.clients.claim();
      }),
  );
});

// Fetch - Network-first strategy for auto-updates
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);

  // Only handle same-origin requests
  if (url.origin !== self.location.origin) {
    return;
  }

  // Only cache tablet-related requests and WASM assets
  if (
    !url.pathname.startsWith("/ui/tablet") &&
    !url.pathname.startsWith("/ui-pkg/")
  ) {
    return;
  }

  event.respondWith(
    fetch(event.request)
      .then((response) => {
        // Clone the response before caching
        const responseToCache = response.clone();

        // Cache successful responses
        if (response.ok) {
          caches.open(CACHE_NAME).then((cache) => {
            cache.put(event.request, responseToCache);
          });
        }

        return response;
      })
      .catch(() => {
        // Network failed, try cache as fallback
        return caches.match(event.request);
      }),
  );
});

// Listen for messages to force update
self.addEventListener("message", (event) => {
  if (event.data === "skipWaiting") {
    self.skipWaiting();
  }
});
