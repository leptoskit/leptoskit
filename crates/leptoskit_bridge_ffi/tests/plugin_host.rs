#![cfg(feature = "app")]

use std::sync::Arc;
use std::sync::Barrier;
use std::sync::mpsc;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::PluginId;
use leptoskit_bridge_ffi::app::PluginHost;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;

/// How long a test waits for a worker or the runtime to report back.
const PATIENCE: Duration = Duration::from_secs(10);

/// Answers with its payload.
struct Echo(&'static str);

impl Plugin for Echo {
	fn name(&self) -> &'static str {
		self.0
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn invoke(&mut self, _: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		Ok(payload)
	}
}

/// Says when a call has entered it, and answers once the test lets it
/// through.
struct Gate(Arc<Barrier>, mpsc::Sender<()>);

impl Plugin for Gate {
	fn name(&self) -> &'static str {
		"gate"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn invoke(&mut self, _: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		let _ = self.1.send(());
		self.0.wait();
		Ok(payload)
	}
}

/// A started guest and a plugin host whose wakes arrive on the channel.
fn host() -> (WasmHost<HeadlessBackend>, PluginHost, mpsc::Receiver<()>) {
	let mut host = WasmHost::new(HeadlessBackend::new(), GUEST).unwrap();
	host.start(320.0, 480.0).unwrap();
	let (woken, wakes) = mpsc::channel();
	let plugins = PluginHost::new(move || {
		let _ = woken.send(());
	});
	(host, plugins, wakes)
}

fn call(host: &mut WasmHost<HeadlessBackend>, plugin: PluginId, payload: &[u8]) -> CallId {
	host.plugins()
		.submit(plugin, MethodId(0), Bytes::from(payload.to_vec()))
		.unwrap()
}

fn reply(host: &mut WasmHost<HeadlessBackend>, id: CallId) -> Option<Result<Vec<u8>, Error>> {
	host.plugins()
		.result(id)
		.unwrap()
		.map(|result| result.clone().map(|bytes| bytes.to_vec()))
}

#[test]
fn synchronous_plugins_answer_inside_the_pump() {
	let (mut host, mut plugins, _wakes) = host();
	let echo = plugins.install(&mut host, Box::new(Echo("echo"))).unwrap();
	let id = call(&mut host, echo, b"hello");

	plugins.pump(&mut host).unwrap();

	assert_eq!(reply(&mut host, id), Some(Ok(b"hello".to_vec())));
}

#[test]
fn worker_plugins_answer_off_the_thread_and_wake_the_runner() {
	let (mut host, mut plugins, wakes) = host();
	let gate = Arc::new(Barrier::new(2));
	let (entered, _entries) = mpsc::channel();
	let worker = plugins
		.install_worker(&mut host, Box::new(Gate(Arc::clone(&gate), entered)))
		.unwrap();
	let id = call(&mut host, worker, b"later");

	plugins.pump(&mut host).unwrap();
	assert_eq!(reply(&mut host, id), None, "the worker is still waiting");

	gate.wait();
	wakes.recv_timeout(PATIENCE).unwrap();
	plugins.pump(&mut host).unwrap();

	assert_eq!(reply(&mut host, id), Some(Ok(b"later".to_vec())));
}

#[test]
fn a_cancelled_worker_call_is_dropped_and_the_plugin_keeps_working() {
	let (mut host, mut plugins, wakes) = host();
	let gate = Arc::new(Barrier::new(2));
	let (entered, entries) = mpsc::channel();
	let worker = plugins
		.install_worker(&mut host, Box::new(Gate(Arc::clone(&gate), entered)))
		.unwrap();
	let cancelled = call(&mut host, worker, b"never");
	plugins.pump(&mut host).unwrap();
	entries.recv_timeout(PATIENCE).unwrap();

	host.plugins().cancel(cancelled).unwrap();
	plugins.pump(&mut host).unwrap();
	gate.wait();
	wakes.recv_timeout(PATIENCE).unwrap();
	plugins.pump(&mut host).unwrap();
	assert_eq!(host.plugins().result(cancelled), Err(Error::InvalidCall));

	let next = call(&mut host, worker, b"again");
	plugins.pump(&mut host).unwrap();
	gate.wait();
	wakes.recv_timeout(PATIENCE).unwrap();
	plugins.pump(&mut host).unwrap();
	assert_eq!(reply(&mut host, next), Some(Ok(b"again".to_vec())));
}

#[test]
fn results_from_before_a_reset_never_reach_the_guest() {
	let (mut host, mut plugins, wakes) = host();
	let gate = Arc::new(Barrier::new(2));
	let (entered, entries) = mpsc::channel();
	let worker = plugins
		.install_worker(&mut host, Box::new(Gate(Arc::clone(&gate), entered)))
		.unwrap();
	let id = call(&mut host, worker, b"stale");
	plugins.pump(&mut host).unwrap();
	entries.recv_timeout(PATIENCE).unwrap();

	plugins.reset();
	gate.wait();
	// The worker saw the cancellation only after it entered the plugin, so
	// its result still arrives, and the pump drops it.
	wakes.recv_timeout(PATIENCE).unwrap();
	plugins.pump(&mut host).unwrap();

	assert_eq!(reply(&mut host, id), None);
}

#[test]
fn calls_to_plugins_the_host_does_not_run_are_unavailable() {
	let (mut host, mut plugins, _wakes) = host();
	let bridge = host.plugins();
	let missing = bridge.register("missing", [MethodId(0)]).unwrap();
	bridge.grant(missing, MethodId(0), true).unwrap();
	let id = call(&mut host, missing, b"?");

	plugins.pump(&mut host).unwrap();

	assert_eq!(reply(&mut host, id), Some(Err(Error::Unavailable)));
}

#[test]
fn a_busy_guest_yields_to_the_event_loop() {
	let (mut host, mut plugins, wakes) = host();
	let echo = plugins.install(&mut host, Box::new(Echo("echo"))).unwrap();
	let ids: Vec<_> = (0..70).map(|_| call(&mut host, echo, b"x")).collect();

	plugins.pump(&mut host).unwrap();
	let answered = ids
		.iter()
		.filter(|id| reply(&mut host, **id).is_some())
		.count();
	assert_eq!(answered, 64);
	wakes.recv_timeout(PATIENCE).unwrap();

	plugins.pump(&mut host).unwrap();
	assert!(ids.iter().all(|id| reply(&mut host, *id).is_some()));
}

#[cfg(feature = "async-host")]
mod asynchronous {
	use leptoskit_bridge::async_host::AsyncPlugin;
	use leptoskit_bridge::async_host::CancellationToken;
	use leptoskit_bridge::async_host::PluginFuture;

	use super::*;

	struct AsyncEcho;

	impl AsyncPlugin for AsyncEcho {
		fn name(&self) -> &'static str {
			"async-echo"
		}

		fn methods(&self) -> &'static [MethodId] {
			&[MethodId(0)]
		}

		fn reply_limit(&self, _: MethodId) -> Result<usize, Error> {
			Ok(1024)
		}

		fn invoke(&self, _: MethodId, payload: Bytes, _: CancellationToken) -> PluginFuture<'_> {
			Box::pin(async move { Ok(payload.into()) })
		}

		fn shutdown(&self) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
			Box::pin(async {})
		}
	}

	#[test]
	fn async_plugins_answer_from_the_runtime() {
		let (mut host, mut plugins, wakes) = host();
		let plugin = plugins
			.install_async(&mut host, Arc::new(AsyncEcho))
			.unwrap();
		let id = call(&mut host, plugin, b"future");

		plugins.pump(&mut host).unwrap();
		wakes.recv_timeout(PATIENCE).unwrap();
		plugins.pump(&mut host).unwrap();

		assert_eq!(reply(&mut host, id), Some(Ok(b"future".to_vec())));
	}
}
