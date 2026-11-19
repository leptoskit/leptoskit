// The admin's pages in a real browser, against pages the admin really
// renders: `pages_for_the_script` in `tests/analytics.rs` writes them, this
// serves them with `assets/admin.mjs`, the pages' Wasm client, and a live
// stream the test feeds, and headless Chrome runs them over the DevTools
// protocol.
//
//   deno test -A crates/leptoskit_admin/tests/script.mjs
//
// It runs that Rust test itself unless `LEPTOSKIT_ADMIN_PAGES` already
// names a directory of pages, and builds the client with `leptoskit build`
// unless `LEPTOSKIT_ADMIN_CLIENT` names a build's output directory.
// `CHROME` names the browser executable.

import {
	assert,
	launch,
	Page,
	removeProfile,
} from "../../leptoskit_app/tests/browser.mjs";

const BASE = "/_leptoskit/admin";
const here = new URL(".", import.meta.url).pathname;

let written = null;

/** The pages' directory, written once for every test in the run. */
function pages() {
	const given = Deno.env.get("LEPTOSKIT_ADMIN_PAGES");
	if (given) return Promise.resolve(given);
	written ??= writePages();
	return written;
}

async function writePages() {
	const directory = await Deno.makeTempDir({ prefix: "admin-pages-" });
	globalThis.addEventListener("unload", () => {
		Deno.removeSync(directory, { recursive: true });
	});
	const { success, stderr } = await new Deno.Command("cargo", {
		args: [
			"test",
			"--package",
			"leptoskit_admin",
			"--test",
			"analytics",
			"--",
			"--ignored",
			"--exact",
			"pages_for_the_script",
		],
		env: { LEPTOSKIT_ADMIN_PAGES: directory },
		stdout: "null",
		stderr: "piped",
	}).output();
	assert(
		success,
		`the pages weren't written:\n${new TextDecoder().decode(stderr)}`,
	);
	return directory;
}

let built = null;

/** The pages' Wasm client: a build's output directory, built once. */
function client() {
	const given = Deno.env.get("LEPTOSKIT_ADMIN_CLIENT");
	if (given) return Promise.resolve(given);
	built ??= buildClient();
	return built;
}

async function buildClient() {
	const directory = await Deno.makeTempDir({ prefix: "admin-client-" });
	globalThis.addEventListener("unload", () => {
		Deno.removeSync(directory, { recursive: true });
	});
	const { success, stderr } = await new Deno.Command("cargo", {
		args: [
			"run",
			"--quiet",
			"--package",
			"leptoskit_cli",
			"--bin",
			"leptoskit",
			"--",
			"build",
			"--platform",
			"web",
			"--out",
			directory,
		],
		cwd: `${here}../../leptoskit_admin_pages`,
		stdout: "null",
		stderr: "piped",
	}).output();
	assert(
		success,
		`the pages' client wasn't built:\n${new TextDecoder().decode(stderr)}`,
	);
	return directory;
}

const TYPES = {
	js: "text/javascript",
	wasm: "application/wasm",
};

