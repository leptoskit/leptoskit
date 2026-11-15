// A real installable worker for the browser lifecycle suite: it installs
// and activates at once, the way the receiver does under a fresh
// registration. `wasm-bindgen-test-runner` serves files from the package
// root, so this script registers same-origin at the scope
// `/tests/fixtures/` with the browser's real install and activation
// machinery — not a mock.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) =>
	event.waitUntil(self.clients.claim()),
);
