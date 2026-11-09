//! Every call for a Rust plugin stays in Rust on the guest thread, including
//! the calls an `AppService` hands back.
//!
//! Solana Kit's wallet service drains the bridge itself and hands back every
//! call it does not own, as any service may. Those calls skipped the
//! interpreter's timer and went to the platform's plugin router, which has
//! no timer: on Android every guest sleep failed as `Unavailable`, so nothing
//! in Reapers that waited on a timer ran, and no hunt could start.
//!
//! The tests find the plugins by id rather than by name, and drive the guest
//! thread's own [`step`], so a Rust plugin added later is held to the same
//! rule without anyone listing it here.

use std::collections::VecDeque;
use std::rc::Rc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge::Plugin;
use leptoskit_bridge::PluginId;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

use super::super::AppService;
use super::super::CloseScheduler;
use super::super::ServiceClose;
use super::super::ServiceWake;
use super::super::ServiceWork;
use super::*;

const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;

/// A method id above any a plugin here declares.
const METHODS: u32 = 64;
/// A plugin id above any the app here registers.
const PLUGINS: u32 = 64;
const PLATFORM: &str = "platform";

/// Hands back every call and cancellation it is given, as a service that
/// owns none of them would.
struct HandsBackEverything;

impl AppService for HandsBackEverything {
	fn poll(
		&mut self,
		host: &mut WasmHost<HeadlessBackend>,
		_: &mut Context<'_>,
	) -> Result<ServiceWork, Error> {
		Ok(ServiceWork {
			requests: std::iter::from_fn(|| host.plugins().next_request()).collect(),
			cancelled: std::iter::from_fn(|| host.plugins().next_cancelled()).collect(),
		})
	}

	fn close(self: Box<Self>) -> ServiceClose {
		Box::pin(std::future::ready(Ok(())))
	}
}

/// A Rust plugin that answers with what it was given.
struct Echo(&'static str);

impl Plugin for Echo {
	fn name(&self) -> &'static str {
		self.0
	}

	fn methods(&self) -> &'static [leptoskit_bridge::MethodId] {
		&[leptoskit_bridge::MethodId(0), leptoskit_bridge::MethodId(3)]
	}

	fn invoke(&mut self, _: leptoskit_bridge::MethodId, payload: Bytes) -> Result<Bytes, Error> {
		Ok(payload)
	}
}

#[cfg(feature = "async-host")]
struct AsyncEcho;

#[cfg(feature = "async-host")]
impl leptoskit_bridge::async_host::AsyncPlugin for AsyncEcho {
	fn name(&self) -> &'static str {
		"async"
	}

	fn methods(&self) -> &'static [leptoskit_bridge::MethodId] {
		&[leptoskit_bridge::MethodId(0)]
	}

	fn reply_limit(&self, _: leptoskit_bridge::MethodId) -> Result<usize, Error> {
		Ok(1024)
	}

	fn invoke(
		&self,
		_: leptoskit_bridge::MethodId,
		payload: Bytes,
		_: leptoskit_bridge::async_host::CancellationToken,
	) -> leptoskit_bridge::async_host::PluginFuture<'_> {
		Box::pin(async move { Ok(payload.into()) })
	}

	fn shutdown(&self) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async {})
	}
}

unsafe extern "C" fn ignore(_: *mut c_void) {}

/// An app with every kind of Rust plugin, one platform plugin, and a service
/// that hands back everything, as Reapers' wallet service does.
fn app() -> App {
	let mut app = App::new(GUEST).unwrap();
	app.enable_timers().unwrap();
	app.enable_frames().unwrap();
	app.install(Box::new(Echo("inline"))).unwrap();
	app.install_worker(Box::new(Echo("worker"))).unwrap();
	app.install_main(Box::new(Echo("main"))).unwrap();
	#[cfg(feature = "async-host")]
	app.install_async(Arc::new(AsyncEcho)).unwrap();
	app.with_host(|host| {
		let id = host
			.plugins()
			.register(PLATFORM, [leptoskit_bridge::MethodId(0)])
			.unwrap();
		host.plugins()
			.grant(id, leptoskit_bridge::MethodId(0), true)
			.unwrap();
	});
	app.defer_service(Box::new(|_| Ok(Box::new(HandsBackEverything))))
		.unwrap();
	// The service closes at once, so a closing app is polled to its end here.
	let schedule: CloseScheduler = Rc::new(|mut closing| {
		let closed = std::pin::Pin::new(&mut closing).poll(&mut Context::from_waker(Waker::noop()));
		assert_eq!(closed, Poll::Ready(Ok(())));
	});
	app.activate_service(ServiceWake::new(|| {}), schedule)
		.unwrap();
	app
}