/** Serves the pages as the admin would, and a live stream to push into. */
function serve(directory, clientDirectory) {
	const streams = new Set();
	const posted = [];
	const encoder = new TextEncoder();
	const page = (name) =>
		Deno.readFile(`${directory}/${name}`).then((body) =>
			new Response(body, { headers: { "content-type": "text/html" } })
		);
	const server = Deno.serve(
		{ hostname: "127.0.0.1", port: 0, onListen() {} },
		async (request) => {
			const url = new URL(request.url);
			switch (url.pathname) {
				case `${BASE}/assets/admin.mjs`:
					return new Response(
						await Deno.readFile(`${here}../assets/admin.mjs`),
						{
							headers: { "content-type": "text/javascript" },
						},
					);
				case `${BASE}/analytics/trends`: {
					const name = url.searchParams.get("by") === "session:channel"
						? "trends-channels"
						: url.searchParams.get("s0") === "$pageview"
						? "trends-pageviews"
						: url.searchParams.get("interval") === "week"
						? "trends-week"
						: "trends-day";
					// A builder changing in place asks for the page's model.
					if (request.headers.get("x-admin-page") === "json") {
						return new Response(
							await Deno.readFile(`${directory}/${name}.json`),
							{ headers: { "content-type": "application/json" } },
						);
					}
					return page(
						url.searchParams.get("workings") === "trends"
							? `${name}-workings.html`
							: `${name}.html`,
					);
				}
				case `${BASE}/analytics/live`:
					return page("live.html");
				case `${BASE}/analytics/library`:
					return page("library.html");
				case `${BASE}/analytics/people`:
					return page("people.html");
				case `${BASE}/learn/glossary`:
					return page("glossary.html");
				case `${BASE}/learn`:
					return page("learn.html");
				case `${BASE}/infra/projects`:
					return page("projects.html");
				case `${BASE}/sign-in`:
					return new Response("<!doctype html><title>Sign in</title>", {
						headers: { "content-type": "text/html" },
					});
				case "/auth/sign-out":
					posted.push({ path: url.pathname, body: await request.text() });
					return Response.json({});
				case `${BASE}/infra/projects/notes-production/overview`:
					return page(
						url.searchParams.get("workings") === "release"
							? "overview-release.html"
							: "overview.html",
					);
				case `${BASE}/infra/projects/notes-production/releases`:
					return page("releases.html");
				case `${BASE}/api/projects/notes-production/actions`:
					posted.push({ path: url.pathname, body: await request.json() });
					return Response.json(
						{ operation: "plan", href: `${BASE}/infra/operations/plan` },
						{ status: 202 },
					);
				case `${BASE}/infra/projects/notes-production/changes`:
					return page("changes.html");
				case `${BASE}/api/analytics/live`: {
					let held;
					const body = new ReadableStream({
						start(controller) {
							held = controller;
							streams.add(controller);
						},
						cancel() {
							streams.delete(held);
						},
					});
					return new Response(body, {
						headers: { "content-type": "text/event-stream" },
					});
				}
				case `${BASE}/api/analytics/dashboards`:
					if (request.method === "GET") {
						return Response.json({
							dashboards: [{ id: "d1", name: "Growth", cards: 2 }],
						});
					}
					posted.push({ path: url.pathname, body: await request.json() });
					return Response.json({ href: `${BASE}/analytics/library` });
				case `${BASE}/api/analytics/insights`:
					posted.push({ path: url.pathname, body: await request.json() });
					return Response.json({ href: `${BASE}/analytics/library` });
				case `${BASE}/api/preferences`:
					posted.push({ path: url.pathname, body: await request.json() });
					return Response.json({});
				default: {
					if (url.pathname.startsWith(`${BASE}/api/analytics/people/`)) {
						if (url.pathname.endsWith("/export")) {
							posted.push({ path: url.pathname, body: null });
							return new Response('{"person":{}}', {
								headers: {
									"content-type": "application/json",
									"content-disposition": 'attachment; filename="person.json"',
								},
							});
						}
						posted.push({ path: url.pathname, body: await request.json() });
						return Response.json({
							events: 1,
							sessions: 1,
							href: `${BASE}/analytics/people`,
						});
					}
					if (url.pathname.startsWith(`${BASE}/analytics/people/`)) {
						return page("person.html");
					}
					if (url.pathname.startsWith(`${BASE}/infra/operations/`)) {
						return page(
							url.searchParams.get("workings") === "operation"
								? "operation-workings.html"
								: "operation.html",
						);
					}
					const file = url.pathname.startsWith(`${BASE}/pkg/`)
						? url.pathname.slice(`${BASE}/pkg/`.length)
						: null;
					if (file && /^[\w.-]+$/.test(file) && TYPES[file.split(".").pop()]) {
						return new Response(
							await Deno.readFile(`${clientDirectory}/pkg/${file}`),
							{ headers: { "content-type": TYPES[file.split(".").pop()] } },
						);
					}
					return new Response("Not found", { status: 404 });
				}
			}
		},
	);
	return {
		origin: `http://127.0.0.1:${server.addr.port}`,
		connected: () => streams.size,
		posted,
		push(kind, data) {
			const message = encoder.encode(
				`event: ${kind}\ndata: ${JSON.stringify(data)}\n\n`,
			);
			for (const stream of streams) stream.enqueue(message);
		},
		async close() {
			for (const stream of streams) stream.close();
			await server.shutdown();
		},
	};
}

/** One browser for every test, opened on demand. */
async function withBrowser(run) {
	const [directory, clientDirectory] = await Promise.all([pages(), client()]);
	const site = serve(directory, clientDirectory);
	const profile = await Deno.makeTempDir({ prefix: "admin-chrome-" });
	const chrome = await launch(profile);
	const page = await Page.connect(chrome.endpoint);
	try {
		await page.send("Runtime.enable");
		await run(page, site);
	} finally {
		await page.close();
		chrome.process.kill();
		await chrome.process.status;
		await site.close();
		await removeProfile(profile);
	}
}

async function open(page, url, { width = 1280, height = 900 } = {}) {
	await page.send("Emulation.setDeviceMetricsOverride", {
		width,
		height,
		deviceScaleFactor: 1,
		mobile: width < 640,
	});
	const loaded = page.once("Page.loadEventFired");
	await page.send("Page.enable");
	await page.send("Page.navigate", { url });
	await loaded;
	// The module script runs after the document parses.
	await page.until("() => document.readyState === 'complete'", "the page");
}

/** Waits for the pages' Wasm to pick up `what` the server drew. */
function hydrated(page, what) {
	return page.until(
		"() => document.documentElement.hasAttribute('data-hydrated')",
		`${what} hydrates`,
	);
}

const KEYS = {
	k: 75,
	Tab: 9,
	ArrowRight: 39,
	ArrowLeft: 37,
	Home: 36,
	End: 35,
	Escape: 27,
	Enter: 13,
	w: 87,
};

async function press(page, key, { shift = false, ctrl = false } = {}) {
	const common = {
		key,
		modifiers: (shift ? 8 : 0) | (ctrl ? 2 : 0),
		code: key.length === 1 ? `Key${key.toUpperCase()}` : key,
		windowsVirtualKeyCode: KEYS[key],
	};
	await page.send("Input.dispatchKeyEvent", {
		type: "keyDown",
		...common,
		...(key === "Enter"
			? { text: "\r" }
			: key === "w" && !ctrl
			? { text: shift ? "W" : "w" }
			: {}),
	});
	await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...common });
}

const focused = (page) =>
	page.evaluate(
		"document.activeElement?.getAttribute('data-testid') ?? document.activeElement?.tagName",
	);

