// The packaged website's code style switch, in headless Chrome. Samples read
// as `view!` markup until a reader picks builder calls; one switch changes
// every sample on the page, the choice holds across a reload and on the docs,
// the head's script applies it before any sample is drawn, copy copies what
// shows, and no page logs an error. Without JavaScript, samples read as
// markup and no switch shows.
//
// Package the site first, then point the test at the artifact (the test fails
// without one):
//
//   LEPTOSKIT_SITE=docs/dist deno test --allow-env --allow-read --allow-net \
//     --allow-run --allow-write crates/leptoskit_app/tests/code_style.mjs
//
// `CHROME` names the browser executable; otherwise the usual install
// locations are tried. See `browser.mjs` for how the test drives Chrome.

import { assert, launch, Page, removeProfile, serve } from "./browser.mjs";

const site = Deno.env.get("LEPTOSKIT_SITE")?.replace(/\/$/, "");
const DOCS_PAGE = "/docs/guide/02-creating.html";

// Before the page parses, record the root's code style when the first sample
// arrives: the style must already be set, or the sample could paint in the
// other style first. Count the writes to it as well: any write restyles the
// whole page, so the head script's should be the only one until a reader
// chooses.
const RECORD_STYLE = `globalThis.__styleWrites = 0;
new MutationObserver((records) => {
	globalThis.__styleWrites += records.length;
}).observe(document, { subtree: true, attributeFilter: ["data-code-style"] });
new MutationObserver((_, observer) => {
	const sample = document.querySelector("#source-panel, figure[data-lk-styles]");
	if (!sample) return;
	observer.disconnect();
	globalThis.__styleAtFirstSample = document.documentElement.dataset.codeStyle ?? "unset";
}).observe(document, { childList: true, subtree: true });`;

// What the page shows: its code style, every sample's visible variants, and
// the checked option of every switch, as JSON-friendly values.
const SHOWN = `(() => {
	const visible = (element) => element?.checkVisibility() ?? false;
	const landing = ["fan-tree", "source"].map((id) => {
		const shown = ["markup", "builder"].filter((style) =>
			visible(document.getElementById(id + "-" + style))
		);
		return id + ":" + shown.join("+");
	});
	const mapLines = [...document.querySelectorAll("#widget-map > div > [id$='-markup'], #widget-map > div > [id$='-builder']")]
		.filter(visible)
		.map((line) => line.id.endsWith("-builder") ? "b" : "m");
	// Samples in a closed tab are left out: nothing of theirs shows.
	const docs = [...document.querySelectorAll("figure[data-lk-styles]")].filter(visible).map((figure) =>
		[...figure.querySelectorAll("pre[data-lk-variant]")]
			.filter(visible)
			.map((pre) => pre.dataset.lkVariant)
			.join("+")
	);
	const radios = [...document.querySelectorAll("[role='radiogroup'][aria-label='Code style'] [role='radio']")];
	return {
		style: document.documentElement.dataset.codeStyle ?? "unset",
		landing,
		map: [...new Set(mapLines)].join("+"),
		mapLines: mapLines.length,
		docs: [...new Set(docs)].join(","),
		docSamples: docs.length,
		switches: radios.filter(visible).length / 2,
		checked: [...new Set(radios
			.filter((radio) => radio.getAttribute("aria-checked") === "true")
			.map((radio) => radio.textContent.trim()))].join(","),
		tabbable: radios.filter((radio) => radio.tabIndex === 0).map((radio) => radio.textContent.trim()),
	};
})()`;

/**
 * Opens a fresh headless Chrome on the packaged site and runs `check` with a
 * page whose `visit(path)` navigates and waits for the load, and with the
 * errors every page has logged. `scripts: false` turns JavaScript off.
 */