fn platform(app: &mut App) -> PluginId {
	app.with_host(|host| host.plugins().resolve(PLATFORM))
		.unwrap()
}

/// Every plugin the bridge has registered, found by asking it about each id.
fn registered_plugins(app: &mut App) -> Vec<PluginId> {
	app.with_host(|host| {
		(0..PLUGINS)
			.map(PluginId)
			.filter(|plugin| {
				(0..METHODS).map(leptoskit_bridge::MethodId).any(|method| {
					host.plugins().check_call(*plugin, method, 0) != Err(Error::Unavailable)
				})
			})
			.collect()
	})
}

/// Submits a call to every method `plugin` grants, as the guest would. The
/// payload is a zero-length sleep, which a timer reads and the rest ignore.
fn call_every_method(app: &mut App, plugin: PluginId) -> Vec<CallId> {
	let sleep = Bytes::copy_from_slice(&leptoskit_bridge::timer::encode(Duration::ZERO).unwrap());

	app.with_host(|host| {
		let granted: Vec<leptoskit_bridge::MethodId> = (0..METHODS)
			.map(leptoskit_bridge::MethodId)
			.filter(|method| {
				host.plugins()
					.check_call(plugin, *method, sleep.len())
					.is_ok()
			})
			.collect();

		granted
			.into_iter()
			.map(|method| {
				host.plugins()
					.submit(plugin, method, sleep.clone())
					.unwrap()
			})
			.collect()
	})
}

/// Runs one guest-thread step and takes what it handed the UI thread: each
/// call's plugin, and whether it came as a Rust job.
fn step_and_take(app: &mut App, shared: &Shared) -> Vec<(PluginId, bool)> {
	step(app, shared, VecDeque::new(), None).unwrap();

	shared
		.lock()
		.requests
		.drain(..)
		.map(|platform| {
			let plugin = PluginId(platform.request.plugin);
			(
				plugin,
				!platform.worker.is_null() || !platform.job.is_null(),
			)
		})
		.collect()
}

fn shared() -> Shared {
	Shared::new(Notifier {
		context: 0,
		notify: ignore,
		release: ignore,
	})
}

#[test]
fn every_plugin_but_the_platforms_is_a_rust_plugin() {
	let mut app = app();
	let platform = platform(&mut app);
	let rust = app.rust_plugins();
	let registered = registered_plugins(&mut app);
	let names = [
		leptoskit_bridge::timer::TIMER_PLUGIN,
		leptoskit_bridge::frame::FRAME_PLUGIN,
		"inline",
		"worker",
		"main",
		#[cfg(feature = "async-host")]
		"async",
		PLATFORM,
	];

	assert_eq!(registered.len(), names.len());

	for name in names {
		let id = app.with_host(|host| host.plugins().resolve(name)).unwrap();
		assert!(registered.contains(&id), "{name} is registered");
	}

	for plugin in registered {
		assert_eq!(
			rust.contains(&plugin),
			plugin != platform,
			"plugin {plugin:?} is a Rust plugin unless it is the platform's",
		);
	}
}

#[test]
fn no_call_for_a_rust_plugin_reaches_the_platforms_router() {
	let mut app = app();
	let shared = shared();
	let platform = platform(&mut app);

	for plugin in app.rust_plugins() {
		let calls = call_every_method(&mut app, plugin);
		assert!(!calls.is_empty(), "plugin {plugin:?} grants a method");

		for (handed, as_job) in step_and_take(&mut app, &shared) {
			assert_eq!(handed, plugin);
			assert!(
				as_job,
				"a call for Rust plugin {plugin:?} reached the platform's router",
			);
		}
	}

	// The platform's own plugin still reaches its router by the same path.
	call_every_method(&mut app, platform);
	assert_eq!(step_and_take(&mut app, &shared), [(platform, false)]);
}

#[test]
fn a_sleep_a_service_hands_back_wakes_the_guest() {
	let mut app = app();
	let shared = shared();
	let timer = app
		.with_host(|host| {
			host.plugins()
				.resolve(leptoskit_bridge::timer::TIMER_PLUGIN)
		})
		.unwrap();
	let [sleep] = call_every_method(&mut app, timer)[..] else {
		panic!("the timer has one method");
	};
	let deadline = Instant::now() + Duration::from_secs(5);

	loop {
		assert_eq!(step_and_take(&mut app, &shared), []);
		let slept = app.with_host(|host| matches!(host.plugins().result(sleep), Ok(Some(Ok(_)))));

		if slept {
			break;
		}

		assert!(Instant::now() < deadline, "the sleep never completed");
		std::thread::sleep(Duration::from_millis(5));
	}
}
