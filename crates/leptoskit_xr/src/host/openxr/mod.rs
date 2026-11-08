//! OpenXR, for Meta Quest, Android XR, and PC headsets.
//!
//! OpenXR can't run a session without a graphics device, and on a headset
//! the frame loop is the display's: `xrWaitFrame` paces it and the frame's
//! predicted display time is the time every pose is located at. So this
//! backend splits in two:
//!
//! - [`System`] loads the OpenXR loader, creates the instance with the
//!   extensions the runtime offers, and reports what the headset can do.
//! - A [`Headset`], made by the renderer through a [`SessionFactory`], owns the
//!   session and its frame loop. The renderer creates its graphics device the
//!   way OpenXR asks (`XR_KHR_vulkan_enable2`), hands the session to
//!   [`HeadsetSession::new`], and calls [`XrRuntime::update`] once per frame
//!   from its render thread; each update waits for the headset's next frame and
//!   locates the views, controllers, and hands at its display time.
//!
//! Without a factory, starting an immersive session fails with
//! `Unavailable`: there is nothing to draw it with.
//!
//! [`XrRuntime::update`]: crate::host::XrRuntime::update

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;

use leptoskit_bridge::Error;
use leptoskit_bridge::PlatformContext;
use openxr as xr;

use super::Backend;
use super::Started;
use super::XrFrame;
use crate::types::*;

mod headset;

pub use headset::HeadsetSession;

/// The headset's view configuration: one view per eye.
pub const VIEW_CONFIGURATION: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

/// The loaded OpenXR runtime, its instance, and the headset.
pub struct System {
	/// The instance, with [`System::extensions`] enabled.
	pub instance: xr::Instance,
	/// The head-mounted display.
	pub system: xr::SystemId,
	/// The extensions enabled on the instance.
	pub extensions: xr::ExtensionSet,
	/// The ways the headset can blend the scene with the world.
	pub blend_modes: Vec<xr::EnvironmentBlendMode>,
	/// Whether the headset tracks hands.
	pub hand_tracking: bool,
}

impl std::fmt::Debug for System {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("System")
			.field("blend_modes", &self.blend_modes)
			.field("hand_tracking", &self.hand_tracking)
			.finish_non_exhaustive()
	}
}

impl System {
	/// Loads the OpenXR loader and finds a head-mounted display. On Android
	/// the platform context supplies the `JavaVM` and the `Activity` the
	/// loader and instance need.
	pub fn load(platform: Option<&PlatformContext>) -> Result<Self, Error> {
		let entry = load_entry(platform)?;
		let available = entry
			.enumerate_extensions()
			.map_err(|_| Error::Unavailable)?;
		let extensions = wanted_extensions(&available);
		let info = xr::ApplicationInfo {
			application_name: "leptoskit",
			application_version: 1,
			engine_name: "leptoskit",
			engine_version: 1,
			api_version: xr::Version::new(1, 0, 0),
		};
		let instance = create_instance(&entry, &info, &extensions, platform)?;
		let system = instance
			.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
			.map_err(|_| Error::Unavailable)?;
		let blend_modes = instance
			.enumerate_environment_blend_modes(system, VIEW_CONFIGURATION)
			.map_err(|_| Error::Native)?;
		let hand_tracking = extensions.ext_hand_tracking
			&& instance.supports_hand_tracking(system).unwrap_or(false);

		Ok(Self {
			instance,
			system,
			extensions,
			blend_modes,
			hand_tracking,
		})
	}

	/// Whether the headset can show the room: an alpha-blended or additive
	/// display, or Meta's passthrough extension.
	#[must_use]
	pub fn passthrough(&self) -> bool {
		self.extensions.fb_passthrough
			|| self.blend_modes.iter().any(|mode| {
				matches!(
					*mode,
					xr::EnvironmentBlendMode::ALPHA_BLEND | xr::EnvironmentBlendMode::ADDITIVE
				)
			})
	}

	/// What this headset can do.
	#[must_use]
	pub fn capabilities(&self) -> Capabilities {
		let mut features = Features::CONTROLLERS | Features::HIT_TEST | Features::ANCHORS;

		if self.hand_tracking {
			features = features | Features::HANDS;
		}

		let mut capabilities = Capabilities::NONE.with(Mode::ImmersiveVr, features);

		if self.passthrough() {
			capabilities = capabilities.with(Mode::ImmersiveAr, features | Features::PASSTHROUGH);
		}

		capabilities
	}
}

fn wanted_extensions(available: &xr::ExtensionSet) -> xr::ExtensionSet {
	let mut extensions = xr::ExtensionSet::default();
	extensions.khr_vulkan_enable2 = available.khr_vulkan_enable2;
	extensions.khr_opengl_es_enable = available.khr_opengl_es_enable;
	extensions.ext_hand_tracking = available.ext_hand_tracking;
	extensions.fb_passthrough = available.fb_passthrough;
	extensions.fb_display_refresh_rate = available.fb_display_refresh_rate;
	extensions.khr_composition_layer_depth = available.khr_composition_layer_depth;
	extensions.ext_local_floor = available.ext_local_floor;

	#[cfg(target_os = "android")]
	{
		extensions.khr_android_create_instance = available.khr_android_create_instance;
	}
	extensions
}