async function browse({ scripts = true, width = 1280, check }) {
	// Fail rather than skip, so a CI run that lost the artifact cannot pass.
	assert(site, "set LEPTOSKIT_SITE to a packaged site, such as docs/dist");
	const server = serve(site);
	const profile = await Deno.makeTempDir({ prefix: "leptoskit-code-style-" });
	const chrome = await launch(profile);
	const errors = [];
	let page;
	try {
		page = await Page.connect(chrome.endpoint);
		page.on("Runtime.exceptionThrown", ({ exceptionDetails }) => {
			errors.push(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		});
		page.on("Runtime.consoleAPICalled", ({ type, args }) => {
			if (type === "error" || type === "assert") {
				errors.push(args.map((arg) => arg.value ?? arg.description).join(" "));
			}
		});
		page.on("Log.entryAdded", ({ entry }) => {
			if (entry.level === "error") {
				errors.push(`${entry.text} ${entry.url ?? ""}`);
			}
		});
		await Promise.all(
			["Page.enable", "Runtime.enable", "Log.enable"].map((m) => page.send(m)),
		);
		await page.send("Emulation.setDeviceMetricsOverride", {
			width,
			height: 900,
			deviceScaleFactor: 1,
			mobile: width < 700,
		});
		if (scripts) {
			await page.send("Page.addScriptToEvaluateOnNewDocument", {
				source: RECORD_STYLE,
			});
		} else {
			await page.send("Emulation.setScriptExecutionDisabled", { value: true });
		}

		const origin = `http://127.0.0.1:${server.addr.port}`;
		page.visit = async (path) => {
			const loaded = page.once("Page.loadEventFired");
			await page.send("Page.navigate", { url: origin + path });
			await loaded;
		};
		page.reload = async () => {
			const loaded = page.once("Page.loadEventFired");
			await page.send("Page.reload");
			await loaded;
		};
		page.shown = () => page.evaluate(SHOWN);
		page.key = async (key, code) => {
			for (const type of ["keyDown", "keyUp"]) {
				await page.send("Input.dispatchKeyEvent", {
					type,
					key,
					code: key,
					windowsVirtualKeyCode: code,
				});
			}
		};
		await check(page, errors);
	} catch (error) {
		const shown = page
			? JSON.stringify(await page.shown().catch(() => ""))
			: "";
		throw new Error(
			`${error.message}\nthe page shows: ${shown}\npage errors:\n${
				errors.join("\n")
			}`,
			{ cause: error },
		);
	} finally {
		await page?.close();
		chrome.process.kill();
		await chrome.process.status;
		await server.shutdown();
		await removeProfile(profile);
	}
}

/** Fails unless `actual` has every field of `expected`, with equal values. */
function assertShows(actual, expected, when) {
	for (const [field, value] of Object.entries(expected)) {
		assert(
			JSON.stringify(actual[field]) === JSON.stringify(value),
			`${when}: ${field} is ${JSON.stringify(actual[field])}, not ${
				JSON.stringify(value)
			}`,
		);
	}
}

/** Fails unless only the head script wrote the page's code style. */
async function assertStyleWrittenOnce(page, when) {
	const writes = await page.evaluate(`globalThis.__styleWrites`);
	assert(writes === 1, `${when}: the style was written ${writes} times`);
}

/** Presses `selector` once it is centred in the viewport. */
async function press(page, selector) {
	await page.evaluate(
		`document.querySelector(${
			JSON.stringify(selector)
		}).scrollIntoView({ block: "center", behavior: "instant" })`,
	);
	await page.click(selector);
}

Deno.test({
	name: "one switch changes every sample, and the choice holds on the docs",
	async fn() {
		await browse({
			async check(page, errors) {
				await page.visit("/");
				const first = await page.shown();
				assertShows(first, {
					style: "markup",
					landing: ["fan-tree:markup", "source:markup"],
					map: "m",
					mapLines: 7,
					switches: 2,
					checked: "Markup",
					tabbable: ["Markup", "Markup"],
				}, "a first visit");
				assert(
					await page.evaluate(`globalThis.__styleAtFirstSample`) === "markup",
					"the style must be set before the first sample is drawn",
				);
				await assertStyleWrittenOnce(page, "a first visit");

				// One switch changes every sample, and the other switch with it.
				await press(page, "#source-style-builder");
				assertShows(await page.shown(), {
					style: "builder",
					landing: ["fan-tree:builder", "source:builder"],
					map: "b",
					mapLines: 4,
					checked: "Builder",
					tabbable: ["Builder", "Builder"],
				}, "after choosing Builder");
				assert(
					await page.evaluate(`document.activeElement.id`) ===
						"source-style-builder",
				);

				// Arrow keys move the choice, and focus, within a switch.
				await page.key("ArrowLeft", 37);
				assertShows(await page.shown(), {
					style: "markup",
					checked: "Markup",
				}, "after pressing the left arrow");
				assert(
					await page.evaluate(`document.activeElement.id`) ===
						"source-style-markup",
					"focus follows the choice",
				);
				await page.key("ArrowRight", 39);
				assertShows(
					await page.shown(),
					{ style: "builder" },
					"after the right arrow",
				);

				// The choice survives a reload, applied before the samples draw.
				await page.reload();
				assertShows(await page.shown(), {
					style: "builder",
					landing: ["fan-tree:builder", "source:builder"],
					checked: "Builder",
				}, "after a reload");
				assert(
					await page.evaluate(`globalThis.__styleAtFirstSample`) === "builder",
					"a saved style must apply before the first sample is drawn",
				);
				await assertStyleWrittenOnce(page, "after a reload");

				// The docs read the same choice.
				await page.visit(DOCS_PAGE);
				const docs = await page.shown();
				assert(
					docs.docSamples > 1,
					`${DOCS_PAGE} has ${docs.docSamples} switchable samples`,
				);
				assertShows(docs, {
					style: "builder",
					docs: "builder",
					switches: docs.docSamples,
					checked: "Builder",
				}, "on a docs page");
				assert(
					await page.evaluate(`globalThis.__styleAtFirstSample`) === "builder",
					"the docs must apply the style before the first sample is drawn",
				);
				await assertStyleWrittenOnce(page, "on a docs page");

				// Copy copies the style that shows.
				const copied = await page.evaluate(`(async () => {
					let copied;
					navigator.clipboard.writeText = async (text) => { copied = text; };
					const figure = document.querySelector("figure[data-lk-styles]");
					figure.querySelector(".lk-copy").click();
					await new Promise((resolve) => setTimeout(resolve, 50));
					const builder = figure.querySelector("pre[data-lk-variant='builder']").innerText;
					return { copied, builder: builder.replace(/\\n$/, "") };
				})()`);
				assert(
					copied.copied && copied.copied === copied.builder,
					`copy copied ${JSON.stringify(copied.copied)}`,
				);

				// Back to markup from the docs, for every page.
				await press(
					page,
					"figure[data-lk-styles] [data-lk-style-option='markup']",
				);
				assertShows(await page.shown(), {
					style: "markup",
					docs: "markup",
					checked: "Markup",
				}, "after choosing Markup on the docs");
				await page.visit("/");
				assertShows(await page.shown(), {
					style: "markup",
					landing: ["fan-tree:markup", "source:markup"],
					checked: "Markup",
				}, "back on the landing page");

				assert(
					errors.length === 0,
					`the pages logged errors:\n${errors.join("\n")}`,
				);
			},
		});
	},
});

Deno.test({
	name: "on a phone, every sample still has its switch",
	async fn() {
		await browse({
			width: 390,
			async check(page, errors) {
				await page.visit("/");
				assertShows(await page.shown(), {
					style: "markup",
					switches: 2,
					checked: "Markup",
				}, "on a phone");
				await press(page, "#source-style-builder");
				assertShows(await page.shown(), {
					style: "builder",
					landing: ["fan-tree:builder", "source:builder"],
				}, "after choosing Builder on a phone");
				assert(
					errors.length === 0,
					`the page logged errors:\n${errors.join("\n")}`,
				);
			},
		});
	},
});

Deno.test({
	name: "without JavaScript, samples read as markup and no switch shows",
	async fn() {
		await browse({
			scripts: false,
			async check(page) {
				await page.visit("/");
				assertShows(await page.shown(), {
					style: "unset",
					landing: ["fan-tree:markup", "source:markup"],
					map: "m",
					switches: 0,
				}, "the landing page without JavaScript");
				await page.visit(DOCS_PAGE);
				assertShows(await page.shown(), {
					style: "unset",
					docs: "markup",
					switches: 0,
				}, "a docs page without JavaScript");
			},
		});
	},
});
