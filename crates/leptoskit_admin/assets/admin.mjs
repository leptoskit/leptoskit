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

/** The control that opened the drawer, which gets focus back when it closes. */
let invoker = null;

const reducedMotion = () =>
	matchMedia("(prefers-reduced-motion: reduce)").matches;

function openDrawer(id, from = null) {
	const panel = byId(`drawer:${id}`);
	if (!panel) {
		return;
	}
	invoker = from instanceof HTMLElement && from !== document.body ? from : null;
	root.setAttribute("data-admin-drawer", id);
	history.replaceState(null, "", `#workings=${id}`);
	panel.querySelector('[data-testid="workings-close"]')?.focus({
		preventScroll: true,
	});
}

function closeDrawer() {
	const open = root.getAttribute("data-admin-drawer");
	root.removeAttribute("data-admin-drawer");
	for (const line of all("[data-linked]")) {
		line.removeAttribute("data-linked");
	}
	for (const status of all('[data-testid="drawer-changed"]')) {
		status.textContent = "";
	}
	if (location.hash.startsWith("#workings=")) {
		history.replaceState(null, "", location.pathname + location.search);
	}
	if (open) {
		// The exact control that opened it; another card can share its id.
		const back = invoker?.isConnected ? invoker : byId(`workings-tab:${open}`);
		back?.focus({ preventScroll: true });
	}
	invoker = null;
}

/** Shows `tab` in every Ask station, keeping the tabs' state in step. */
function selectAsk(tab) {
	root.setAttribute("data-admin-ask", tab);
	syncAsk();
	if (tab === "sql") {
		preference({ kind: "use_note", note: "ask-sql" });
	}
}

function syncAsk() {
	const tab = root.getAttribute("data-admin-ask") ?? "rust";
	for (const button of all('[data-testid^="ask-tab:"]')) {
		const on = button.getAttribute("data-testid") === `ask-tab:${tab}`;
		button.setAttribute("aria-selected", String(on));
		button.tabIndex = on ? 0 : -1;
	}
}

/** Arrow keys, Home, and End move along a tab list, as tabs expect. */
function moveAsk(current, key) {
	const tabs = Array.from(
		current.closest('[role="tablist"]')?.querySelectorAll('[role="tab"]') ??
			[],
	);
	const index = tabs.indexOf(current);
	if (index < 0) return false;
	const next = {
		ArrowRight: tabs[(index + 1) % tabs.length],
		ArrowLeft: tabs[(index - 1 + tabs.length) % tabs.length],
		Home: tabs[0],
		End: tabs[tabs.length - 1],
	}[key];
	if (!next) return false;
	selectAsk(next.getAttribute("data-testid").slice("ask-tab:".length));
	next.focus();
	return true;
}

