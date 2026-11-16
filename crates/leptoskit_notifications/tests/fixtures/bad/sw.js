// A worker that never installs: its install step rejects, so the browser
// marks the worker redundant and nothing ever activates. An enrollment
// that waits for this worker's activation must say so and stop, never
// hang. (A listener that merely throws is only reported — the rejection
// has to ride `waitUntil` to fail the install.)
self.addEventListener("install", (event) => {
	event.waitUntil(Promise.reject(new Error("this worker never installs")));
});