Deno.test("a breakdown has the same colours after a builder change as after a reload", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?s0=%24pageview&interval=day`,
		);
		await hydrated(page, "the builder");
		await page.evaluate("window.beforeSwap = true");
		await page.click(
			'[data-testid="menu-toggle"][aria-description="breakdown"]',
		);
		await page.until(
			`() => document.querySelector('[data-testid="menu-panel"]:popover-open [data-testid="menu-item"][href*="by=session%3Achannel"]')`,
			"the breakdown menu opens",
		);
		await page.click('[data-testid="menu-item"][href*="by=session%3Achannel"]');
		await page.until(
			"() => new URLSearchParams(location.search).get('by') === 'session:channel'",
			"the channel breakdown arrives",
		);
		assert(
			await page.evaluate("window.beforeSwap === true"),
			"the builder swapped without a reload",
		);
		const colours = () =>
			page.evaluate(`(() => {
			const chart = document.querySelector('[data-testid="trend-chart"]');
			return {
				lines: Array.from(chart.querySelectorAll('svg')).map(line => getComputedStyle(line).color),
				swatches: Array.from(chart.firstElementChild.children).map(row => getComputedStyle(row.firstElementChild).backgroundColor),
			};
		})()`);
		const swapped = await colours();
		await open(page, await page.evaluate("location.href"));
		const reloaded = await colours();
		assert(
			new Set(reloaded.swatches).size >= 3,
			`the fixture has distinct channels: ${JSON.stringify(reloaded)}`,
		);
		assert(
			JSON.stringify(swapped) === JSON.stringify(reloaded),
			`colours survive a swap: ${JSON.stringify({ swapped, reloaded })}`,
		);
		assert(
			!swapped.swatches.includes("rgba(0, 0, 0, 0)"),
			"every channel has a visible swatch",
		);
	});
});

Deno.test("confirmation dialogs contain focus and restore their opener without closing Workings", async () => {
	await withBrowser(async (page, site) => {
		const sheetOpen = () =>
			page.evaluate(
				`document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal') === true`,
			);
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/overview?workings=release`,
		);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the overview hydrates",
		);
		await page.evaluate(`(() => {
			const held = document.createElement('button');
			held.id = 'already-inert';
			held.inert = true;
			document.body.append(held);
			document.querySelector('[data-testid="action:status"]').focus();
		})()`);
		await press(page, "Enter");
		await page.until(
			`() => document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal')`,
			"the health dialog",
		);
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "confirm-yes"`,
			"the confirmation receives focus",
		);
		await press(page, "Tab", { shift: true });
		assert(
			await focused(page) === "confirm-no",
			`Shift+Tab stays inside the dialog, not ${await focused(page)}`,
		);
		await press(page, "Tab");
		assert(
			await focused(page) === "confirm-yes",
			"Tab moves back to the confirmation",
		);
		assert(
			await page.evaluate(
				`document.querySelector('[data-testid="drawer:release"]').getClientRects().length > 0`,
			),
			"the drawer stays open behind the dialog",
		);
		await press(page, "Escape");
		try {
			await page.until(
				`() => document.activeElement?.getAttribute("data-testid") === "action:status"`,
				"Escape restores the exact opener",
			);
		} catch {
			throw new Error(
				`Escape restores the exact opener: focus is on ${await focused(
					page,
				)}, the sheet is ${(await sheetOpen()) ? "open" : "closed"}`,
			);
		}
		assert(!(await sheetOpen()), "Escape closes the dialog");
		assert(
			await page.evaluate(
				`document.querySelector('[data-testid="drawer:release"]').getClientRects().length > 0`,
			),
			"Escape dismisses only the dialog",
		);
		assert(
			await page.evaluate("document.getElementById('already-inert').inert"),
			"closing leaves other inert states alone",
		);
		await press(page, "Enter");
		await page.until(
			`() => document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal')`,
			"the reopened dialog",
		);
		await page.click('[data-testid="confirm-no"]');
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "action:status"`,
			"Cancel restores the opener too",
		);
		assert(!(await sheetOpen()), "Cancel closes the dialog");

		await open(page, `${site.origin}${BASE}/analytics/trends`);
		await hydrated(page, "the builder");
		// Pointer activation need not focus the opener (notably in Safari).
		await page.evaluate(
			"document.querySelector('[data-testid=\"save-insight\"]').click()",
		);
		await page.until(
			"() => document.querySelector('[data-testid=\"confirm-input\"]')",
			"the save dialog",
		);
		assert(
			await focused(page) === "confirm-input",
			"a text dialog starts in its input",
		);
		await press(page, "Tab", { shift: true });
		assert(
			await focused(page) === "confirm-yes",
			`Shift+Tab wraps from the first input, not ${await focused(page)}`,
		);
		await press(page, "Tab");
		assert(await focused(page) === "confirm-input", "Tab wraps to the input");
		await press(page, "Escape");
		try {
			await page.until(
				`() => document.activeElement?.getAttribute("data-testid") === "save-insight"`,
				"pointer activation also restores the opener",
			);
		} catch {
			throw new Error(
				`pointer activation also restores the opener: focus is on ${await focused(
					page,
				)}, the sheet is ${(await sheetOpen()) ? "open" : "closed"}`,
			);
		}
	});
});

Deno.test("a live event's struct badge follows the event, not the row it copies", async () => {
	await withBrowser(async (page, site) => {
		await open(page, `${site.origin}${BASE}/analytics/live`);
		const badge = (row) =>
			page.evaluate(`(() => {
				const row = document.querySelectorAll('[data-testid="live-event"]')[${row}];
				const badge = row.querySelector('[data-testid="struct-badge"]');
				return {
					name: row.getAttribute("aria-description"),
					shown: getComputedStyle(badge).display !== "none",
				};
			})()`);

		// The page starts with the untyped pageview on top.
		assert(
			JSON.stringify(await badge(0)) ===
				JSON.stringify({ name: "$pageview", shown: false }),
			"the newest row is the untyped pageview",
		);
		for (let tries = 0; !site.connected() && tries < 100; tries++) {
			await new Promise((resolve) => setTimeout(resolve, 50));
		}
		assert(site.connected(), "the page follows the live stream");

		const event = (seq, name, typed) => ({
			seq,
			name,
			distinct_id: "anon-live",
			timestamp: new Date().toISOString(),
			properties: { $pathname: "/notes" },
			typed,
		});
		site.push("event", event(10, "note_created", true));
		await page.until(
			`() => document.querySelector('[data-testid="live-event"]').getAttribute("aria-description") === "note_created"`,
			"the typed event arrives",
		);
		const typed = await badge(0);
		assert(typed.shown, "a typed event after an untyped one shows `struct`");

		site.push("event", event(11, "$pageview", false));
		await page.until(
			`() => document.querySelector('[data-testid="live-event"]').getAttribute("aria-description") === "$pageview"`,
			"the untyped event arrives",
		);
		const untyped = await badge(0);
		assert(!untyped.shown, "an untyped event after a typed one hides it");
		assert((await badge(1)).shown, "the typed row keeps its badge");
	});
});

Deno.test("the Ask tabs are tabs: selection, arrow keys, Home, and End", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?interval=week&workings=trends`,
		);
		await hydrated(page, "the builder");
		const state = () =>
			page.evaluate(`(() => {
				const drawer = document.querySelector('[data-testid="drawer:trends"]');
				const tabs = Array.from(drawer.querySelectorAll('[role="tab"]'));
				return {
					selected: tabs.filter((tab) => tab.getAttribute("aria-selected") === "true")
						.map((tab) => tab.textContent.trim()),
					stops: tabs.filter((tab) => tab.tabIndex === 0).map((tab) => tab.textContent.trim()),
					shown: Array.from(drawer.querySelectorAll('[role="tabpanel"]'))
						.filter((panel) => getComputedStyle(panel).display !== "none")
						.map((panel) => document.getElementById(panel.getAttribute("aria-labelledby")).textContent.trim()),
				};
			})()`);
		const expect = async (name) => {
			const now = await state();
			assert(
				JSON.stringify(now) ===
					JSON.stringify({ selected: [name], stops: [name], shown: [name] }),
				`${name} is the selected tab, the tab stop, and the shown panel: ${
					JSON.stringify(now)
				}`,
			);
		};

		await expect("Rust");
		await page.evaluate(
			`document.querySelector('[data-testid="drawer:trends"] [data-testid="ask-tab:rust"]').focus()`,
		);
		await press(page, "ArrowRight");
		await expect("SQL");
		assert(await focused(page) === "ask-tab:sql", "focus follows the arrow");
		await press(page, "End");
		await expect("HTTP");
		await press(page, "ArrowRight");
		await expect("Rust");
		await press(page, "ArrowLeft");
		await expect("HTTP");
		await press(page, "Home");
		await expect("Rust");
	});
});

