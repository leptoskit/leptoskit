// What the visitor-page browser proofs share: the real demo service on
// ephemeral ports, a scratch volume, headless Chrome over the DevTools
// protocol (through the repository's own `browser.mjs`), and a few helpers
// that drive it the way a visitor does.
//
// The service needs three authorities and two cookie hosts. The site is
// `localhost:<site>`, the private bind is `127.0.0.1:<site>`, and the admin
// is `127.0.0.1:<admin>`: the identity provider accepts plain-HTTP redirects
// only to `localhost`, `127.0.0.1`, and `[::1]`, so `admin.localhost` is
// refused. The admin's port is a byte forwarder to the service's one
// listener, which keeps `Host` intact, so the service sees a request for the
// admin's authority. The browser keeps `localhost` and `127.0.0.1` cookies
// apart, as it would two real hosts.

import {
	assert,
	launch,
	Page,
	removeProfile,
} from "../../../crates/leptoskit_app/tests/browser.mjs";

export { assert };

/** A free loopback port: bind to 0, read the port, release it. */
export function freePort() {
	const listener = Deno.listen({ hostname: "127.0.0.1", port: 0 });
	const { port } = listener.addr;
	listener.close();
	return port;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The service binary: `DEMO_SERVICE_BIN`, else the debug build. */
function serviceBinary() {
	const configured = Deno.env.get("DEMO_SERVICE_BIN");
	if (configured && !Deno.env.get("LEPTOS_SITE_ROOT")) {
		throw new Error(
			"LEPTOS_SITE_ROOT must name the pages' built client: run `leptoskit build " +
				"--release --platform web` in examples/demo_service/pages and name its output",
		);
	}
	if (configured) return configured;
	throw new Error(
		"DEMO_SERVICE_BIN must name a debug build of demo_service: the debug clock " +
			"and failure hooks the proofs use are compiled out of release builds",
	);
}

/**
 * A running demo service on a scratch volume, with the admin's forwarder.
 * `restart` replaces the process on the same volume and ports, as a
 * same-day release does.
 */
export class Service {
	constructor(volume, sitePort, adminPort, extra) {
		this.volume = volume;
		this.sitePort = sitePort;
		this.adminPort = adminPort;
		this.extra = extra;
		this.siteOrigin = `http://localhost:${sitePort}`;
		this.adminOrigin = `http://127.0.0.1:${adminPort}`;
		this.child = null;
		this.forwarder = null;
	}

	static async start(extra = {}) {
		const volume = await Deno.makeTempDir({ prefix: "demo-volume-" });
		await Deno.chmod(volume, 0o700);
		const service = new Service(volume, freePort(), freePort(), extra);
		service.#forward();
		await service.spawn();
		return service;
	}

	/** Forwards the admin's port to the service, bytes untouched. */
	#forward() {
		const listener = Deno.listen({
			hostname: "127.0.0.1",
			port: this.adminPort,
		});
		this.forwarder = listener;
		(async () => {
			try {
				for await (const connection of listener) {
					Deno.connect({ hostname: "127.0.0.1", port: this.sitePort }).then(
						(upstream) => {
							connection.readable.pipeTo(upstream.writable).catch(() => {});
							upstream.readable.pipeTo(connection.writable).catch(() => {});
						},
						() => connection.close(),
					);
				}
			} catch {
				// The listener closed.
			}
		})();
	}

	/** Starts the process. Resolves when it answers `/healthz`. */
	async spawn(extra = this.extra) {
		this.extra = extra;
		this.child = new Deno.Command(serviceBinary(), {
			env: {
				DEMO_BIND: `127.0.0.1:${this.sitePort}`,
				LEPTOSKIT_SERVICE_VOLUME_STATE: this.volume,
				DEMO_SITE_ORIGIN: this.siteOrigin,
				DEMO_ADMIN_ORIGIN: this.adminOrigin,
				...extra,
			},
			stdin: "null",
			stdout: "null",
			stderr: "null",
		}).spawn();
		await this.until("/healthz", (status) => status === 200, 30_000);
	}

	/** Polls a path on the private bind until `accept(status)`. */
	async until(path, accept, timeout = 60_000) {
		const deadline = Date.now() + timeout;
		for (;;) {
			try {
				const response = await fetch(
					`http://127.0.0.1:${this.sitePort}${path}`,
				);
				await response.body?.cancel();
				if (accept(response.status)) return;
			} catch {
				// Not listening yet.
			}
			if (Date.now() > deadline) {
				throw new Error(`timed out waiting for ${path}`);
			}
			await sleep(100);
		}
	}

	ready(timeout) {
		return this.until("/readyz", (status) => status === 200, timeout);
	}

	/** Stops the process the way a supervisor does. */
	async stop() {
		if (!this.child) return;
		try {
			this.child.kill("SIGTERM");
			await this.child.status;
		} catch {
			// Already gone.
		}
		this.child = null;
	}

	async restart(extra = this.extra) {
		await this.stop();
		await this.spawn(extra);
		await this.ready();
	}

	async close() {
		await this.stop();
		try {
			this.forwarder?.close();
		} catch {
			// Already closed.
		}
		await Deno.remove(this.volume, { recursive: true }).catch(() => {});
	}
}

