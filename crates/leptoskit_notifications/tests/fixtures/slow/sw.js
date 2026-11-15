// A worker that takes its time: install waits 700 ms before
// `skipWaiting`, so an enrollment that races the install instead of
// waiting for activation can be told apart from one that waits.
self.addEventListener("install", (event) => {
	event.waitUntil(
		new Promise((resolve) => setTimeout(resolve, 700)).then(() => self.skipWaiting()),
	);
});
self.addEventListener("activate", (event) =>
	event.waitUntil(self.clients.claim()),
);
