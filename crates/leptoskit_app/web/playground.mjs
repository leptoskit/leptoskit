const source = document.querySelector("#source");
const status = document.querySelector("#status");
const run = document.querySelector("#run");
const output = document.querySelector("#output");
const diagnostics = document.querySelector("#diagnostics");
const starter = source.value;
const examples = {
	Counter: starter,
	Form: starter.replace(
		'<Text>"A small idea starts here"</Text>',
		'<TextInput placeholder="Your next idea" />',
	),
	List: starter.replace(
		'<Text>"A small idea starts here"</Text>',
		'<View>{["Sketch", "Build", "Ship"].map(|item| view! { <Text>{item}</Text> })}</View>',
	),
	Motion: starter.replace(
		'<Text>"A small idea starts here"</Text>',
		'<Text motion=Motion::enter(Pose::new().scale(0.3)).spring(Spring::BOUNCY)>"Hello, motion"</Text>',
	),
	Layout: starter.replace(
		'"p-8 gap-4 flex-col"',
		'"p-8 gap-4 flex-row flex-wrap"',
	),
};

let frame;
let bundle;
let bundleSent = false;
let lastHeartbeat = 0;
let lastCompileSeconds = "0.0";
let active;

function stop() {
	frame?.remove();
	frame = undefined;
	active?.abort();
	status.textContent = "Stopped · Reload to start the app again";
}

function mount() {
	stop();

	if (!bundle) return;

	frame = document.createElement("iframe");
	bundleSent = false;
	frame.title = "Your compiled LeptosKit app";
	frame.sandbox = "allow-scripts";
	frame.src = "/play/frame.html";
	output.replaceChildren(frame);
	lastHeartbeat = Date.now();
	status.textContent = "Starting preview…";
}

addEventListener("message", (event) => {
	if (
		!frame || event.source !== frame.contentWindow || event.origin !== "null"
	) return;
	if (event.data?.type === "ready" && !bundleSent) {
		bundleSent = true;
		frame.contentWindow.postMessage({ type: "bundle", ...bundle }, "*");
	}
	if (event.data?.type === "heartbeat") lastHeartbeat = Date.now();
	if (event.data?.type === "mounted") {
		status.textContent =
			`Running · your app is live · compiled in ${lastCompileSeconds}s`;
	}
	if (event.data?.type === "runtime-error") {
		status.textContent = `Runtime error: ${event.data.message}`;
	}
});
setInterval(() => {
	if (frame && Date.now() - lastHeartbeat > 4000) {
		status.textContent = "Preview is not responding · use Stop or Reload";
	}
}, 1000);
function showDiagnostics(result) {
	for (const item of result.diagnostics ?? []) {
		const button = document.createElement("button");
		button.textContent = `${item.line}:${item.column} · ${item.message}`;
		button.onclick = () => {
			const lines = source.value.split("\n");
			const offset = lines.slice(0, item.line - 1).reduce((n, line) =>
				n + line.length + 1, 0) +
				Array.from(lines[item.line - 1] ?? "").slice(0, item.column - 1)
					.join("").length;
			source.focus();
			source.setSelectionRange(offset, offset);
		};
		diagnostics.append(button);
	}

	const text = document.createElement("pre");
	text.textContent = result.stderr.replace(/\u001b\[[0-9;]*m/g, "");
	diagnostics.append(text);
}
async function queueStatus(service, controller) {
	try {
		const response = await fetch(`${service}/status`, {
			signal: controller.signal,
			credentials: "omit",
		});

		if (!response.ok) return "queue status unavailable";

		const queue = await response.json();
		return `${queue.running} running · ${queue.queued} queued`;
	} catch {
		return "queue status unavailable";
	}
}

async function compile() {
	if (run.disabled) return;
	run.disabled = true;
	diagnostics.replaceChildren();
	const began = performance.now();
	const controller = new AbortController();
	active = controller;
	const service = document.querySelector("#playground").dataset.service.replace(
		/\/$/,
		"",
	);
	let queueState = "waiting for a worker";
	const elapsed = setInterval(() => {
		status.textContent = `Compiling · ${queueState} · ${
			((performance.now() - began) / 1000).toFixed(1)
		}s`;
	}, 100);
	const queuePoll = setInterval(async () => {
		queueState = await queueStatus(service, controller);
	}, 1000);
	const timeout = setTimeout(() => controller.abort(), 45000);
	try {
		const response = await fetch(`${service}/compile`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ source: source.value }),
			signal: controller.signal,
			credentials: "omit",
		});
		if (response.status === 503 || response.status === 429) {
			status.textContent = `Service busy · retry in ${
				response.headers.get("Retry-After") ?? 5
			} seconds`;
			return;
		}
		if (response.status === 413) {
			status.textContent = "Source is too large for this service";
			return;
		}

		const result = await response.json();
		if (!response.ok) {
			status.textContent = result.error ??
				"Compile service rejected the request";
			return;
		}

		if (result.wasm) {
			lastCompileSeconds = ((performance.now() - began) / 1000).toFixed(1);
			bundle = result;
			mount();
		} else {
			status.textContent = "Compiler found a problem";
			showDiagnostics(result);
		}
	} catch (error) {
		status.textContent = error.name === "AbortError"
			? "Compilation stopped or timed out"
			: "Compile service unavailable · your code is still here. Try Run again.";
	} finally {
		clearInterval(elapsed);
		clearInterval(queuePoll);
		clearTimeout(timeout);
		controller.abort();
		run.disabled = false;
		if (active === controller) active = undefined;
	}
}
run.onclick = compile;
document.querySelector("#reload").onclick = mount;
document.querySelector("#stop").onclick = stop;
document.querySelector("#starter").onchange = (event) => {
	source.value = examples[event.target.value];
};
document.querySelector("#share").onclick = () => {
	location.hash = encodeURIComponent(source.value);
	status.textContent =
		"Share this page’s URL · the fragment contains your code";
};
if (location.hash.length > 1 && location.hash.length <= 200000) {
	try {
		source.value = decodeURIComponent(location.hash.slice(1));
	} catch {
		status.textContent = "This share link could not be read";
	}
}

addEventListener("keydown", (event) => {
	if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
		event.preventDefault();
		compile();
	}
});
let releaseTab = false;
source.addEventListener("blur", () => {
	releaseTab = false;
});
source.addEventListener("keydown", (event) => {
	if (event.key === "Escape") {
		releaseTab = true;
		return;
	}

	if (event.key !== "Tab") return;

	if (releaseTab) {
		releaseTab = false;
		return;
	}

	event.preventDefault();
	const start = source.selectionStart;
	const end = source.selectionEnd;
	if (start === end && !event.shiftKey) {
		source.setRangeText("    ", start, end, "end");
		return;
	}

	const first = source.value.lastIndexOf("\n", start - 1) + 1;
	const last = source.value.indexOf("\n", end > start ? end - 1 : end);
	const boundary = last < 0 ? source.value.length : last;
	const lines = source.value.slice(first, boundary).split("\n");
	const indented = lines.map((line) =>
		event.shiftKey ? line.replace(/^( {1,4}|\t)/, "") : `    ${line}`
	).join("\n");
	source.setRangeText(indented, first, boundary, "select");
});
