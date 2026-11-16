// The public demo's behaviour: the band's countdown and disclosure, the
// front door's choice, real email-code sign-in with the demo inbox, the
// account menu, the private notebook, and the new-day dialog.
//
// Everything the page shows from the network is put in with `textContent`,
// never as markup. The server hands this script one JSON blob
// (`#demo-state`): the day, the server's clock, the admin's address, and
// this browser's own people. Nothing here invents a code, a time, or an
// address: the code is read from the private inbox, and the countdown is
// measured from the server's clock to the end of the generation's day.
(() => {
	"use strict";

	const stateElement = document.getElementById("demo-state");
	if (!stateElement) return;

	const S = JSON.parse(stateElement.textContent);
	const root = document.documentElement;
	const loadedAt = performance.now();

	const $ = (selector, scope = document) => scope.querySelector(selector);
	const tid = (id, scope = document) =>
		scope.querySelector(`[data-testid="${id}"]`);
	const tids = (prefix, scope = document) =>
		Array.from(scope.querySelectorAll(`[data-testid^="${prefix}"]`));
	const pad = (number) => String(number).padStart(2, "0");

	// ---------- Time, from the server's clock ----------

	const serverNow = () => S.nowMs + (performance.now() - loadedAt);

	/** The end of the generation's UTC day, in Unix milliseconds. */
	const dayEnd = (() => {
		const [year, month, day] = String(S.day).split("-").map(Number);
		return Date.UTC(year, month - 1, day + 1);
	})();

	const secondsToReset = () =>
		Math.max(0, Math.round((dayEnd - serverNow()) / 1000));

	/** The same wording the server renders into the band. */
	const countdownText = (seconds) => {
		if (seconds < 600) {
			return `${Math.floor(seconds / 60)}m ${pad(seconds % 60)}s`;
		}
		const hours = Math.floor(seconds / 3600);
		const minutes = Math.floor((seconds % 3600) / 60);
		return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
	};

	const utcTime = (millis) => {
		const date = new Date(millis);
		return `${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}`;
	};
	const utcClock = (millis) =>
		`${utcTime(millis)}:${pad(new Date(millis).getUTCSeconds())}`;

	const clock = (seconds) => `${Math.floor(seconds / 60)}:${pad(seconds % 60)}`;

	// ---------- Small helpers ----------

	/** Announces a short message to screen readers without moving focus. */
	const announcer = (() => {
		const element = document.createElement("div");
		element.className = "sr-only";
		element.setAttribute("role", "status");
		element.setAttribute("aria-live", "polite");
		document.body.append(element);
		return (message) => {
			element.textContent = "";
			window.setTimeout(() => {
				element.textContent = message;
			}, 30);
		};
	})();

	/** Builds an element whose text can only ever be text. */
	const make = (tag, className, text) => {
		const element = document.createElement(tag);
		if (className) element.className = className;
		if (text !== undefined) element.textContent = text;
		return element;
	};

	const reducedMotion = () =>
		window.matchMedia("(prefers-reduced-motion: reduce)").matches;

	/** A JSON answer, or `null` when the body is not JSON. */
	const jsonOf = async (response) => {
		try {
			return await response.json();
		} catch {
			return null;
		}
	};

	/** The seconds a refusal asks the caller to wait. */
	const retrySeconds = (response, body) => {
		const header = Number(response.headers.get("Retry-After"));
		const fromBody = body && Number(body.retry_after);
		const seconds = Number.isFinite(header) && header > 0 ? header : fromBody;
		return Number.isFinite(seconds) && seconds > 0 ? Math.ceil(seconds) : null;
	};

	const waitWords = (seconds) => {
		if (seconds == null) return "in a little while";
		if (seconds < 90) return `in ${seconds} s`;
		return `in ${Math.ceil(seconds / 60)} min`;
	};

	// ---------- The band ----------

	const band = () => {
		const reset = tid("band-reset");
		const late = tid("band-late");
		const toggle = tid("band-toggle");

		const tick = () => {
			const seconds = secondsToReset();
			if (reset) {
				const isLate = seconds < 600;
				reset.textContent = seconds === 0
					? "Resetting at 00:00 UTC"
					: `${isLate ? "! " : ""}Resets in ${
						countdownText(seconds)
					} (00:00 UTC)`;
				reset.toggleAttribute("data-late", isLate);
				if (late) {
					late.textContent = isLate
						? "· anything you do now is cleared then"
						: "";
				}
			}
		};

		tick();
		window.setInterval(tick, 1000);

		if (toggle) {
			toggle.addEventListener("click", () => {
				const open = !root.hasAttribute("data-band");
				if (open) root.setAttribute("data-band", "open");
				else root.removeAttribute("data-band");
				toggle.setAttribute("aria-expanded", String(open));
			});
		}
	};

	// ---------- Disclosures and links ----------

	const disclosures = () => {
		const route = tid("route-toggle");
		if (route) {
			route.addEventListener("click", () => {
				const open = route.getAttribute("aria-expanded") !== "true";
				route.setAttribute("aria-expanded", String(open));
				if (open) root.setAttribute("data-route", "open");
				else root.removeAttribute("data-route");
			});
		}

		// The admin opens beside this page, so both stay in view.
		for (
			const link of document.querySelectorAll('a[data-testid^="open-admin"]')
		) {
			link.target = "_blank";
			link.rel = "noopener";
			link.setAttribute(
				"aria-label",
				`${link.getAttribute("aria-label")} (opens in a new tab)`,
			);
		}
	};

	// ---------- The new-day dialog ----------

	const resetDialog = () => {
		const scrim = tid("reset-scrim");
		const dialog = tid("reset-dialog");
		if (!scrim || !dialog) return { show() {} };

		const title = $("#reset-title");
		const keep = tid("reset-keep");
		const start = tid("reset-start");
		let opener = null;

		const siblings = () =>
			Array.from(scrim.parentElement.children).filter((child) =>
				child !== scrim
			);

		const close = () => {
			root.removeAttribute("data-reset");
			for (const sibling of siblings()) sibling.inert = false;
			const target = opener && document.contains(opener) ? opener : $("#main");
			if (target) target.focus({ preventScroll: true });
		};

		keep.addEventListener("click", close);
		scrim.addEventListener("keydown", (event) => {
			if (event.key === "Escape") {
				event.preventDefault();
				close();
			} else if (event.key === "Tab") {
				const stops = [start, keep];
				const first = stops[0];
				const last = stops[stops.length - 1];
				if (event.shiftKey && document.activeElement === first) {
					event.preventDefault();
					last.focus();
				} else if (!event.shiftKey && document.activeElement === last) {
					event.preventDefault();
					first.focus();
				} else if (!stops.includes(document.activeElement)) {
					event.preventDefault();
					first.focus();
				}
			}
		});

		return {
			show() {
				if (root.hasAttribute("data-reset")) return;
				opener = document.activeElement;
				root.setAttribute("data-reset", "");
				for (const sibling of siblings()) sibling.inert = true;
				title.focus({ preventScroll: true });
			},
		};
	};

	// ---------- The generation stream ----------

	/**
	 * Watches `/demo/generation`. One EventSource at a time; a failure
	 * closes it and reconnects after a bounded, doubling wait (2 s up to
	 * 30 s). A 503 or a dropped stream proves nothing: only an event that
	 * names a different day than this page was made for means the demo
	 * reset.
	 */
	const watchGeneration = (dialog) => {
		const FIRST = 2000;
		const LONGEST = 30000;
		let source = null;
		let timer = 0;
		let delay = FIRST;
		let done = false;

		const schedule = () => {
			if (done || timer) return;
			timer = window.setTimeout(() => {
				timer = 0;
				connect();
			}, delay);
			delay = Math.min(delay * 2, LONGEST);
		};

		const connect = () => {
			if (done || source) return;
			try {
				source = new EventSource("/demo/generation");
			} catch {
				source = null;
				schedule();
				return;
			}
			source.addEventListener("generation", (event) => {
				delay = FIRST;
				let day = null;
				try {
					day = JSON.parse(event.data).day;
				} catch {
					return;
				}
				if (typeof day === "string" && day !== S.day) {
					done = true;
					source?.close();
					source = null;
					dialog.show();
				}
			});
			source.onerror = () => {
				// Closing stops the browser's own, unbounded retries.
				source?.close();
				source = null;
				schedule();
			};
		};

		window.addEventListener("pagehide", () => {
			source?.close();
			source = null;
		});
		window.addEventListener("pageshow", (event) => {
			if (event.persisted) connect();
		});

		connect();
	};

	// ---------- Choosing a person ----------

	/** A short alert in the region the page keeps for it. */
	const message = (tone, title, body) => {
		const element = make("div", `msg msg-${tone}`);
		element.setAttribute("role", tone === "bad" ? "alert" : "status");
		const glyph = make("span", "msg-glyph", tone === "bad" ? "✕" : "!");
		glyph.setAttribute("aria-hidden", "true");
		const copy = make("div", "msg-copy");
		copy.append(make("p", "msg-title", title), make("p", "msg-body", body));
		element.append(glyph, copy);
		return element;
	};

	/**
	 * Asks the server for this browser's person: the real same-origin
	 * mint. It answers with a redirect to sign-in, which the page then
	 * opens. Returns `null` on success, or the message to show.
	 */
	const mint = async (persona) => {
		let response;
		try {
			response = await fetch("/demo/pass", {
				method: "POST",
				headers: { Accept: "application/json" },
				body: new URLSearchParams({ persona, return_to: S.returnTo }),
				credentials: "same-origin",
			});
		} catch {
			return {
				tone: "bad",
				title: "Couldn't reach Notes.",
				body: "Check your connection and try again.",
			};
		}

		if (response.ok && response.redirected) {
			window.location.assign(response.url);
			return null;
		}

		const body = await jsonOf(response);

		if (response.status === 429) {
			return {
				tone: "warn",
				title: "That's a lot of new people from your network.",
				body: `The demo hands out a few an hour to each network. Try again ${
					waitWords(retrySeconds(response, body))
				}.`,
				full: false,
			};
		}
		if (response.status === 503 && body && body.error === "full") {
			return {
				tone: "warn",
				title: "Today's demo is full.",
				body:
					`It has handed out all of today's people. It opens again at 00:00 UTC, in ${
						countdownText(secondsToReset())
					}.`,
				full: true,
			};
		}
		if (response.status === 503) {
			return {
				tone: "warn",
				title: "Notes is opening today's demo.",
				body: "Try again in a few seconds.",
			};
		}
		return {
			tone: "bad",
			title: "That choice didn't go through.",
			body: "Reload the page and try again.",
		};
	};

	const frontDoor = () => {
		const area = tid("mint-notice");
		const buttons = ["ada", "grace"].map((persona) => ({
			persona,
			button: tid(`mint:${persona}`),
		}));
		let busy = false;

		const setBusy = (value) => {
			busy = value;
			for (const { button } of buttons) button.disabled = value;
		};

		for (const { persona, button } of buttons) {
			button.addEventListener("click", async () => {
				if (busy) return;
				setBusy(true);
				area.replaceChildren();
				const problem = await mint(persona);
				if (!problem) return;
				area.append(message(problem.tone, problem.title, problem.body));
				announcer(problem.title);
				setBusy(problem.full === true);
			});
		}
	};

	// ---------- Sign-in ----------

	const signIn = (dialog) => {
		const me = S.identity;
		const mailbox = tid("mailbox");
		const alert = tid("notice-alert");
		const status = tid("notice-status");
		const codeInput = tid("code");
		const emailInput = tid("email");
		const verify = tid("verify");
		const send = tid("send");
		const resend = tid("resend");
		const cooldown = tid("cooldown");
		const sendHelp = tid("send-help");
		const stations = tids("station:");
		const panel = tid("station-panel");
		const go = tid("choose-go");
		const wide = tid("code-help-wide");
		const peek = tid("peek");

		let state = root.dataset.state;
		let messages = [];
		let lostAtMs = S.mail?.lostAtMs ?? null;
		let cooldownUntil = 0;
		let cooldownTimer = 0;
		let busy = false;
		let arrived = null;
		let chosen = S.preselect;

		// The real attributes the markup cannot express.
		if (codeInput) {
			codeInput.setAttribute("name", "code");
			codeInput.setAttribute("maxlength", "9");
			codeInput.setAttribute("inputmode", "text");
			codeInput.setAttribute("autocapitalize", "characters");
			codeInput.setAttribute("autocorrect", "off");
			codeInput.setAttribute("spellcheck", "false");
			codeInput.setAttribute("enterkeyhint", "go");
		}
		if (emailInput) {
			emailInput.setAttribute("readonly", "");
			emailInput.setAttribute("type", "email");
			emailInput.setAttribute("name", "email");
		}
		if (alert) alert.id = "code-error";

		// Labels the markup cannot associate: clicking one goes to its field.
		for (
			const [label, field] of [["code-label", codeInput], [
				"email-label",
				emailInput,
			]]
		) {
			const element = document.getElementById(label);
			if (element && field) {
				element.addEventListener("click", () => field.focus());
			}
		}

		// ----- Stations (a tab list: one tab stop, arrows, Home, End) -----

		const names = S.stations.map((station) => station.name);
		let selected = "send";

		const selectStation = (name, focus) => {
			selected = name;
			const info = S.stations.find((station) => station.name === name);
			for (const station of stations) {
				const on = station.dataset.testid === `station:${name}`;
				station.setAttribute("aria-selected", String(on));
				station.tabIndex = on ? 0 : -1;
				if (on && focus) station.focus();
			}
			tid("explain-sig").textContent = info.sig;
			tid("explain-text").textContent = info.text;
			panel.setAttribute("aria-labelledby", `station-tab-${name}`);
		};

		/** Lights the stations: those behind are done, `now` is where it is. */
		const lightStations = (doneIndex, nowName) => {
			names.forEach((name, index) => {
				const station = tid(`station:${name}`);
				station.toggleAttribute("data-done", index <= doneIndex);
				station.toggleAttribute("data-now", name === nowName);
			});
			const strip = tid("stations");
			strip.style.setProperty("--fill", `${Math.max(0, doneIndex) * 20}%`);
			if (nowName) selectStation(nowName, false);
		};

		const tablist = tid("stations");
		tablist.addEventListener("keydown", (event) => {
			const index = names.indexOf(selected);
			let next = null;
			if (event.key === "ArrowRight" || event.key === "ArrowDown") {
				next = (index + 1) % names.length;
			} else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
				next = (index - 1 + names.length) % names.length;
			} else if (event.key === "Home") next = 0;
			else if (event.key === "End") next = names.length - 1;
			if (next === null) return;
			event.preventDefault();
			selectStation(names[next], true);
		});
		for (const station of stations) {
			station.addEventListener(
				"click",
				() =>
					selectStation(station.dataset.testid.slice("station:".length), false),
			);
		}

		// ----- Notices -----

		const show = (container, title, body) => {
			tid("status-title", container).textContent = title;
			tid("status-body", container).textContent = body;
			container.setAttribute("data-shown", "");
		};
		const clearNotices = () => {
			for (const container of [alert, status]) {
				container.removeAttribute("data-shown");
				tid("status-title", container).textContent = "";
				tid("status-body", container).textContent = "";
			}
			codeInput?.removeAttribute("aria-invalid");
			codeInput?.setAttribute("aria-describedby", "code-help");
		};
		const showAlert = (title, body) => {
			show(alert, title, body);
			status.removeAttribute("data-shown");
		};
		const showStatus = (title, body) => {
			show(status, title, body);
			alert.removeAttribute("data-shown");
		};

		// ----- Rendering -----

		/** The newest message this person still has here. */
		const newest = () => messages[messages.length - 1] ?? null;

		const codeOf = (item) => {
			const match = /([A-Z0-9]{4}-[A-Z0-9]{4})\s*$/.exec(item.subject);
			return match ? match[1] : null;
		};

		const renderMailbox = () => {
			mailbox.replaceChildren();

			if (state === "choose" || !me) {
				mailbox.append(
					listening("Choose who's signing in. Their mail will land here."),
				);
				return;
			}

			const current = state === "lost" ? null : newest();

			if (state === "lost") {
				const gone = make("article", "mail gone");
				gone.setAttribute("aria-label", "A code this server no longer has");
				gone.append(
					make(
						"p",
						"mail-subject",
						`A code was sent at ${
							utcTime(lostAtMs)
						} UTC, but this server no longer has it.`,
					),
					make(
						"p",
						"mail-text",
						"Codes live only in the memory of the server that sent them, so this one is gone from here. A new release can do that. Your account and any session are fine.",
					),
				);
				const actions = make("div", "mail-actions");
				const again = make("button", "btn-paper", "Send a new code");
				again.type = "button";
				again.dataset.testid = "mail-send";
				again.dataset.action = "send";
				actions.append(again);
				gone.append(actions);
				mailbox.append(gone);
			}

			if (!messages.length && state !== "lost") {
				mailbox.append(
					listening(
						"Waiting for mail. Press Send a code and watch it land here.",
					),
				);
				return;
			}

			for (const item of messages.slice().reverse()) {
				const isCurrent = item === current;
				const article = make(
					"article",
					`mail${isCurrent ? "" : " old"}${
						isCurrent && item.receipt === arrived ? " arrived" : ""
					}`,
				);
				article.setAttribute(
					"aria-label",
					isCurrent
						? "Newest message"
						: "Older message, replaced by a newer code",
				);

				const meta = make("dl", "mail-meta");
				for (
					const [term, value, mono] of [
						["From", "Notes", false],
						["To", item.to, true],
						["At", `${utcClock(Date.parse(item.sent_at))} UTC`, false],
					]
				) {
					const dd = make("dd", mono ? "mono" : "", value);
					meta.append(make("dt", "", term), dd);
				}
				article.append(meta, make("p", "mail-subject", item.subject));
				if (isCurrent) article.append(make("p", "mail-text", item.text));

				const code = codeOf(item);
				if (code) {
					const row = make("div", "mail-code-row");
					row.append(make("span", "mail-code", code));
					if (isCurrent) {
						const fill = make("button", "btn-paper", "Fill in code");
						fill.type = "button";
						fill.dataset.testid = "fill";
						fill.dataset.action = "fill";
						fill.dataset.code = code;
						row.append(fill);
					} else {
						row.append(
							make(
								"span",
								"mail-note",
								"Replaced: a newer code stops this one working.",
							),
						);
					}
					article.append(row);
				}
				mailbox.append(article);
			}
			arrived = null;
		};

		const listening = (text) => {
			const row = make("div", "listening");
			row.setAttribute("data-testid", "listening");
			const pulse = make("span", "pulse");
			pulse.setAttribute("aria-hidden", "true");
			row.append(pulse, make("span", "", text));
			return row;
		};

		const renderControls = () => {
			const waiting = Date.now() < cooldownUntil;
			const seconds = Math.max(
				0,
				Math.ceil((cooldownUntil - Date.now()) / 1000),
			);
			if (send) {
				send.disabled = busy || waiting;
				send.textContent = state === "lost" ? "Send a new code" : "Send a code";
			}
			if (resend) resend.disabled = busy || waiting;
			if (cooldown) {
				cooldown.textContent = waiting ? `available in ${clock(seconds)}` : "";
			}
			if (sendHelp && state !== "choose") {
				sendHelp.textContent = waiting
					? `Another code is available in ${clock(seconds)}.`
					: "Notes emails a one-time code. Here, the email waits in your demo inbox.";
			}
			if (verify) verify.disabled = busy;
			root.toggleAttribute("data-busy", busy);
		};

		const tickCooldown = () => {
			renderControls();
			if (Date.now() >= cooldownUntil) {
				window.clearInterval(cooldownTimer);
				cooldownTimer = 0;
			}
		};
		const startCooldown = (seconds) => {
			cooldownUntil = Date.now() + seconds * 1000;
			window.clearInterval(cooldownTimer);
			cooldownTimer = window.setInterval(tickCooldown, 1000);
			renderControls();
		};

		const setState = (next) => {
			state = next;
			root.dataset.state = next;
			if (next === "choose" || next === "ready") lightStations(-1, "send");
			else if (next === "lost") lightStations(0, "mail");
			else if (codeInput && codeInput.value) lightStations(2, "verify");
			else lightStations(1, "read");
			renderMailbox();
			renderControls();
		};

		// ----- The inbox: read once after a send, or when asked -----

		/**
		 * Reads this pass's inbox. `true` when it answered, `false` when it
		 * could not (the problem is already on the page).
		 */
		const loadInbox = async () => {
			let response;
			try {
				response = await fetch("/demo/inbox", {
					headers: { Accept: "application/json" },
					cache: "no-store",
					credentials: "same-origin",
				});
			} catch {
				showAlert(
					"Couldn't reach Notes.",
					"Check your connection, then check for mail again.",
				);
				return false;
			}

			if (response.status === 401) {
				const body = await jsonOf(response);
				passGone(body && body.pass === "new_day");
				return false;
			}
			if (!response.ok) {
				showAlert(
					"The inbox didn't answer.",
					"Try Check for mail again in a few seconds.",
				);
				return false;
			}

			const body = await jsonOf(response);
			if (!body || !Array.isArray(body.messages) || !Array.isArray(body.lost)) {
				showAlert(
					"The inbox didn't answer.",
					"Try Check for mail again in a few seconds.",
				);
				return false;
			}

			messages = body.messages
				.filter((item) => item.to === me.email && item.persona === me.persona)
				.sort((a, b) => Date.parse(a.sent_at) - Date.parse(b.sent_at));
			const lost = body.lost.find((item) => item.persona === me.persona);
			lostAtMs = lost ? Date.parse(lost.sent_at) : null;

			// A code another process sent that this one no longer holds is the
			// current truth; older mail it still shows is replaced.
			if (lostAtMs !== null) {
				setState("lost");
				showStatus(
					"Your last code isn't here any more.",
					`This server no longer has the code it sent at ${
						utcTime(lostAtMs)
					} UTC. Send a new code; your account and any session are untouched.`,
				);
			} else if (messages.length) setState("sent");
			else setState("ready");
			return true;
		};

		const passGone = (newDay) => {
			setState(me ? "ready" : "choose");
			if (send) send.disabled = true;
			if (resend) resend.disabled = true;
			if (verify) verify.disabled = true;
			showAlert(
				newDay ? "A new demo day started." : "This browser's pass is gone.",
				"Choose who to be again to get a new person. Cleared cookies and the daily reset both do this.",
			);
			const link = make("a", "msg-link", "Start again");
			link.href = S.signInTarget;
			tid("status-body", alert).after(link);
		};

		// ----- Sending and verifying: Auth's real routes -----

		const authCall = async (path, payload) => {
			try {
				const response = await fetch(path, {
					method: "POST",
					headers: {
						"Content-Type": "application/json",
						Accept: "application/json",
					},
					body: JSON.stringify(payload),
					credentials: "same-origin",
				});
				return { response, body: await jsonOf(response.clone()) };
			} catch {
				return null;
			}
		};

		const refusal = (result, doing) => {
			if (!result) {
				showAlert(
					"Couldn't reach Notes.",
					"Check your connection and try again.",
				);
				return;
			}
			const { response, body } = result;
			const code = body && body.error;

			if (response.status === 403 && code === "not_your_identity") {
				passGone(false);
			} else if (response.status === 429 && code === "rate_limited") {
				const seconds = retrySeconds(response, body);
				const newest_ = newest();
				const still =
					newest_ && Date.parse(newest_.sent_at) + 600000 > Date.now()
						? ` Your newest code still works until ${
							utcTime(Date.parse(newest_.sent_at) + 600000)
						} UTC, if it's in your inbox.`
						: "";
				showStatus(
					"That's enough codes for now.",
					`Try again ${waitWords(seconds)}.${still}`,
				);
				if (seconds) startCooldown(seconds);
			} else if (response.status === 400 && code === "invalid_code") {
				showAlert(
					"That code didn't work.",
					"It may be mistyped, already used, or expired. Check your demo inbox, or send a new code.",
				);
				codeInput.setAttribute("aria-invalid", "true");
				codeInput.setAttribute("aria-describedby", "code-error code-help");
				codeInput.focus();
			} else if (response.status === 503 || !body) {
				showAlert(
					"Notes is busy or restarting.",
					`Try ${doing} again in a few seconds.`,
				);
			} else {
				showAlert(
					"That didn't go through.",
					`Try ${doing} again. If it keeps happening, reload the page.`,
				);
			}
		};

		const sendCode = async () => {
			if (busy || Date.now() < cooldownUntil || !me) return;
			busy = true;
			clearNotices();
			lightStations(-1, "send");
			renderControls();
			const result = await authCall("/auth/email/send", { email: me.email });
			busy = false;

			if (!result || !result.response.ok) {
				renderControls();
				refusal(result, "sending");
				if (result && result.response.status === 429) await loadInbox();
				return;
			}

			startCooldown(30);
			arrived = null;
			const before = new Set(messages.map((item) => item.receipt));
			if (await loadInbox()) {
				if (state === "lost") return;
				const fresh = messages.find((item) => !before.has(item.receipt));
				if (fresh) {
					arrived = fresh.receipt;
					renderMailbox();
					announcer("Code sent. It is in your demo inbox.");
					codeInput.value = "";
					codeInput.focus();
				} else {
					showAlert(
						"The code is on its way, but the inbox has nothing yet.",
						"Press Check for mail in a moment.",
					);
				}
			}
		};

		const verifyCode = async () => {
			if (busy || !me) return;
			const typed = codeInput.value.trim();
			if (!typed) {
				showAlert(
					"Type the code from your demo inbox.",
					"Or press Fill in code on the newest message.",
				);
				codeInput.setAttribute("aria-invalid", "true");
				codeInput.setAttribute("aria-describedby", "code-error code-help");
				codeInput.focus();
				return;
			}
			busy = true;
			clearNotices();
			lightStations(2, "verify");
			renderControls();
			const result = await authCall("/auth/email/verify", {
				email: me.email,
				code: typed,
			});
			busy = false;

			if (result && result.response.ok && result.body && result.body.session) {
				lightStations(4, null);
				selectStation("session", false);
				announcer("Signed in.");
				// Only a path the server validated for this page.
				window.setTimeout(
					() => window.location.assign(S.returnTo),
					reducedMotion() ? 0 : 350,
				);
				return;
			}
			renderControls();
			refusal(result, "signing in");
		};

		// ----- Wiring -----

		const mailActions = (event) => {
			const target = event.target.closest("[data-action]");
			if (!target) return;
			if (target.dataset.action === "send") sendCode();
			if (target.dataset.action === "fill") {
				codeInput.value = target.dataset.code;
				clearNotices();
				lightStations(2, "verify");
				announcer("Code filled in. Press Sign in.");
				verify.scrollIntoView({
					block: "center",
					behavior: reducedMotion() ? "auto" : "smooth",
				});
				verify.focus({ preventScroll: true });
			}
		};
		mailbox.addEventListener("click", mailActions);

		if (send) send.addEventListener("click", sendCode);
		if (resend) resend.addEventListener("click", sendCode);
		if (verify) verify.addEventListener("click", verifyCode);
		if (codeInput) {
			codeInput.addEventListener("keydown", (event) => {
				if (event.key === "Enter") {
					event.preventDefault();
					verifyCode();
				}
			});
			codeInput.addEventListener("input", () => {
				codeInput.removeAttribute("aria-invalid");
				if (alert.hasAttribute("data-shown")) clearNotices();
				if (state === "sent") {
					lightStations(
						codeInput.value ? 2 : 1,
						codeInput.value ? "verify" : "read",
					);
				}
			});
		}
		tid("inbox-refresh")?.addEventListener("click", async () => {
			clearNotices();
			if (!me) return;
			if (await loadInbox()) {
				announcer(
					messages.length ? "Inbox checked." : "Nothing new in your inbox.",
				);
			}
		});

		// "Open your demo inbox" is only offered where the inbox sits below
		// the form; there it jumps to the newest message's action.
		const narrow = window.matchMedia("(max-width: 1023px)");
		const placePeek = () => {
			if (peek) peek.hidden = !narrow.matches;
			if (wide) wide.hidden = narrow.matches;
		};
		placePeek();
		narrow.addEventListener("change", placePeek);
		if (peek) {
			peek.addEventListener("click", (event) => {
				event.preventDefault();
				const action = mailbox.querySelector("[data-action]");
				tid("inbox").scrollIntoView({
					block: "start",
					behavior: reducedMotion() ? "auto" : "smooth",
				});
				if (action) action.focus({ preventScroll: true });
				else tid("inbox-refresh").focus({ preventScroll: true });
			});
		}

		// ----- Choosing -----

		const radios = tids("choose:");
		const pick = (persona, focus) => {
			chosen = persona;
			for (const radio of radios) {
				const on = radio.dataset.testid === `choose:${persona}`;
				radio.setAttribute("aria-checked", String(on));
				radio.tabIndex = on ? 0 : -1;
				if (on && focus) radio.focus();
			}
			go.disabled = false;
			go.textContent = `Continue as ${persona === "ada" ? "Ada" : "Grace"}`;
		};
		if (radios.length) {
			for (const radio of radios) {
				const persona = radio.dataset.testid.slice("choose:".length);
				radio.addEventListener("click", () => pick(persona, false));
				radio.addEventListener("keydown", (event) => {
					const keys = [
						"ArrowDown",
						"ArrowRight",
						"ArrowUp",
						"ArrowLeft",
						"Home",
						"End",
					];
					if (!keys.includes(event.key)) return;
					event.preventDefault();
					const index = radios.indexOf(radio);
					let next = index;
					if (event.key === "ArrowDown" || event.key === "ArrowRight") {
						next = (index + 1) % radios.length;
					} else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
						next = (index - 1 + radios.length) % radios.length;
					} else if (event.key === "Home") next = 0;
					else next = radios.length - 1;
					pick(radios[next].dataset.testid.slice("choose:".length), true);
				});
			}
			if (chosen) pick(chosen, false);
			else {
				// Nothing chosen yet: the group is still one tab stop.
				radios.forEach((radio, index) => {
					radio.tabIndex = index === 0 ? 0 : -1;
				});
			}
		}

		const chooseOrSwap = async (persona, trigger) => {
			if (busy) return;
			busy = true;
			clearNotices();
			trigger.disabled = true;
			const problem = await mint(persona);
			if (!problem) return;
			busy = false;
			trigger.disabled = false;
			(problem.tone === "bad" ? showAlert : showStatus)(
				problem.title,
				problem.body,
			);
			if (problem.full) trigger.disabled = true;
		};
		go?.addEventListener("click", () => chosen && chooseOrSwap(chosen, go));
		tid("swap")?.addEventListener("click", (event) => {
			const other = me.persona === "ada" ? "grace" : "ada";
			chooseOrSwap(other, event.currentTarget);
		});

		// ----- First paint -----

		lightStations(-1, "send");
		setState(state);
		if (me && (S.mail.held > 0 || S.mail.lostAtMs)) loadInbox();
		else if (state === "choose" && radios.length) {
			radios[0].tabIndex = chosen ? -1 : 0;
		}
		window.addEventListener(
			"pagehide",
			() => window.clearInterval(cooldownTimer),
		);
		void dialog;
	};

	// ---------- Notes ----------

	const signOut = async () => {
		try {
			const response = await fetch("/auth/sign-out", {
				method: "POST",
				credentials: "same-origin",
			});
			return response.ok;
		} catch {
			return false;
		}
	};

	const toast = (text) => {
		const element = make("div", "toast", text);
		element.setAttribute("role", "alert");
		document.body.append(element);
		window.setTimeout(() => element.remove(), 8000);
	};

	// ---------- The notebook ----------
	//
	// The person's saved notes come from `GET /demo/notes`, and the form
	// saves through `POST /demo/notes`. Note text goes in with `textContent`
	// only. The web renderer has no multi-line field, so the markup's text
	// input is swapped for a `<textarea>` carrying every attribute the markup
	// gave it. Each function takes the `book`: the notebook's elements and
	// its small state.

	/** Characters as the server counts them: Unicode scalar values, trimmed. */
	const scalarLength = (text) => Array.from(text.trim()).length;

	const plural = (number, word) =>
		`${number} ${word}${number === 1 ? "" : "s"}`;

	const swapForTextarea = (input) => {
		const area = document.createElement("textarea");
		for (const { name, value } of Array.from(input.attributes)) {
			if (name !== "type" && name !== "value") area.setAttribute(name, value);
		}
		area.setAttribute("name", "note");
		area.setAttribute("rows", "4");
		area.setAttribute("autocomplete", "off");
		input.replaceWith(area);
		return area;
	};

	/** The notebook's elements and state, or `null` when the page has none. */
	const openNotebook = (dialog) => {
		const input = tid("note-text");
		if (!S.notebook || !input || !tid("note-save")) return null;
		return {
			config: S.notebook,
			dialog,
			area: swapForTextarea(input),
			save: tid("note-save"),
			count: tid("notes-count"),
			chars: tid("note-chars"),
			error: tid("note-error"),
			alertBox: tid("note-alert"),
			savedBox: tid("note-saved"),
			unconfirmedBox: tid("note-unconfirmed"),
			list: tid("note-list"),
			empty: tid("note-empty"),
			actor: tid("note-actor"),
			notes: [],
			saving: false,
			announcedFull: false,
		};
	};

	const updateChars = (book) => {
		const used = scalarLength(book.area.value);
		book.chars.textContent = `${used} / ${book.config.maxChars}`;
		book.chars.toggleAttribute("data-over", used > book.config.maxChars);
	};

	/** A problem with the field, tied to it, or its absence. */
	const setFieldError = (book, text, invalid = true) => {
		book.error.textContent = text;
		if (text && invalid) book.area.setAttribute("aria-invalid", "true");
		else book.area.removeAttribute("aria-invalid");
		book.area.setAttribute(
			"aria-describedby",
			text ? "note-help note-error" : "note-help",
		);
	};

	const clearResults = (book) => {
		for (const box of [book.alertBox, book.savedBox, book.unconfirmedBox]) {
			box.removeAttribute("data-shown");
		}
		tid("note-sign-in")?.remove();
	};

	const showResult = (book, box, title, body) => {
		clearResults(book);
		tid("status-title", box).textContent = title;
		tid("status-body", box).textContent = body;
		box.setAttribute("data-shown", "");
	};

	const noteItem = (note) => {
		const item = make("div", "note");
		item.setAttribute("role", "listitem");
		item.dataset.testid = "note";
		item.append(
			make("p", "note-text", note.text),
			make("p", "note-when", `${utcTime(note.created_at)} UTC`),
		);
		return item;
	};

	const renderNotes = (book) => {
		const { config, notes } = book;
		book.list.replaceChildren(...notes.map(noteItem));
		book.empty.hidden = notes.length > 0;
		book.empty.textContent =
			"Nothing saved yet. Your first note will be here, and only you can read it.";
		book.count.textContent = `${notes.length} of ${config.maxNotes} notes`;
		const full = notes.length >= config.maxNotes;
		book.area.disabled = full;
		book.save.disabled = full || book.saving;
		if (full && !book.announcedFull) {
			book.announcedFull = true;
			setFieldError(
				book,
				`Your notebook is full: ${config.maxNotes} notes is the limit for a demo day.`,
				false,
			);
		}
	};

	/** What a refused visit means here: the day ended, or the sign-in did. */
	const showSignedOut = (book, body) => {
		if (body?.pass === "new_day") {
			showResult(
				book,
				book.alertBox,
				"The demo reset",
				"Your notebook belonged to yesterday's demo. What you typed is still in the box.",
			);
			book.dialog.show();
			return;
		}
		showResult(
			book,
			book.alertBox,
			"You're signed out",
			"Sign in again to keep writing. What you typed is still in the box.",
		);
		const link = make("a", "msg-link", "Sign in again");
		link.href = "/sign-in?return_to=%2Fnotes";
		link.dataset.testid = "note-sign-in";
		tid("status-body", book.alertBox).parentElement.append(link);
	};

	const requestNotes = async () => {
		const response = await fetch("/demo/notes", {
			headers: { Accept: "application/json" },
			credentials: "same-origin",
			cache: "no-store",
		});
		return { response, body: await jsonOf(response) };
	};

	const loadNotes = async (book) => {
		try {
			const { response, body } = await requestNotes();
			if (response.status === 401) {
				showSignedOut(book, body);
				book.empty.textContent = "Sign in again to see your notes.";
			} else if (response.ok && Array.isArray(body?.notes)) {
				book.notes = body.notes;
				renderNotes(book);
			} else {
				throw new Error("unusable answer");
			}
		} catch {
			// A failed refresh of a list already on screen stays quiet.
			if (book.notes.length > 0) return;
			book.empty.hidden = false;
			book.empty.textContent =
				"Couldn't load your notes. Reload the page to try again.";
		}
	};

	/** What is wrong with the draft before it is sent, or `null`. */
	const draftProblem = (book) => {
		const used = scalarLength(book.area.value);
		const { maxChars } = book.config;
		if (used === 0) return "Write something first.";
		if (used <= maxChars) return null;
		return `That's ${
			plural(used - maxChars, "character")
		} over the ${maxChars} limit.`;
	};

	const postNote = async (text) => {
		try {
			const response = await fetch("/demo/notes", {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
					Accept: "application/json",
				},
				body: JSON.stringify({ text }),
				credentials: "same-origin",
			});
			return { response, body: await jsonOf(response) };
		} catch {
			return { response: null, body: null };
		}
	};

	const setSaving = (book, saving) => {
		book.saving = saving;
		book.save.disabled = saving;
		book.save.textContent = saving ? "Saving…" : "Save note";
		if (saving) book.save.setAttribute("aria-busy", "true");
		else book.save.removeAttribute("aria-busy");
		book.area.readOnly = saving;
	};

	/** A saved note: in the list, with what became of its event. */
	const reportSaved = (book, body) => {
		const { maxNotes } = book.config;
		book.notes = [body.note, ...book.notes].slice(0, maxNotes);
		book.area.value = "";
		updateChars(book);
		renderNotes(book);
		book.list.firstElementChild?.setAttribute("data-new", "");

		if (body.analytics === "confirmed") {
			book.actor.textContent = body.actor;
			showResult(
				book,
				book.savedBox,
				"Saved. Analytics heard about it.",
				`Your note is private to this account. Its note_created event (${
					plural(body.words, "word")
				}, none of your text) reached analytics.`,
			);
			announcer("Note saved. Its note_created event reached analytics.");
			return;
		}
		showResult(
			book,
			book.unconfirmedBox,
			"Saved. Analytics didn't confirm.",
			"Your note is saved in your notebook. We couldn't confirm that its note_created event reached analytics, so it may not show in the admin. Nothing will resend it.",
		);
		announcer("Note saved. Analytics delivery could not be confirmed.");
	};

	/** Everything that is not a saved note: each keeps what was typed. */
	const reportRefusal = async (book, response, body) => {
		const { maxNotes } = book.config;
		if (response === null) {
			showResult(
				book,
				book.alertBox,
				"Couldn't reach Notes",
				"Your note may not have saved. Check Saved notes before trying again; what you typed is still in the box.",
			);
			await loadNotes(book);
		} else if (response.status === 401) {
			showSignedOut(book, body);
		} else if (response.status === 409 && body?.error === "notes_full") {
			showResult(
				book,
				book.alertBox,
				"Your notebook is full",
				`${maxNotes} notes is the limit for a demo day, and everything resets at 00:00 UTC. What you typed is still in the box.`,
			);
			await loadNotes(book);
		} else if (response.status === 422) {
			const problem = body?.error === "empty_note"
				? "Write something first."
				: `That's over the ${book.config.maxChars} character limit.`;
			setFieldError(book, problem);
			announcer(problem);
		} else {
			showResult(
				book,
				book.alertBox,
				"That didn't save",
				"Something went wrong on our side. What you typed is still in the box, so you can try again.",
			);
		}
	};

	const submitNote = async (book) => {
		if (book.saving) return;
		const problem = draftProblem(book);
		if (problem) {
			setFieldError(book, problem);
			announcer(problem);
			book.area.focus();
			return;
		}

		setFieldError(book, "");
		clearResults(book);
		const held = document.activeElement;
		setSaving(book, true);
		const { response, body } = await postNote(book.area.value);
		setSaving(book, false);

		if (response?.status === 201 && body?.note) reportSaved(book, body);
		else await reportRefusal(book, response, body);

		if (book.notes.length < book.config.maxNotes) book.save.disabled = false;
		// Disabling the button dropped focus; give it back to where it was.
		if (
			held && document.contains(held) && !held.disabled &&
			document.activeElement === document.body
		) {
			held.focus({ preventScroll: true });
		}
	};

	const notebook = (dialog) => {
		const book = openNotebook(dialog);
		if (!book) return;

		book.save.addEventListener("click", () => submitNote(book));
		book.area.addEventListener("keydown", (event) => {
			if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
				event.preventDefault();
				submitNote(book);
			}
		});
		book.area.addEventListener("input", () => {
			updateChars(book);
			if (book.area.hasAttribute("aria-invalid")) setFieldError(book, "");
		});
		tid("note-label")?.addEventListener("click", () => book.area.focus());

		updateChars(book);
		loadNotes(book);
	};

	const notes = (dialog) => {
		notebook(dialog);

		const toggle = tid("menu-toggle");
		const panel = tid("menu-panel");
		if (!toggle || !panel) return;

		const items = () => Array.from(panel.querySelectorAll('[role="menuitem"]'));
		const open = () => {
			toggle.setAttribute("aria-expanded", "true");
			items()[0]?.focus();
		};
		const close = (returnFocus) => {
			toggle.setAttribute("aria-expanded", "false");
			if (returnFocus) toggle.focus();
		};

		toggle.addEventListener("click", () => {
			if (toggle.getAttribute("aria-expanded") === "true") close(true);
			else open();
		});
		panel.addEventListener("keydown", (event) => {
			const list = items();
			const index = list.indexOf(document.activeElement);
			if (event.key === "Escape") {
				event.preventDefault();
				close(true);
			} else if (event.key === "ArrowDown") {
				event.preventDefault();
				list[(index + 1) % list.length].focus();
			} else if (event.key === "ArrowUp") {
				event.preventDefault();
				list[(index - 1 + list.length) % list.length].focus();
			} else if (event.key === "Tab") {
				close(false);
			}
		});
		document.addEventListener("click", (event) => {
			if (!event.target.closest('[data-testid="menu-wrap"]')) close(false);
		});

		tid("sign-out").addEventListener("click", async (event) => {
			const button = event.currentTarget;
			button.disabled = true;
			if (await signOut()) window.location.assign("/?signed_out=1");
			else {
				button.disabled = false;
				toast("Couldn't sign out. Try again in a moment.");
			}
		});

		tid("switch-account").addEventListener("click", async (event) => {
			const button = event.currentTarget;
			button.disabled = true;
			// Switching is signing out, then choosing the other person.
			if (!(await signOut())) {
				button.disabled = false;
				toast("Couldn't switch account. Try again in a moment.");
				return;
			}
			const other = S.identity.persona === "ada" ? "grace" : "ada";
			const problem = await mint(other);
			if (problem) window.location.assign("/");
		});
	};

	// ---------- Start ----------

	band();
	disclosures();
	const dialog = resetDialog();
	watchGeneration(dialog);

	if (S.page === "front") frontDoor();
	else if (S.page === "sign-in") signIn(dialog);
	else if (S.page === "notes") notes(dialog);
})();
