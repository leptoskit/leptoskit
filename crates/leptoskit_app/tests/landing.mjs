function assert(value, message = "assertion failed") {
	if (!value) throw new Error(message);
}

// The module defers the live example and wires the code style switches: it
// must not animate anything itself, it must import the client once, when the
// example nears the viewport or receives focus, and one switch must change
// every sample.
function environment({ observer = true, options = [], style } = {}) {
	globalThis.window = new EventTarget();
	window.scrollBy = () => {};
	globalThis.document = new EventTarget();
	document.documentElement = { dataset: style ? { codeStyle: style } : {} };
	document.querySelectorAll = () => options;
	const demo = new EventTarget();
	document.getElementById = (id) => (id === "live-demo" ? demo : null);
	const observers = [];

	if (observer) {
		globalThis.IntersectionObserver = class {
			constructor(callback, options) {
				this.callback = callback;
				this.options = options;
				this.observed = new Set();
				observers.push(this);
			}
			observe(target) {
				this.observed.add(target);
			}
			disconnect() {
				this.observed.clear();
			}
		};
		window.IntersectionObserver = globalThis.IntersectionObserver;
	} else {
		delete globalThis.IntersectionObserver;
	}

	return { demo, observers };
}

Deno.test("the client loads once the example nears the viewport", async () => {
	const env = environment();
	await import("../web/landing.mjs?near");
	assert(env.observers.length === 1, "one observer watches the example");
	const [loader] = env.observers;
	assert(loader.observed.has(env.demo));
	assert(loader.options.rootMargin === "250px");
	loader.callback([{ target: env.demo, isIntersecting: false }]);
	assert(loader.observed.size === 1, "a distant example keeps waiting");
	loader.callback([{ target: env.demo, isIntersecting: true }]);
	assert(loader.observed.size === 0, "the observer lets go once it loads");
});

Deno.test("keyboard focus loads the example before it scrolls into view", async () => {
	const env = environment();
	await import("../web/landing.mjs?focus");
	env.demo.dispatchEvent(new Event("focusin"));
	// Loading twice would be harmless but wasteful: the import is shared.
	env.observers[0].callback([{ target: env.demo, isIntersecting: true }]);
	assert(env.observers[0].observed.size === 0);
});

Deno.test("the module animates nothing itself", async () => {
	const env = environment();
	const animated = [];
	globalThis.requestAnimationFrame = () =>
		animated.push("requestAnimationFrame");
	window.matchMedia = () => animated.push("matchMedia");
	env.demo.animate = () => animated.push("animate");
	await import("../web/landing.mjs?still");
	env.observers[0].callback([{ target: env.demo, isIntersecting: true }]);
	env.demo.dispatchEvent(new Event("focusin"));
	delete globalThis.requestAnimationFrame;
	assert(
		animated.length === 0,
		`landing.mjs animated with ${animated.join(", ")}`,
	);
});

/** A switch's option: a radio that knows its group and where it is drawn. */
function option(id, group) {
	const radio = new EventTarget();
	radio.id = id;
	radio.attributes = {};
	radio.tabIndex = 0;
	radio.parentElement = group;
	radio.setAttribute = (name, value) => (radio.attributes[name] = value);
	radio.getBoundingClientRect = () => ({ top: 100 });
	radio.focus = () => (focused = radio);

	group.options.push(radio);

	return radio;
}
let focused;

function switches() {
	const groups = ["map-style", "source-style"].map((id) => {
		const group = { id, options: [] };
		group.querySelectorAll = () => group.options;
		return group;
	});
	return groups.flatMap((group) => [
		option(`${group.id}-markup`, group),
		option(`${group.id}-builder`, group),
	]);
}

/** The options' checked states, in order, as `m` and `b` for checked. */
const checked = (options) =>
	options.map((radio) =>
		radio.attributes["aria-checked"] === "true"
			? radio.id.endsWith("-builder") ? "b" : "m"
			: "."
	).join("");

Deno.test("every switch starts on the style the head script applied", async () => {
	const options = switches();
	environment({ options, style: "builder" });
	// Any write to the root's attribute restyles the whole page, so the module
	// leaves the head script's choice as it found it.
	const root = { style: "builder", writes: 0 };
	document.documentElement.dataset = {
		get codeStyle() {
			return root.style;
		},
		set codeStyle(style) {
			root.style = style;
			root.writes += 1;
		},
	};
	await import("../web/landing.mjs?saved");
	assert(
		root.writes === 0,
		`the module rewrote the style ${root.writes} times`,
	);
	assert(document.documentElement.dataset.codeStyle === "builder");
	assert(checked(options) === ".b.b", `checked ${checked(options)}`);
	assert(options[1].tabIndex === 0 && options[0].tabIndex === -1);
});

Deno.test("one switch changes every sample and remembers it", async () => {
	const options = switches();
	environment({ options });
	const saved = [];
	const storage = globalThis.localStorage;
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: { setItem: (key, value) => saved.push(`${key}=${value}`) },
	});
	try {
		await import("../web/landing.mjs?choose");
		assert(document.documentElement.dataset.codeStyle === "markup");
		assert(checked(options) === "m.m.", `checked ${checked(options)}`);

		options[3].dispatchEvent(new Event("click"));
		assert(document.documentElement.dataset.codeStyle === "builder");
		assert(checked(options) === ".b.b", `checked ${checked(options)}`);
		assert(saved.join() === "lk-code-style=builder", saved.join());

		const left = new Event("keydown");
		left.key = "ArrowLeft";
		options[1].dispatchEvent(left);
		assert(focused === options[0], "the arrow keys move within one switch");
		assert(checked(options) === "m.m.", `checked ${checked(options)}`);

		const other = new Event("storage");
		other.key = "lk-code-style";
		other.newValue = "builder";
		window.dispatchEvent(other);
		assert(checked(options) === ".b.b", "another tab's choice applies here");
	} finally {
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			value: storage,
		});
	}
});
