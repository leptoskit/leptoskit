// The admin's behaviour. The server renders every page whole; this script
// only adds what a click or a key changes: the Workings drawer, copying,
// margin notes, the theme, sign-in and sign-out, actions, and live
// progress. Elements are found by their `data-testid`.

const BASE = "/_leptoskit/admin";
const root = document.documentElement;
const byId = (id) =>
	document.querySelector(`[data-testid="${CSS.escape(id)}"]`);
const all = (selector) => Array.from(document.querySelectorAll(selector));

async function post(path, body) {
	const response = await fetch(path, {
		method: "POST",
		headers: { "content-type": "application/json" },
		credentials: "same-origin",
		body: JSON.stringify(body),
	});
	let data = null;
	try {
		data = await response.json();
	} catch {
		data = null;
	}
	return { ok: response.ok, status: response.status, data };
}

function preference(change) {
	post(`${BASE}/api/preferences`, change).catch(() => {});
}

function notice(message) {
	const element = byId("notice");
	if (element) {
		element.textContent = message;
	}
}

// Workings ------------------------------------------------------------------

function openDrawer(id) {
	if (!byId(`drawer:${id}`)) {
		return;
	}
	root.setAttribute("data-admin-drawer", id);
	history.replaceState(null, "", `#workings=${id}`);
	byId("workings-close")?.focus({ preventScroll: true });
}

function closeDrawer() {
	const open = root.getAttribute("data-admin-drawer");
	root.removeAttribute("data-admin-drawer");
	for (const line of all("[data-linked]")) {
		line.removeAttribute("data-linked");
	}
	if (location.hash.startsWith("#workings=")) {
		history.replaceState(null, "", location.pathname + location.search);
	}
	if (open) {
		byId(`workings-tab:${open}`)?.focus({ preventScroll: true });
	}
}

function setWorkingsMode(on) {
	if (on) {
		root.setAttribute("data-admin-mode", "workings");
	} else {
		root.removeAttribute("data-admin-mode");
	}
	byId("workings-mode")?.setAttribute("aria-pressed", String(on));
	preference({ kind: "workings_mode", on });
}

function linkKey(key) {
	openDrawer("plan");
	for (const line of all("[data-linked]")) {
		line.removeAttribute("data-linked");
	}
	const lines = all(
		`[data-testid="code-line:${CSS.escape(key)}"], [data-testid^="code-line:${
			CSS.escape(key)
		}."]`,
	);
	for (const line of lines) {
		line.setAttribute("data-linked", "");
	}
	for (const row of all(`[data-testid="plan-row:${CSS.escape(key)}"]`)) {
		row.setAttribute("data-linked", "");
	}
	lines[0]?.scrollIntoView({ block: "center", behavior: "smooth" });
}

// Copying -------------------------------------------------------------------

async function copy(text, button) {
	try {
		await navigator.clipboard.writeText(text);
		if (button) {
			const label = button.textContent;
			button.textContent = "Copied";
			setTimeout(() => {
				button.textContent = label;
			}, 1200);
		}
	} catch {
		notice("The browser didn't allow copying. Select the text instead.");
	}
}

function codeText(block) {
	return Array.from(block.querySelectorAll('[data-testid="code-text"]'))
		.map((line) => line.textContent)
		.join("\n");
}

// The confirm sheet -----------------------------------------------------------

/** Text with `backticked` commands set as code, built without innerHTML. */
function prose(text) {
	return text.split("`").map((part, index) => {
		if (index % 2 === 0) return document.createTextNode(part);
		const code = document.createElement("code");
		code.textContent = part;
		code.style.cssText = "font-family:var(--admin-font-mono);font-size:.92em";
		return code;
	});
}