/** Opens or closes a station; on phones stations start as headers. */
function setStation(station, open) {
	station.toggleAttribute("data-open", open);
	station
		.querySelector(':scope > [data-testid="station-toggle"]')
		?.setAttribute("aria-expanded", String(open));
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

function linkKey(key, from) {
	openDrawer("plan", from);
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

/**
 * A sheet that asks before doing something. `options.input` adds a text
 * field ({ label, value, placeholder }); `options.mustMatch` keeps the
 * button disabled until the field holds exactly that text, for what can't
 * be undone; `options.choices` adds a list of radio buttons ({ value,
 * label }). It resolves to `false`, or to `{ value, choice }`.
 */
function confirmSheet(title, explanation, options = {}) {
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
		card.append(heading, body);
		const fieldStyle =
			"padding:8px 12px;border-radius:8px;border:1px solid var(--admin-line-strong);background:var(--admin-surface);color:var(--admin-ink);font:14px var(--admin-font-sans)";
		let choice = null;
		if (options.choices?.length) {
			const list = document.createElement("div");
			list.setAttribute("role", "radiogroup");
			list.style.cssText = "display:flex;flex-direction:column;gap:6px";
			for (const [index, option] of options.choices.entries()) {
				const label = document.createElement("label");
				label.style.cssText =
					"display:flex;gap:8px;align-items:center;cursor:pointer";
				const radio = document.createElement("input");
				radio.type = "radio";
				radio.name = "confirm-choice";
				radio.value = option.value;
				radio.setAttribute("data-testid", `confirm-choice:${index}`);
				if (index === 0) {
					radio.checked = true;
					choice = option.value;
				}
				radio.addEventListener("change", () => {
					choice = option.value;
					if (field) field.disabled = option.value !== "";
				});
				label.append(radio, document.createTextNode(option.label));
				list.append(label);
			}
			card.append(list);
		}
		let field = null;
		if (options.input) {
			const label = document.createElement("label");
			label.style.cssText =
				"display:flex;flex-direction:column;gap:6px;font-weight:600;color:var(--admin-ink-2)";
			label.textContent = options.input.label;
			field = document.createElement("input");
			field.type = "text";
			field.value = options.input.value ?? "";
			field.placeholder = options.input.placeholder ?? "";
			field.setAttribute("data-testid", "confirm-input");
			field.style.cssText = fieldStyle;
			if (choice !== null && choice !== "") field.disabled = true;
			label.append(field);
			card.append(label);
		}
		const status = document.createElement("p");
		status.setAttribute("role", "status");
		status.setAttribute("data-testid", "confirm-status");
		status.style.cssText = "margin:0;color:var(--admin-bad);font-size:13px";
		card.append(status, row);
		if (options.danger) {
			go.style.background = "var(--admin-bad)";
			go.style.borderColor = "var(--admin-bad)";
			go.style.color = "#fff";
			go.style.boxShadow = "none";
		}
		const check = () => {
			if (options.mustMatch !== undefined) {
				go.disabled = field?.value.trim() !== options.mustMatch;
				go.style.opacity = go.disabled ? ".5" : "1";
			}
		};
		field?.addEventListener("input", check);
		check();
		veil.append(card);
		document.body.append(veil);
		(field && !field.disabled ? field : go).focus();
		const done = (answer) => {
			veil.remove();
			resolve(answer);
		};
		cancel.addEventListener("click", () => done(false));
		go.addEventListener("click", () => {
			if (go.disabled) return;
			done({ value: field?.value.trim() ?? "", choice });
		});
		field?.addEventListener("keydown", (event) => {
			if (event.key === "Enter" && !go.disabled) {
				done({ value: field.value.trim(), choice });
			}
		});
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

// Analytics -------------------------------------------------------------------

const pageParams = () =>
	Object.fromEntries(new URLSearchParams(location.search));

/** Lines of each code block in the drawers, by drawer and block. */
function drawerCode(scope) {
	const code = new Map();
	for (const drawer of scope.querySelectorAll('[data-testid^="drawer:"]')) {
		const blocks = Array.from(drawer.querySelectorAll('[data-testid="code"]'))
			.map((block) =>
				new Set(
					Array.from(block.querySelectorAll('[data-testid="code-text"]'))
						.map((line) => line.textContent),
				)
			);
		code.set(drawer.getAttribute("data-testid"), blocks);
	}
	return code;
}

let swapping = false;

/**
 * What finds an element again once a swap has replaced it: its region, its
 * test id or tag, and its link and name.
 */
function identify(element) {
	if (!(element instanceof HTMLElement)) return null;
	const region = element.closest(
		'[data-testid="main"], [data-testid="drawer-aside"]',
	);
	if (!region) return null;
	const id = element.getAttribute("data-testid");
	return {
		region: region.getAttribute("data-testid"),
		selector: id
			? `[data-testid="${CSS.escape(id)}"]`
			: element.tagName.toLowerCase(),
		href: element.getAttribute("href"),
		label: element.getAttribute("aria-label") ?? element.textContent.trim(),
	};
}

function locate(identity) {
	if (!identity) return null;
	return Array.from(
		byId(identity.region)?.querySelectorAll(identity.selector) ?? [],
	).find((element) =>
		element.getAttribute("href") === identity.href &&
		(element.getAttribute("aria-label") ?? element.textContent.trim()) ===
			identity.label
	) ?? null;
}

/** The open drawer's scroll position and opened stations, to keep. */
function drawerPlace() {
	const open = root.getAttribute("data-admin-drawer");
	const panel = open ? byId(`drawer:${open}`) : null;
	return {
		panel: open && `drawer:${open}`,
		scroll: panel?.querySelector('[data-testid="drawer-scroll"]')?.scrollTop ??
			0,
		stations: all(
			'[data-testid^="drawer:"] [data-testid^="station:"][data-open]',
		)
			.map((station) =>
				`${
					station.closest('[data-testid^="drawer:"]').getAttribute(
						"data-testid",
					)
				}|${station.getAttribute("data-testid")}`
			),
	};
}

function restoreDrawerPlace(place) {
	for (const key of place.stations) {
		const [drawer, station] = key.split("|");
		const found = byId(drawer)?.querySelector(
			`[data-testid="${CSS.escape(station)}"]`,
		);
		if (found) setStation(found, true);
	}
	const scroller = place.panel &&
		byId(place.panel)?.querySelector('[data-testid="drawer-scroll"]');
	if (scroller) scroller.scrollTop = place.scroll;
}

/**
 * After someone changes the builder, brings the open drawer's first changed
 * line into view, scrolling only the drawer, and says which station
 * changed. Focus stays where it is.
 */
function revealChange() {
	const open = root.getAttribute("data-admin-drawer");
	const panel = open ? byId(`drawer:${open}`) : null;
	if (!panel) return;
	const changed = Array.from(panel.querySelectorAll("[data-changed]"));
	if (!changed.length) return;
	// A line in an Ask tab nobody is looking at can't be shown; its station
	// still can.
	const tab = root.getAttribute("data-admin-ask") ?? "rust";
	const line = changed.find((candidate) => {
		const tabPanel = candidate.closest('[data-testid^="ask-panel:"]');
		return !tabPanel ||
			tabPanel.getAttribute("data-testid") === `ask-panel:${tab}`;
	});
	const station = (line ?? changed[0]).closest('[data-testid^="station:"]');
	if (!station) return;
	setStation(station, true);
	const name = station.querySelector('h3, [role="heading"]')?.textContent ??
		"A station";
	const text = line?.querySelector('[data-testid="code-text"]')?.textContent
		.trim().replace(/[,;]$/, "") ?? "";
	const status = panel.querySelector('[data-testid="drawer-changed"]');
	if (status) {
		status.textContent = text
			? `${name} changed: ${text.length > 48 ? `${text.slice(0, 47)}…` : text}`
			: `${name} changed`;
	}
	const scroller = panel.querySelector('[data-testid="drawer-scroll"]');
	const target = line ?? station;
	if (!scroller) return;
	const box = target.getBoundingClientRect();
	const view = scroller.getBoundingClientRect();
	if (box.top >= view.top + 24 && box.bottom <= view.bottom - 24) return;
	scroller.scrollTo({
		top: Math.max(0, scroller.scrollTop + box.top - view.top - view.height / 3),
		behavior: reducedMotion() ? "auto" : "smooth",
	});
}

/**
 * Fetches `href` and puts its page in place of this one, keeping the open
 * drawer open where it was and washing the lines of code that changed.
 * `reveal` is for a change someone made: the drawer then shows it.
 */
async function swapTo(href, push, reveal = false) {
	if (swapping) return false;
	swapping = true;
	try {
		const response = await fetch(href, { credentials: "same-origin" });
		if (!response.ok || response.redirected) {
			location.assign(href);
			return true;
		}
		const next = new DOMParser().parseFromString(
			await response.text(),
			"text/html",
		);
		const main = byId("main");
		const nextMain = next.querySelector('[data-testid="main"]');
		if (!main || !nextMain) {
			location.assign(href);
			return true;
		}
		const before = drawerCode(document);
		const focused = identify(document.activeElement);
		const opener = identify(invoker);
		const place = drawerPlace();
		main.replaceChildren(...nextMain.childNodes);
		const aside = byId("drawer-aside");
		const nextAside = next.querySelector('[data-testid="drawer-aside"]');
		if (aside && nextAside) {
			aside.replaceChildren(...nextAside.childNodes);
		}
		syncAsk();
		restoreDrawerPlace(place);
		if (invoker && !invoker.isConnected) invoker = locate(opener);
		locate(focused)?.focus({ preventScroll: true });
		const style = document.querySelector("head style");
		const nextStyle = next.querySelector("head style");
		if (style && nextStyle && style.textContent !== nextStyle.textContent) {
			style.textContent = nextStyle.textContent;
		}
		document.title = next.title;
		for (const [drawer, blocks] of drawerCode(document)) {
			const old = before.get(drawer);
			if (!old) continue;
			const panel = byId(drawer);
			const rendered = Array.from(
				panel?.querySelectorAll('[data-testid="code"]') ?? [],
			);
			for (const [index, block] of rendered.entries()) {
				const seen = old[index];
				if (!seen) continue;
				for (
					const line of block.querySelectorAll('[data-testid^="code-line"]')
				) {
					const text = line.querySelector('[data-testid="code-text"]')
						?.textContent;
					if (text && text.trim() && !seen.has(text)) {
						line.setAttribute("data-changed", "");
						line.closest('[data-testid^="station:"]')?.setAttribute(
							"data-changed-station",
							"",
						);
					}
				}
			}
		}
		if (push) {
			history.pushState(null, "", href + location.hash);
		}
		if (reveal) {
			revealChange();
		}
		return true;
	} catch {
		location.assign(href);
		return true;
	} finally {
		swapping = false;
	}
}

/** Whether a link only changes this page's query, so it can swap in place. */
/** Closes every open builder menu. */
function closeMenus() {
	for (
		const toggle of document.querySelectorAll(
			'[data-testid="menu-toggle"][aria-expanded="true"]',
		)
	) {
		toggle.setAttribute("aria-expanded", "false");
	}
}

function swappable(link) {
	if (!(link instanceof HTMLAnchorElement) || !byId("live-refresh")) {
		return false;
	}
	const url = new URL(link.href, location.href);
	return url.origin === location.origin && url.pathname === location.pathname;
}

window.addEventListener("popstate", () => {
	if (byId("live-refresh")) {
		swapTo(location.href, false);
	}
});

/** A sparkline's line and dot, redrawn from `values`. */
function redrawSpark(spark, values) {
	if (!spark || values.length < 2) return;
	const highest = Math.max(...values, Number.EPSILON);
	const points = values.map((value, index) => [
		(index / (values.length - 1)) * 97,
		25 - (value / highest) * 22,
	]);
	const path = spark.querySelector("path");
	if (path) {
		path.setAttribute(
			"d",
			points.map(([x, y], index) =>
				`${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`
			)
				.join(" "),
		);
	}
	const dot = spark.lastElementChild;
	if (dot && dot.tagName !== "svg") {
		const [x, y] = points[points.length - 1];
		dot.style.left = `${x.toFixed(1)}%`;
		dot.style.top = `${((y / 28) * 100).toFixed(1)}%`;
	}
}

let paused = false;
const waiting = [];
let lastLatest = null;
let lastRefresh = Date.now();

function filterLive() {
	const filter = byId("live-filter")?.value.trim().toLowerCase() ?? "";
	for (const row of all('[data-testid="live-event"]')) {
		const name = (row.getAttribute("aria-description") ?? "").toLowerCase();
		row.toggleAttribute(
			"data-filtered",
			filter !== "" && !name.includes(filter),
		);
	}
}

function addLiveEvent(event) {
	const list = byId("live-events");
	const template = list?.querySelector('[data-testid="live-event"]');
	if (!list || !template) {
		location.reload();
		return;
	}
	const row = template.cloneNode(true);
	row.setAttribute("aria-description", event.name);
	const at = new Date(event.timestamp);
	const time = Number.isNaN(at.getTime()) ? "" : at.toISOString().slice(11, 19);
	const toggle = row.querySelector('[data-testid="how-toggle"]');
	toggle?.setAttribute("aria-expanded", "false");
	toggle?.setAttribute(
		"aria-label",
		`${event.name} from ${event.distinct_id} at ${time}`,
	);
	const cells = toggle?.firstElementChild?.children ?? [];
	if (cells[0]) cells[0].textContent = time;
	const title = cells[1];
	if (title) {
		const name = title.firstElementChild;
		if (name) name.textContent = event.name;
		// Every row has the badge; this event decides whether it shows, not
		// the row it was copied from.
		const badge = title.querySelector('[data-testid="struct-badge"]');
		if (event.typed) badge?.removeAttribute("aria-hidden");
		else badge?.setAttribute("aria-hidden", "true");
	}
	if (cells[2]) cells[2].textContent = event.properties?.$pathname ?? "";
	if (cells[3]) cells[3].textContent = event.distinct_id;
	const panel = row.querySelector('[data-testid="how-panel"]');
	const code = panel?.querySelector('[data-testid="code"]');
	const line = code?.querySelector('[data-testid="code-line"]');
	if (code && line) {
		for (const old of code.querySelectorAll('[data-testid="code-line"]')) {
			old.remove();
		}
		const copyButton = code.querySelector('[data-testid="copy"]');
		for (
			const text of JSON.stringify(event.properties ?? {}, null, 2).split("\n")
		) {
			const next = line.cloneNode(true);
			const span = next.querySelector('[data-testid="code-text"]');
			if (span) span.textContent = text;
			code.insertBefore(next, copyButton);
		}
	}
	list.prepend(row);
	for (const extra of Array.from(list.children).slice(200)) {
		extra.remove();
	}
	filterLive();
}

function followAnalytics() {
	if (!byId("pulse-rate") || !("EventSource" in window)) {
		return;
	}
	const tail = Boolean(byId("live-events"));
	const source = new EventSource(
		`${BASE}/api/analytics/live${tail ? "?events=1" : ""}`,
	);
	source.addEventListener("pulse", (message) => {
		const pulse = JSON.parse(message.data);
		const rate = byId("pulse-rate");
		if (rate) rate.textContent = `${Math.round(pulse.per_second)}/s`;
		byId("live-pulse")?.setAttribute(
			"aria-label",
			`Live events: ${pulse.per_second.toFixed(1)} a second`,
		);
		const dot = byId("pulse-dot");
		if (dot) {
			dot.style.background = pulse.status.state === "live"
				? "var(--admin-ember)"
				: "var(--admin-ink-3)";
		}
		redrawSpark(byId("pulse-spark"), pulse.seconds);
		const liveRate = byId("live-rate");
		if (liveRate) liveRate.textContent = `${pulse.per_second.toFixed(1)}/s`;
		const minute = byId("live-minute");
		if (minute) minute.textContent = pulse.per_minute.toLocaleString("en");
		redrawSpark(byId("live-spark"), pulse.seconds);
		// A page that draws from the data re-reads it when there's more, at
		// most every ten seconds, unless someone is in the middle of something.
		if (lastLatest === null) {
			lastLatest = pulse.latest;
		} else if (
			pulse.latest > lastLatest && byId("live-refresh") &&
			Date.now() - lastRefresh > 10_000 &&
			!document.querySelector(
				'[data-testid="how-toggle"][aria-expanded="true"],[data-testid="menu-toggle"][aria-expanded="true"]',
			) &&
			!byId("confirm-sheet") &&
			!(document.activeElement instanceof HTMLInputElement)
		) {
			lastLatest = pulse.latest;
			lastRefresh = Date.now();
			swapTo(location.href, false);
		}
	});
	if (tail) {
		source.addEventListener("event", (message) => {
			const event = JSON.parse(message.data);
			if (paused) {
				waiting.push(event);
				const pause = byId("live-pause");
				if (pause) pause.textContent = `Resume (${waiting.length})`;
			} else {
				addLiveEvent(event);
			}
		});
	}
}

/** Which insight the page shows: its kind, saved id, and name. */
function pageInsight() {
	const [kind, id, ...name] = (
		byId("save-insight")?.getAttribute("aria-description") ??
			"web||Web overview"
	).split("|");
	return { kind, insight: id || null, name: name.join("|") };
}

async function saveInsight() {
	const { kind, insight, name } = pageInsight();
	const answer = await confirmSheet(
		insight ? "Save changes" : "Save",
		insight
			? "Saves what the builder shows now over this insight. Dashboards that hold it update."
			: "Saves this insight to the library, where anyone with the admin can open it.",
		{ input: { label: "Name", value: name } },
	);
	if (!answer) return;
	const result = await post(`${BASE}/api/analytics/insights`, {
		kind,
		insight,
		name: answer.value,
		params: pageParams(),
	}).catch(() => ({ ok: false, data: null }));
	if (result.ok && result.data?.href) {
		location.assign(result.data.href);
	} else {
		notice(
			result.data?.message ?? "The admin couldn't save it. Nothing changed.",
		);
	}
}

async function addToDashboard(button) {
	const { kind, insight, name } = pageInsight();
	const listed = await fetch(`${BASE}/api/analytics/dashboards`, {
		credentials: "same-origin",
	}).then((response) => response.json()).catch(() => ({ dashboards: [] }));
	const choices = (listed.dashboards ?? []).map((dashboard) => ({
		value: dashboard.id,
		label: `${dashboard.name} (${dashboard.cards} card${
			dashboard.cards === 1 ? "" : "s"
		})`,
	}));
	choices.push({ value: "", label: "A new dashboard" });
	const answer = await confirmSheet(
		"Add to dashboard",
		`Adds “${button.getAttribute("aria-description") ?? name}” as a card${
			insight ? "" : ", saving it to the library first"
		}.`,
		{ choices, input: { label: "New dashboard's name", value: "Product" } },
	);
	if (!answer) return;
	const result = await post(`${BASE}/api/analytics/dashboards`, {
		kind,
		insight,
		name,
		params: pageParams(),
		dashboard: answer.choice || null,
		dashboard_name: answer.choice ? null : answer.value,
	}).catch(() => ({ ok: false, data: null }));
	if (result.ok && result.data?.href) {
		location.assign(result.data.href);
	} else {
		notice(
			result.data?.message ?? "The admin couldn't add it. Nothing changed.",
		);
	}
}

async function exportPerson(button) {
	const id = button.getAttribute("aria-description");
	const answer = await confirmSheet(
		"Export",
		"Downloads everything stored about this person as JSON: their properties, ids, events, and sessions. Handle it as personal data.",
	);
	if (!answer) return;
	const response = await fetch(`${BASE}/api/analytics/people/${id}/export`, {
		credentials: "same-origin",
	}).catch(() => null);
	if (!response?.ok) {
		notice("The admin couldn't export them.");
		return;
	}
	const link = document.createElement("a");
	link.href = URL.createObjectURL(await response.blob());
	link.download = `person-${id}.json`;
	link.click();
	setTimeout(() => URL.revokeObjectURL(link.href), 1000);
}

async function deletePerson(button) {
	const [id, ...name] = (button.getAttribute("aria-description") ?? "").split(
		"|",
	);
	const confirmation = name.join("|");
	const answer = await confirmSheet(
		"Delete",
		`Deletes this person, every id merged into them, and all their events and sessions, for good. Type \`${confirmation}\` to confirm.`,
		{
			input: { label: "Type their name to confirm", placeholder: confirmation },
			mustMatch: confirmation,
			danger: true,
		},
	);
	if (!answer) return;
	const result = await post(`${BASE}/api/analytics/people/${id}/delete`, {
		confirm: answer.value,
	}).catch(() => ({ ok: false, data: null }));
	if (result.ok && result.data?.href) {
		location.assign(result.data.href);
	} else {
		notice(
			result.data?.message ??
				"The admin couldn't delete them. Nothing was deleted.",
		);
	}
}

function searchPeople() {
	const params = new URLSearchParams();
	for (
		const [key, id] of [
			["search", "people-search"],
			["property", "people-property"],
			["value", "people-value"],
		]
	) {
		const value = byId(id)?.value.trim();
		if (value) params.set(key, value);
	}
	location.assign(`${BASE}/analytics/people${params.size ? `?${params}` : ""}`);
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
	// A builder control only changes this page's query: fetch it in place.
	const link = event.target instanceof Element
		? event.target.closest("a[href]")
		: null;
	if (
		link && event.button === 0 && !event.metaKey && !event.ctrlKey &&
		!event.shiftKey && !event.altKey && swappable(link)
	) {
		event.preventDefault();
		closeMenus();
		await swapTo(link.href, true, true);
		return;
	}
	// A press outside an open menu closes it.
	if (
		!(event.target instanceof Element &&
			event.target.closest(
				'[data-testid="menu-panel"],[data-testid="menu-toggle"]',
			))
	) {
		closeMenus();
	}
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
			openDrawer(drawer, target);
		}
		return;
	}
	if (id.startsWith("plan-row:")) {
		event.preventDefault();
		linkKey(id.slice("plan-row:".length), target);
		return;
	}
	if (id.startsWith("action:")) {
		event.preventDefault();
		if (target.getAttribute("aria-busy") !== "true") {
			await runAction(target);
		}
		return;
	}
	if (id.startsWith("ask-tab:")) {
		selectAsk(id.slice("ask-tab:".length));
		return;
	}
	if (id.startsWith("note-dismiss:")) {
		const note = id.slice("note-dismiss:".length);
		byId(`note:${note}`)?.remove();
		preference({ kind: "dismiss_note", note });
		return;
	}

	switch (id) {
		case "station-toggle": {
			const station = target.closest('[data-testid^="station:"]');
			if (station) setStation(station, !station.hasAttribute("data-open"));
			break;
		}
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
		case "save-insight":
			await saveInsight();
			break;
		case "add-to-dashboard":
			await addToDashboard(target);
			break;
		case "person-export":
			await exportPerson(target);
			break;
		case "person-delete":
			await deletePerson(target);
			break;
		case "people-apply":
			searchPeople();
			break;
		case "live-pause": {
			paused = !paused;
			target.setAttribute("aria-pressed", String(paused));
			if (paused) {
				target.textContent = "Resume";
			} else {
				target.textContent = "Pause";
				for (const waited of waiting.splice(0)) {
					addLiveEvent(waited);
				}
			}
			preference({ kind: "use_note", note: "live-pause" });
			break;
		}
		case "menu-toggle": {
			const open = target.getAttribute("aria-expanded") !== "true";
			closeMenus();
			target.setAttribute("aria-expanded", String(open));
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
	if (
		event.target instanceof HTMLElement &&
		(event.target.getAttribute("data-testid") ?? "").startsWith("ask-tab:") &&
		!event.metaKey && !event.ctrlKey && !event.altKey &&
		moveAsk(event.target, event.key)
	) {
		event.preventDefault();
		return;
	}
	if (event.key === "Escape") {
		if (
			document.querySelector(
				'[data-testid="menu-toggle"][aria-expanded="true"]',
			)
		) {
			closeMenus();
		} else if (root.hasAttribute("data-admin-menu")) {
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
		openDrawer(
			first.getAttribute("data-testid").slice("card:".length),
			card ? document.activeElement : null,
		);
	}
});

document.addEventListener("input", (event) => {
	if (
		event.target instanceof HTMLInputElement &&
		event.target.getAttribute("data-testid") === "search"
	) {
		search(event.target);
	}
	if (
		event.target instanceof HTMLInputElement &&
		event.target.getAttribute("data-testid") === "live-filter"
	) {
		filterLive();
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
	if (
		event.key === "Enter" && event.target instanceof HTMLInputElement &&
		(event.target.getAttribute("data-testid") ?? "").startsWith("people-")
	) {
		searchPeople();
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
followAnalytics();
