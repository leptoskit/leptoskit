// The admin's script in a real browser, against pages the admin really
// renders: `pages_for_the_script` in `tests/analytics.rs` writes them, this
// serves them with `assets/admin.mjs` and a live stream the test feeds, and
// headless Chrome runs them over the DevTools protocol.
//
//   deno test -A crates/leptoskit_admin/tests/script.mjs
//
// It runs that Rust test itself unless `LEPTOSKIT_ADMIN_PAGES` already
// names a directory of pages. `CHROME` names the browser executable.

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

/** Serves the pages as the admin would, and a live stream to push into. */
function serve(directory) {
	const streams = new Set();
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
				case `${BASE}/analytics/trends`:
					if (url.searchParams.get("by") === "session:channel") {
						return page("trends-channels.html");
					}
					if (url.searchParams.get("s0") === "$pageview") {
						return page("trends-pageviews.html");
					}
					return page(
						url.searchParams.get("interval") === "week"
							? "trends-week.html"
							: "trends-day.html",
					);
				case `${BASE}/analytics/live`:
					return page("live.html");
				case `${BASE}/infra/projects/notes-production/overview`:
					return page("overview.html");
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
				case `${BASE}/api/preferences`:
					return Response.json({});
				default:
					return new Response("Not found", { status: 404 });
			}
		},
	);
	return {
		origin: `http://127.0.0.1:${server.addr.port}`,
		connected: () => streams.size,
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
	const directory = await pages();
	const site = serve(directory);
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

const KEYS = {
	Tab: 9,
	ArrowRight: 39,
	ArrowLeft: 37,
	Home: 36,
	End: 35,
	Escape: 27,
	Enter: 13,
	w: 87,
};

async function press(page, key, { shift = false } = {}) {
	const common = {
		key,
		modifiers: shift ? 8 : 0,
		code: key.length === 1 ? `Key${key.toUpperCase()}` : key,
		windowsVirtualKeyCode: KEYS[key],
	};
	await page.send("Input.dispatchKeyEvent", {
		type: "keyDown",
		...common,
		...(key === "Enter" ? { text: "\r" } : key === "w" ? { text: "w" } : {}),
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
		await page.evaluate("window.beforeSwap = true");
		await page.click(
			'[data-testid="menu-toggle"][aria-description="breakdown"]',
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
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/overview#workings=release`,
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
			"() => document.querySelector('[data-testid=\"confirm-sheet\"]')",
			"the health dialog",
		);
		assert(
			await focused(page) === "confirm-yes",
			"the confirmation receives focus",
		);
		await press(page, "Tab");
		assert(
			await focused(page) === "confirm-no",
			`Tab wraps inside the dialog, not ${await focused(page)}`,
		);
		await press(page, "Tab", { shift: true });
		assert(
			await focused(page) === "confirm-yes",
			"Shift+Tab wraps back to confirmation",
		);
		assert(
			await page.evaluate("document.querySelector('[data-lk-root]').inert"),
			"the background is inert while confirming",
		);
		await press(page, "Escape");
		assert(
			await focused(page) === "action:status",
			"Escape restores the exact opener",
		);
		assert(
			await page.evaluate(
				"document.documentElement.getAttribute('data-admin-drawer') === 'release'",
			),
			"Escape dismisses only the dialog",
		);
		assert(
			await page.evaluate(
				"!document.querySelector('[data-lk-root]').inert && document.getElementById('already-inert').inert",
			),
			"closing restores prior inert states",
		);
		await press(page, "Enter");
		await page.until(
			"() => document.querySelector('[data-testid=\"confirm-sheet\"]')",
			"the reopened dialog",
		);
		await page.click('[data-testid="confirm-no"]');
		assert(
			await focused(page) === "action:status",
			"Cancel restores the opener too",
		);

		await open(page, `${site.origin}${BASE}/analytics/trends`);
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
		assert(
			await focused(page) === "save-insight",
			"pointer activation also restores the opener",
		);
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
			`${site.origin}${BASE}/analytics/trends?interval=week#workings=trends`,
		);
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
		// A plan row opens the plan's Workings too; Escape returns to the row,
		// not to the plan card's mint tab.
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/changes`,
		);
		await page.evaluate(`(() => {
			document.documentElement.removeAttribute("data-admin-drawer");
			document.querySelector('[data-testid^="plan-row:"]').focus();
		})()`);
		await press(page, "Enter");
		await page.until(
			"() => document.documentElement.getAttribute('data-admin-drawer') === 'plan'",
			"the plan's Workings opens",
		);
		await press(page, "Escape");
		assert(
			(await focused(page)).startsWith("plan-row:"),
			`focus returns to the plan row, not ${await focused(page)}`,
		);

		// W from inside a card returns to what had focus there.
		await open(
			page,
			`${site.origin}${BASE}/infra/projects/notes-production/overview`,
		);
		await page.evaluate(
			`document.querySelector('[data-testid="card:release"] a[href$="/releases"]').focus()`,
		);
		const before = await page.evaluate(
			"document.activeElement.getAttribute('aria-label') ?? document.activeElement.textContent",
		);
		await press(page, "w");
		await page.until(
			"() => document.documentElement.getAttribute('data-admin-drawer') === 'release'",
			"W opens the card's Workings",
		);
		await press(page, "Escape");
		const after = await page.evaluate(
			"document.activeElement.getAttribute('aria-label') ?? document.activeElement.textContent",
		);
		assert(after === before, `focus returns to ${before}, not ${after}`);
	});
});

Deno.test("a builder change shows what it changed in the open drawer, and nothing else moves", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?interval=day#workings=trends`,
		);
		await page.evaluate(
			`document.querySelector('[data-testid="main"] a[aria-label="Week"]').focus()`,
		);
		await press(page, "Enter");
		await page.until(
			"() => location.search.includes('interval=week') && document.querySelector('[data-testid=\"drawer:trends\"] [data-changed]')",
			"the page swaps in place and washes the changed lines",
		);
		const result = await page.until(
			`() => {
			const drawer = document.querySelector('[data-testid="drawer:trends"]');
			const scroller = drawer.querySelector('[data-testid="drawer-scroll"]');
			const line = drawer.querySelector('[data-testid="station:ask"] [data-changed]');
			const box = line?.getBoundingClientRect();
			const view = scroller.getBoundingClientRect();
			const settled = box && box.top >= view.top && box.bottom <= view.bottom;
			return settled && {
				status: drawer.querySelector('[data-testid="drawer-changed"]').textContent,
				marked: drawer.querySelector('[data-testid="station:ask"]').hasAttribute("data-changed-station"),
				mainScroll: document.querySelector('[data-testid="main"]').scrollTop,
				focus: document.activeElement.getAttribute("aria-label"),
			};
		}`,
			"the changed Ask line scrolls into the drawer's view",
		);
		assert(
			result.status.startsWith("Ask changed: ") &&
				result.status.includes("Interval::Week"),
			`the drawer says what changed: ${result.status}`,
		);
		assert(result.marked, "the Ask station is marked changed");
		assert(result.mainScroll === 0, "the page itself didn't scroll");
		assert(result.focus === "Week", `focus stays on Week, not ${result.focus}`);

		// With the drawer closed, a change doesn't open or announce anything.
		await press(page, "Escape");
		await page.evaluate(
			`document.querySelector('[data-testid="main"] a[aria-label="Day"]').click()`,
		);
		await page.until(
			"() => location.search.includes('interval=day')",
			"the page swaps back",
		);
		const quiet = await page.evaluate(`({
			open: document.documentElement.hasAttribute("data-admin-drawer"),
			status: document.querySelector('[data-testid="drawer:trends"] [data-testid="drawer-changed"]').textContent.trim(),
		})`);
		assert(!quiet.open && quiet.status === "", JSON.stringify(quiet));
	});
});

Deno.test("on a phone, stations are headers you open, and Copy covers no code", async () => {
	await withBrowser(async (page, site) => {
		await open(
			page,
			`${site.origin}${BASE}/analytics/trends?interval=day#workings=trends`,
			{ width: 400, height: 860 },
		);
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
			`() => document.querySelector('[data-testid="drawer:trends"] [data-testid="station:ask"]')?.hasAttribute("data-open")`,
			"the changed station opens",
		);
		const after = (await stations()).find((station) =>
			station.name === "station:send"
		);
		assert(after.open, "a station someone opened stays open across the swap");
	});
});
