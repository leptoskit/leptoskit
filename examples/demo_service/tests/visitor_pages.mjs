// The public demo's visitor pages, in headless Chrome against the real
// service: the front door, a real emailed-code sign-in through the demo
// inbox, Notes' welcome, the real admin hand-off, the new-day dialog, lost
// mail after a restart, the page served while a generation builds, and the
// private notebook with its real `note_created` event in the real admin.
//
// Nothing here forges a session or a code: every sign-in goes through the
// pages' own controls and Auth's real routes, and the code is read from the
// private inbox the page draws.
//
// Build the debug service (the debug clock and failure hooks are compiled out
// of release builds), then point the test at it:
//
//   cargo build -p demo_service
//   (cd examples/demo_service/pages && leptoskit build --release --platform web --out <client>)
//   DEMO_SERVICE_BIN=<path to target/debug/demo_service> LEPTOS_SITE_ROOT=<client> \
//   deno test --allow-env --allow-read --allow-net --allow-run --allow-write \
//     examples/demo_service/tests/visitor_pages.mjs
//
// `DEMO_SHOTS=<directory>` also saves screenshots at 1280 and 400 px, in
// light and dark, and an `index.html` gallery of them. `CHROME` names the
// browser executable; see `crates/leptoskit_app/tests/browser.mjs`.

import { assert, Browser, Service } from "./visitor_harness.mjs";

const SHOTS = Deno.env.get("DEMO_SHOTS");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** 2026-11-15T17:48:00Z and the same day's next midnight, plus a little. */
const NOON = 1_794_764_880_000;
const NEXT_DAY = 1_794_787_200_000 + 30_000;
/** 2026-11-15T23:55:56Z: 4 minutes 4 seconds before the reset. */
const LATE = 1_794_787_200_000 - 244_000;

const LOOKS = [
	{ tag: "1280-light", width: 1280, height: 900, dark: false },
	{ tag: "1280-dark", width: 1280, height: 900, dark: true },
	{ tag: "400-light", width: 400, height: 860, dark: false },
	{ tag: "400-dark", width: 400, height: 860, dark: true },
];

const gallery = [];

/** Waits, across navigations, until `expression` is truthy in the page. */
async function waitFor(browser, expression, description, timeout = 30_000) {
	const deadline = Date.now() + timeout;
	for (;;) {
		try {
			const value = await browser.eval(`(() => (${expression}))()`);
			if (value) return value;
		} catch {
			// The page is navigating.
		}
		if (Date.now() > deadline) throw new Error(`timed out: ${description}`);
		await sleep(100);
	}
}

const path = (browser) => browser.eval("location.pathname");

/** Looks at the current page in every look, and keeps what it sees. */
async function capture(browser, name, caption, { looks = LOOKS } = {}) {
	for (const look of looks) {
		await browser.look(look);
		await sleep(250);
		const overflow = await browser.overflowX();
		assert(
			overflow <= 0,
			`${name} (${look.tag}) scrolls sideways by ${overflow}px`,
		);
		const saved = await browser.shot(SHOTS, `${name}-${look.tag}`);
		if (saved) {
			gallery.push({
				name,
				tag: look.tag,
				caption,
				file: `${name}-${look.tag}.png`,
			});
		}
	}
	await browser.look(LOOKS[0]);
}

/** The data a page not yet hydrated hands its script. */
const demoState = (browser) =>
	browser.eval(`JSON.parse(document.getElementById("demo-state").textContent)`);

/** Chooses a persona on the front door with the mouse. */
async function choose(browser, service, persona) {
	await browser.go(`${service.siteOrigin}/`);
	await browser.click(`mint:${persona}`);
	try {
		await waitFor(
			browser,
			`location.pathname === "/sign-in" && document.readyState === "complete"`,
			"sign-in",
		);
	} catch (error) {
		throw new Error(
			`${error.message}; path=${await path(browser)}; notice=${await browser
				.text("mint-notice")}`,
			{ cause: error },
		);
	}
}

/** Sends a code and signs in with it, through the page's own controls. */
async function sendAndSignIn(browser, expectPath) {
	await browser.click("send");
	await waitFor(
		browser,
		`document.querySelector('[data-testid="fill"]')`,
		"the code in the inbox",
	);
	await browser.click("fill");
	assert(
		(await browser.focused()) === "verify",
		"Fill in code moves focus to Sign in",
	);
	await browser.click("verify");
	await waitFor(
		browser,
		`location.pathname === ${JSON.stringify(expectPath)}`,
		`arrival at ${expectPath}`,
	);
}

