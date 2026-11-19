// The leptoskit notifications receiver: the service-worker half of Web Push
// enrollment.
//
// Your app serves this script — `leptoskit_notifications::browser`
// builds it with your inbox route baked in — same-origin from its root (for
// example `/sw.js`) with a JavaScript content type, and registers it. On a
// `push` event it reads the notifications service's envelope — the title,
// the body, the notification's id, the send's `data`, and the options that
// shape the banner: an image, a badge count, a click link, and the web
// presentation set — and shows what arrived with the platform's own
// presentation, tagged with the notification's id so a repeat replaces
// rather than stacks, and tells the pages this origin has open that it
// arrived. The service never pushes a data message — `lkdata` — to a
// browser, because a push that shows nothing costs a site its
// subscription; one that arrives anyway is handed to a page in view, and
// shows one banner when there is none. On a click it closes the banner
// and focuses a window of this origin the person already has. A page
// whose app is listening is handed the tap — the notification's id, the
// send's link, the pressed action button's id, and the send's data — and
// answers; a page that doesn't answer is brought to the send's own link,
// when it carries one that resolves to this origin, and otherwise to your
// inbox route, with the notification's id and the pressed button's id on
// the address. With no window open, that address opens in a new one. As
// it activates it claims the pages this origin already has open, so the
// tab a person enrolled from is one a click can route.
//
// This worker holds no credentials and calls no server: nothing fetches,
// nothing evaluates, and no read state changes here — the authenticated app
// marks what it reads once it is open. An envelope that is missing,
// oversized, malformed, or typed wrong still shows one truthful banner
// ("New notification") under a stable tag, so a broken sender cannot
// silently swallow a delivery, and untrusted payload content can never
// choose where a click navigates: the link is honored only when it
// resolves to this script's own origin, and everything else stays in the
// notification's data.