function confirmSheet(title, explanation) {
	return new Promise((resolve) => {
		const veil = document.createElement("div");
		veil.setAttribute("data-testid", "confirm-sheet");
		veil.style.cssText =
			"position:absolute;inset:0;z-index:50;display:flex;align-items:center;justify-content:center;padding:16px;background:rgba(22,17,31,.45)";
		const card = document.createElement("div");
		card.setAttribute("role", "dialog");
		card.setAttribute("aria-modal", "true");
		card.setAttribute("aria-labelledby", "confirm-title");
		card.style.cssText =
			"width:100%;max-width:440px;display:flex;flex-direction:column;gap:14px;padding:20px;border-radius:12px;border:1px solid var(--admin-line);background:var(--admin-surface);color:var(--admin-ink);font:14px/1.5 var(--admin-font-sans)";
		const heading = document.createElement("h2");
		heading.id = "confirm-title";
		heading.textContent = title;
		heading.style.cssText = "margin:0;font-size:17px;font-weight:600";
		const body = document.createElement("p");
		body.append(...prose(explanation));
		body.style.cssText = "margin:0;color:var(--admin-ink-2)";
		const row = document.createElement("div");
		row.style.cssText =
			"display:flex;gap:10px;justify-content:flex-end;flex-wrap:wrap";
		const cancel = document.createElement("button");
		cancel.textContent = "Cancel";
		cancel.setAttribute("data-testid", "confirm-no");
		cancel.style.cssText =
			"padding:8px 16px;border-radius:8px;border:1px solid var(--admin-line-strong);background:var(--admin-surface);color:var(--admin-ink);font-weight:600;cursor:pointer";
		const go = document.createElement("button");
		go.textContent = title;
		go.setAttribute("data-testid", "confirm-yes");
		go.style.cssText =
			"padding:8px 16px;border-radius:8px;border:1px solid var(--admin-button);background:var(--admin-button);color:var(--admin-button-text);font-weight:600;cursor:pointer;box-shadow:4px 4px 0 var(--admin-ember-warm)";
		row.append(cancel, go);
		card.append(heading, body, row);
		veil.append(card);
		document.body.append(veil);
		go.focus();
		const done = (answer) => {
			veil.remove();
			resolve(answer);
		};
		cancel.addEventListener("click", () => done(false));
		go.addEventListener("click", () => done(true));
		veil.addEventListener("click", (event) => {
			if (event.target === veil) {
				done(false);
			}
		});
		veil.addEventListener("keydown", (event) => {
			if (event.key === "Escape") {
				done(false);
			}
		});
	});
}

// Actions -------------------------------------------------------------------

function projectId() {
	const match = location.pathname.match(/\/infra\/projects\/([^/]+)/);
	return match ? decodeURIComponent(match[1]) : null;
}

function actionRequest(id) {
	const [, kind, ...rest] = id.split(":");
	const argument = rest.join(":");
	const version = () => byId("version:deploy")?.value?.trim() ?? "";
	switch (kind) {
		case "plan":
		case "status":
		case "backup":
			return { kind };
		case "apply":
			return { kind: "apply", plan_id: argument };
		case "deploy":
			return { kind: "deploy", version: version() };
		case "first-release":
			return { kind: "first_release", version: version() };
		case "maintenance":
			return { kind: "maintenance", version: version() };
		case "rollback":
			return { kind: "rollback", version: argument || version() };
		default:
			return null;
	}
}

async function runAction(button) {
	const id = button.getAttribute("data-testid");
	const project = projectId();
	const request = actionRequest(id);
	if (!project || !request) {
		return;
	}
	if ("version" in request && !request.version) {
		notice("Type an image tag or a git commit first.");
		byId("version:deploy")?.focus();
		return;
	}
	const title = button.textContent.trim();
	const explanation = button.getAttribute("aria-description") ?? "";
	if (!(await confirmSheet(title, explanation))) {
		return;
	}
	button.setAttribute("aria-busy", "true");
	button.setAttribute("aria-disabled", "true");
	const label = button.textContent;
	button.textContent = `${label.replace(/…$/, "")}…`;
	const result = await post(
		`${BASE}/api/projects/${encodeURIComponent(project)}/actions`,
		{
			...request,
			idempotency_key: crypto.randomUUID(),
		},
	).catch(() => ({ ok: false, data: null }));
	if (result.ok && result.data?.href) {
		if (request.kind === "plan" || request.kind === "apply") {
			preference({ kind: "use_note", note: "plan-review" });
		}
		location.assign(result.data.href);
		return;
	}
	button.removeAttribute("aria-busy");
	button.removeAttribute("aria-disabled");
	button.textContent = label;
	notice(
		result.data?.message ??
			"The admin couldn't reach the server. Nothing was sent.",
	);
	byId("notice")?.scrollIntoView({ block: "center" });
}

// Live progress ---------------------------------------------------------------

