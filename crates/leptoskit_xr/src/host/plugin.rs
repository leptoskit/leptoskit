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
	/// The depth model in the app's `assets/` folder, such as
	/// `depth/depth_anything_v2_metric_indoor_small.onnx`: with it, sessions
	/// on phones that measure no depth get depth estimated from the camera
	/// picture. It must be `leptoskit_depth::MODEL`; a missing or different
	/// file is logged and leaves depth off. Needs the crate's
	/// `depth-estimation` feature, which `leptoskit build` turns on when this
	/// is set.
	pub depth_model: Option<String>,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			max_frame_rate: 60,
			replay: None,
			replay_loop: true,
			depth_model: None,
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
			PRESENT => {
				runtime.present(decode::<Present>(payload)?.scene);
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
	// On Quest, headset sessions draw the app's scenes, from an immersive
	// activity when the app is a panel.
	#[cfg(all(target_os = "android", feature = "quest"))]
	let immersive = context.platform().cloned().map(|platform| {
		super::immersive::Immersive::new(platform, leptoskit_3d::host::Hub::global())
	});
	#[cfg(not(all(target_os = "android", feature = "quest")))]
	let immersive = None;

	let runtime = match &options.replay {
		Some(path) => XrRuntime::new(replay(&context.data_dir, path, options.replay_loop)?),
		None => XrRuntime::new(platform_backend(context, immersive.as_ref())),
	};

	// A depth crate's estimator, installed before or after.
	super::camera::adopt(&runtime);

	#[cfg(all(target_os = "android", feature = "quest"))]
	if let Some(immersive) = &immersive {
		immersive.bind(&runtime);
	}

	if let Some(path) = &options.depth_model {
		estimate_depth(context, path);
	}

	// Scenes that ask for the camera backdrop draw over this runtime's
	// sessions.
	#[cfg(feature = "scene3d")]
	super::scene3d::SessionFeed::install(&runtime, leptoskit_3d::host::Hub::global());

	XrPlugin::new(runtime, options.max_frame_rate)
		.driven()
		.map_err(|_| Error::Native)
}

/// Loads the depth model in the background and installs it once it loads.
#[cfg(feature = "depth-estimation")]
fn estimate_depth(context: &AppContext, path: &str) {
	super::estimated_depth::load_in_background(context.platform().cloned(), path.to_owned());
}

#[cfg(not(feature = "depth-estimation"))]
fn estimate_depth(_: &AppContext, path: &str) {
	eprintln!(
		"leptoskit-xr: warning: depth-model = {path:?} is set, but leptoskit_xr was built without \
		 its `depth-estimation` feature, so phones that measure no depth get none. `leptoskit \
		 build` turns it on; a host built another way must enable it."
	);
}

fn replay(data_dir: &Path, path: &Path, looping: bool) -> Result<ReplayBackend, Error> {
	let recording = Recording::open(&data_dir.join(path)).map_err(|_| Error::InvalidArgument)?;
	Ok(ReplayBackend::new(recording).looping(looping))
}

#[cfg(all(target_os = "android", feature = "quest"))]
type Immersive = super::immersive::Immersive;
/// Nothing makes headset sessions outside Quest builds.
#[cfg(not(all(target_os = "android", feature = "quest")))]
#[derive(Debug)]
enum Immersive {}

/// Every backend this platform has, richest first, with headset sessions
/// made by `immersive` where there is one.
fn platform_backend(context: &AppContext, immersive: Option<&Immersive>) -> impl Backend + 'static {
	let _ = immersive;
	let backends = Composite::new();

	#[cfg(all(target_os = "ios", target_arch = "aarch64"))]
	let backends = backends.with(super::arkit::ArKitBackend::new());

	#[cfg(target_os = "android")]
	let backends = backends.with(super::arcore::ArCoreBackend::new(
		context.platform().cloned(),
	));

	// Headsets come before the rotation sensor; on a phone without an OpenXR
	// runtime this backend offers nothing and the sensor answers.
	#[cfg(all(
		feature = "openxr",
		any(
			target_os = "android",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		)
	))]
	let backends = backends.with({
		let backend = super::openxr::OpenXrBackend::new(context.platform().cloned());

		#[cfg(all(target_os = "android", feature = "quest"))]
		let backend = match immersive {
			Some(immersive) => backend.with_renderer(immersive.factory()),
			None => backend,
		};

		backend
	});

	// Phones without AR look around by turning. Android's also show the back
	// camera through Camera2; every iPhone that runs current iOS has ARKit.
	#[cfg(target_os = "ios")]
	let backends = backends.with(super::OrientationBackend::new(super::DeviceRotation::new()));

	#[cfg(target_os = "android")]
	let backends = backends.with(
		super::OrientationBackend::new(super::DeviceRotation::new())
			.with_camera(super::camera2::Camera2Source::new()),
	);

	let _ = context;
	backends
}