#[cfg(target_os = "android")]
fn platform_info(platform: Option<&PlatformContext>) -> Result<xr::AndroidPlatformInfo, Error> {
	let (vm, activity) = platform
		.and_then(PlatformContext::android_handles)
		.ok_or(Error::Unavailable)?;
	// SAFETY: The host lends its `JavaVM*` and its `Activity`'s global
	// reference for the lifetime of the platform context, which outlives
	// the instance.
	Ok(unsafe { xr::AndroidPlatformInfo::new(vm, activity) })
}

fn load_entry(platform: Option<&PlatformContext>) -> Result<xr::Entry, Error> {
	#[cfg(target_os = "android")]
	let info = platform_info(platform)?;
	#[cfg(not(target_os = "android"))]
	let info = {
		let _ = platform;
	};

	// SAFETY: Opens the system's OpenXR loader (on Android, the Khronos
	// loader packaged with the app), which exports the entry points the
	// crate resolves.
	unsafe { xr::Entry::load(&info) }.map_err(|_| Error::Unavailable)
}

fn create_instance(
	entry: &xr::Entry,
	info: &xr::ApplicationInfo<'_>,
	extensions: &xr::ExtensionSet,
	platform: Option<&PlatformContext>,
) -> Result<xr::Instance, Error> {
	#[cfg(target_os = "android")]
	let platform = platform_info(platform)?;
	#[cfg(not(target_os = "android"))]
	let platform = {
		let _ = platform;
	};

	entry
		.create_instance(info, extensions, &[], &platform)
		.map_err(|_| Error::Unavailable)
}

/// A running headset session; see [`HeadsetSession`].
pub trait Headset: Send {
	/// Handles the runtime's events, then waits for the headset's next frame
	/// and locates everything at its display time. `None` while the session
	/// isn't visible, or before the runtime has made it ready.
	fn update(&mut self) -> Result<Option<XrFrame>, Error>;
	/// Casts a ray against the session's tracked surfaces.
	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		crate::hit_test::hit_test(request, &frame.views, &frame.planes, frame.depth.as_ref())
	}
	/// Asks the runtime to end the session.
	fn stop(&mut self);
}

/// A shared headset session: the backend updates it and the renderer draws
/// into it.
pub type SharedHeadset = Arc<Mutex<dyn Headset>>;

/// Makes the graphics device and the OpenXR session for a session the guest
/// asked for. The renderer supplies it.
pub type SessionFactory =
	Box<dyn FnMut(&Arc<System>, &SessionConfig, Features) -> Result<SharedHeadset, Error> + Send>;

/// Headset sessions through OpenXR.
pub struct OpenXrBackend {
	platform: Option<PlatformContext>,
	system: OnceLock<Option<Arc<System>>>,
	factory: Option<SessionFactory>,
	headset: Option<SharedHeadset>,
}

impl std::fmt::Debug for OpenXrBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("OpenXrBackend")
			.field("system", &self.system.get())
			.field("renderer", &self.factory.is_some())
			.field("running", &self.headset.is_some())
			.finish_non_exhaustive()
	}
}

impl OpenXrBackend {
	/// A backend that loads OpenXR on first use. Android needs the host's
	/// platform context.
	#[must_use]
	pub fn new(platform: Option<PlatformContext>) -> Self {
		Self {
			platform,
			system: OnceLock::new(),
			factory: None,
			headset: None,
		}
	}

	/// Lets sessions start, with `factory` making each one's graphics device
	/// and OpenXR session.
	#[must_use]
	pub fn with_renderer(mut self, factory: SessionFactory) -> Self {
		self.factory = Some(factory);
		self
	}

	/// The loaded system, loading it on first call. `None` without a headset
	/// or a loader, as on a phone.
	pub fn system(&self) -> Option<Arc<System>> {
		self.system
			.get_or_init(|| System::load(self.platform.as_ref()).ok().map(Arc::new))
			.clone()
	}

	fn headset(&self) -> Result<std::sync::MutexGuard<'_, dyn Headset + 'static>, Error> {
		Ok(self
			.headset
			.as_ref()
			.ok_or(Error::Unavailable)?
			.lock()
			.unwrap_or_else(PoisonError::into_inner))
	}
}

impl Backend for OpenXrBackend {
	fn capabilities(&self) -> Capabilities {
		self.system()
			.map_or(Capabilities::NONE, |system| system.capabilities())
	}

	fn prepare(&mut self, mode: Mode, _install: bool) -> Availability {
		match self.system() {
			Some(system) if system.capabilities().supports(mode) => Availability::Ready,
			_ => Availability::Unsupported,
		}
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		let system = self.system().ok_or(Error::Unavailable)?;
		let factory = self.factory.as_mut().ok_or(Error::Unavailable)?;
		self.headset = Some(factory(&system, config, features)?);

		let space = match config.reference_space {
			ReferenceSpace::LocalFloor | ReferenceSpace::World => config.reference_space,
			ReferenceSpace::Local => ReferenceSpace::Local,
		};

		Ok(Started::new(space, features))
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		self.headset()?.update()
	}

	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		match self.headset() {
			Ok(mut headset) => headset.hit_test(request, frame),
			Err(_) => Vec::new(),
		}
	}

	fn stop(&mut self) {
		if let Some(headset) = self.headset.take() {
			headset
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.stop();
		}
	}
}
