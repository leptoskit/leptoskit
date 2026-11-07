// The page's motion is CSS that leptoskit's renderer writes; this module
// wires the code style switches and defers the live example. Neither waits
// for the Wasm client. Without JavaScript the samples read as markup and the
// static example stays readable.

// Code style ----------------------------------------------------------------
// Samples read as `view!` markup or builder calls. The head's theme script
// applies the saved style before the first paint; either switch changes
// every sample, and the docs remember the choice too.
const root = document.documentElement;
const styleOptions = [
	...document.querySelectorAll(
		'[role="radiogroup"][id$="-style"] > [role="radio"]',
	),
];

/** @param {Element} option */
function styleOf(option) {
	return option.id.endsWith("-builder") ? "builder" : "markup";
}

/** @param {string} style */
function showStyle(style) {
	const chosen = style === "builder" ? "builder" : "markup";

	// Any write to the root's attribute restyles the whole page, even an
	// unchanged one, so the style the head script applied is left as it is.
	if (root.dataset.codeStyle !== chosen) root.dataset.codeStyle = chosen;

	for (const option of styleOptions) {
		const checked = styleOf(option) === chosen;
		option.setAttribute("aria-checked", String(checked));
		option.tabIndex = checked ? 0 : -1;
	}
}

/** @param {HTMLElement} option */
function chooseStyle(option) {
	const before = option.getBoundingClientRect().top;
	showStyle(styleOf(option));
	try {
		localStorage.setItem("lk-code-style", root.dataset.codeStyle);
	} catch {
		// Storage can be unavailable; the choice lasts until the page closes.
	}
	// Samples above this one change height too; keep it where it was.
	window.scrollBy({
		top: option.getBoundingClientRect().top - before,
		behavior: "instant",
	});
}

for (const option of styleOptions) {
	option.addEventListener("click", () => chooseStyle(option));
	option.addEventListener("keydown", (event) => {
		const step = event.key === "ArrowRight" || event.key === "ArrowDown"
			? 1
			: event.key === "ArrowLeft" || event.key === "ArrowUp"
			? -1
			: 0;
		if (!step || !option.parentElement) return;

		event.preventDefault();
		const group = [...option.parentElement.querySelectorAll('[role="radio"]')];
		const next =
			group[(group.indexOf(option) + step + group.length) % group.length];
		chooseStyle(next);
		next.focus();
	});
}

if (styleOptions.length > 0) showStyle(root.dataset.codeStyle);
window.addEventListener("storage", (event) => {
	if (event.key === "lk-code-style") showStyle(event.newValue);
});

// Live example --------------------------------------------------------------
// It loads the real LeptosKit Wasm client when the example comes near the
// viewport or receives focus, so the first paint never waits for it.
const demo = document.getElementById("live-demo");
let client;

function loadDemo() {
	client ??= import("/bootstrap.js");

	return client;
}

if (demo) {
	if ("IntersectionObserver" in window) {
		const loader = new IntersectionObserver((entries) => {
			if (entries.some((entry) => entry.isIntersecting)) {
				loader.disconnect();
				loadDemo().catch(() => {});
			}
		}, { rootMargin: "250px" });
		loader.observe(demo);
	} else {
		loadDemo().catch(() => {});
	}
	demo.addEventListener("focusin", () => loadDemo().catch(() => {}), {
		once: true,
	});
}
