// The leptoskit notifications receiver: the service-worker half of Web Push
// enrollment.
//
// Your app serves this script — `leptoskit_notifications::browser`
// builds it with your inbox route baked in — same-origin from its root (for
// example `/sw.js`) with a JavaScript content type, and registers it. On a
// `push` event it reads the notifications service's envelope — the title,
// the body, the notification's id, and the send's `data` — and shows what
// arrived with the platform's own presentation, tagged with the
// notification's id so a repeat replaces rather than stacks. On a click it
// closes the banner, focuses a window of this origin the person already
// has, and otherwise opens your inbox route with the notification's id.
//
// This worker holds no credentials and calls no server: nothing fetches,
// nothing evaluates, and no read state changes here — the authenticated app
// marks what it reads once it is open. An envelope that is missing,
// oversized, malformed, or typed wrong still shows one truthful banner
// ("New notification") under a stable tag, so a broken sender cannot
// silently swallow a delivery, and untrusted payload content can never
// choose where a click navigates.

(() => {
	"use strict";

	// The inbox route your build substituted in: the only URL a click may
	// open, on this origin, plus the notification's id as a query
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

	// Reads what arrived into what to show: a title, a body, a stable
	// tag, and the notification's id for the click. Never throws:
	// whatever a push carried, the answer is a banner.
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
			tag: tag(payload.lkn),
			id,
			lk: plainObject(payload.lk),
		};
	}

	// The stable tag: the notification's id when it is a plain short
	// string, the fallback otherwise. Two pushes for one notification
	// carry the same tag, so wherever the platform coalesces, a repeat
	// replaces.
	function tag(lkn) {
		return typeof lkn === "string" && PLAIN_TEXT.test(lkn) ? lkn : FALLBACK_TAG;
	}

	// The notification's id for URLs: only a real UUID passes, and only
	// this ever reaches the route a click opens.
	function identifier(lkn) {
		return typeof lkn === "string" && NOTIFICATION_ID.test(lkn) ? lkn : null;
	}

	function clipped(text, maxChars) {
		return text.length > maxChars ? text.slice(0, maxChars - 1) + "…" : text;
	}

	function plainObject(value) {
		return value && typeof value === "object" && !Array.isArray(value)
			? value
			: undefined;
	}

	// The URL a click opens: the built-in route on this origin, with the
	// notification's id when the envelope carried one. Payload content
	// cannot add a path, a host, a scheme, or anything else — a
	// hand-edited or mis-built route that left this origin is refused here
	// rather than navigated.
	function targetUrl(data) {
		let url;

		try {
			url = new URL(ROUTE, self.location.origin);
		} catch (_error) {
			return null;
		}

		if (url.origin !== self.location.origin) {
			return null;
		}

		const id = identifier(data && data.lkn);

		if (id) {
			url.searchParams.set("lkn", id);
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

	// One push: show what arrived. The registration's own presentation is
	// used, so what a banner looks like is the OS's business, not this
	// script's.
	async function deliver(event) {
		const envelope = envelopeOf(event.data);
		const options = { body: envelope.body, tag: envelope.tag };

		if (envelope.id !== null || envelope.lk !== undefined) {
			options.data = { lkn: envelope.id, lk: envelope.lk };
		}

		await self.registration.showNotification(envelope.title, options);
	}

	// One click: close the banner, then put the person on the
	// notification. An app window of this origin is focused and brought
	// to the inbox route; without one, the route opens in a new window.
	// The route is the constant this script was built with, checked
	// against this origin at click time.
	async function open(event) {
		event.notification.close();

		const url = targetUrl(event.notification && event.notification.data);
		let client = null;

		try {
			const matches = await self.clients.matchAll({
				type: "window",
				includeUncontrolled: true,
			});

			for (const match of matches) {
				if (match.url && sameOrigin(match.url)) {
					client = match;
					break;
				}
			}
		} catch (_error) {
			// No window list, so no window to focus; the route still
			// opens.
		}

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

	// The event wiring belongs to a service worker's own global only. A
	// page that evaluates this script — your build, or the tests — gets
	// the same functions to call directly, and no listeners.
	if (
		typeof self.registration === "object" &&
		self.registration !== null &&
		typeof self.registration.showNotification === "function"
	) {
		self.addEventListener("push", (event) => {
			event.waitUntil(deliver(event));
		});

		self.addEventListener("notificationclick", (event) => {
			event.waitUntil(open(event));
		});
	}

	self.lkn = { deliver, envelopeOf, open, targetUrl };
})();