Deno.test("Escape gives focus back to the exact control that opened Workings", async () => {
	await withBrowser(async (page, site) => {
		const shown = (id) =>
			page.evaluate(
				`document.querySelector('[data-testid="${id}"]')?.getClientRects().length > 0`,
			);
		// A plan row opens the plan's Workings too, with the lines that cause
		// it marked; Escape returns to the row, not to the plan card's mint
		// tab.
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/changes`,
		);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"Changes hydrates",
		);
		if (await shown("drawer:plan")) {
			await page.click(
				'[data-testid="drawer:plan"] [data-testid="workings-close"]',
			);
			await page.until(
				`() => !document.querySelector('[data-testid="drawer:plan"]')?.getClientRects().length`,
				"the plan's Workings closes",
			);
		}
		const row = await page.evaluate(`(() => {
			const row = document.querySelector('[data-testid^="plan-row:"]');
			row.focus();
			return row.getAttribute("data-testid");
		})()`);
		await press(page, "Enter");
		await page.until(
			`() => document.querySelector('[data-testid="drawer:plan"]')?.getClientRects().length > 0`,
			"the plan's Workings opens",
		);
		const key = row.slice("plan-row:".length);
		const marked = await page.evaluate(
			`Array.from(document.querySelectorAll('[data-testid="drawer:plan"] [data-testid^="code-line:"]')).filter((line) => getComputedStyle(line).backgroundColor !== "rgba(0, 0, 0, 0)").map((line) => line.getAttribute("data-testid"))`,
		);
		assert(
			marked.length > 0 &&
				marked.every((line) =>
					line === `code-line:${key}` || line.startsWith(`code-line:${key}.`)
				),
			`the row marks the lines that cause it: ${JSON.stringify(marked)}`,
		);
		await press(page, "Escape");
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === ${
				JSON.stringify(row)
			}`,
			`focus returns to the plan row, not ${await focused(page)}`,
		);

		// W from inside a card opens that card's drawer and returns to what
		// had focus there.
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/overview`,
		);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the overview hydrates",
		);
		await page.evaluate(
			`document.querySelector('[data-testid="card:release"] a[href$="/releases"]').focus()`,
		);
		const before = await page.evaluate(
			"document.activeElement.getAttribute('aria-label') ?? document.activeElement.textContent",
		);
		await press(page, "w");
		await page.until(
			`() => document.querySelector('[data-testid="drawer:release"]')?.getClientRects().length > 0`,
			"W opens the card's Workings",
		);
		await press(page, "Escape");
		await page.until(
			`() => (document.activeElement.getAttribute('aria-label') ?? document.activeElement.textContent) === ${
				JSON.stringify(before)
			}`,
			`focus returns to ${before}`,
		);
	});
});

Deno.test("a project's actions are the pages' own Rust: ask, then send", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/releases`,
		);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"Releases hydrates",
		);
		assert(
			await page.evaluate(
				"!document.querySelector('script[src$=\"admin.mjs\"]')",
			),
			"the page loads no hand-written script",
		);

		// Deploy needs a version first, and says so.
		await page.click('[data-testid="action:deploy"]');
		await page.until(
			`() => document.querySelector('[data-testid="notice"]')?.textContent.includes("Type an image tag")`,
			"Deploy asks for a version",
		);
		assert(
			await focused(page) === "version:deploy",
			`the version field takes focus, not ${await focused(page)}`,
		);

		await page.send("Input.insertText", { text: "v43" });
		await page.click('[data-testid="action:deploy"]');
		await page.until(
			`() => document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal')`,
			"Deploy asks first",
		);
		const says = await page.evaluate(`(() => {
			const explains = document.querySelector('[data-testid="action:deploy"]')
				.getAttribute("aria-description").replaceAll("\`", "");
			const sheet = document.querySelector('[data-testid="confirm-sheet"]').textContent;
			return { explains, sheet };
		})()`);
		assert(
			says.explains.length > 0 && says.sheet.includes(says.explains),
			`the sheet says what runs: ${JSON.stringify(says)}`,
		);
		const opened = page.once("Page.loadEventFired");
		await page.click('[data-testid="confirm-yes"]');
		// A sent action opens the operation it started.
		await opened;
		const sent = site.posted.find((entry) => entry.path.endsWith("/actions"));
		assert(
			sent?.body.kind === "deploy" && sent.body.version === "v43" &&
				sent.body.idempotency_key.length > 0,
			`the request: ${JSON.stringify(sent)}`,
		);
	});
});