function followOperation() {
	const live = byId("live");
	const timeline = byId("timeline");
	if (!live || !timeline || !("EventSource" in window)) {
		return;
	}
	const source = new EventSource(`${location.pathname}/events`);
	const seen = new Set(
		all('[data-testid="event"]').map((_, index) => index + 1),
	);
	source.addEventListener("progress", (message) => {
		const event = JSON.parse(message.data);
		if (seen.has(event.sequence)) {
			return;
		}
		seen.add(event.sequence);
		const rows = all('[data-testid="event"]');
		const template = rows[rows.length - 1];
		if (!template) {
			location.reload();
			return;
		}
		const row = template.cloneNode(true);
		// The row that was last becomes an ordinary step: its connector
		// shows and its dot stops pulsing, like the first row's.
		const settled = rows[0];
		if (settled && settled !== template) {
			for (const index of [0, 1]) {
				const from = settled.children[index];
				const to = template.children[index];
				if (from && to) {
					to.className = from.className;
					to.setAttribute(
						"data-lk-style",
						from.getAttribute("data-lk-style") ?? "",
					);
				}
			}
		}
		const texts = row.lastElementChild?.children ?? [];
		if (texts[0]) {
			texts[0].textContent = event.message;
		}
		if (texts[1]) {
			texts[1].textContent = `${event.phase[0].toUpperCase()}${
				event.phase.slice(1)
			} · just now`;
		}
		timeline.append(row);
		live.textContent = `Following live: ${event.message}`;
	});
	source.addEventListener("done", () => {
		source.close();
		location.reload();
	});
	source.addEventListener("error", () => {
		live.textContent =
			"Lost the live connection. Reload to catch up; the operation keeps running.";
	});
}

// Search ----------------------------------------------------------------------

function searchTargets() {
	const links = all('[data-testid="rail"] a, [data-testid^="lesson:"]').map((
		link,
	) => ({
		label: (link.getAttribute("aria-label") || link.textContent || "").trim(),
		href: link.getAttribute("href"),
	}));
	const extra = [
		{ label: "Glossary", href: `${BASE}/learn/glossary` },
		{ label: "Your progress", href: `${BASE}/learn` },
	];
	return [...links, ...extra].filter((target) => target.label && target.href);
}

function search(input) {
	let results = byId("search-results");
	if (!results) {
		results = document.createElement("div");
		results.setAttribute("data-testid", "search-results");
		results.setAttribute("role", "listbox");
		results.style.cssText =
			"position:absolute;z-index:45;top:52px;left:50%;transform:translateX(-50%);width:min(360px,calc(100vw - 32px));display:flex;flex-direction:column;padding:6px;border-radius:12px;border:1px solid var(--admin-line);background:var(--admin-surface);font:14px var(--admin-font-sans)";
		document.body.append(results);
	}
	const query = input.value.trim().toLowerCase();
	results.replaceChildren();
	if (!query) {
		return;
	}
	for (
		const target of searchTargets().filter((target) =>
			target.label.toLowerCase().includes(query)
		).slice(0, 8)
	) {
		const link = document.createElement("a");
		link.href = target.href;
		link.textContent = target.label;
		link.setAttribute("role", "option");
		link.style.cssText =
			"padding:8px 10px;border-radius:8px;color:var(--admin-ink);text-decoration:none";
		results.append(link);
	}
	if (!results.childElementCount) {
		const empty = document.createElement("p");
		empty.textContent =
			"Nothing matches. Analytics screens and saved insights arrive with Analytics.";
		empty.style.cssText = "margin:0;padding:8px 10px;color:var(--admin-ink-3)";
		results.append(empty);
	}
}

// Wiring ----------------------------------------------------------------------

