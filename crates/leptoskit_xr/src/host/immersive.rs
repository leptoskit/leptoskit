//! Meta Quest: headset sessions drawn from the app's own activity or, in a
//! panel app, from an immersive activity opened over the panel.
//!
//! Horizon OS shows an OpenXR session only from an activity in the
//! `com.oculus.intent.category.VR` category. A panel app's launcher activity
//! is a 2D panel, so when the guest starts an `ImmersiveVr` or `ImmersiveAr`
//! session, [`Immersive::factory`] opens `NativeImmersiveActivity` (which the
//! Quest build declares) and hands the runtime a [`Pending`] headset that
//! reports no frames until a render thread has made the real session there:
//!
//! 1. it waits for the activity to start;
//! 2. it makes the Vulkan device and the OpenXR session through
//!    [`openxr_vulkan::factory`], and takes over driving the runtime;
//! 3. every frame it updates the runtime, which waits for the headset, and
//!    draws the scene the guest chose with `Session::present` into both eyes
//!    through [`scene3d::draw`], over passthrough in an `ImmersiveAr` session;
//! 4. when the guest stops the session, the session ends, or the activity goes
//!    away, it lets the device go, finishes the activity, and brings the panel
//!    back in Horizon Home.
//!
//! The guest keeps running in the panel's activity the whole time; only the
//! drawing moves.
//!
//! [`openxr_vulkan::factory`]: super::render::openxr_vulkan::factory
//! [`scene3d::draw`]: super::scene3d::draw

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::JClass;
use jni::objects::JObject;
use jni::objects::JValue;
use leptoskit_3d::SceneId;
use leptoskit_3d::TargetColor;
use leptoskit_3d::host::Hub;
use leptoskit_bridge::Error;
use leptoskit_bridge::PlatformContext;

use super::XrFrame;
use super::XrRuntime;
use super::openxr::Headset;
use super::openxr::SessionFactory;
use super::openxr::SharedHeadset;
use super::openxr::System;
use super::render::FrameTextures;
use super::render::openxr_vulkan;
use super::render::openxr_vulkan::VulkanHeadset;
use crate::types::*;

/// The Kotlin side: `dev.leptoskit.mobile.NativeImmersive`.
const BRIDGE: &str = "dev.leptoskit.mobile.NativeImmersive";
/// How long the immersive activity may take to start.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the render thread looks for the activity, or for a frame while
/// the headset isn't showing the session.
const POLL: Duration = Duration::from_millis(10);

/// Makes headset sessions for one runtime; see the module's documentation.
#[derive(Clone)]
pub struct Immersive {
	platform: PlatformContext,
	hub: &'static Hub,
	runtime: Arc<OnceLock<Weak<XrRuntime>>>,
}

impl std::fmt::Debug for Immersive {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Immersive").finish_non_exhaustive()
	}
}

impl Immersive {
	/// Sessions for the app whose context the host passed, drawing the
	/// scenes `hub` holds.
	#[must_use]
	pub fn new(platform: PlatformContext, hub: &'static Hub) -> Self {
		Self {
			platform,
			hub,
			runtime: Arc::new(OnceLock::new()),
		}
	}

	/// The runtime whose backend uses [`Immersive::factory`], which its
	/// render thread drives.
	pub fn bind(&self, runtime: &Arc<XrRuntime>) {
		let _ = self.runtime.set(Arc::downgrade(runtime));
	}

	/// The [`SessionFactory`] for `OpenXrBackend::with_renderer`. It fails
	/// with `Unavailable` where the app has no immersive activity (any build
	/// but Quest's).
	pub fn factory(&self) -> SessionFactory {
		let immersive = self.clone();

		Box::new(
			move |system: &Arc<System>, config: &SessionConfig, features: Features| {
				if bridge::open(&immersive.platform) != Some(true) {
					return Err(Error::Unavailable);
				}

				let pending = Arc::new(Mutex::new(Pending::default()));
				let stop = Arc::clone(&pending.lock().unwrap_or_else(PoisonError::into_inner).stop);
				let run = Run {
					immersive: immersive.clone(),
					system: Arc::clone(system),
					config: *config,
					features,
					pending: Arc::clone(&pending),
					stop,
				};
				std::thread::Builder::new()
					.name("leptoskit-xr-immersive".into())
					.spawn(move || run.run())
					.map_err(|_| Error::Native)?;

				Ok(pending as SharedHeadset)
			},
		)
	}
}

