//! Rust plugins for runners that own their `WasmHost` directly, such as the
//! generated Linux app.
//!
//! The runner installs plugins through the registrant, then calls
//! [`PluginHost::pump`] after every guest entry point and whenever the wake
//! it supplied runs. Synchronous plugins answer inline on the runner's
//! thread, worker plugins on a small pool of threads, and async plugins on
//! the shared runtime. Off-thread results wait in a channel until the next
//! pump, so the guest only ever runs on the runner's thread.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::PluginId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::Request;
use leptoskit_native::NativeBackend;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

use super::AppContext;
use super::FailedPlugin;
use super::run_worker;

/// How many calls one pump answers before it yields to the event loop.
const PUMP_LIMIT: usize = 64;
/// The threads that run worker plugins' blocking calls.
const WORKER_THREADS: usize = 4;

type Wake = Arc<dyn Fn() + Send + Sync>;
type Job = Box<dyn FnOnce() + Send>;

/// The Rust plugins one runner installed, and the calls they are running.
pub struct PluginHost {
	plugins: BTreeMap<u32, Box<dyn Plugin>>,
	workers: BTreeMap<u32, Arc<Mutex<Box<dyn Plugin + Send>>>>,
	#[cfg(feature = "async-host")]
	async_host: super::AsyncHost,
	pool: Option<WorkerPool>,
	running: BTreeMap<u64, Running>,
	/// Tells results from before [`Self::reset`] apart from current ones.
	generation: u64,
	finished: mpsc::Sender<Finished>,
	results: mpsc::Receiver<Finished>,
	wake: Wake,
}

/// How to stop a call that left the runner's thread.
enum Running {
	Worker(Arc<AtomicBool>),
	#[cfg(feature = "async-host")]
	Async(leptoskit_bridge::async_host::CancellationToken),
}

impl Running {
	fn cancel(&self) {
		match self {
			Self::Worker(cancelled) => cancelled.store(true, Ordering::Release),
			#[cfg(feature = "async-host")]
			Self::Async(token) => token.cancel(),
		}
	}
}

/// A result from a worker thread or the async runtime.
struct Finished {
	generation: u64,
	id: CallId,
	result: Result<Reply, Error>,
}

