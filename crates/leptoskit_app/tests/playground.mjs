// Live-service proof. Package with LEPTOSKIT_PLAY_SERVICE set to this service,
// configure PLAY_ORIGINS for the test site's origin, or use the same-origin
// compile proxy supplied by a test host. No synthetic compiler success.
import { assert, launch, Page, removeProfile, serve } from "./browser.mjs";

const site = Deno.env.get("LEPTOSKIT_SITE");
const service = Deno.env.get("LEPTOSKIT_PLAY_SERVICE") ??
	"http://127.0.0.1:4702";

function observePreview(page, contexts, errors) {
	page.on("Runtime.executionContextCreated", ({ context }, sessionId) => {
		if (context.auxData?.isDefault) {
			contexts.set(context.auxData.frameId, { id: context.id, sessionId });
		}
	});
	page.on("Target.attachedToTarget", ({ sessionId, targetInfo }) => {
		if (targetInfo.type === "iframe") {
			page.send("Runtime.enable", {}, sessionId);
		}
	});
	page.on("Runtime.consoleAPICalled", ({ type, args }) => {
		if (type === "error") {
			errors.push(args.map((a) => a.value ?? a.description).join(" "));
		}
	});
	page.on("Runtime.exceptionThrown", ({ exceptionDetails }, sessionId) => {
		if (!sessionId) {
			errors.push(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		}
	});
}

async function assertPreviewSandbox(page, contexts) {
	assert(
		await page.evaluate(
			"document.querySelector('iframe').getAttribute('sandbox') === 'allow-scripts'",
		),
		"opaque iframe sandbox",
	);
	const { frameTree } = await page.send("Page.getFrameTree");
	const context =
		Array.from(contexts.values()).find((value) => value.sessionId) ??
			contexts.get(frameTree.childFrames?.[0]?.frame.id);
	assert(context, "preview execution context exists");
	const evaluate = async (expression) => {
		const result = await page.send("Runtime.evaluate", {
			contextId: context.id,
			expression,
			returnByValue: true,
			awaitPromise: true,
		}, context.sessionId);
		assert(
			!result.exceptionDetails,
			JSON.stringify(result.exceptionDetails),
		);
		return result.result.value;
	};
	assert(
		await evaluate("self.origin === 'null'"),
		"preview origin must be opaque",
	);

	assert(
		await evaluate(`new Promise((resolve) => {
		let bundles = 0;
		const listener = (event) => { if (event.data?.type === 'bundle') bundles += 1; };
		addEventListener('message', listener);
		parent.postMessage({ type: 'ready' }, '*');
		parent.postMessage({ type: 'ready' }, '*');
		setTimeout(() => { removeEventListener('message', listener); resolve(bundles); }, 300);
	})`) === 0,
		"repeated frame readiness cannot request repeated large bundles",
	);
	assert(
		await evaluate(`new Promise((resolve) => {
		addEventListener('securitypolicyviolation', (event) => resolve(event.effectiveDirective === 'connect-src'), { once: true });
		fetch('https://example.invalid/').catch(() => {});
		setTimeout(() => resolve(false), 1000);
	})`),
		"preview CSP denies network connections",
	);
	await evaluate("document.querySelector('button').click()");
	assert(
		await evaluate("document.body.textContent.includes('1 ideas shipped')"),
		"press updates state",
	);
}

async function assertCompilerError(page) {
	await page.evaluate(
		"document.querySelector('#source').value = '\\npub fn app() { missing_symbol(); }'",
	);
	await page.click("#run");
	await page.until(
		"() => document.querySelector('#diagnostics button')?.textContent.startsWith('2:')",
		"error mapped to user line 2",
		45000,
	);
}

async function assertHangRecovery(page) {
	// This is intentionally a real synchronous loop. A browser sharing the
	// renderer thread will fail this check; do not replace it with a mock.
	await page.evaluate(
		"document.querySelector('#source').value = 'use leptoskit::prelude::*; pub fn app() -> impl Fn() -> Element { || { loop {} } }'",
	);
	await page.click("#run");
	await Promise.race([
		page.until(
			"() => document.querySelector('#status').textContent.includes('not responding')",
			"hang watchdog",
			45000,
		),
		new Promise((_, reject) =>
			setTimeout(
				() => reject(new Error("preview blocked the parent watchdog")),
				50000,
			)
		),
	]);
	await page.click("#stop");
	assert(
		await page.evaluate("!document.querySelector('iframe')"),
		"Stop removes the unresponsive frame",
	);
}

Deno.test({
	name: "playground compiles real Rust and contains its runtime",
	ignore: Deno.env.get("LEPTOSKIT_PLAY_OFFLINE") === "1",
	async fn() {
		assert(site, "set LEPTOSKIT_SITE to the packaged site");
		const server = serve(site, { playgroundService: service, port: 4701 });
		const profile = await Deno.makeTempDir({ prefix: "leptoskit-playground-" });
		const chrome = await launch(profile);
		let page;
		const errors = [];
		const contexts = new Map();
		try {
			page = await Page.connect(chrome.endpoint);
			observePreview(page, contexts, errors);
			await page.send("Page.enable");
			await page.send("Runtime.enable");
			await page.send("Target.setAutoAttach", {
				autoAttach: true,
				waitForDebuggerOnStart: false,
				flatten: true,
			});
			const loaded = page.once("Page.loadEventFired");
			await page.send("Page.navigate", {
				url: `http://127.0.0.1:${server.addr.port}/play/`,
			});
			await loaded;
			await page.evaluate(
				`document.querySelector('#playground').dataset.service = ${
					JSON.stringify(service)
				}`,
			);
			await page.click("#run");
			await page.until(
				"() => document.querySelector('#status').textContent.includes('your app is live')",
				"starter compilation and mount",
				45000,
			);
			await assertPreviewSandbox(page, contexts);
			const screenshot = Deno.env.get("LEPTOSKIT_PLAY_SCREENSHOT");
			if (screenshot) {
				const { data } = await page.send("Page.captureScreenshot", {
					format: "png",
				});
				await Deno.writeFile(
					screenshot,
					Uint8Array.from(atob(data), (character) => character.charCodeAt(0)),
				);
			}

			await assertCompilerError(page);
			await assertHangRecovery(page);
			assert(
				errors.length === 0,
				`parent console errors: ${errors.join("\n")}`,
			);
		} finally {
			await page?.close();
			chrome.process.kill();
			await chrome.process.status;
			await server.shutdown();
			await removeProfile(profile);
		}
	},
});

Deno.test("packaged playground preserves source when offline", async () => {
	assert(site, "set LEPTOSKIT_SITE to the packaged site");
	const server = serve(site);
	const profile = await Deno.makeTempDir({ prefix: "leptoskit-play-offline-" });
	const chrome = await launch(profile);
	let page;
	try {
		page = await Page.connect(chrome.endpoint);
		await page.send("Page.enable");
		await page.send("Runtime.enable");
		const loaded = page.once("Page.loadEventFired");
		await page.send("Page.navigate", {
			url: `http://127.0.0.1:${server.addr.port}/play/`,
		});
		await loaded;
		const before = await page.evaluate(
			"document.querySelector('#source').value",
		);
		assert(
			before.includes("view!"),
			"real Rust starter is visible on first paint",
		);
		await page.evaluate(
			"globalThis.fetch = async () => { throw new TypeError('Offline'); }",
		);
		await page.click("#run");
		await page.until(
			"() => document.querySelector('#status').textContent.includes('service unavailable')",
			"offline state",
		);
		assert(
			await page.evaluate("document.querySelector('#source').value") === before,
			"offline failure preserves edits",
		);
		assert(
			await page.evaluate("!document.querySelector('#run').disabled"),
			"Run can retry",
		);
	} finally {
		await page?.close();
		chrome.process.kill();
		await chrome.process.status;
		await server.shutdown();
		await removeProfile(profile);
	}
});