/// A session whose activity is still opening: no frames until the render
/// thread has made the real one.
#[derive(Default)]
pub struct Pending {
	headset: Option<SharedHeadset>,
	failed: bool,
	stop: Arc<AtomicBool>,
}

impl Headset for Pending {
	fn update(&mut self) -> Result<Option<XrFrame>, Error> {
		if self.failed {
			return Err(Error::Unavailable);
		}

		match &self.headset {
			Some(headset) => {
				headset
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.update()
			}
			None => Ok(None),
		}
	}

	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		match &self.headset {
			Some(headset) => {
				headset
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.hit_test(request, frame)
			}
			None => Vec::new(),
		}
	}

	fn stop(&mut self) {
		self.stop.store(true, Ordering::Release);

		if let Some(headset) = &self.headset {
			headset
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.stop();
		}
	}
}

/// What the render thread needs.
struct Run {
	immersive: Immersive,
	system: Arc<System>,
	config: SessionConfig,
	features: Features,
	pending: Arc<Mutex<Pending>>,
	stop: Arc<AtomicBool>,
}

impl Run {
	fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
		self.pending.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn stopped(&self) -> bool {
		self.stop.load(Ordering::Acquire)
	}

	fn runtime(&self) -> Option<Arc<XrRuntime>> {
		self.immersive.runtime.get().and_then(Weak::upgrade)
	}

	fn run(self) {
		let result = self.drive();

		if result.is_err() {
			self.pending().failed = true;
		}

		self.pending().headset = None;

		if let Some(runtime) = self.runtime() {
			runtime.set_external_driver(false);

			// The session ended without the guest asking: tell the guest.
			if !self.stopped() {
				runtime.stop();
			}
		}

		bridge::close(&self.immersive.platform);
	}

	/// Waits for the activity, makes the session, and draws until it ends.
	fn drive(&self) -> Result<(), Error> {
		let started = Instant::now();

		while bridge::current(&self.immersive.platform) != Some(true) {
			if self.stopped() {
				return Ok(());
			}

			if started.elapsed() > LAUNCH_TIMEOUT {
				return Err(Error::Unavailable);
			}

			std::thread::sleep(POLL);
		}

		let made: Arc<Mutex<Option<(openxr_vulkan::Gpu, VulkanHeadset)>>> = Arc::default();
		let slot = Arc::clone(&made);
		let mut factory = openxr_vulkan::factory(move |gpu, headset| {
			*slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((gpu, headset));
		});
		let headset = factory(&self.system, &self.config, self.features)?;
		let (gpu, vulkan) = made
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.take()
			.ok_or(Error::Native)?;
		let runtime = self.runtime().ok_or(Error::Unavailable)?;
		// This thread drives the session from now on, before the runtime's
		// own driver can reach the headset.
		runtime.set_external_driver(true);
		self.pending().headset = Some(headset);
		let gpu = leptoskit_3d::Gpu::from_parts(gpu.instance, gpu.adapter, gpu.device, gpu.queue);
		let result = self.frames(&runtime, &gpu, vulkan);
		// The scenes drawn here move back to a view's device; the session
		// goes before the device, as OpenXR asks.
		self.immersive.hub.forget_device(&gpu);
		self.pending().headset = None;
		result
	}