impl PluginHost {
	/// A host with no plugins. `wake` runs on worker and runtime threads when
	/// a result is ready; it must only ask the runner's event loop to call
	/// [`Self::pump`], never pump itself.
	pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
		let (finished, results) = mpsc::channel();
		Self {
			plugins: BTreeMap::new(),
			workers: BTreeMap::new(),
			#[cfg(feature = "async-host")]
			async_host: super::AsyncHost::default(),
			pool: None,
			running: BTreeMap::new(),
			generation: 0,
			finished,
			results,
			wake: Arc::new(wake),
		}
	}

	/// Installs a plugin that answers on the runner's thread and grants its
	/// declared methods.
	///
	/// # Errors
	/// The bridge's error when the name is taken or the plugin is invalid.
	pub fn install<B: NativeBackend>(
		&mut self,
		host: &mut WasmHost<B>,
		plugin: Box<dyn Plugin>,
	) -> Result<PluginId, Error> {
		let id = register(host, plugin.name(), plugin.methods())?;
		self.plugins.insert(id.0, plugin);
		Ok(id)
	}

	/// Installs a plugin whose blocking calls run on the worker threads.
	/// Calls to one plugin run one at a time.
	///
	/// # Errors
	/// The bridge's error when the name is taken or the plugin is invalid.
	pub fn install_worker<B: NativeBackend>(
		&mut self,
		host: &mut WasmHost<B>,
		plugin: Box<dyn Plugin + Send>,
	) -> Result<PluginId, Error> {
		let id = register(host, plugin.name(), plugin.methods())?;
		self.workers.insert(id.0, Arc::new(Mutex::new(plugin)));
		Ok(id)
	}

	/// Installs a plugin that must run on the UI thread. The runner's thread
	/// is the UI thread, so it answers inline like [`Self::install`].
	///
	/// # Errors
	/// The bridge's error when the name is taken or the plugin is invalid.
	pub fn install_main<B: NativeBackend>(
		&mut self,
		host: &mut WasmHost<B>,
		plugin: Box<dyn Plugin + Send>,
	) -> Result<PluginId, Error> {
		self.install(host, plugin)
	}

	/// Installs a plugin whose calls are futures on the shared runtime.
	///
	/// # Errors
	/// The bridge's error when the name is taken, or
	/// [`Error::TooLarge`] when a method's reply limit is over the host's.
	#[cfg(feature = "async-host")]
	pub fn install_async<B: NativeBackend>(
		&mut self,
		host: &mut WasmHost<B>,
		plugin: Arc<dyn leptoskit_bridge::async_host::AsyncPlugin>,
	) -> Result<PluginId, Error> {
		self.async_host.install(host.plugins(), plugin)
	}

	/// Cancels every running call and forgets every plugin, before the runner
	/// replaces its guest. Install the plugins again into the new guest.
	/// Results of the cancelled calls are dropped when they arrive.
	pub fn reset(&mut self) {
		for running in std::mem::take(&mut self.running).into_values() {
			running.cancel();
		}

		self.plugins.clear();
		self.workers.clear();
		#[cfg(feature = "async-host")]
		{
			self.async_host = super::AsyncHost::default();
		}
		self.generation += 1;
	}

	/// Cancels, starts, and completes plugin calls, and lets the guest read
	/// the replies. Call it after each guest entry point and after the wake.
	///
	/// It answers at most 64 calls, then wakes the runner to continue on its
	/// next turn, so a busy guest can't starve the event loop.
	///
	/// # Errors
	/// A guest trap while it reads replies.
	pub fn pump<B: NativeBackend>(&mut self, host: &mut WasmHost<B>) -> Result<(), WasmError> {
		let mut answered = 0;

		loop {
			self.cancel(host);
			let mut completed = self.collect(host);

			while let Some(request) = next_request(host) {
				completed |= self.dispatch(host, request);
				answered += 1;

				if answered >= PUMP_LIMIT {
					break;
				}
			}

			if completed {
				match host.wake_plugins() {
					Ok(()) | Err(WasmError::MissingExport(_)) => {}
					Err(error) => return Err(error),
				}
			}

			if answered >= PUMP_LIMIT {
				(self.wake)();

				return Ok(());
			}

			if !completed {
				return Ok(());
			}
		}
	}

	/// Stops the calls the guest cancelled. Timers are the host's own.
	fn cancel<B: NativeBackend>(&mut self, host: &mut WasmHost<B>) {
		while let Some(id) = host
			.take_routed_cancellation()
			.or_else(|| host.plugins().next_cancelled())
		{
			if host.cancel_timer(id) {
				continue;
			}

			if let Some(running) = self.running.remove(&id.0) {
				running.cancel();
			}
		}
	}

	/// Completes the calls that finished off the runner's thread. Returns
	/// whether any reached the guest.
	fn collect<B: NativeBackend>(&mut self, host: &mut WasmHost<B>) -> bool {
		let mut completed = false;

		while let Ok(finished) = self.results.try_recv() {
			if finished.generation != self.generation
				|| self.running.remove(&finished.id.0).is_none()
			{
				continue;
			}

			// A call the guest cancelled meanwhile has no slot left.
			completed |= host
				.plugins()
				.complete_reply(finished.id, finished.result)
				.is_ok();
		}

		completed
	}

	/// Starts one call. Returns whether it completed inline.
	fn dispatch<B: NativeBackend>(&mut self, host: &mut WasmHost<B>, request: Request) -> bool {
		if let Some(plugin) = self.plugins.get_mut(&request.plugin.0) {
			let (name, methods) = (plugin.name(), plugin.methods());
			let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				plugin.invoke(request.method, request.payload)
			}))
			.unwrap_or_else(|_| {
				// The plugin's state is unknown after a panic, so it answers no
				// more calls, like a worker plugin whose mutex a panic
				// poisoned.
				let failed = self
					.plugins
					.insert(request.plugin.0, Box::new(FailedPlugin { name, methods }));
				let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(failed)));
				Err(Error::Native)
			});

			return host.plugins().complete(request.id, reply).is_ok();
		}

		if let Some(plugin) = self.workers.get(&request.plugin.0) {
			let plugin = Arc::clone(plugin);
			let cancelled = Arc::new(AtomicBool::new(false));
			let flag = Arc::clone(&cancelled);
			let report = self.reporter(request.id);
			let job: Job = Box::new(move || {
				let result = run_worker(&plugin, request.method, request.payload, &flag);
				report(result.map(Reply::from));
			});

			return match self.pool().and_then(|pool| pool.run(job)) {
				Ok(()) => {
					self.running
						.insert(request.id.0, Running::Worker(cancelled));
					false
				}
				Err(error) => host.plugins().complete(request.id, Err(error)).is_ok(),
			};
		}

		#[cfg(feature = "async-host")]
		if let Some(job) = self.async_host.job(
			host.plugins(),
			request.id,
			request.plugin.0,
			request.method,
			&request.payload,
		) {
			self.running
				.insert(request.id.0, Running::Async(job.cancellation()));
			super::async_jobs::spawn(job, self.reporter(request.id));

			return false;
		}

		// Nothing in this host answers the plugin: the registrant left it out.
		host.plugins()
			.complete(request.id, Err(Error::Unavailable))
			.is_ok()
	}

	/// Sends a call's result back to the runner's thread and wakes it.
	fn reporter(&self, id: CallId) -> impl FnOnce(Result<Reply, Error>) + Send + 'static {
		let finished = self.finished.clone();
		let wake = Arc::clone(&self.wake);
		let generation = self.generation;

		move |result| {
			// The host is gone when the send fails; the result has nowhere to
			// go.
			if finished
				.send(Finished {
					generation,
					id,
					result,
				})
				.is_ok()
			{
				wake();
			}
		}
	}

	fn pool(&mut self) -> Result<&WorkerPool, Error> {
		if self.pool.is_none() {
			self.pool = Some(WorkerPool::new()?);
		}

		self.pool.as_ref().ok_or(Error::Native)
	}
}