Deno.test("the visitor pages", async (t) => {
	const clock = await Deno.makeTempFile({ prefix: "demo-clock-" });
	await Deno.writeTextFile(clock, String(NOON));
	const service = await Service.start({
		DEMO_TEST_CLOCK: clock,
		DEMO_TEST_CLOCK_TICK_MS: "200",
	});
	await service.ready();
	const browser = await Browser.open();
	await browser.page.send("Page.addScriptToEvaluateOnNewDocument", {
		// Counts EventSources, so the proofs can say there is never more than one.
		source: `(() => {
			const Native = window.EventSource;
			const tally = (window.__sources = { open: 0, max: 0, made: 0 });
			window.EventSource = class extends Native {
				constructor(...args) {
					super(...args);
					tally.made += 1;
					tally.open += 1;
					tally.max = Math.max(tally.max, tally.open);
					const close = this.close.bind(this);
					let closed = false;
					this.close = () => {
						if (!closed) {
							closed = true;
							tally.open -= 1;
						}
						close();
					};
				}
			};
		})();`,
	});
	await browser.look(LOOKS[0]);

	try {
		await t.step(
			"the front door answers, wears the admin's colours, and asks nothing else",
			async () => {
				await browser.go(`${service.siteOrigin}/`);
				assert(
					/^Resets in 6h 1[12]m/.test(await browser.text("band-reset")),
					"the band counts to the server's midnight",
				);
				assert(await browser.visible("mint:ada"), "Continue as Ada shows");
				assert(await browser.visible("mint:grace"), "Continue as Grace shows");
				const tokens = await browser.eval(
					`getComputedStyle(document.documentElement).getPropertyValue("--admin-ember").trim()`,
				);
				assert(
					tokens === "#e5304f",
					`the admin's ember token is shared (${tokens})`,
				);
				const foreign = browser.requests.filter((url) =>
					!url.startsWith(service.siteOrigin) && !url.startsWith("data:")
				);
				assert(foreign.length === 0, `no request leaves the site: ${foreign}`);
				assert(
					await browser.eval(`!!document.querySelector("meta[name=robots]")`),
					"pages are not indexed",
				);
				await capture(browser, "01-front-door", "Front door, first visit");

				// Without scripts the choice is still a plain form post.
				await browser.page.send("Emulation.setScriptExecutionDisabled", {
					value: true,
				});
				await browser.go(`${service.siteOrigin}/`, { scripts: false });
				assert(
					await browser.eval(
						`!!document.querySelector('noscript') && document.documentElement.innerHTML.includes('action="/demo/pass"')`,
					),
					"a no-script form posts the choice",
				);
				await browser.page.send("Emulation.setScriptExecutionDisabled", {
					value: false,
				});
			},
		);

		await t.step(
			"the band opens, says what is real, and counts down late",
			async () => {
				await browser.go(`${service.siteOrigin}/`);
				assert(
					!(await browser.visible("band-panel")),
					"the panel starts closed",
				);
				await browser.click("band-toggle");
				assert(await browser.visible("band-panel"), "the panel opens");
				assert(
					(await browser.text("band-panel")).includes("Everything resets"),
					"it discloses the reset",
				);
				assert(
					(await browser.text("band-panel")).includes("Infrastructure"),
					"it discloses the stand-in cloud",
				);
				await capture(
					browser,
					"02-front-door-whats-real",
					"What's real? open",
					{ looks: [LOOKS[1], LOOKS[2]] },
				);
				await browser.click("band-toggle");
				assert(!(await browser.visible("band-panel")), "the panel closes");
			},
		);

		await t.step(
			"keyboard: Ada from the front door to the real admin overview",
			async () => {
				await browser.clearCookies();
				await browser.go(`${service.siteOrigin}/`);
				await browser.tabTo("mint:ada");
				await browser.press("Enter");
				await waitFor(
					browser,
					`location.pathname === "/sign-in" && document.readyState === "complete"`,
					"sign-in",
				);
				assert(
					(await demoState(browser)).identity.persona === "ada",
					"the page signs in this browser's own Ada",
				);
				const email = await browser.eval(
					`document.querySelector('[data-testid="email"]').value`,
				);
				assert(
					/^ada-[0-9a-f]{16}@demo\.invalid$/.test(email),
					`a minted address, read-only: ${email}`,
				);
				assert(
					await browser.eval(
						`document.querySelector('[data-testid="email"]').readOnly`,
					),
					"the address is read-only",
				);
				assert(await browser.visible("listening"), "a fresh inbox is empty");
				assert(
					!(await browser.visible("fill")),
					"no code exists before a send",
				);
				await capture(browser, "04-sign-in-ready", "Sign in, ready to send");

				await browser.tabTo("send");
				await browser.press("Enter");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="fill"]')`,
					"the code in the inbox",
				);
				assert(
					(await browser.focused()) === "code",
					"focus moves to the code field",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="code"]').value`,
					)) === "",
					"the code is never put in the form on its own",
				);
				await browser.tabTo("fill");
				await capture(
					browser,
					"05-sign-in-sent",
					"Sign in, code in the demo inbox",
				);
				await browser.press("Enter");
				assert(
					(await browser.focused()) === "verify",
					"Fill in code moves focus to Sign in",
				);
				const typed = await browser.eval(
					`document.querySelector('[data-testid="code"]').value`,
				);
				assert(
					/^[A-Z0-9]{4}-[A-Z0-9]{4}$/.test(typed),
					"the form holds the inbox's code",
				);
				await browser.press("Enter");
				await waitFor(browser, `location.pathname === "/notes"`, "Notes");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="welcome-title"]')`,
					"the welcome",
				);
				assert(
					(await browser.text("welcome-title")) === "Welcome, Ada.",
					"Notes welcomes the real account",
				);
				const link = await browser.eval(
					`(() => { const a = document.querySelector('a[data-testid="open-admin"]'); return { href: a.href, target: a.target, rel: a.rel }; })()`,
				);
				assert(
					link.href.startsWith(service.adminOrigin),
					`the admin link is the configured origin: ${link.href}`,
				);
				assert(
					link.target === "_blank" && link.rel.includes("noopener"),
					"the admin opens in a new tab",
				);
				assert(
					(await browser.text("how-you-got-here")).length > 0,
					"Notes explains the hand-off",
				);
				assert(
					(await browser.text("band-reset")).includes("Resets in"),
					"every Notes page discloses the reset",
				);
				await capture(browser, "09-notes-ada", "Notes, signed in as Ada");

				await browser.go(link.href);
				await browser.click("continue");
				await waitFor(
					browser,
					`location.origin === ${
						JSON.stringify(service.adminOrigin)
					} && !location.pathname.endsWith("/sign-in")`,
					"the admin overview",
				);
				await waitFor(
					browser,
					`document.title.includes("Overview") || document.body.innerText.includes("Overview")`,
					"the overview",
				);
				assert(
					!(await browser.eval(
						`!!document.querySelector('[data-testid="refusal"]')`,
					)),
					"Ada is not refused",
				);
				await capture(browser, "10-admin-overview", "The real admin, as Ada", {
					looks: [LOOKS[0], LOOKS[3]],
				});
			},
		);

		await t.step(
			"Grace is refused by the admin, then switches to Ada through the site",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "grace");
				assert(
					(await demoState(browser)).identity.persona === "grace",
					"Grace is chosen",
				);
				await sendAndSignIn(browser, "/notes");
				assert(
					(await browser.text("welcome-title")) === "Welcome, Grace.",
					"Notes welcomes Grace",
				);
				assert(
					(await browser.text("admin-card")).includes("no admin role"),
					"Grace's card says why",
				);
				await capture(browser, "11-notes-grace", "Notes, signed in as Grace");

				const href = await browser.eval(
					`document.querySelector('a[data-testid="open-admin"]').href`,
				);
				await browser.go(href);
				await browser.click("continue");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="refusal"]')`,
					"the admin's refusal",
				);
				await capture(browser, "12-admin-refusal", "The admin refuses Grace", {
					looks: [LOOKS[0], LOOKS[2]],
				});
				await browser.click("switch-account");
				await waitFor(
					browser,
					`location.origin === ${
						JSON.stringify(service.siteOrigin)
					} && location.pathname === "/sign-in"`,
					"back on the site",
				);
				await waitFor(browser, `document.readyState === "complete"`, "loaded");
				const state = await demoState(browser);
				assert(
					state.flow === "switch",
					"the page knows the admin asked who is signing in",
				);
				assert(
					state.returnTo.startsWith("/oauth/authorize?"),
					"the admin's request travels as the return target",
				);
				assert(await browser.visible("pane-choose"), "the choice shows");
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="choose:ada"]').getAttribute("aria-checked")`,
					)) === "true",
					"Ada is preselected",
				);
				assert(
					(await browser.text("flow-line")).includes("Grace"),
					"the page says who is signed in now",
				);
				await capture(
					browser,
					"08-sign-in-switch",
					"Sign in, switching from Grace",
				);

				await browser.click("choose-go");
				await waitFor(
					browser,
					`document.documentElement.dataset.state === "ready" && document.readyState === "complete"`,
					"Ada ready",
				);
				assert(
					(await demoState(browser)).identity.persona === "ada",
					"now signing in as Ada",
				);
				await browser.click("send");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="fill"]')`,
					"the code",
				);
				await browser.click("fill");
				await browser.click("verify");
				await waitFor(
					browser,
					`location.origin === ${
						JSON.stringify(service.adminOrigin)
					} && !location.pathname.endsWith("/sign-in")`,
					"the admin overview",
				);
				assert(
					!(await browser.eval(
						`!!document.querySelector('[data-testid="refusal"]')`,
					)),
					"Ada gets in",
				);
			},
		);

		await t.step(
			"return_to stays a local path through choice, swap, and verification",
			async () => {
				await browser.clearCookies();
				for (
					const bad of [
						"//evil.example/x",
						"/%5Cevil.example",
						"https://evil.example/",
						"/admin",
						"javascript:alert(1)",
					]
				) {
					await browser.go(
						`${service.siteOrigin}/sign-in?return_to=${
							encodeURIComponent(bad)
						}`,
					);
					assert(
						(await demoState(browser)).returnTo === "/notes",
						`${bad} falls back to /notes`,
					);
					await browser.go(
						`${service.siteOrigin}/?return_to=${encodeURIComponent(bad)}`,
					);
					// The hydrated front door keeps its target in the plain form a
					// visitor without scripts posts.
					assert(
						await browser.eval(
							`document.querySelector("noscript").textContent.includes('name="return_to" value="/notes"')`,
						),
						`${bad} falls back on the front door`,
					);
				}
				const kept = "/oauth/authorize?client_id=x&scope=openid+roles";
				await browser.go(
					`${service.siteOrigin}/sign-in?return_to=${encodeURIComponent(kept)}`,
				);
				assert(
					(await demoState(browser)).returnTo === kept,
					"a legitimate authorize request is kept",
				);
				assert(
					(await demoState(browser)).flow === "admin",
					"and the page says the admin sent them",
				);
				await browser.click("choose:grace");
				await browser.click("choose-go");
				await waitFor(
					browser,
					`document.readyState === "complete" && location.pathname === "/sign-in" && document.documentElement.dataset.state === "ready"`,
					"ready",
				);
				assert(
					(await demoState(browser)).returnTo === kept,
					"the choice keeps it",
				);
				await browser.click("swap");
				await waitFor(
					browser,
					`document.readyState === "complete" && document.querySelector('[data-testid="who-name"]')?.textContent === "Ada"`,
					"swapped",
				);
				assert((await demoState(browser)).returnTo === kept, "a swap keeps it");
				assert(
					(await browser.text("flow-line")).includes(
						"straight back to the Notes admin",
					),
					"the page says where they go next",
				);
				await capture(
					browser,
					"07-sign-in-from-admin",
					"Sign in, arriving from the admin",
					{ looks: [LOOKS[0], LOOKS[2]] },
				);
			},
		);

		await t.step(
			"a wrong code is announced, marked, and recoverable",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				await browser.click("send");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="fill"]')`,
					"the code",
				);
				await browser.click("code");
				await browser.type("ZZZZ-ZZZZ");
				await browser.click("verify");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="notice-alert"][data-shown]')`,
					"the alert",
				);
				assert(
					(await browser.text("notice-alert")).includes(
						"That code didn't work",
					),
					"the failure is named",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="notice-alert"]').getAttribute("role")`,
					)) === "alert",
					"it is announced",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="code"]').getAttribute("aria-invalid")`,
					)) === "true",
					"the field is marked invalid",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="code"]').getAttribute("aria-describedby")`,
					)).includes("code-error"),
					"the error is linked to the field",
				);
				assert(
					(await path(browser)) === "/sign-in",
					"the visitor stays on sign-in",
				);
				await capture(browser, "06-sign-in-wrong-code", "Sign in, wrong code");
				// The real code still works afterwards.
				await browser.click("fill");
				await browser.click("verify");
				await waitFor(
					browser,
					`location.pathname === "/notes"`,
					"Notes after recovering",
				);
			},
		);

		await t.step(
			"the stations are one tab stop, with arrows, Home, and End",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				const stops = await browser.eval(
					`[...document.querySelectorAll('[role="tab"]')].filter((tab) => tab.tabIndex === 0).length`,
				);
				assert(stops === 1, `one tab stop, not ${stops}`);
				await browser.tabTo("station:send");
				await browser.press("ArrowRight");
				assert(
					(await browser.focused()) === "station:mail",
					"the right arrow moves on",
				);
				await browser.press("End");
				assert(
					(await browser.focused()) === "station:session",
					"End goes to the last",
				);
				await browser.press("ArrowRight");
				assert((await browser.focused()) === "station:send", "the arrows wrap");
				await browser.press("ArrowLeft");
				await browser.press("Home");
				assert(
					(await browser.focused()) === "station:send",
					"Home goes to the first",
				);
				assert(
					(await browser.text("explain-sig")).startsWith(
						"POST /auth/email/send",
					),
					"a station names its real call",
				);
			},
		);

		await t.step(
			"lost mail after a same-day restart is named, and a resend works",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				await browser.click("send");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="fill"]')`,
					"the code",
				);
				const old = await browser.eval(
					`document.querySelector('[data-testid="fill"]').dataset.code`,
				);
				await service.restart();
				await browser.go(`${service.siteOrigin}/sign-in?as=ada`);
				await waitFor(
					browser,
					`document.documentElement.dataset.state === "lost"`,
					"the lost state",
				);
				await waitFor(
					browser,
					`document.querySelector('.mail.gone')`,
					"the tombstone",
				);
				assert(!(await browser.visible("fill")), "no stale code is offered");
				assert(
					!(await browser.text("mailbox")).includes(old),
					"the lost code is not shown as if it were current",
				);
				assert(
					(await browser.text("mailbox")).includes("no longer has it"),
					"it says the server no longer has it",
				);
				assert(
					!(await browser.text("mailbox")).includes("release took over"),
					"it does not claim a deploy happened",
				);
				await capture(
					browser,
					"07-sign-in-lost",
					"Sign in, code lost after a restart",
				);
				await browser.click("send");
				await waitFor(
					browser,
					`document.documentElement.dataset.state === "sent" && document.querySelector('[data-testid="fill"]')`,
					"a fresh code",
				);
				await browser.click("fill");
				await browser.click("verify");
				await waitFor(
					browser,
					`location.pathname === "/notes"`,
					"Notes after the resend",
				);
			},
		);

		await t.step(
			"a restart between send and inbox read preserves the lost-mail explanation",
			async () => {
				// Reuse the pass with its other persona: a fresh inbox without
				// another pass or the preceding persona's resend cooldown.
				await browser.click("menu-toggle");
				await browser.click("sign-out");
				await waitFor(browser, `location.pathname === "/"`, "signed out");
				await choose(browser, service, "grace");
				const paused = browser.page.once("Fetch.requestPaused");
				await browser.page.send("Fetch.enable", {
					patterns: [{ urlPattern: "*/demo/inbox", requestStage: "Request" }],
				});
				let timeout;
				try {
					await browser.click("send");
					const request = await Promise.race([
						paused,
						new Promise((_, reject) => {
							timeout = setTimeout(
								() => reject(new Error("inbox request did not arrive")),
								10_000,
							);
						}),
					]);
					clearTimeout(timeout);
					await service.restart();
					await browser.page.send("Fetch.continueRequest", {
						requestId: request.requestId,
					});
					await waitFor(
						browser,
						`document.documentElement.dataset.state === "lost"`,
						"lost mail after sending",
					);
					assert(
						!(await browser.visible("notice-alert")),
						"lost mail is not replaced by a delivery-pending error",
					);
					assert(
						(await browser.text("notice-status")).includes(
							"last code isn't here",
						),
						"the durable receipt explains the missing message",
					);
					assert(
						!(await browser.visible("fill")),
						"there is no unavailable code to fill",
					);
				} finally {
					clearTimeout(timeout);
					await browser.page.send("Fetch.disable");
				}
			},
		);

		await t.step(
			"pass states: a pass that is gone says so and offers a way back",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				// The pass disappears under an open page (cleared cookies).
				await browser.clearCookies();
				await browser.click("send");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="notice-alert"][data-shown]')`,
					"the notice",
				);
				assert(
					(await browser.text("notice-alert")).includes("pass is gone"),
					"it says the pass is gone",
				);
				assert(
					await browser.eval(`!!document.querySelector('.msg-link')`),
					"and offers a way to start again",
				);
				await capture(
					browser,
					"06-sign-in-pass-gone",
					"Sign in, the pass is gone",
					{ looks: [LOOKS[0], LOOKS[2]] },
				);
			},
		);

		await t.step(
			"signed out, the pass stays: the front door welcomes the same person back",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				await sendAndSignIn(browser, "/notes");
				await browser.click("menu-toggle");
				assert(await browser.visible("menu-panel"), "the account menu opens");
				assert(
					(await browser.focused()) === "open-admin",
					"focus enters the menu",
				);
				await capture(browser, "09-notes-menu", "The account menu", {
					looks: [LOOKS[0], LOOKS[2]],
				});
				await browser.press("Escape");
				assert(!(await browser.visible("menu-panel")), "Escape closes it");
				assert(
					(await browser.focused()) === "menu-toggle",
					"and returns focus to the button",
				);
				await browser.click("menu-toggle");
				const errorsBefore = browser.errors.length;
				await service.stop();
				try {
					await browser.click("sign-out");
					await waitFor(
						browser,
						`document.querySelector('.toast')`,
						"the failed sign-out explanation",
						5000,
					);
					assert(
						await browser.eval(
							`!document.querySelector('[data-testid="sign-out"]').disabled`,
						),
						"sign out can be retried after a connection failure",
					);
					assert(
						(await path(browser)) === "/notes",
						"a failed sign-out keeps the current page",
					);
					assert(
						browser.errors.length === errorsBefore,
						"the failure handler does not throw",
					);
				} finally {
					await service.spawn();
					await service.ready();
				}
				await browser.click("sign-out");
				await waitFor(
					browser,
					`location.pathname === "/" && document.readyState === "complete"`,
					"the front door",
				);
				assert(
					await browser.visible("notice-welcome-back"),
					"the same person is offered back",
				);
				assert(
					(await browser.text("notice-welcome-back")).includes(
						"Signed out of Notes",
					),
					"it says so",
				);
				await capture(
					browser,
					"03-front-door-signed-out",
					"Front door, signed out of Notes",
					{ looks: [LOOKS[0], LOOKS[3]] },
				);
				// Anonymous Notes goes to sign-in, and back to Notes after.
				await browser.go(`${service.siteOrigin}/notes`);
				await waitFor(
					browser,
					`location.pathname === "/sign-in"`,
					"redirect to sign-in",
				);
				assert(
					(await demoState(browser)).returnTo === "/notes",
					"and back to Notes after",
				);
			},
		);

		await t.step(
			"a late page warns, and reduced motion stops the motion",
			async () => {
				await Deno.writeTextFile(clock, String(LATE));
				await sleep(800);
				await service.ready();
				await browser.clearCookies();
				await browser.go(`${service.siteOrigin}/`);
				await waitFor(
					browser,
					`document.querySelector('[data-testid="band-reset"]')`,
					"the band",
				);
				const late = await browser.text("band-reset");
				assert(late.startsWith("! Resets in"), `the countdown warns: ${late}`);
				assert(
					(await browser.text("band-late")).includes("cleared then"),
					"and says what is lost",
				);
				assert(
					(await browser.eval(
						`getComputedStyle(document.querySelector('[data-testid="band-reset"]')).color`,
					)) !== "rgb(244, 239, 232)",
					"in the warning colour",
				);
				await capture(browser, "13-front-door-late", "Ten minutes to reset", {
					looks: [LOOKS[0], LOOKS[2]],
				});

				await choose(browser, service, "ada");
				await browser.look({ ...LOOKS[0], reducedMotion: true });
				const motion = await browser.eval(
					`(() => { const p = document.querySelector('[data-testid="pulse"]'); return p ? getComputedStyle(p).animationName : "absent"; })()`,
				);
				assert(
					motion === "none" || motion === "absent",
					`reduced motion stops the pulse: ${motion}`,
				);
				await browser.look(LOOKS[0]);
				await Deno.writeTextFile(clock, String(NOON));
				await sleep(800);
				await service.ready();
			},
		);

		await t.step(
			"a new day opens a focused dialog once, and the page never holds two streams",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "ada");
				await sendAndSignIn(browser, "/notes");
				await waitFor(browser, `window.__sources.made >= 1`, "the stream");
				await browser.click("menu-toggle");
				await browser.press("Escape");
				// A second visitor waits on the hydrated front door, which hears
				// the new day over its live connection.
				const door = await Browser.open();
				await door.look(LOOKS[0]);
				await door.go(`${service.siteOrigin}/`);
				await Deno.writeTextFile(clock, String(NEXT_DAY));
				try {
					await waitFor(
						door,
						`document.querySelector('dialog[data-testid="reset-dialog"]')?.open`,
						"the front door's new-day dialog",
						90_000,
					);
					assert(
						(await door.eval(
							`document.querySelector('[data-testid="reset-dialog"]').getAttribute("role")`,
						)) === "alertdialog",
						"the front door's dialog is an alert dialog",
					);
					try {
						await waitFor(
							door,
							`document.activeElement?.dataset.testid === "reset-title"`,
							"focus moves to the front door's dialog title",
							5_000,
						);
					} catch (error) {
						throw new Error(
							`${error.message}; focus is on ${await door.eval(
								`document.activeElement?.outerHTML.slice(0, 200)`,
							)}`,
							{ cause: error },
						);
					}
					await capture(
						door,
						"15-front-door-reset-dialog",
						"The demo just reset, on the front door",
						{ looks: [LOOKS[0], LOOKS[3]] },
					);
					await door.press("Escape");
					assert(
						!(await door.eval(
							`document.querySelector('dialog[data-testid="reset-dialog"]')?.open ?? false`,
						)),
						"Escape closes the front door's dialog",
					);
					await sleep(3000);
					assert(
						!(await door.eval(
							`document.querySelector('dialog[data-testid="reset-dialog"]')?.open ?? false`,
						)),
						"and it does not reopen by itself",
					);
				} finally {
					await door.close();
				}
				await waitFor(
					browser,
					`document.documentElement.hasAttribute("data-reset")`,
					"the new-day dialog",
					90_000,
				);
				assert(
					(await browser.focused()) === "reset-title",
					"focus moves into the dialog",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="reset-dialog"]').getAttribute("role")`,
					)) === "alertdialog",
					"it is an alert dialog",
				);
				const sources = await browser.eval(`window.__sources`);
				assert(
					sources.max <= 1,
					`never two streams at once (${JSON.stringify(sources)})`,
				);
				assert(
					sources.made <= 12,
					`bounded reconnects (${JSON.stringify(sources)})`,
				);
				assert(
					await browser.eval(
						`document.querySelector('[data-testid="band"]').inert`,
					),
					"the page behind is inert",
				);
				await capture(browser, "14-notes-reset-dialog", "The demo just reset", {
					looks: [LOOKS[0], LOOKS[3]],
				});
				await browser.press("Tab");
				assert(
					(await browser.focused()) === "reset-start",
					"Tab stays in the dialog",
				);
				await browser.press("Tab");
				assert(
					(await browser.focused()) === "reset-keep",
					"Tab reaches the last control",
				);
				await browser.press("Tab");
				assert((await browser.focused()) === "reset-start", "and wraps");
				await browser.shiftTab();
				assert(
					(await browser.focused()) === "reset-keep",
					"Shift+Tab wraps back",
				);
				await browser.press("Escape");
				assert(
					!(await browser.eval(
						`document.documentElement.hasAttribute("data-reset")`,
					)),
					"Escape closes it",
				);
				assert(
					!(await browser.eval(
						`document.querySelector('[data-testid="band"]').inert`,
					)),
					"the page is live again",
				);
				await sleep(3000);
				assert(
					!(await browser.eval(
						`document.documentElement.hasAttribute("data-reset")`,
					)),
					"it does not reopen by itself",
				);

				// The visitor's way on is the front door; the old pass is a new-day notice.
				await browser.go(`${service.siteOrigin}/`);
				assert(
					await browser.visible("notice-new-day"),
					"yesterday's pass is explained, not an error",
				);
				await capture(
					browser,
					"03-front-door-new-day",
					"Front door, a new demo day",
					{ looks: [LOOKS[0], LOOKS[2]] },
				);
				// Old sessions fail through the server: Notes sends them to sign in.
				await browser.go(`${service.siteOrigin}/notes`);
				await waitFor(
					browser,
					`location.pathname === "/sign-in"`,
					"an old session fails",
				);
				await Deno.writeTextFile(clock, String(NOON));
			},
		);
	} finally {
		await browser.close();
		await service.close();
	}

	if (SHOTS) await writeGallery(gallery);
});

