//! The `xr` runtime plugin: the guest's methods, served from an
//! [`XrRuntime`].

use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::pin::pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;

use leptoskit_bridge::AppContext;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use serde::Deserialize;

use super::Backend;
use super::Composite;
use super::Driver;
use super::Recording;
use super::ReplayBackend;
use super::XrRuntime;
use crate::protocol::*;
use crate::types::*;
use crate::wire::decode;
use crate::wire::encode;

/// The largest reply: a full frame with every plane at its longest outline,
/// every anchor, and every input source with a tracked hand.
const MAX_REPLY_BYTES: usize = 256 * 1024;

/// The host's settings, from `[native.plugin-options.xr]` in
/// `leptoskit.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields, default)]
pub struct Options {
	/// The most frames per second any guest receives; a session asking for
	/// more gets this many.
	pub max_frame_rate: u32,
	/// Plays this recording instead of using the device, for simulators. A
	/// relative path is read from the app's data directory.
	pub replay: Option<PathBuf>,
	/// Plays the recording over and over.
	pub replay_loop: bool,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			max_frame_rate: 60,
			replay: None,
			replay_loop: true,
		}
	}
}

/// Serves the guest's `xr` calls.
#[derive(Debug)]
pub struct XrPlugin {
	runtime: Arc<XrRuntime>,
	max_frame_rate: u32,
	driver: Mutex<Option<Driver>>,
}

impl XrPlugin {
	/// A plugin over `runtime`. A renderer that draws the session shares the
	/// same runtime and drives it with [`XrRuntime::update`].
	pub fn new(runtime: Arc<XrRuntime>, max_frame_rate: u32) -> Self {
		Self {
			runtime,
			max_frame_rate: max_frame_rate.clamp(1, *SessionConfig::FRAME_RATES.end()),
			driver: Mutex::new(None),
		}
	}

	/// Also starts the runtime's own driver thread, which updates sessions
	/// while no renderer does.
	pub fn driven(self) -> std::io::Result<Self> {
		let driver = self.runtime.drive()?;
		*self
			.driver
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = Some(driver);
		Ok(self)
	}

	/// The runtime, for the renderer.
	pub fn runtime(&self) -> &Arc<XrRuntime> {
		&self.runtime
	}

	fn limited(&self, mut config: SessionConfig) -> SessionConfig {
		config.frame_rate = config.frame_rate.min(self.max_frame_rate);
		config
	}

	fn call(&self, method: MethodId, payload: &[u8]) -> Result<Vec<u8>, Error> {
		let runtime = &self.runtime;

		Ok(match method {
			CAPABILITIES => {
				decode::<()>(payload)?;
				encode(&runtime.capabilities())
			}
			PREPARE => {
				let request = decode::<Prepare>(payload)?;
				encode(&runtime.prepare(request.mode, request.install))
			}
			START => encode(&runtime.start(self.limited(decode(payload)?))?),
			CONFIGURE => encode(&runtime.configure(self.limited(decode(payload)?))?),
			HIT_TEST => encode(&runtime.hit_test(&decode(payload)?)?),
			ADD_ANCHOR => {
				let request = decode::<AddAnchor>(payload)?;
				encode(&runtime.add_anchor(request.pose, request.plane)?)
			}
			REMOVE_ANCHOR => {
				runtime.remove_anchor(decode(payload)?)?;
				Vec::new()
			}
			STOP => {
				decode::<()>(payload)?;
				runtime.stop();
				Vec::new()
			}
			_ => return Err(Error::Unavailable),
		})
	}
}

/// Resolves with `Cancelled` as soon as `cancellation` fires, or with
/// `future`'s output.
async fn cancellable<T>(
	cancellation: &CancellationToken,
	future: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
	let mut cancelled = pin!(cancellation.cancelled());
	let mut future = pin!(future);

	std::future::poll_fn(|context| {
		if cancelled.as_mut().poll(context).is_ready() {
			return Poll::Ready(Err(Error::Cancelled));
		}

		future.as_mut().poll(context)
	})
	.await
}

impl AsyncPlugin for XrPlugin {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, Error> {
		if METHODS.contains(&method) {
			Ok(MAX_REPLY_BYTES)
		} else {
			Err(Error::Unavailable)
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			if cancellation.is_cancelled() {
				return Err(Error::Cancelled);
			}

			let reply = if method == NEXT_FRAME {
				let request = decode::<NextFrame>(&payload)?;
				let frame = cancellable(&cancellation, self.runtime.next_frame(request)).await?;
				encode(&frame)
			} else {
				self.call(method, &payload)?
			};

			Ok(Bytes::from(reply).into())
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async {
			self.runtime.stop();
			self.driver
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.take();
		})
	}
}

/// Builds the plugin the generated host registers: the platform's backends,
/// or a recording when `[native.plugin-options.xr] replay` names one, with
/// the runtime's driver thread running.
pub fn from_context(context: &AppContext, options_json: &str) -> Result<XrPlugin, Error> {
	let options: Options =
		serde_json::from_str(options_json).map_err(|_| Error::InvalidArgument)?;
	let runtime = match &options.replay {
		Some(path) => XrRuntime::new(replay(&context.data_dir, path, options.replay_loop)?),
		None => XrRuntime::new(platform_backend(context)),
	};

	XrPlugin::new(runtime, options.max_frame_rate)
		.driven()
		.map_err(|_| Error::Native)
}

fn replay(data_dir: &Path, path: &Path, looping: bool) -> Result<ReplayBackend, Error> {
	let recording = Recording::open(&data_dir.join(path)).map_err(|_| Error::InvalidArgument)?;
	Ok(ReplayBackend::new(recording).looping(looping))
}

/// Every backend this platform has, richest first.
pub fn platform_backend(context: &AppContext) -> impl Backend + 'static {
	let backends = Composite::new();

	#[cfg(all(target_os = "ios", target_arch = "aarch64"))]
	let backends = backends.with(super::arkit::ArKitBackend::new());

	#[cfg(target_os = "android")]
	let backends = backends.with(super::arcore::ArCoreBackend::new(
		context.platform().cloned(),
	));

	#[cfg(any(target_os = "ios", target_os = "android"))]
	let backends = backends.with(super::OrientationBackend::new(super::DeviceRotation::new()));

	let _ = context;
	backends
}