impl Drop for PluginHost {
	fn drop(&mut self) {
		for running in self.running.values() {
			running.cancel();
		}
	}
}

/// The app's own data and cache directories on a desktop, named by its
/// package id and created when missing: `$XDG_DATA_HOME/<id>` and
/// `$XDG_CACHE_HOME/<id>`, or `~/.local/share/<id>` and `~/.cache/<id>` when
/// those aren't set.
///
/// # Errors
/// [`std::io::ErrorKind::InvalidInput`] for an id that isn't one path
/// segment, [`std::io::ErrorKind::NotFound`] when neither the XDG variable
/// nor `HOME` names a directory, and the error creating a directory.
pub fn desktop_context(application: &str) -> std::io::Result<AppContext> {
	if application.is_empty()
		|| matches!(application, "." | "..")
		|| application.contains(['/', '\\', '\0'])
	{
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!("`{application}` can't name an app directory"),
		));
	}

	let directory = |variable: &str, fallback: &str| {
		let base = std::env::var_os(variable)
			.map(std::path::PathBuf::from)
			.filter(|base| base.is_absolute())
			.or_else(|| {
				std::env::var_os("HOME")
					.map(std::path::PathBuf::from)
					.filter(|home| home.is_absolute())
					.map(|home| home.join(fallback))
			})
			.ok_or_else(|| {
				std::io::Error::new(
					std::io::ErrorKind::NotFound,
					format!("neither {variable} nor HOME names a directory"),
				)
			})?;
		let directory = base.join(application);
		std::fs::create_dir_all(&directory)?;
		Ok::<_, std::io::Error>(directory)
	};

	Ok(AppContext::new(
		directory("XDG_DATA_HOME", ".local/share")?,
		directory("XDG_CACHE_HOME", ".cache")?,
	))
}

/// Registers `name` with the guest's bridge and grants every method.
fn register<B: NativeBackend>(
	host: &mut WasmHost<B>,
	name: &str,
	methods: &[MethodId],
) -> Result<PluginId, Error> {
	let bridge = host.plugins();
	let id = bridge.register(name, methods.iter().copied())?;

	for method in methods {
		bridge.grant(id, *method, true)?;
	}

	Ok(id)
}

/// The next call for a plugin, skipping any the guest already cancelled.
fn next_request<B: NativeBackend>(host: &mut WasmHost<B>) -> Option<Request> {
	while let Some(request) = host.take_routed_request() {
		if matches!(host.plugins().result(request.id), Ok(None)) {
			return Some(request);
		}
	}

	host.next_host_request()
}

/// Fixed threads that run worker calls. They stop once the host drops the
/// pool and the call each one is running returns.
struct WorkerPool {
	jobs: mpsc::Sender<Job>,
}

impl WorkerPool {
	fn new() -> Result<Self, Error> {
		let (jobs, queue) = mpsc::channel::<Job>();
		let queue = Arc::new(Mutex::new(queue));

		for index in 0..WORKER_THREADS {
			let queue = Arc::clone(&queue);
			std::thread::Builder::new()
				.name(format!("leptoskit-worker-{index}"))
				.spawn(move || {
					loop {
						let job = match queue.lock() {
							Ok(queue) => queue.recv(),
							Err(_) => return,
						};

						match job {
							Ok(job) => job(),
							Err(_) => return,
						}
					}
				})
				.map_err(|_| Error::Native)?;
		}

		Ok(Self { jobs })
	}

	fn run(&self, job: Job) -> Result<(), Error> {
		self.jobs.send(job).map_err(|_| Error::Native)
	}
}
