// Presenter guest translation service worker (#999): it exists so the page
// installs as an app. No fetch handler and no cache on purpose: every request
// goes to the network, so a deploy never leaves a stale player behind.
self.addEventListener("install", () => {
  self.skipWaiting();
});