/** Headless Chrome, one profile, one page. */
export class Browser {
	static async open() {
		const profile = await Deno.makeTempDir({ prefix: "demo-chrome-" });
		const { process, endpoint } = await launch(profile);
		const page = await Page.connect(endpoint);
		const browser = new Browser(profile, process, page);
		await page.send("Page.enable");
		await page.send("Runtime.enable");
		await page.send("Network.enable");
		page.on("Network.requestWillBeSent", ({ request }) => {
			browser.requests.push(request.url);
		});
		page.on("Runtime.exceptionThrown", ({ exceptionDetails }) => {
			browser.errors.push(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		});
		page.on("Runtime.consoleAPICalled", ({ type, args }) => {
			if (type === "error") {
				browser.errors.push(
					args.map((a) => a.value ?? a.description).join(" "),
				);
			}
		});
		return browser;
	}

	constructor(profile, process, page) {
		this.profile = profile;
		this.process = process;
		this.page = page;
		/** Every URL the page asked for. */
		this.requests = [];
		/** Script errors and console errors. */
		this.errors = [];
	}

	/** Emulates the viewport and the visitor's preferences. */
	async look({ width, height = 900, dark = false, reducedMotion = false }) {
		await this.page.send("Emulation.setDeviceMetricsOverride", {
			width,
			height,
			deviceScaleFactor: 1,
			mobile: width < 500,
		});
		await this.page.send("Emulation.setEmulatedMedia", {
			features: [
				{ name: "prefers-color-scheme", value: dark ? "dark" : "light" },
				{
					name: "prefers-reduced-motion",
					value: reducedMotion ? "reduce" : "no-preference",
				},
			],
		});
	}

	/**
	 * Loads a page. A page that brings the Wasm client is not ready until the
	 * client has attached to it, which the client marks on the root element;
	 * `scripts: false` says the browser will not run it.
	 */
	async go(url, { scripts = true } = {}) {
		const loaded = this.page.once("Page.loadEventFired");
		await this.page.send("Page.navigate", { url });
		await loaded;
		if (!scripts) return;
		const deadline = Date.now() + 30_000;
		for (;;) {
			const attached = await this.page.evaluate(
				`!document.querySelector('link[rel="modulepreload"]') || document.documentElement.hasAttribute("data-hydrated")`,
			).catch(() => false);
			if (attached) return;
			if (Date.now() > deadline) {
				throw new Error(`the client never attached to ${url}`);
			}
			await sleep(50);
		}
	}

	/** Waits for the next navigation to finish. */
	async settle(timeout = 15_000) {
		await Promise.race([
			this.page.once("Page.loadEventFired"),
			sleep(timeout),
		]);
	}

	url() {
		return this.page.evaluate("location.href");
	}

	eval(expression) {
		return this.page.evaluate(expression);
	}

	until(predicate, description, timeout) {
		return this.page.until(predicate, description, timeout);
	}

	/** The element whose `data-testid` is `id`. */
	sel(id) {
		return `[data-testid="${id}"]`;
	}

	click(id) {
		return this.page.click(this.sel(id));
	}

	/** Whether an element is visible to a reader. */
	visible(id) {
		return this.eval(
			`(() => { const e = document.querySelector('${
				this.sel(id)
			}'); return !!e && e.checkVisibility(); })()`,
		);
	}

	text(id) {
		return this.eval(
			`document.querySelector('${this.sel(id)}')?.textContent?.trim() ?? null`,
		);
	}

	/** The testid of the focused element, or its tag. */
	focused() {
		return this.eval(
			`document.activeElement?.dataset?.testid ?? document.activeElement?.id ?? document.activeElement?.tagName`,
		);
	}

	/** Presses one key on the focused element, as a keyboard does. */
	async press(key) {
		const named = {
			Enter: { code: "Enter", vk: 13, text: "\r" },
			Tab: { code: "Tab", vk: 9 },
			Escape: { code: "Escape", vk: 27 },
			ArrowDown: { code: "ArrowDown", vk: 40 },
			ArrowUp: { code: "ArrowUp", vk: 38 },
			ArrowLeft: { code: "ArrowLeft", vk: 37 },
			ArrowRight: { code: "ArrowRight", vk: 39 },
			Home: { code: "Home", vk: 36 },
			End: { code: "End", vk: 35 },
			" ": { code: "Space", vk: 32, text: " " },
		}[key];
		const base = {
			key,
			code: named.code,
			windowsVirtualKeyCode: named.vk,
			nativeVirtualKeyCode: named.vk,
		};
		await this.page.send("Input.dispatchKeyEvent", {
			type: named.text ? "keyDown" : "rawKeyDown",
			...base,
			text: named.text,
		});
		await this.page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
	}

	/** Presses a key with modifiers held (Alt 1, Ctrl 2, Meta 4, Shift 8). */
	async chord(key, modifiers) {
		const named = { Enter: { code: "Enter", vk: 13, text: "\r" } }[key];
		const base = {
			key,
			code: named.code,
			windowsVirtualKeyCode: named.vk,
			nativeVirtualKeyCode: named.vk,
			modifiers,
		};
		await this.page.send("Input.dispatchKeyEvent", {
			type: "rawKeyDown",
			...base,
		});
		await this.page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
	}

	async shiftTab() {
		const base = {
			key: "Tab",
			code: "Tab",
			windowsVirtualKeyCode: 9,
			modifiers: 8,
		};
		await this.page.send("Input.dispatchKeyEvent", {
			type: "rawKeyDown",
			...base,
		});
		await this.page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
	}

	/** Types literal text into the focused field. */
	type(text) {
		return this.page.send("Input.insertText", { text });
	}

	/** Tabs until the focused element is `id`, at most `limit` times. */
	async tabTo(id, limit = 40) {
		for (let step = 0; step < limit; step++) {
			if ((await this.focused()) === id) return step;
			await this.press("Tab");
		}
		throw new Error(
			`never reached ${id} by Tab; focus is on ${await this.focused()}`,
		);
	}

	/** No page may scroll sideways. */
	overflowX() {
		return this.eval(
			"Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth",
		);
	}

	async shot(directory, name) {
		if (!directory) return null;
		await Deno.mkdir(directory, { recursive: true });
		const { data } = await this.page.send("Page.captureScreenshot", {
			format: "png",
			captureBeyondViewport: false,
		});
		const path = `${directory}/${name}.png`;
		await Deno.writeFile(
			path,
			Uint8Array.from(atob(data), (c) => c.charCodeAt(0)),
		);
		return path;
	}

	async clearCookies() {
		await this.page.send("Network.clearBrowserCookies");
	}

	async close() {
		try {
			await this.page.close();
		} catch {
			// Already closed.
		}
		try {
			this.process.kill();
			await this.process.status;
		} catch {
			// Already gone.
		}
		await removeProfile(this.profile);
	}
}

/** The code in the newest message of the demo inbox, read from the page. */
export function inboxCode(browser) {
	return browser.eval(
		`document.querySelector('[data-action="fill"]')?.dataset.code ?? null`,
	);
}
