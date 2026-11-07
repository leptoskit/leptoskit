// This module runs only in the opaque-origin preview. No parent DOM access.
let started = false;
const heartbeatMilliseconds = 500;
addEventListener("message", async (event) => {
	if (event.source !== parent || started || event.data?.type !== "bundle") {
		return;
	}

	started = true;
	const { js: javascript, wasm } = event.data;
	const bytes = Uint8Array.from(atob(wasm), (c) => c.charCodeAt(0));
	const url = URL.createObjectURL(
		new Blob([javascript], { type: "text/javascript" }),
	);
	try {
		const module = await import(url);
		await module.default({ module_or_path: bytes });
		parent.postMessage({ type: "mounted" }, "*");
	} catch (error) {
		parent.postMessage({ type: "runtime-error", message: String(error) }, "*");
	} finally {
		URL.revokeObjectURL(url);
	}
});
setInterval(
	() => parent.postMessage({ type: "heartbeat" }, "*"),
	heartbeatMilliseconds,
);
parent.postMessage({ type: "ready" }, "*");
