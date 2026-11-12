import assert from "node:assert/strict";
import { LiveConnection } from "./live.ts";

// Drive transport stalls without wall-clock waits or a network dependency.
let now = 0;
let nextTimer = 0;
const timers = new Map();
globalThis.setTimeout = (run, delay = 0) => {
	const id = ++nextTimer;
	timers.set(id, { run, at: now + delay });
	return id;
};
globalThis.clearTimeout = (id) => timers.delete(id);
globalThis.setInterval = (run, delay) => {
	const id = ++nextTimer;
	timers.set(id, { run, at: now + delay, interval: delay });
	return id;
};
globalThis.clearInterval = globalThis.clearTimeout;
Date.now = () => now;
Math.random = () => 0;
const advance = (duration) => {
	const end = now + duration;
	let steps = 0;
	while (true) {
		const first = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
		if (first === undefined || first[1].at > end) break;
		assert.ok(++steps < 1000, "retry timers must not spin");
		const [id, timer] = first;
		now = timer.at;
		if (timer.interval === undefined) timers.delete(id);
		else timer.at += timer.interval;
		timer.run();
	}
	now = end;
};

const sockets = [];
class Socket {
	readyState = 0;
	onopen = null;
	onclose = null;
	onmessage = null;
	onerror = null;
	frames = [];
	closed = 0;
	constructor() {
		sockets.push(this);
	}
	send(data) {
		assert.equal(this.readyState, 1, "only an open socket may send");
		const frame = JSON.parse(data);
		this.frames.push(frame);
		if (frame.type === "ping") this.receive({ type: "pong" });
	}
	close() {
		this.closed++;
		this.readyState = 3;
		this.onclose?.({});
	}
	async open() {
		this.readyState = 1;
		this.onopen?.({});
		await Promise.resolve();
	}
	receive(frame) {
		this.onmessage?.({ data: JSON.stringify(frame) });
	}
}

const connection = (options = {}) => new LiveConnection("ws://example.test/live", {
	WebSocket: Socket,
	minBackoffMs: 1,
	maxBackoffMs: 1,
	...options,
});
const welcome = (socket) => socket.receive({
	type: "welcome",
	connection: "live-test",
	version: 1,
	heartbeat_seconds: 30,
});
const finish = (live) => {
	live.close();
	assert.equal(timers.size, 0, "closing must cancel every timer");
	sockets.length = 0;
};

// A TCP/WebSocket upgrade that never completes must not strand subscribers.
{
	const live = connection();
	live.subscribe("notes", {}, () => {});
	advance(9999);
	assert.equal(sockets.length, 1, "give the initial connection its deadline");
	advance(1);
	assert.equal(sockets.length, 2, "retry a stalled WebSocket upgrade");
	assert.equal(sockets[0].closed, 1);
	finish(live);
}

// Opening a socket is insufficient: the live protocol must welcome it too.
{
	const live = connection({ connectTimeoutMs: 50 });
	live.subscribe("notes", {}, () => {});
	await sockets[0].open();
	advance(50);
	assert.equal(sockets.length, 2, "retry an open socket without a welcome");
	finish(live);
}

// A reconnect can stall too. Keep its subscriptions and pending mutation,
// retry once, ignore stale callbacks, then deliver the restarted server's data.
{
	const live = connection({ connectTimeoutMs: 50 });
	const values = [];
	live.subscribe("notes", {}, (value) => values.push(value));
	await sockets[0].open();
	welcome(sockets[0]);
	sockets[0].receive({ type: "done", request: 1, version: 1, ok: ["before"] });
	advance(10000);
	assert.equal(sockets.length, 1, "welcome cancels the connection deadline");
	sockets[0].close();
	advance(0);
	const stalled = sockets[1];
	const pending = live.mutate("add_note", { body: "after" }, "same-write");
	advance(50);
	assert.equal(sockets.length, 3, "retry the stalled reconnect exactly once");
	stalled.onclose?.({});
	stalled.onopen?.({});
	stalled.receive({ type: "welcome", connection: "stale" });
	advance(0);
	assert.equal(sockets.length, 3, "stale callbacks must not replace the retry");
	const resumed = sockets[2];
	await resumed.open();
	welcome(resumed);
	assert.deepEqual(resumed.frames.filter((frame) => frame.type === "sub"), [
		{ type: "sub", id: 1, query: "notes", args: {} },
	]);
	assert.deepEqual(resumed.frames.filter((frame) => frame.type === "mutate"), [
		{ type: "mutate", request: 2, name: "add_note", args: { body: "after" }, key: "same-write" },
	]);
	resumed.receive({ type: "transition", from: 1, to: 2, updates: [
		{ id: 1, ok: ["before", "after"] },
	] });
	resumed.receive({ type: "done", request: 2, version: 2, ok: "written" });
	assert.equal(await pending, "written");
	assert.deepEqual(values, [["before"], ["before", "after"]]);
	advance(50000);
	assert.equal(sockets.length, 3, "a healthy connection must remain live");
	finish(live);
}

// Closing while connecting leaves no delayed reconnect behind.
{
	const live = connection({ connectTimeoutMs: 50 });
	live.subscribe("notes", {}, () => {});
	finish(live);
	advance(10000);
	assert.equal(sockets.length, 0);
}
console.log("PASS: stalled upgrades, missing welcomes, reconnect recovery, and cleanup");