Deno.test("while a generation builds, the page says so and needs no script", async () => {
	const failing = await Deno.makeTempFile({ prefix: "demo-fail-" });
	const service = await Service.start({
		DEMO_TEST_FAIL_INIT: failing,
		DEMO_TEST_RETRY_MS: "1500",
	});
	const browser = await Browser.open();
	try {
		await service.until("/healthz", (status) => status === 200);
		const response = await fetch(`http://127.0.0.1:${service.sitePort}/`);
		const html = await response.text();
		assert(
			response.status === 503,
			`a page answers 503, not ${response.status}`,
		);
		assert(response.headers.get("retry-after") === "5", "with Retry-After: 5");
		assert(
			response.headers.get("cache-control") === "no-store",
			"never stored",
		);
		assert(
			html.includes('http-equiv="refresh" content="5"'),
			"and refreshes itself",
		);
		assert(!html.includes("<script"), "it carries no script");
		assert(
			!/https?:\/\/(?!www\.w3\.org)/.test(html.replace(/http-equiv/g, "")),
			"and no address to fetch",
		);
		const ready = await fetch(`http://127.0.0.1:${service.sitePort}/readyz`);
		await ready.body?.cancel();
		assert(ready.status === 503, "readiness still says not ready");
		const health = await fetch(`http://127.0.0.1:${service.sitePort}/healthz`);
		await health.body?.cancel();
		assert(health.status === 200, "health still says alive");

		await browser.page.send("Emulation.setScriptExecutionDisabled", {
			value: true,
		});
		await browser.look(LOOKS[0]);
		await browser.go(`http://localhost:${service.sitePort}/`);
		const heading = await browser.eval(
			`document.querySelector("h1")?.textContent`,
		);
		assert(
			heading === "Notes is opening today's demo.",
			`the heading: ${heading}`,
		);
		assert(
			!(await browser.eval(`document.body.innerText`)).includes("reset"),
			"it does not claim a reset happened",
		);
		await capture(browser, "15-opening", "Opening today's demo (503)");
		await browser.look({ ...LOOKS[0], reducedMotion: true });
		const animation = await browser.eval(
			`getComputedStyle(document.querySelector('[data-testid="assemble-1"]')).animationName`,
		);
		assert(
			animation === "none",
			`reduced motion shows the finished mark: ${animation}`,
		);
		await browser.page.send("Emulation.setScriptExecutionDisabled", {
			value: false,
		});

		// Lift the failure: the next bounded retry opens the day.
		await Deno.remove(failing);
		await service.ready(60_000);
		const open = await fetch(`http://127.0.0.1:${service.sitePort}/`, {
			headers: { host: `localhost:${service.sitePort}` },
		});
		await open.body?.cancel();
	} finally {
		await browser.close();
		await service.close();
	}
	if (SHOTS) await writeGallery(gallery);
});

