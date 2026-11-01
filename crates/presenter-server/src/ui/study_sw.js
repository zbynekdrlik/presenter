// Presenter Štúdium service worker (#851): it exists so the page installs as an
// app. No fetch handler and no cache on purpose: every request goes straight to
// the network (no offline mode, spec Q3), so a deploy can never leave a stale
// reader behind.
self.addEventListener("install", () => {
  self.skipWaiting();
});