(() => {
	"use strict";

	// The inbox route your build substituted in: the default URL a click
	// opens, on this origin, plus the notification's id as a query
	// parameter.
	const ROUTE = "__LKN_INBOX_ROUTE__";

	// A push's largest honest size. The service's encrypted record is
	// bounded at 4 KiB, so anything larger than that in plaintext is not
	// this sender's envelope, and its content is never parsed.
	const MAX_ENVELOPE_BYTES = 4096;

	// The visible-text bounds, mirroring what the service enforced on the
	// send. Longer content is clipped, not dropped.
	const MAX_TITLE_CHARS = 200;
	const MAX_BODY_CHARS = 2000;

	// The bounds of the options the service puts in the envelope,
	// mirrored from its own checks: an option longer than its bound is
	// not honored, the same way a mistyped one is not.
	const MAX_URL_CHARS = 1024;
	const MAX_ACTIONS = 2;
	const MAX_ACTION_CHARS = 100;
	const MAX_LANG_CHARS = 35;

	// How long a click waits for an open page to say it took the tap
	// before the page is brought to the address instead. An app that
	// listens answers at once; this bounds the wait for one that doesn't.
	const ACK_WAIT_MS = 500;

	// The tag an id-less or broken envelope falls back to, so even those
	// collapse instead of stacking.
	const FALLBACK_TAG = "leptoskit";

	// The generic banner a broken or empty envelope still shows.
	const FALLBACK_TITLE = "New notification";

	// A notification id, the only shape allowed onto the address line of
	// the route a click opens. Anything else stays in the notification's
	// data alone.
	const NOTIFICATION_ID =
		/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

	// A tag is plain printable ASCII, without control characters.
	const PLAIN_TEXT = /^[\x20-\x7e]{1,64}$/;

	// A BCP 47 language tag, as loose as the service's own check.
	const LANGUAGE_TAG = /^[a-zA-Z0-9-]{1,35}$/;

	// Reads what arrived into what to show: a title, a body, a stable
	// tag, the notification's id for the click, and the options that
	// shape the banner — a silent data flag, an image, a badge count, a
	// click link, and the web presentation set. Never throws: whatever a
	// push carried, the answer is either a banner or a deliberate
	// silence, never a broken one.
	function envelopeOf(data) {
		let payload = null;

		if (data) {
			// The size is checked before any read: a push too large to
			// be this sender's envelope is not parsed at all.
			const bytes = data.arrayBuffer();

			if (bytes && bytes.byteLength <= MAX_ENVELOPE_BYTES) {
				try {
					payload = JSON.parse(data.text());
				} catch (_error) {
					payload = null;
				}
			}
		}

		if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
			return {
				title: FALLBACK_TITLE,
				body: "",
				tag: FALLBACK_TAG,
				id: null,
				lk: undefined,
				data: false,
				silent: false,
				image: null,
				badge: null,
				link: null,
				web: null,
			};
		}

		const id = identifier(payload.lkn);
		const title = typeof payload.title === "string" && payload.title.length > 0
			? clipped(payload.title, MAX_TITLE_CHARS)
			: null;
		const body = typeof payload.body === "string"
			? clipped(payload.body, MAX_BODY_CHARS)
			: "";

		return {
			title: title ?? FALLBACK_TITLE,
			body: title === null ? "" : body,
			tag: tag(typeof payload.lktag === "string" ? payload.lktag : payload.lkn),
			id,
			lk: plainObject(payload.lk),
			data: payload.lkdata === true,
			silent: payload.silent === true,
			image: urlString(payload.image),
			badge: badgeCount(payload.lkbadge),
			link: urlString(payload.lklink),
			web: presentation(payload.lkw),
		};
	}

	// The stable tag: the collapse key the send chose (`lktag`) when
	// the push carries one, the notification's id otherwise, each only
	// when it is a plain short string, and the fallback when not. Two
	// pushes under one tag replace each other wherever the platform
	// coalesces.
	function tag(lkn) {
		return typeof lkn === "string" && PLAIN_TEXT.test(lkn) ? lkn : FALLBACK_TAG;
	}

	// The notification's id for URLs: only a real UUID passes, and only
	// this ever reaches the route a click opens.
	function identifier(lkn) {
		return typeof lkn === "string" && NOTIFICATION_ID.test(lkn) ? lkn : null;
	}

	// An option that names a URL: a bounded string, or nothing. The
	// click's own rules decide where a URL may actually go.
	function urlString(value) {
		return typeof value === "string" &&
				value.length > 0 &&
				value.length <= MAX_URL_CHARS
			? value
			: null;
	}

	// A badge count: a whole number the platform could set, or nothing.
	function badgeCount(value) {
		return typeof value === "number" && Number.isSafeInteger(value) &&
				value >= 0
			? value
			: null;
	}

	// The web presentation set, as the service sends it: every field
	// validated on its own, because the notification options are
	// untrusted payload content like everything else.
	function presentation(value) {
		const set = plainObject(value);
		if (!set) {
			return null;
		}

		const read = {
			icon: urlString(set.icon),
			badge: urlString(set.badge),
			requireInteraction: set.require_interaction === true,
			dir: set.dir === "auto" || set.dir === "ltr" || set.dir === "rtl"
				? set.dir
				: null,
			lang: typeof set.lang === "string" && LANGUAGE_TAG.test(set.lang)
				? set.lang
				: null,
			actions: [],
		};

		if (Array.isArray(set.actions)) {
			for (const action of set.actions.slice(0, MAX_ACTIONS)) {
				const actionId = typeof action?.action === "string" &&
						action.action.length > 0 &&
						action.action.length <= MAX_ACTION_CHARS
					? action.action
					: null;
				const title = typeof action?.title === "string" &&
						action.title.length > 0 &&
						action.title.length <= MAX_ACTION_CHARS
					? action.title
					: null;

				if (actionId !== null && title !== null) {
					read.actions.push({ action: actionId, title });
				}
			}
		}

		return read;
	}

	function clipped(text, maxChars) {
		return text.length > maxChars ? text.slice(0, maxChars - 1) + "…" : text;
	}

	function plainObject(value) {
		return value && typeof value === "object" && !Array.isArray(value)
			? value
			: undefined;
	}

	// Sets the app badge the platform shows, where the platform can: a
	// count the send named, resolved by the service to the person's live
	// unread number at push time. A zero clears it. The answer is waited
	// for, so the push's lifetime covers it and the browser can't stop
	// the worker with the badge still unset. Failures — an unsupported
	// platform, a refused badge — are the platform's own answer,
	// swallowed on purpose: a badge is a courtesy, never the delivery.
	async function setBadge(count) {
		if (count === null || typeof navigator === "undefined") {
			return;
		}

		try {
			if (count === 0 && typeof navigator.clearAppBadge === "function") {
				await navigator.clearAppBadge();
			} else if (typeof navigator.setAppBadge === "function") {
				await navigator.setAppBadge(count);
			}
		} catch (_error) {
			// The badge API refused; the banner is unaffected.
		}
	}

	// The pressed action button's id: the browser's own `event.action`,
	// bounded like every action the service sends. A click on the
	// banner's body is no action.
	function pressedAction(action) {
		return typeof action === "string" && action.length > 0 &&
				action.length <= MAX_ACTION_CHARS
			? action
			: null;
	}

	// The send's link as a path on this origin — what a page can route
	// to without loading again — or nothing when the link is missing,
	// malformed, or anywhere else.
	function ownPath(link) {
		if (typeof link !== "string" || link.length === 0) {
			return null;
		}

		try {
			const linked = new URL(link, self.location.origin);

			return linked.origin === self.location.origin
				? linked.pathname + linked.search + linked.hash
				: null;
		} catch (_error) {
			return null;
		}
	}

	// The URL a click opens: the send's own link when it resolves to this
	// origin, and otherwise the built-in route — checked against this
	// origin again at click time — with the notification's id when the
	// envelope carried one. Payload content cannot add a path, a host, a
	// scheme, or anything else — a link off this origin is refused here
	// rather than navigated. A click on one of the banner's action
	// buttons adds that button's id as `lka`, as a query value the URL
	// encodes: it can name an action, never a second parameter or a place.
	// The send's link carries the notification's id too, so the page it
	// opens knows which notification brought the person there.
	function targetUrl(data, action) {
		const link = data && typeof data.lklink === "string" ? data.lklink : null;
		const pressed = pressedAction(action);
		const id = identifier(data && data.lkn);

		if (link) {
			try {
				const linked = new URL(link, self.location.origin);

				if (linked.origin === self.location.origin) {
					if (id) {
						linked.searchParams.set("lkn", id);
					}

					if (pressed) {
						linked.searchParams.set("lka", pressed);
					}

					return linked.href;
				}
			} catch (_error) {
				// A malformed link falls through to the route.
			}
		}

		let url;

		try {
			url = new URL(ROUTE, self.location.origin);
		} catch (_error) {
			return null;
		}

		if (url.origin !== self.location.origin) {
			return null;
		}

		if (id) {
			url.searchParams.set("lkn", id);
		}

		if (pressed) {
			url.searchParams.set("lka", pressed);
		}

		return url.href;
	}

	// Whether a window client belongs to this origin. Nothing from a push
	// decides this; only the browser's own client list.
	function sameOrigin(clientUrl) {
		try {
			return new URL(clientUrl).origin === self.location.origin;
		} catch (_error) {
			return false;
		}
	}

	// The windows of this origin the person has open, controlled by this
	// worker or not. Nothing from a push decides this; only the browser's
	// own client list.
	async function pages() {
		try {
			const matches = await self.clients.matchAll({
				type: "window",
				includeUncontrolled: true,
			});

			return matches.filter((match) => match.url && sameOrigin(match.url));
		} catch (_error) {
			return [];
		}
	}

	// Tells one page what happened, as a message its app's receiver
	// reads. Answers whether the page could be told at all.
	function tell(page, event) {
		try {
			if (typeof page.postMessage === "function") {
				page.postMessage({ lkn_event: event });

				return true;
			}
		} catch (_error) {
			// A page that can't be posted to is a page that wasn't told.
		}

		return false;
	}

	// The taps handed to a page and not yet answered, by token.
	const unanswered = new Map();
	let handed = 0;

	// Hands a tap to a page and waits, briefly, for its app to say it
	// took it. A page with no listening app never answers, and the
	// caller then brings it to the address instead.
	function hand(page, event) {
		handed += 1;
		const token = Date.now().toString(36) + "-" + handed.toString(36);

		return new Promise((resolve) => {
			unanswered.set(token, resolve);

			if (!tell(page, { ...event, token })) {
				unanswered.delete(token);
				resolve(false);

				return;
			}

			setTimeout(() => {
				if (unanswered.delete(token)) {
					resolve(false);
				}
			}, ACK_WAIT_MS);
		});
	}

	// One message from a page: its app's answer to a tap it was handed.
	// Anything else is not this worker's.
	function message(event) {
		const token = event && event.data && event.data.lkn_ack;
		const resolve = typeof token === "string" ? unanswered.get(token) : null;

		if (resolve) {
			unanswered.delete(token);
			resolve(true);
		}
	}

	// One push: set the badge it asked for and show what arrived. The
	// two run together and both are waited for, so a slow badge never
	// holds the banner back and neither outlives the push's lifetime.
	async function deliver(event) {
		const envelope = envelopeOf(event.data);

		await Promise.all([setBadge(envelope.badge), present(envelope)]);
	}

	// Shows what arrived and tells the open pages. The registration's own
	// presentation is used, so what a banner looks like is the OS's
	// business, not this script's.
	async function present(envelope) {
		const open = await pages();

		if (envelope.data) {
			// The service never pushes a data message to a browser: a
			// push that shows nothing makes Chrome show a banner of its
			// own and makes Safari drop the subscription. One that
			// arrives anyway goes to the pages that are open, and is
			// silent only while one of them is in view — the one case
			// browsers allow.
			for (const page of open) {
				tell(page, {
					kind: "data",
					id: envelope.id,
					data: envelope.lk ?? null,
					badge: envelope.badge,
				});
			}

			if (open.some((page) => page.visibilityState === "visible")) {
				return;
			}

			const quiet = { tag: envelope.tag };

			if (envelope.id !== null) {
				quiet.data = { lkn: envelope.id };
			}

			await self.registration.showNotification(envelope.title, quiet);

			return;
		}

		const options = { body: envelope.body, tag: envelope.tag };

		if (envelope.silent) {
			options.silent = true;
		}

		if (envelope.image) {
			options.image = envelope.image;
		}

		if (envelope.web) {
			if (envelope.web.icon) {
				options.icon = envelope.web.icon;
			}

			if (envelope.web.badge) {
				options.badge = envelope.web.badge;
			}

			if (envelope.web.actions.length > 0) {
				options.actions = envelope.web.actions;
			}

			if (envelope.web.requireInteraction) {
				options.requireInteraction = true;
			}

			if (envelope.web.dir) {
				options.dir = envelope.web.dir;
			}

			if (envelope.web.lang) {
				options.lang = envelope.web.lang;
			}
		}

		if (
			envelope.id !== null || envelope.lk !== undefined ||
			envelope.link !== null
		) {
			options.data = {
				lkn: envelope.id,
				lk: envelope.lk,
				lklink: envelope.link,
			};
		}

		await self.registration.showNotification(envelope.title, options);

		// An open page hears that it arrived, so an inbox on screen can
		// refresh. Only a push this worker could read by id is worth
		// telling about.
		if (envelope.id !== null) {
			for (const page of open) {
				tell(page, {
					kind: "shown",
					id: envelope.id,
					data: envelope.lk ?? null,
				});
			}
		}
	}

	// One click: close the banner, then put the person on the
	// notification. An app window of this origin is focused and handed
	// the tap; a page that doesn't answer is brought to the address
	// instead, and without a window the address opens in a new one. The
	// route is the constant this script was built with, checked against
	// this origin at click time.
	async function open(event) {
		event.notification.close();

		// `event.action` is the browser's own: the id of the action button
		// that was pressed, and the empty string for the banner's body.
		const data = event.notification && event.notification.data;
		const url = targetUrl(data, event.action);
		const client = (await pages())[0] ?? null;

		if (client) {
			try {
				await client.focus();
			} catch (_error) {
				// Focus was refused, so open the route instead of leaving
				// the person on a banner that is already closed.
				if (url) {
					await self.clients.openWindow(url);
				}

				return;
			}

			const taken = await hand(client, {
				kind: "opened",
				id: identifier(data && data.lkn),
				link: ownPath(data && data.lklink),
				action: pressedAction(event.action),
				data: plainObject(data && data.lk) ?? null,
			});

			if (taken) {
				// The page's own app routes the tap; loading it again
				// would only throw its state away.
				return;
			}

			if (url && typeof client.navigate === "function") {
				try {
					await client.navigate(url);
				} catch (_error) {
					// The window is focused where it was; that is enough.
				}
			}

			return;
		}

		if (url) {
			await self.clients.openWindow(url);
		}
	}

	// One activation: take the pages this origin already has open. A
	// worker a page has just registered doesn't control that page until
	// its next load, and `WindowClient.navigate` is refused for a page the
	// worker doesn't control — so without the claim, a click focuses the
	// very tab the person enrolled from and leaves it where it was. This
	// worker handles no fetches, so claiming a page changes nothing about
	// how that page loads.
	async function activate() {
		try {
			await self.clients.claim();
		} catch (_error) {
			// Only the active worker may claim; a refusal leaves pages to
			// be controlled from their next load, as before.
		}
	}

	// The event wiring belongs to a service worker's own global only. A
	// page that evaluates this script — your build, or the tests — gets
	// the same functions to call directly, and no listeners.
	if (
		typeof self.registration === "object" &&
		self.registration !== null &&
		typeof self.registration.showNotification === "function"
	) {
		self.addEventListener("activate", (event) => {
			event.waitUntil(activate());
		});

		self.addEventListener("push", (event) => {
			event.waitUntil(deliver(event));
		});

		self.addEventListener("notificationclick", (event) => {
			event.waitUntil(open(event));
		});

		self.addEventListener("message", message);
	}

	self.lkn = { activate, deliver, envelopeOf, message, open, targetUrl };
})();