/** The escaped-markup note: every character here must stay text. */
const MARKUP_NOTE =
	`<img src=x onerror="window.__pwned=1"> & "quotes" <b>hi</b>`;

/** The page's own record of what `/demo/notes` answered, newest last. */
const answers = (browser) => browser.eval(`window.__notes`);

/** Waits until the nth `/demo/notes` POST answer has arrived. */
async function postAnswer(browser, nth) {
	await waitFor(
		browser,
		`window.__notes.filter((n) => n.method === "POST").length >= ${nth}`,
		`POST /demo/notes answer #${nth}`,
	);
	const posts = (await answers(browser)).filter((n) => n.method === "POST");
	return { ...posts[nth - 1], json: JSON.parse(posts[nth - 1].body || "null") };
}

Deno.test("a visitor writes a private note and finds its real event in the admin", async (t) => {
	// A volume of its own: this journey mints two passes, and the real
	// hourly limit is per network address.
	const service = await Service.start();
	await service.ready();
	const browser = await Browser.open();
	await browser.page.send("Page.addScriptToEvaluateOnNewDocument", {
		// Records every real answer `/demo/notes` gives the page.
		source: `(() => {
			const Native = window.fetch.bind(window);
			window.__notes = [];
			window.fetch = async (input, init) => {
				const url = new URL(typeof input === "string" ? input : input.url, location.href);
				const method = (init && init.method) || "GET";
				if (url.pathname !== "/demo/notes") return Native(input, init);
				try {
					const response = await Native(input, init);
					const copy = response.clone();
					copy.text().then((body) => window.__notes.push({
						method,
						status: response.status,
						cacheControl: response.headers.get("cache-control"),
						body,
					}));
					return response;
				} catch (error) {
					window.__notes.push({ method, status: 0, cacheControl: null, body: "" });
					throw error;
				}
			};
		})();`,
	});
	await browser.look(LOOKS[0]);
	const field = 'textarea[data-testid="note-text"]';
	let actor = null;
	let liveHref = null;

	try {
		await t.step(
			"Ada signs in and finds an empty, labelled notebook",
			async () => {
				await choose(browser, service, "ada");
				await sendAndSignIn(browser, "/notes");
				await waitFor(
					browser,
					`document.querySelector('${field}')`,
					"the textarea",
				);
				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-empty"]')?.textContent.startsWith("Nothing saved yet")`,
					"the empty notebook",
				);
				assert(
					(await browser.text("notes-count")) === "0 of 20 notes",
					"the count is out of twenty",
				);
				assert(
					(await browser.eval(
								`document.querySelector('${field}').getAttribute("aria-labelledby")`,
							)) === "note-label" &&
						(await browser.eval(
								`document.getElementById("note-label").textContent`,
							)) ===
							"Write a note",
					"the field is labelled",
				);
				assert(
					(await browser.text("note-chars")) === "0 / 280",
					"the 280-character limit is visible",
				);
				assert(
					!(await browser.visible("note-saved")) &&
						!(await browser.visible("note-alert")),
					"no result shows before a save",
				);
				assert(
					(await browser.text("welcome-title")) === "Welcome, Ada.",
					"the welcome and the account menu stay",
				);
				assert(await browser.visible("menu-toggle"), "the account menu stays");
				liveHref = await browser.eval(
					`document.querySelector('[data-testid="open-admin-live"]').href`,
				);
				assert(
					liveHref === `${service.adminOrigin}/_leptoskit/admin/analytics/live`,
					`Live is the configured admin's: ${liveHref}`,
				);
				await capture(
					browser,
					"16-notebook-empty",
					"Notebook, nothing saved yet",
				);
			},
		);

		await t.step(
			"the keyboard saves; markup stays text; the real answer says analytics heard it",
			async () => {
				await browser.click("note-text");
				await browser.type(MARKUP_NOTE);
				assert(
					(await browser.text("note-chars")) ===
						`${[...MARKUP_NOTE].length} / 280`,
					"the count follows the text",
				);
				await browser.chord("Enter", 2);
				const saved = await postAnswer(browser, 1);
				assert(saved.status === 201, `created, not ${saved.status}`);
				assert(
					saved.cacheControl === "no-store",
					`never stored: ${saved.cacheControl}`,
				);
				assert(
					saved.json.analytics === "confirmed",
					`confirmed: ${saved.body}`,
				);
				assert(saved.json.words === 6, `six words: ${saved.json.words}`);
				assert(
					saved.json.note.text === MARKUP_NOTE,
					"the text came back whole",
				);
				actor = saved.json.actor;
				assert(
					/^v-[0-9a-f]{64}$/.test(actor),
					`a pseudonymous actor: ${actor}`,
				);

				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-saved"][data-shown]')`,
					"the confirmed result",
				);
				assert(
					(await browser.text("note-actor")) === actor,
					"the page shows the exact actor",
				);
				assert(
					(await browser.text("note-saved")).includes("note_created"),
					"it names the real event",
				);
				assert(
					(await browser.focused()) === "note-text",
					"focus stays in the field for the next note",
				);
				assert(
					(await browser.eval(`document.querySelector('${field}').value`)) ===
						"",
					"the field is cleared after a save",
				);
				const item = await browser.eval(`(() => {
					const note = document.querySelector('[data-testid="note-list"] [data-testid="note"] p');
					return { text: note.textContent, children: note.children.length,
						pwned: window.__pwned === undefined,
						images: document.querySelectorAll('[data-testid="note-list"] img, [data-testid="note-list"] b').length };
				})()`);
				assert(
					item.text === MARKUP_NOTE && item.children === 0 && item.images === 0,
					`the note is text, not markup: ${JSON.stringify(item)}`,
				);
				assert(item.pwned, "no script from a note ever ran");
				assert(
					(await browser.text("notes-count")) === "1 of 20 notes",
					"the count follows",
				);
				const said = await browser.eval(
					`[...document.querySelectorAll('[role="status"].sr-only')].map((e) => e.textContent).join("|")`,
				);
				assert(
					said.includes(
						"Note saved. Its note_created event reached analytics.",
					),
					`the result is announced once: ${said}`,
				);
				await capture(
					browser,
					"17-notebook-saved",
					"Notebook, first note saved and confirmed",
				);
			},
		);

		await t.step(
			"the second note is saved from the Save button, by keyboard",
			async () => {
				await browser.type("second note, plain");
				await browser.tabTo("note-save");
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="note-save"]').disabled`,
					)) === false,
					"Save is a real, enabled button",
				);
				await browser.press("Enter");
				const saved = await postAnswer(browser, 2);
				assert(saved.status === 201 && saved.json.words === 3, `${saved.body}`);
				assert(saved.json.actor === actor, "the same person, the same actor");
				await waitFor(
					browser,
					`document.querySelectorAll('[data-testid="note-list"] [data-testid="note"]').length === 2`,
					"two notes",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="note-list"] [data-testid="note"] p').textContent`,
					)) === "second note, plain",
					"the newest is first",
				);
				assert(
					(await browser.focused()) === "note-save",
					"focus returns to the button that was used",
				);
			},
		);

		await t.step(
			"a reload keeps the notes, and nothing sends an event again",
			async () => {
				const before = await browser.eval(`window.__notes.length`);
				await browser.go(`${service.siteOrigin}/notes`);
				await waitFor(
					browser,
					`document.querySelectorAll('[data-testid="note-list"] [data-testid="note"]').length === 2`,
					"both notes after a reload",
				);
				const reread = (await answers(browser)).filter((n) =>
					n.method === "GET"
				);
				assert(
					reread.length === 1 && reread[0].status === 200,
					"one read, answered",
				);
				assert(
					JSON.parse(reread[0].body).actor === actor,
					"the same actor after the reload",
				);
				assert(before >= 2, "the earlier saves were answered");
				assert(
					(await browser.text("notes-count")) === "2 of 20 notes",
					"the count survives",
				);
				await capture(
					browser,
					"18-notebook-two-notes",
					"Notebook after a reload, two notes",
				);
			},
		);

		await t.step(
			"an empty or too-long note is a field error, announced, with no request",
			async () => {
				const posts = () =>
					browser.eval(
						`window.__notes.filter((n) => n.method === "POST").length`,
					);
				await browser.click("note-save");
				assert(
					(await browser.text("note-error")) === "Write something first.",
					"empty is named at the field",
				);
				assert(
					(await browser.eval(
								`document.querySelector('${field}').getAttribute("aria-invalid")`,
							)) === "true" &&
						(await browser.eval(
							`document.querySelector('${field}').getAttribute("aria-describedby")`,
						)).includes("note-error"),
					"the error is tied to the field",
				);
				assert(
					(await browser.focused()) === "note-text",
					"focus goes to the field",
				);
				assert((await posts()) === 0, "no request for an empty note");

				await browser.type("x".repeat(285));
				assert(
					await browser.eval(
						`document.querySelector('[data-testid="note-chars"]').hasAttribute("data-over")`,
					),
					"the count turns over the limit",
				);
				await browser.chord("Enter", 2);
				assert(
					(await browser.text("note-error")).includes(
						"5 characters over the 280 limit",
					),
					`it says by how much: ${await browser.text("note-error")}`,
				);
				assert((await posts()) === 0, "no request for a too-long note");
				await capture(
					browser,
					"19-notebook-too-long",
					"Notebook, a note over the limit",
				);
				await browser.eval(`(() => {
				const area = document.querySelector('${field}');
				area.value = "";
				area.dispatchEvent(new Event("input", { bubbles: true }));
			})()`);
				assert(
					(await browser.text("note-error")) === "",
					"editing clears the error",
				);
			},
		);

		await t.step(
			"the real admin's Live shows the matching note_created event",
			async () => {
				await browser.go(liveHref);
				await waitFor(
					browser,
					`document.querySelector('[data-testid="continue"]')`,
					"the admin's sign-in card",
				);
				await browser.click("continue");
				await waitFor(
					browser,
					`location.origin === ${
						JSON.stringify(service.adminOrigin)
					} && !location.pathname.endsWith("/sign-in")`,
					"the admin",
				);
				await browser.go(liveHref);
				await waitFor(
					browser,
					`document.querySelector('[data-testid="live-events"]')`,
					"the Live page",
				);
				// Live is "a few seconds after they happen": reload until the
				// admin's own list has caught up, and say how many tries it took.
				let rows = [];
				let tries = 0;
				for (; tries < 8 && rows.length < 2; tries++) {
					if (tries > 0) {
						await sleep(1500);
						await browser.go(liveHref);
						await waitFor(
							browser,
							`document.querySelector('[data-testid="live-events"]')`,
							"the Live page again",
						);
					}
					rows = await browser.eval(
						`[...document.querySelectorAll('[data-testid="live-event"]')]
				.map((row) => ({
					name: row.getAttribute("aria-description"),
					label: row.querySelector('[data-testid="how-toggle"]')?.getAttribute("aria-label") ?? "",
				}))
				.filter((row) => row.label.includes(${JSON.stringify(actor)}))`,
					);
				}
				console.log(`Live listed the visitor's events after ${tries} load(s)`);
				assert(
					rows.length === 2 && rows.every((row) => row.name === "note_created"),
					`Live shows exactly the visitor's two real events: ${
						JSON.stringify(rows)
					}`,
				);
				await browser.eval(`(() => {
				const filter = document.querySelector('[data-testid="live-filter"]');
				filter.value = "note_created";
				filter.dispatchEvent(new Event("input", { bubbles: true }));
			})()`);
				assert(
					await browser.eval(
						`[...document.querySelectorAll('[data-testid="live-event"]:not([data-filtered])')].every((row) => row.getAttribute("aria-description") === "note_created")`,
					),
					"the name filter the page offers narrows to the event",
				);
				await capture(
					browser,
					"20-admin-live-note",
					"The real admin's Live, with the visitor's event",
					{
						looks: [LOOKS[0], LOOKS[3]],
					},
				);
			},
		);

		await t.step(
			"a failed save keeps the text, says so, and recovers",
			async () => {
				await browser.go(`${service.siteOrigin}/notes`);
				await waitFor(
					browser,
					`document.querySelectorAll('[data-testid="note-list"] [data-testid="note"]').length === 2`,
					"the notebook",
				);
				await browser.click("note-text");
				await browser.type("written while the server is down");
				await service.stop();
				await browser.click("note-save");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-alert"][data-shown]')`,
					"the failure",
				);
				assert(
					(await browser.text("note-alert")).includes("Couldn't reach Notes"),
					"the failure is named",
				);
				assert(
					(await browser.eval(
						`document.querySelector('${field}').value`,
					)) === "written while the server is down",
					"the typed text is kept",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="note-save"]').disabled`,
					)) === false,
					"the button is usable again",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="note-alert"]').getAttribute("role")`,
					)) === "alert",
					"the failure is announced by its own alert",
				);
				assert(
					(await browser.text("notes-count")) === "2 of 20 notes",
					"the notes already on screen stay",
				);
				await capture(
					browser,
					"21-notebook-failed",
					"Notebook, the server could not be reached",
					{
						looks: [LOOKS[0], LOOKS[2]],
					},
				);

				await service.spawn();
				await service.ready();
				await browser.click("note-save");
				// The refused attempt was this page's first POST; the retry is its second.
				const retried = await postAnswer(browser, 2);
				assert(
					retried.status === 201 &&
						retried.json.note.text === "written while the server is down",
					`the retried note is saved after the restart: ${retried.body}`,
				);
				await waitFor(
					browser,
					`!document.querySelector('[data-testid="note-alert"][data-shown]') && document.querySelector('[data-testid="note-saved"][data-shown]')`,
					"the failure gives way to the result",
				);
			},
		);

		await t.step(
			"a full notebook says so, and a save into it is refused cleanly",
			async () => {
				// Fill all but one place through the real route, as a second tab would.
				await browser.eval(`(async () => {
				for (let index = 0; index < 16; index++) {
					const response = await fetch("/demo/notes", {
						method: "POST",
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify({ text: "filler " + index }),
						credentials: "same-origin",
					});
					if (response.status !== 201) throw new Error("filler " + response.status);
				}
			})()`);
				await browser.go(`${service.siteOrigin}/notes`);
				await waitFor(
					browser,
					`document.querySelectorAll('[data-testid="note-list"] [data-testid="note"]').length === 19`,
					"nineteen notes",
				);
				assert(
					(await browser.text("notes-count")) === "19 of 20 notes",
					"19 of 20",
				);
				await browser.click("note-text");
				await browser.type("the twentieth, in this tab");
				// Another tab takes the last place first.
				await browser.eval(`fetch("/demo/notes", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ text: "taken elsewhere" }),
				credentials: "same-origin",
			}).then((r) => r.status)`);
				await browser.click("note-save");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-alert"][data-shown]')`,
					"the full notebook's alert",
				);
				assert(
					(await browser.text("note-alert")).includes("Your notebook is full"),
					"the 409 is named",
				);
				await waitFor(
					browser,
					`document.querySelector('${field}').disabled`,
					"the field closes",
				);
				assert(
					(await browser.eval(`document.querySelector('${field}').value`)) ===
						"the twentieth, in this tab",
					"what was typed is not thrown away",
				);
				assert(
					(await browser.text("notes-count")) === "20 of 20 notes",
					"20 of 20",
				);
				assert(
					(await browser.text("note-error")).includes("notebook is full"),
					"the full state is explained at the field",
				);
				await capture(
					browser,
					"22-notebook-full",
					"Notebook, twenty of twenty",
				);
			},
		);

		await t.step(
			"another visitor cannot see the note, and has an actor of their own",
			async () => {
				await browser.clearCookies();
				await choose(browser, service, "grace");
				await sendAndSignIn(browser, "/notes");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-empty"]')?.textContent.startsWith("Nothing saved yet")`,
					"Grace's own, empty notebook",
				);
				assert(
					(await browser.text("notes-count")) === "0 of 20 notes",
					"Grace starts empty",
				);
				const graces = (await answers(browser)).filter((n) =>
					n.method === "GET"
				).pop();
				assert(
					graces.status === 200 &&
						!graces.body.includes("second note") &&
						!graces.body.includes("filler") &&
						!graces.body.includes(actor),
					`Grace's list holds nothing of Ada's: ${graces.body}`,
				);

				await browser.click("note-text");
				await browser.type("grace writes too");
				await browser.chord("Enter", 2);
				const saved = await postAnswer(browser, 1);
				assert(saved.json.analytics === "confirmed", `${saved.body}`);
				assert(
					saved.json.actor !== actor,
					"a different person has a different actor",
				);
				assert(
					(await browser.text("note-saved")).includes(
						"Grace has no admin role",
					),
					"Grace is told Live will turn her away",
				);
				await capture(browser, "23-notebook-grace", "Grace's notebook, saved", {
					looks: [LOOKS[0], LOOKS[2], LOOKS[3]],
				});
			},
		);

		await t.step(
			"reduced motion shows a new note without animating it",
			async () => {
				await browser.look({ ...LOOKS[0], reducedMotion: true });
				await browser.click("note-text");
				await browser.type("calm note");
				await browser.chord("Enter", 2);
				await waitFor(
					browser,
					`document.querySelectorAll('[data-testid="note-list"] [data-testid="note"]').length === 2`,
					"the calm note",
				);
				const motion = await browser.eval(
					`getComputedStyle(document.querySelector('[data-testid="note-list"] [data-testid="note"][data-new]')).animationName`,
				);
				assert(
					motion === "none",
					`reduced motion stops the entrance: ${motion}`,
				);
				await browser.look(LOOKS[0]);
			},
		);

		await t.step(
			"a signed-out save is named, keeps the text, and offers the way back",
			async () => {
				await browser.eval(
					`fetch("/auth/sign-out", { method: "POST", credentials: "same-origin" }).then((r) => r.status)`,
				);
				await browser.click("note-text");
				await browser.type("typed after signing out");
				await browser.click("note-save");
				await waitFor(
					browser,
					`document.querySelector('[data-testid="note-alert"][data-shown]')`,
					"the signed-out alert",
				);
				assert(
					(await browser.text("note-alert")).includes("You're signed out"),
					"a lost session is named",
				);
				assert(
					(await browser.eval(`document.querySelector('${field}').value`)) ===
						"typed after signing out",
					"the text is kept",
				);
				assert(
					(await browser.eval(
						`document.querySelector('[data-testid="note-sign-in"]').getAttribute("href")`,
					)) ===
						"/sign-in?return_to=%2Fnotes",
					"the way back returns here",
				);
				const refused = (await answers(browser)).filter((n) =>
					n.method === "POST"
				).pop();
				assert(
					refused.status === 401,
					`the real answer is a 401: ${refused.status}`,
				);
				assert(
					!refused.body.includes("typed after signing out") &&
						!refused.body.includes("grace writes too") &&
						!refused.body.includes("calm note"),
					"a refusal carries no note",
				);
				await capture(
					browser,
					"24-notebook-signed-out",
					"Notebook, the sign-in ended",
					{
						looks: [LOOKS[0], LOOKS[2]],
					},
				);
			},
		);

		const foreign = browser.requests.filter((url) =>
			!url.startsWith(service.siteOrigin) &&
			!url.startsWith(service.adminOrigin) &&
			!url.startsWith("data:")
		);
		assert(foreign.length === 0, `no request leaves the demo: ${foreign}`);
		assert(
			browser.errors.every((error) =>
				/Failed to load resource|net::ERR|Failed to fetch/.test(error)
			),
			`no script error, only the stopped server's refusals: ${browser.errors}`,
		);
	} finally {
		await browser.close();
		await service.close();
	}

	if (SHOTS) await writeGallery(gallery);
});