Deno.test("a builder change shows what it changed in the open drawer, and nothing else moves", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?interval=day&workings=trends`,
		);
		await hydrated(page, "the builder");
		await page.evaluate("window.beforeSwap = true");
		await page.evaluate(
			`document.querySelector('[data-testid="main"] a[aria-label="Week"]').focus()`,
		);
		await press(page, "Enter");
		await page.until(
			`() => location.search.includes('interval=week') && document.querySelector('[data-testid="drawer:trends"] [aria-description="changed"]')`,
			"the page changes in place and washes the changed lines",
		);
		const result = await page.until(
			`() => {
			const drawer = document.querySelector('[data-testid="drawer:trends"]');
			const scroller = drawer.querySelector('[data-testid="drawer-scroll"]');
			const line = drawer.querySelector('[data-testid="station:ask"] [data-testid^="code-line"][aria-description="changed"]');
			const box = line?.getBoundingClientRect();
			const view = scroller.getBoundingClientRect();
			const settled = box && box.top >= view.top && box.bottom <= view.bottom;
			return settled && {
				status: drawer.querySelector('[data-testid="drawer-changed"]').textContent.trim(),
				marked: drawer.querySelector('[data-testid="station:ask"]').getAttribute("aria-description"),
				mainScroll: document.querySelector('[data-testid="main"]').scrollTop,
				focus: document.activeElement.getAttribute("aria-label"),
				title: document.title,
			};
		}`,
			"the changed Ask line scrolls into the drawer's view",
		);
		assert(
			result.status.startsWith("Ask changed: ") &&
				result.status.includes("Interval::Week"),
			`the drawer says what changed: ${result.status}`,
		);
		assert(result.marked === "changed", "the Ask station is marked changed");
		assert(result.mainScroll === 0, "the page itself didn't scroll");
		assert(result.focus === "Week", `focus stays on Week, not ${result.focus}`);
		assert(
			await page.evaluate("window.beforeSwap === true"),
			"the builder changed without a reload",
		);

		// Back brings the day back, in place.
		await page.evaluate("history.back()");
		await page.until(
			`() => location.search.includes('interval=day') && document.querySelector('[data-testid="main"] a[aria-label="Day"]')?.getAttribute("aria-current") === "page"`,
			"Back shows the day again",
		);
		assert(
			await page.evaluate("window.beforeSwap === true"),
			"Back changed it in place too",
		);

		// With the drawer closed, a change doesn't open or announce anything.
		await press(page, "Escape");
		await page.until(
			`() => document.querySelector('[data-testid="drawer:trends"]')?.getClientRects().length === 0`,
			"the drawer closes",
		);
		await page.evaluate(
			`document.querySelector('[data-testid="main"] a[aria-label="Week"]').click()`,
		);
		await page.until(
			`() => location.search.includes('interval=week') && document.querySelector('[data-testid="main"] a[aria-label="Week"]')?.getAttribute("aria-current") === "page"`,
			"the page changes again",
		);
		const quiet = await page.evaluate(`({
			open: document.querySelector('[data-testid="drawer:trends"]').getClientRects().length > 0,
			status: document.querySelector('[data-testid="drawer:trends"] [data-testid="drawer-changed"]').textContent.trim(),
		})`);
		assert(!quiet.open && quiet.status === "", JSON.stringify(quiet));
	});
});

Deno.test("Save and Add to dashboard ask through the sheet, then open what they made", async () => {
	await withBrowser(async (page, site) => {
		await open(page, `${site.origin}${BASE}/analytics/trends?interval=week`);
		await hydrated(page, "the builder");
		await page.click('[data-testid="add-to-dashboard"]');
		await page.until(
			`() => document.querySelector('[data-testid="confirm-choices"]')`,
			"the dashboards to choose from",
		);
		const sheet = await page.evaluate(`({
			choices: Array.from(document.querySelectorAll('[data-testid="confirm-choices"] [role="radio"]'))
				.map((radio) => [radio.textContent.trim(), radio.getAttribute("aria-checked")]),
			field: document.querySelector('[data-testid="confirm-input"]').disabled,
		})`);
		assert(
			JSON.stringify(sheet.choices) ===
				JSON.stringify([["Growth (2 cards)", "true"], [
					"A new dashboard",
					"false",
				]]),
			`the dashboards, the first chosen: ${JSON.stringify(sheet.choices)}`,
		);
		assert(sheet.field, "a new dashboard's name waits for that choice");
		await page.click(
			'[data-testid="confirm-choices"] [role="radio"]:last-child',
		);
		await page.until(
			`() => !document.querySelector('[data-testid="confirm-input"]').disabled`,
			"choosing a new dashboard takes its name",
		);
		await page.click(
			'[data-testid="confirm-choices"] [role="radio"]:first-child',
		);
		let navigated = page.once("Page.loadEventFired");
		await page.click('[data-testid="confirm-yes"]');
		await navigated;
		assert(
			await page.evaluate("location.pathname") === `${BASE}/analytics/library`,
			"the dashboard's library opens",
		);
		const added = site.posted.find((post) =>
			post.path === `${BASE}/api/analytics/dashboards`
		);
		assert(
			added?.body.dashboard === "d1" && added.body.dashboard_name === null &&
				added.body.kind === "trends" && added.body.insight === null &&
				added.body.params.interval === "week",
			`it adds this trend to Growth: ${JSON.stringify(added)}`,
		);

		await open(page, `${site.origin}${BASE}/analytics/trends?interval=week`);
		await hydrated(page, "the builder");
		await page.click('[data-testid="save-insight"]');
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "confirm-input"`,
			"the name, ready to change",
		);
		assert(
			await page.evaluate(
				`document.querySelector('[data-testid="confirm-input"]').value !== ""`,
			),
			"the name starts as the suggested one",
		);
		await page.evaluate(`(() => {
			const input = document.querySelector('[data-testid="confirm-input"]');
			input.select();
		})()`);
		await page.send("Input.insertText", { text: "Weekly notes" });
		navigated = page.once("Page.loadEventFired");
		await press(page, "Enter");
		await navigated;
		assert(
			await page.evaluate("location.pathname") === `${BASE}/analytics/library`,
			"the saved insight's library opens",
		);
		const saved = site.posted.find((post) =>
			post.path === `${BASE}/api/analytics/insights`
		);
		assert(
			saved?.body.name === "Weekly notes" && saved.body.kind === "trends",
			`it saves under the typed name: ${JSON.stringify(saved)}`,
		);
	});
});