	/// Updates the runtime and draws the presented scene, frame by frame.
	fn frames(
		&self,
		runtime: &XrRuntime,
		gpu: &leptoskit_3d::Gpu,
		mut vulkan: VulkanHeadset,
	) -> Result<(), Error> {
		let textures = FrameTextures::new();
		let size = vulkan.size();

		while !self.stopped() && bridge::current(&self.immersive.platform) == Some(true) {
			let frame = runtime.update(runtime.now())?;
			let scene = runtime.presented();
			let drew = vulkan.render(|eyes| {
				let layers: Vec<&wgpu::TextureView> = eyes.iter().collect();
				let drawn = frame.as_ref().zip(scene).and_then(|(frame, scene)| {
					self.immersive
						.hub
						.draw_scene(SceneId(scene), gpu, frame.time, |renderer| {
							super::scene3d::draw(
								renderer,
								frame,
								&textures,
								TargetColor::Layers(&layers),
								openxr_vulkan::FORMAT,
								size,
							)
						})
				});

				if !matches!(drawn, Some(Ok(()))) {
					clear(gpu, eyes);
				}
			})?;

			if !drew {
				std::thread::sleep(POLL);
			}
		}

		Ok(())
	}
}

/// Clears the eyes to transparent, so passthrough (or black) shows where no
/// scene is drawn.
fn clear(gpu: &leptoskit_3d::Gpu, eyes: &[wgpu::TextureView]) {
	let mut encoder = gpu
		.device()
		.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("xr clear"),
		});

	for eye in eyes {
		encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("xr clear"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: eye,
				depth_slice: None,
				resolve_target: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
					store: wgpu::StoreOp::Store,
				},
			})],
			..wgpu::RenderPassDescriptor::default()
		});
	}

	gpu.queue().submit([encoder.finish()]);
}

/// The calls into `NativeImmersive`, through the app's class loader, since
/// the render thread's own can't see app classes.
mod bridge {
	use super::*;

	fn with_bridge<T>(
		platform: &PlatformContext,
		body: impl FnOnce(&mut JNIEnv<'_>, &JClass<'_>, &JObject<'_>) -> Option<T>,
	) -> Option<T> {
		let (vm, context) = platform.android_handles()?;
		// SAFETY: The host lends its process `JavaVM*` for the lifetime of the
		// platform context.
		let vm = unsafe { JavaVM::from_raw(vm.cast()) }.ok()?;
		let mut env = vm.attach_current_thread_as_daemon().ok()?;
		// SAFETY: `context` is the host's global reference, kept alive by the
		// platform context; `JObject` borrows it without deleting it.
		let context = unsafe { JObject::from_raw(context.cast()) };
		let result = env
			.with_local_frame(8, |env| {
				let loader = env
					.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
					.l()?;
				let name = env.new_string(BRIDGE)?;
				let class: JClass<'_> = env
					.call_method(
						&loader,
						"loadClass",
						"(Ljava/lang/String;)Ljava/lang/Class;",
						&[JValue::Object(&name)],
					)?
					.l()?
					.into();
				Ok::<_, jni::errors::Error>(body(env, &class, &context))
			})
			.ok()
			.flatten();

		if env.exception_check().unwrap_or(false) {
			let _ = env.exception_clear();
		}

		result
	}

	/// Opens the immersive activity; `Some(false)` where the app doesn't
	/// declare it.
	pub(super) fn open(platform: &PlatformContext) -> Option<bool> {
		with_bridge(platform, |env, class, context| {
			env.call_static_method(
				class,
				"open",
				"(Landroid/content/Context;)Z",
				&[JValue::Object(context)],
			)
			.ok()?
			.z()
			.ok()
		})
	}

	/// Whether the immersive activity is running.
	pub(super) fn current(platform: &PlatformContext) -> Option<bool> {
		with_bridge(platform, |env, class, _| {
			let activity = env
				.call_static_method(class, "current", "()Landroid/app/Activity;", &[])
				.ok()?
				.l()
				.ok()?;
			Some(!activity.is_null())
		})
	}

	/// Finishes the immersive activity and brings the panel back in Home.
	pub(super) fn close(platform: &PlatformContext) {
		let _ = with_bridge(platform, |env, class, context| {
			env.call_static_method(
				class,
				"close",
				"(Landroid/content/Context;)V",
				&[JValue::Object(context)],
			)
			.ok()
			.map(drop)
		});
	}
}