/** A compact local gallery of every capture, grouped by state. */
async function writeGallery(items) {
	const byName = new Map();
	for (const item of items) {
		if (!byName.has(item.name)) {
			byName.set(item.name, { caption: item.caption, shots: [] });
		}
		byName.get(item.name).shots.push(item);
	}
	const sections = [...byName.entries()]
		.sort(([a], [b]) => a.localeCompare(b))
		.map(([name, group]) =>
			`<section><h2>${name} <small>${group.caption}</small></h2><div class="row">${
				group.shots.map((shot) =>
					`<figure class="${
						shot.tag.startsWith("400") ? "narrow" : "wide"
					}"><a href="${shot.file}"><img src="${shot.file}" alt="${group.caption}, ${shot.tag}" loading="lazy"></a><figcaption>${shot.tag}</figcaption></figure>`
				).join("")
			}</div></section>`
		).join("\n");
	const html =
		`<!doctype html><meta charset="utf-8"><title>Demo visitor pages</title>
<style>body{font:14px system-ui;margin:24px;background:#faf6f0;color:#16111f}h2{margin:28px 0 8px;font-size:16px}h2 small{font-weight:400;color:#6e6578;margin-left:8px}.row{display:flex;gap:16px;align-items:flex-start;flex-wrap:wrap}figure{margin:0}figure img{display:block;border:1px solid #cdbfad;border-radius:6px}.wide img{width:420px}.narrow img{width:190px}figcaption{font:12px monospace;color:#6e6578;margin-top:4px}</style>
<h1>Demo visitor pages</h1><p>Real pages from the real service; 1280 and 400 px, light and dark.</p>
${sections}`;
	await Deno.writeTextFile(`${SHOTS}/index.html`, html);
}