document.addEventListener("click", async (event) => {
	// The control pressed, not the decoration inside it (the Workings
	// switch's track, a row's text).
	const target = event.target instanceof Element
		? (event.target.closest("button[data-testid], a[data-testid]") ??
			event.target.closest("[data-testid]"))
		: null;
	if (!(target instanceof Element)) {
		if (root.hasAttribute("data-admin-menu")) {
			root.removeAttribute("data-admin-menu");
		}
		return;
	}
	const id = target.getAttribute("data-testid");

	if (id.startsWith("workings-tab:")) {
		event.preventDefault();
		const drawer = id.slice("workings-tab:".length);
		if (root.getAttribute("data-admin-drawer") === drawer) {
			closeDrawer();
		} else {
			openDrawer(drawer);
		}
		return;
	}
	if (id.startsWith("plan-row:")) {
		event.preventDefault();
		linkKey(id.slice("plan-row:".length));
		return;
	}
	if (id.startsWith("action:")) {
		event.preventDefault();
		if (target.getAttribute("aria-busy") !== "true") {
			await runAction(target);
		}
		return;
	}
	if (id.startsWith("note-dismiss:")) {
		const note = id.slice("note-dismiss:".length);
		byId(`note:${note}`)?.remove();
		preference({ kind: "dismiss_note", note });
		return;
	}

	switch (id) {
		case "workings-close":
			closeDrawer();
			break;
		case "workings-mode":
			setWorkingsMode(root.getAttribute("data-admin-mode") !== "workings");
			break;
		case "copy": {
			const block = target.closest('[data-testid="code"]');
			if (block) {
				await copy(codeText(block), target);
			}
			break;
		}
		case "copy-all": {
			const drawer = target.closest('[data-testid^="drawer:"]');
			if (drawer) {
				const blocks = Array.from(
					drawer.querySelectorAll('[data-testid="code"]'),
				);
				await copy(blocks.map(codeText).join("\n\n"), target);
			}
			break;
		}
		case "theme": {
			const dark = root.getAttribute("data-lk-appearance") === "dark" ||
				(!root.hasAttribute("data-lk-appearance") &&
					matchMedia("(prefers-color-scheme: dark)").matches);
			const next = dark ? "light" : "dark";
			root.setAttribute("data-lk-appearance", next);
			document.cookie =
				`admin_theme=${next}; path=/; max-age=31536000; samesite=lax`;
			break;
		}
		case "avatar":
			event.stopPropagation();
			root.toggleAttribute("data-admin-menu");
			break;
		case "sign-out":
			await post("/auth/sign-out", {}).catch(() => {});
			location.assign(`${BASE}/sign-in`);
			break;
		case "continue":
		case "switch-account": {
			// Switching asks the main site to sign someone in again
			// (`prompt=login`), rather than reusing the account it has.
			target.setAttribute("aria-busy", "true");
			const result = await post("/auth/oauth/start", {
				provider: "main",
				redirect_to: `${BASE}/sign-in`,
			}).catch(() => ({ ok: false, data: null }));
			if (result.ok && result.data?.url) {
				const url = new URL(result.data.url);
				if (id === "switch-account") url.searchParams.set("prompt", "login");
				location.assign(url.href);
			} else {
				target.removeAttribute("aria-busy");
				const message = document.createElement("p");
				message.setAttribute("role", "status");
				message.textContent = result.data?.message ??
					"Couldn't reach the main site. Check the admin's connection to it and try again.";
				message.style.cssText =
					"margin:0;color:var(--admin-bad);font-size:14px";
				target.closest('[data-testid="sign-in"], [data-testid="refusal"]')
					?.append(message);
			}
			break;
		}
		case "how-toggle":
			target.setAttribute(
				"aria-expanded",
				String(target.getAttribute("aria-expanded") !== "true"),
			);
			break;
		case "search-open": {
			const input = byId("search");
			if (input) {
				input.closest("div").style.display = "flex";
				input.focus();
			}
			break;
		}
		default:
			break;
	}
});

document.addEventListener("keydown", (event) => {
	const typing = event.target instanceof HTMLInputElement ||
		event.target instanceof HTMLTextAreaElement;
	if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
		event.preventDefault();
		byId("search")?.focus();
		return;
	}
	if (event.key === "Escape") {
		if (root.hasAttribute("data-admin-menu")) {
			root.removeAttribute("data-admin-menu");
		} else if (root.hasAttribute("data-admin-drawer")) {
			closeDrawer();
		}
		return;
	}
	if (
		typing || event.metaKey || event.ctrlKey || event.altKey ||
		event.key.toLowerCase() !== "w"
	) {
		return;
	}
	event.preventDefault();
	if (event.shiftKey) {
		setWorkingsMode(root.getAttribute("data-admin-mode") !== "workings");
		return;
	}
	if (root.hasAttribute("data-admin-drawer")) {
		closeDrawer();
		return;
	}
	const card = document.activeElement?.closest('[data-testid^="card:"]');
	const first = card ?? document.querySelector('[data-testid^="card:"]');
	if (first) {
		openDrawer(first.getAttribute("data-testid").slice("card:".length));
	}
});

document.addEventListener("input", (event) => {
	if (
		event.target instanceof HTMLInputElement &&
		event.target.getAttribute("data-testid") === "search"
	) {
		search(event.target);
	}
});

document.addEventListener("keydown", (event) => {
	if (
		event.key === "Enter" &&
		event.target instanceof HTMLInputElement &&
		event.target.getAttribute("data-testid") === "search"
	) {
		const first = byId("search-results")?.querySelector("a");
		if (first) {
			location.assign(first.getAttribute("href"));
		}
	}
});

// A link can open a card with its workings showing: `#workings=plan`.
const linked = location.hash.match(/^#workings=([\w:-]+)$/);
if (linked) {
	openDrawer(linked[1]);
}
// The plan's workings open by default the first three times.
if (root.getAttribute("data-admin-drawer") === "plan" && !linked) {
	preference({ kind: "use_note", note: "plan-workings" });
}
followOperation();