Deno.test("on a phone, stations are headers you open, and Copy covers no code", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?interval=day&workings=trends`,
			{ width: 400, height: 860 },
		);
		await hydrated(page, "the builder");
		const stations = () =>
			page.evaluate(
				`Array.from(document.querySelectorAll('[data-testid="drawer:trends"] [data-testid^="station:"]')).map((station) => ({
				name: station.getAttribute("data-testid"),
				open: getComputedStyle(station.querySelector('[data-testid="station-body"]')).display !== "none",
				expanded: station.querySelector('[data-testid="station-toggle"]').getAttribute("aria-expanded"),
			}))`,
			);
		const start = await stations();
		assert(
			start.every((station) => !station.open && station.expanded === "false"),
			`every station starts as its header: ${JSON.stringify(start)}`,
		);

		await page.click(
			'[data-testid="drawer:trends"] [data-testid="station:send"] [data-testid="station-toggle"]',
		);
		const send = (await stations()).find((station) =>
			station.name === "station:send"
		);
		assert(send.open && send.expanded === "true", JSON.stringify(send));

		const overlaps = await page.evaluate(
			`Array.from(document.querySelectorAll('[data-testid="drawer:trends"] [data-testid="station:send"] [data-testid="code"]')).flatMap((block) => {
			const copy = block.querySelector('[data-testid="copy"]').getBoundingClientRect();
			return Array.from(block.querySelectorAll('[data-testid="code-text"]')).filter((text) => {
				const box = text.getBoundingClientRect();
				return box.left < copy.right && box.right > copy.left && box.top < copy.bottom && box.bottom > copy.top;
			}).map((text) => text.textContent);
		})`,
		);
		assert(overlaps.length === 0, `Copy covers ${JSON.stringify(overlaps)}`);

		// A builder change opens the station it changed.
		await page.evaluate(
			`document.querySelector('[data-testid="main"] a[aria-label="Week"]').click()`,
		);
		await page.until(
			`() => document.querySelector('[data-testid="drawer:trends"] [data-testid="station:ask"] [data-testid="station-toggle"]')?.getAttribute("aria-expanded") === "true"`,
			"the changed station opens",
		);
		const after = (await stations()).find((station) =>
			station.name === "station:send"
		);
		assert(after.open, "a station someone opened stays open across the swap");
	});
});

Deno.test("Glossary is the pages' own Rust: theme, Workings mode, search, and the account menu", async () => {
	await withBrowser(async (page, site) => {
		// A panic in the pages' Wasm surfaces as an uncaught exception.
		const thrown = [];
		page.on("Runtime.exceptionThrown", (event) => {
			thrown.push(
				event.exceptionDetails?.exception?.description ??
					event.exceptionDetails?.text,
			);
		});
		await open(page, `${site.origin}${BASE}/learn/glossary`);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the page hydrates",
		);
		assert(
			await page.evaluate(
				"!document.querySelector('script[src$=\"admin.mjs\"]')",
			),
			"the page loads no hand-written script",
		);

		// The theme flips what the page shows and remembers it.
		const appearance = () =>
			page.evaluate(
				"document.documentElement.getAttribute('data-lk-appearance')",
			);
		const before = await appearance();
		await page.click('[data-testid="theme"]');
		const after = await appearance();
		assert(
			(after === "dark" || after === "light") && after !== before,
			`the theme flips: ${before} to ${after}`,
		);
		assert(
			await page.evaluate(`document.cookie.includes("admin_theme=${after}")`),
			"the choice is kept in a cookie the server reads",
		);

		// Shift+W flips Workings mode and saves it.
		const wasOn = await page.evaluate(
			`document.querySelector('[data-testid="workings-mode"]').getAttribute("aria-pressed") === "true"`,
		);
		await press(page, "w", { shift: true });
		await page.until(
			`() => document.querySelector('[data-testid="workings-mode"]').getAttribute("aria-pressed") === "${!wasOn}"`,
			"Shift+W flips Workings mode",
		);
		for (let tries = 0; !site.posted.length && tries < 100; tries++) {
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
		assert(
			JSON.stringify(site.posted[0]) ===
				JSON.stringify({
					path: `${BASE}/api/preferences`,
					body: { kind: "workings_mode", on: !wasOn },
				}),
			`the mode is saved: ${JSON.stringify(site.posted)}`,
		);

		// Ctrl+K finds a screen from anywhere; Enter goes to the first match.
		await press(page, "k", { ctrl: true });
		await page.until(
			"() => document.activeElement?.getAttribute('data-testid') === 'search'",
			"Ctrl+K moves focus to search",
		);
		await page.send("Input.insertText", { text: "trend" });
		await page.until(
			`() => Array.from(document.querySelectorAll('[data-testid="search-results"] [role="option"]')).map((option) => option.textContent.trim()).join() === "Trends"`,
			"search shows what matches",
		);
		const navigated = page.once("Page.loadEventFired");
		await press(page, "Enter");
		await navigated;
		assert(
			await page.evaluate("location.pathname") === `${BASE}/analytics/trends`,
			"Enter opens the first match",
		);

		// The account menu signs out at the main site, then goes to sign-in.
		await open(page, `${site.origin}${BASE}/learn/glossary`);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the page hydrates again",
		);
		await page.click('[data-testid="avatar"]');
		await page.until(
			`() => document.querySelector('[data-testid="avatar-menu"]')?.matches(':popover-open')`,
			"the account menu opens",
		);
		const left = page.once("Page.loadEventFired");
		await page.click('[data-testid="sign-out"]');
		await left;
		assert(
			site.posted.some((post) => post.path === "/auth/sign-out"),
			"the main site is told",
		);
		assert(
			await page.evaluate("location.pathname") === `${BASE}/sign-in`,
			"and the admin's sign-in page opens",
		);
		assert(thrown.length === 0, `nothing threw: ${thrown.join("\n")}`);
	});
});

Deno.test("Learn is the pages' own Rust, and its lessons are links", async () => {
	await withBrowser(async (page, site) => {
		const thrown = [];
		page.on("Runtime.exceptionThrown", (event) => {
			thrown.push(
				event.exceptionDetails?.exception?.description ??
					event.exceptionDetails?.text,
			);
		});
		await open(page, `${site.origin}${BASE}/learn`);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the page hydrates",
		);
		assert(
			await page.evaluate(
				"!document.querySelector('script[src$=\"admin.mjs\"]')",
			),
			"the page loads no hand-written script",
		);
		assert(
			await page.evaluate(
				"document.querySelector('[data-testid=\"up-next\"]') !== null",
			),
			"the lesson up next is on the page",
		);

		// The server's markup and the client's tree agree, so the shell
		// answers presses: the theme flips.
		const appearance = () =>
			page.evaluate(
				"document.documentElement.getAttribute('data-lk-appearance')",
			);
		const before = await appearance();
		await page.click('[data-testid="theme"]');
		assert((await appearance()) !== before, "the theme flips");

		await page.click('[data-testid="lesson:first-typed-event"]');
		await page.until(
			`() => location.pathname === "${BASE}/learn/analytics/first-typed-event"`,
			"a lesson row opens its lesson",
		);
		assert(thrown.length === 0, `nothing threw: ${thrown.join("\n")}`);
	});
});

Deno.test("Workings drawers are the pages' own Rust: open, close, focus, W, and a link that opens one", async () => {
	await withBrowser(async (page, site) => {
		const thrown = [];
		page.on("Runtime.exceptionThrown", (event) => {
			thrown.push(
				event.exceptionDetails?.exception?.description ??
					event.exceptionDetails?.text,
			);
		});
		const shown = (id) =>
			page.evaluate(
				`(() => { const element = document.querySelector('[data-testid="${id}"]'); return !!element && element.getClientRects().length > 0; })()`,
			);

		await open(page, `${site.origin}${BASE}/infra/projects`);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"Projects hydrates",
		);
		assert(
			await page.evaluate(
				"document.querySelector('[data-testid=\"project:notes-production\"]') !== null",
			),
			"Projects lists the project",
		);

		await open(page, `${site.origin}${BASE}/infra/operations/plan`);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the operation hydrates",
		);
		assert(
			await page.evaluate(
				"!document.querySelector('script[src$=\"admin.mjs\"]')",
			),
			"the page loads no hand-written script",
		);
		assert(!(await shown("drawer:operation")), "the drawer starts closed");

		// The mint tab opens it and Close takes focus; Escape closes it and
		// gives focus back to the tab.
		await page.evaluate(
			"document.querySelector('[data-testid=\"workings-tab:operation\"]').focus()",
		);
		await press(page, "Enter");
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "workings-close"`,
			"Close takes focus",
		);
		assert(await shown("drawer:operation"), "the tab opens its drawer");
		await press(page, "Escape");
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "workings-tab:operation"`,
			"focus goes back to the tab",
		);
		assert(!(await shown("drawer:operation")), "Escape closes the drawer");

		// W opens the page's first drawer and closes it again.
		await press(page, "w");
		await page.until(
			`() => document.querySelector('[data-testid="drawer:operation"]')?.getClientRects().length > 0`,
			"W opens the drawer",
		);
		await press(page, "w");
		await page.until(
			`() => !document.querySelector('[data-testid="drawer:operation"]')?.getClientRects().length`,
			"W closes it",
		);

		// A link can open a card with its workings showing.
		await open(
			page,
			`${site.origin}${BASE}/infra/operations/plan?workings=operation`,
		);
		await page.until(
			"() => document.documentElement.hasAttribute('data-hydrated')",
			"the linked page hydrates",
		);
		assert(await shown("drawer:operation"), "?workings= opens the drawer");
		assert(thrown.length === 0, `nothing threw: ${thrown.join("\n")}`);
	});
});

Deno.test("People and a person are the pages' own Rust: search, export, and a delete that asks for their name", async () => {
	await withBrowser(async (page, site) => {
		const hydrated = (what) =>
			page.until(
				"() => document.documentElement.hasAttribute('data-hydrated')",
				what,
			);
		const sheetOpen = () =>
			page.evaluate(
				`document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal') === true`,
			);
		await open(page, `${site.origin}${BASE}/analytics/people`);
		await hydrated("People hydrates");
		assert(
			await page.evaluate(
				"!document.querySelector('script[src$=\"admin.mjs\"]')",
			),
			"the page loads no hand-written script",
		);

		// Enter in a field searches by what the fields hold.
		await page.click('[data-testid="people-search"]');
		await page.send("Input.insertText", { text: "member 01" });
		let navigated = page.once("Page.loadEventFired");
		await press(page, "Enter");
		await navigated;
		assert(
			await page.evaluate("location.search") === "?search=member%2001",
			`the search is in the address: ${await page.evaluate("location.search")}`,
		);

		await open(page, `${site.origin}${BASE}/analytics/people/someone`);
		await hydrated("the person hydrates");
		const name = await page.evaluate(
			"document.querySelector('h1').textContent",
		);

		// Export is a download the browser makes once the person agrees.
		const downloads = await Deno.makeTempDir({ prefix: "admin-downloads-" });
		await page.send("Page.setDownloadBehavior", {
			behavior: "allow",
			downloadPath: downloads,
		});
		await page.click('[data-testid="person-export"]');
		await page.until(
			`() => document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal')`,
			"Export asks first",
		);
		const link = await page.evaluate(`(() => {
			const yes = document.querySelector('[data-testid="confirm-yes"]');
			return { tag: yes.tagName, download: yes.getAttribute("download"), href: yes.getAttribute("href") };
		})()`);
		assert(
			link.tag === "A" && link.download?.startsWith("person-") &&
				link.href.endsWith("/export"),
			`the confirmation is a download link: ${JSON.stringify(link)}`,
		);
		await page.click('[data-testid="confirm-yes"]');
		for (
			let tries = 0;
			!site.posted.some((entry) => entry.path.endsWith("/export")) &&
			tries < 100;
			tries++
		) {
			await new Promise((resolve) => setTimeout(resolve, 50));
		}
		assert(
			site.posted.some((entry) => entry.path.endsWith("/export")),
			"the browser fetched the export",
		);
		await page.until(
			`() => !document.querySelector('[data-testid="confirm-sheet"]')?.matches(':modal')`,
			"the sheet closes after the download",
		);
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "person-export"`,
			"focus goes back to Export",
		);
		await Deno.remove(downloads, { recursive: true });

		// Delete waits for their name, typed exactly.
		await page.click('[data-testid="person-delete"]');
		await page.until(
			`() => document.activeElement?.getAttribute("data-testid") === "confirm-input"`,
			"the name field takes focus",
		);
		const disabled = () =>
			page.evaluate(
				`document.querySelector('[data-testid="confirm-yes"]').disabled`,
			);
		assert(await disabled(), "Delete waits for the name");
		const half = Math.floor(name.length / 2);
		await page.send("Input.insertText", { text: name.slice(0, half) });
		assert(await disabled(), "half the name isn't enough");
		await press(page, "Enter");
		assert(await sheetOpen(), "Enter does nothing until the name matches");
		await page.send("Input.insertText", { text: name.slice(half) });
		await page.until(
			`() => !document.querySelector('[data-testid="confirm-yes"]').disabled`,
			"the whole name enables Delete",
		);
		navigated = page.once("Page.loadEventFired");
		await press(page, "Enter");
		await navigated;
		const sent = site.posted.find((entry) => entry.path.endsWith("/delete"));
		assert(
			sent?.body.confirm === name,
			`the request: ${JSON.stringify(sent)}`,
		);
		assert(
			await page.evaluate("location.pathname") === `${BASE}/analytics/people`,
			"a deleted person leaves for the list",
		);
	});
});
