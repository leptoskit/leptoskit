//! Hosting scenes in a native app: the process's scenes, the asset loader,
//! the `leptoskit.scene3d` plugin a guest reaches them through, and the
//! views that draw them into native surfaces.
//!
//! The guest never renders. Its ops arrive through the plugin on whatever
//! thread the bridge runs it, are applied to the scene under a short lock,
//! and ask the scene's views for a frame. Each view draws on its platform's
//! display link (`CADisplayLink`, `Choreographer`), and keeps asking for
//! frames only while something animates.
//!
//! A scene whose backdrop is [`Backdrop::Camera`] draws over a live camera
//! picture when something has installed a [`CameraFeed`], such as an XR
//! session (`leptoskit_xr`'s `scene3d` feature). The feed then chooses the
//! viewpoint as well as the picture, and the view keeps drawing on every
//! display frame while the picture runs.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::OnceLock;
use std::sync::Weak;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;

use crate::Asset;
use crate::Backend;
use crate::Error;
use crate::Event;
use crate::Gpu;
use crate::RenderTarget;
use crate::Renderer;
use crate::World;
use crate::protocol::*;
use crate::sanitize::*;

/// The plugin's registered name.
pub const PLUGIN: &str = "leptoskit.scene3d";
/// Applies a batch: the scene's id (`u32`, little endian), then the batch
/// [`encode_ops`] wrote. Replies with nothing.
pub const APPLY: MethodId = MethodId(1);
/// Waits for the scene's next events: the scene's id. Replies with a batch
/// [`decode_events`] reads, once there is at least one event.
pub const NEXT_EVENTS: MethodId = MethodId(2);
/// Forgets a scene and everything in it: the scene's id. Replies with
/// nothing.
pub const CLOSE: MethodId = MethodId(3);
/// The largest reply to [`NEXT_EVENTS`], in bytes.
pub const MAX_EVENTS_REPLY: usize = 1 << 20;

/// Where a host's asset paths lead.
pub trait AssetSource: Send + Sync {
	/// Reads the asset at `path`, a safe relative path (see
	/// [`is_safe_asset_path`]).
	fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

/// Assets in a folder, such as an app bundle's resources.
#[derive(Clone, Debug)]
pub struct Directory(pub PathBuf);

impl AssetSource for Directory {
	fn read(&self, path: &str) -> Result<Vec<u8>, String> {
		if !is_safe_asset_path(path) {
			return Err(format!(
				"{path:?} is not a relative path inside the app's assets"
			));
		}

		let full = self.0.join(path);
		let size = std::fs::metadata(&full)
			.map_err(|error| format!("{path} could not be read: {error}"))?
			.len();

		if size > MAX_ASSET_BYTES as u64 {
			return Err(format!("{path} is larger than 128 MiB"));
		}

		std::fs::read(&full).map_err(|error| format!("{path} could not be read: {error}"))
	}
}

impl<F> AssetSource for F
where
	F: Fn(&str) -> Result<Vec<u8>, String> + Send + Sync,
{
	fn read(&self, path: &str) -> Result<Vec<u8>, String> {
		self(path)
	}
}

type FrameWaker = Arc<dyn Fn() + Send + Sync>;

/// A live camera picture that scenes with the camera backdrop draw over,
/// such as a phone's AR session. Install one with [`Hub::set_camera_feed`].
pub trait CameraFeed: Send + Sync {
	/// Whether a picture is coming now, so a view that shows it keeps
	/// drawing on every display frame.
	fn live(&self) -> bool;

	/// What draws one view over the picture. Each view keeps its own, with
	/// its own textures, for as long as its scene asks for the camera.
	fn painter(&self) -> Box<dyn CameraPainter>;
}

/// Draws one view's scene over a [`CameraFeed`]'s picture.
pub trait CameraPainter: Send {
	/// Draws `renderer`'s scene into `target` over the newest picture, from
	/// the feed's own viewpoint. Returns `None`, having drawn nothing, when
	/// there is no picture now; the view then draws as it would without a
	/// feed.
	fn draw(
		&mut self,
		renderer: &mut Renderer,
		target: &CameraTarget<'_>,
	) -> Option<Result<(), Error>>;
}

/// Where a [`CameraPainter`] draws: the view's surface texture for this
/// frame, and how the view sits on the screen.
#[derive(Clone, Copy, Debug)]
pub struct CameraTarget<'a> {
	/// The surface texture's view.
	pub color: &'a wgpu::TextureView,
	/// Its format.
	pub format: wgpu::TextureFormat,
	/// Its size in pixels.
	pub size: [u32; 2],
	/// Clockwise quarter turns of the interface from the device's natural
	/// orientation, 0 to 3, as the host reported with [`View::set_rotation`].
	pub quarter_turns: u32,
}

/// A scene and who is waiting on it.
struct SceneState {
	/// The scene before any view has drawn it; afterwards it lives in the
	/// renderer.
	world: Option<World>,
	renderer: Option<Renderer>,
	/// Futures waiting for events.
	waiters: Vec<Waker>,
	/// Views showing the scene, to ask for a frame when it changes.
	views: HashMap<u64, FrameWaker>,
	/// The display time the scene last advanced to, in seconds.
	time: Option<f64>,
	closed: bool,
}

impl SceneState {
	fn world_mut(&mut self) -> &mut World {
		match (&mut self.renderer, &mut self.world) {
			(Some(renderer), _) => renderer.world_mut(),
			(None, Some(world)) => world,
			(None, None) => unreachable!("a scene always has a world or a renderer"),
		}
	}

	fn world(&self) -> &World {
		match (&self.renderer, &self.world) {
			(Some(renderer), _) => renderer.world(),
			(None, Some(world)) => world,
			(None, None) => unreachable!("a scene always has a world or a renderer"),
		}
	}

	/// The scene's renderer on `gpu`, made on first use. A scene drawn on
	/// another device (a headset's, then a view's again) moves to this one.
	fn renderer_on(&mut self, gpu: &Gpu) -> &mut Renderer {
		if self
			.renderer
			.as_ref()
			.is_some_and(|renderer| !renderer.gpu().same_device(gpu))
			&& let Some(renderer) = self.renderer.take()
		{
			self.world = Some(renderer.into_world());
		}

		let renderer = match self.renderer.take() {
			Some(renderer) => renderer,
			None => Renderer::with_world(gpu, self.world.take().unwrap_or_default()),
		};

		self.renderer.insert(renderer)
	}

	/// Moves the scene's time forward to the display time `time`, once per
	/// display frame however many views draw it. A clock that jumps back by
	/// more than a second (another display's) starts counting from there.
	fn advance_to(&mut self, time: f64) {
		match self.time {
			Some(last) if time <= last => {
				if last - time > 1.0 {
					self.time = Some(time);
				}
			}
			last => {
				let elapsed = last.map_or(0.0, |last| (time - last) as f32);
				self.time = Some(time);

				if let Some(renderer) = &mut self.renderer {
					renderer.advance(elapsed);
				}
			}
		}
	}

	/// Wakes event waiters if events are waiting, and every view.
	fn notify(&mut self) {
		if self.world().has_events() {
			for waiter in self.waiters.drain(..) {
				waiter.wake();
			}
		}

		for view in self.views.values() {
			view();
		}
	}
}

struct Scene(Mutex<SceneState>);

impl Scene {
	fn lock(&self) -> MutexGuard<'_, SceneState> {
		self.0
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}
}

/// An asset to load for a scene.
struct Job {
	scene: Weak<Scene>,
	asset: AssetId,
	path: String,
	source: Arc<dyn AssetSource>,
}

/// Every scene in the process, the device they draw with, and the loader
/// that reads their assets.
pub struct Hub {
	scenes: Mutex<HashMap<SceneId, Arc<Scene>>>,
	source: Mutex<Option<Arc<dyn AssetSource>>>,
	backend: Mutex<Backend>,
	instance: OnceLock<wgpu::Instance>,
	gpu: Mutex<Option<Gpu>>,
	loader: Mutex<Option<mpsc::Sender<Job>>>,
	next_view: std::sync::atomic::AtomicU64,
	camera: Mutex<Option<Arc<dyn CameraFeed>>>,
}

impl Default for Hub {
	fn default() -> Self {
		Self::new()
	}
}

impl Hub {
	/// An empty hub. Apps use [`Hub::global`]; tests make their own.
	pub fn new() -> Self {
		Self {
			scenes: Mutex::new(HashMap::new()),
			source: Mutex::new(None),
			backend: Mutex::new(Backend::Auto),
			instance: OnceLock::new(),
			gpu: Mutex::new(None),
			loader: Mutex::new(None),
			next_view: std::sync::atomic::AtomicU64::new(1),
			camera: Mutex::new(None),
		}
	}

	/// The process's hub, which the plugin and the native views share.
	pub fn global() -> &'static Self {
		static HUB: OnceLock<Hub> = OnceLock::new();
		HUB.get_or_init(Self::new)
	}

	/// Where asset paths lead. Until it is set, every load fails.
	pub fn set_asset_source(&self, source: impl AssetSource + 'static) {
		*lock(&self.source) = Some(Arc::new(source));
	}

	/// Where scenes that ask for the camera backdrop get their picture, or
	/// `None` to draw them without one (transparent behind the models).
	/// Views pick it up on their next frame; call [`Hub::wake_views`] when
	/// its picture starts or stops.
	pub fn set_camera_feed(&self, feed: Option<Arc<dyn CameraFeed>>) {
		*lock(&self.camera) = feed;
		self.wake_views();
	}

	fn camera_feed(&self) -> Option<Arc<dyn CameraFeed>> {
		lock(&self.camera).clone()
	}

	/// Asks every view of every scene for a frame, as when a camera feed's
	/// picture starts or stops. Wakers only schedule work, so this never
	/// draws.
	pub fn wake_views(&self) {
		let scenes: Vec<Arc<Scene>> = lock(&self.scenes).values().cloned().collect();

		for scene in scenes {
			for view in scene.lock().views.values() {
				view();
			}
		}
	}

	/// Draws `scene` with a renderer on `gpu`, such as a headset's device,
	/// after moving it forward to the display time `time` (seconds) as a view
	/// does. The scene moves to `gpu` while it is drawn there, and back to a
	/// view's device when a view draws it next. `None`, having drawn
	/// nothing, when no such scene is open.
	pub fn draw_scene<R>(
		&self,
		scene: SceneId,
		gpu: &Gpu,
		time: f64,
		draw: impl FnOnce(&mut Renderer) -> R,
	) -> Option<R> {
		let scene = lock(&self.scenes).get(&scene).cloned()?;
		let mut state = scene.lock();

		if state.closed {
			return None;
		}

		state.renderer_on(gpu);
		state.advance_to(time);
		let renderer = state.renderer.as_mut()?;
		let result = draw(renderer);

		if renderer.world().has_events() {
			for waiter in state.waiters.drain(..) {
				waiter.wake();
			}
		}

		Some(result)
	}

	/// Moves every scene drawn on `gpu` off it, before that device goes away,
	/// as a headset's does when its session ends. The next view that draws
	/// each scene uploads it to its own device, and is asked to.
	pub fn forget_device(&self, gpu: &Gpu) {
		let scenes: Vec<Arc<Scene>> = lock(&self.scenes).values().cloned().collect();

		for scene in scenes {
			let mut state = scene.lock();

			if state
				.renderer
				.as_ref()
				.is_some_and(|renderer| renderer.gpu().same_device(gpu))
				&& let Some(renderer) = state.renderer.take()
			{
				state.world = Some(renderer.into_world());
				state.notify();
			}
		}
	}

	/// The graphics API to draw with. It takes effect only before the first
	/// view attaches; returns whether it did.
	pub fn set_backend(&self, backend: Backend) -> bool {
		if self.instance.get().is_some() {
			return false;
		}
		*lock(&self.backend) = backend;
		true
	}

	/// The wgpu instance views create their surfaces from.
	pub fn instance(&self) -> &wgpu::Instance {
		self.instance
			.get_or_init(|| pollster::block_on(Gpu::instance(*lock(&self.backend))))
	}

	/// The device, created on first use with an adapter that can present to
	/// `surface`.
	pub fn gpu(&self, surface: Option<&wgpu::Surface<'_>>) -> Result<Gpu, Error> {
		let mut gpu = lock(&self.gpu);

		if let Some(gpu) = &*gpu {
			return Ok(gpu.clone());
		}

		let created = pollster::block_on(Gpu::request_on(self.instance().clone(), surface))?;
		*gpu = Some(created.clone());
		Ok(created)
	}

	fn scene(&self, id: SceneId) -> Option<Arc<Scene>> {
		let mut scenes = lock(&self.scenes);

		if let Some(scene) = scenes.get(&id) {
			return Some(scene.clone());
		}

		if scenes.len() >= MAX_SCENES {
			return None;
		}

		let scene = Arc::new(Scene(Mutex::new(SceneState {
			world: Some(World::new()),
			renderer: None,
			waiters: Vec::new(),
			views: HashMap::new(),
			time: None,
			closed: false,
		})));
		scenes.insert(id, scene.clone());
		Some(scene)
	}

	/// Applies a batch of encoded ops to a scene, creating the scene on
	/// first use.
	pub fn apply(&self, id: SceneId, batch: &[u8]) -> Result<(), DecodeError> {
		let ops = decode_ops(batch)?;
		self.apply_ops(id, ops)
	}

	/// Applies ops to a scene, creating the scene on first use.
	pub fn apply_ops(&self, id: SceneId, ops: Vec<Op>) -> Result<(), DecodeError> {
		let scene = self
			.scene(id)
			.ok_or(DecodeError("the app already has the most scenes it may"))?;
		let mut state = scene.lock();
		let world = state.world_mut();
		world.apply(ops);
		let loads = world.take_loads();
		self.load(&scene, world, loads);
		state.notify();
		Ok(())
	}

	/// Takes a scene's waiting events.
	pub fn take_events(&self, id: SceneId) -> Vec<Event> {
		lock(&self.scenes)
			.get(&id)
			.map(|scene| scene.lock().world_mut().take_events())
			.unwrap_or_default()
	}

	fn poll_events(&self, id: SceneId, context: &Context<'_>) -> Poll<Vec<Event>> {
		let Some(scene) = lock(&self.scenes).get(&id).cloned() else {
			return Poll::Ready(Vec::new());
		};
		let mut state = scene.lock();

		if state.closed {
			return Poll::Ready(Vec::new());
		}

		let events = state.world_mut().take_events();

		if events.is_empty() {
			if !state
				.waiters
				.iter()
				.any(|waiter| waiter.will_wake(context.waker()))
			{
				state.waiters.push(context.waker().clone());
			}
			Poll::Pending
		} else {
			Poll::Ready(events)
		}
	}

	/// Forgets a scene. Its views draw nothing from then on, and waiters
	/// wake with no events.
	pub fn close(&self, id: SceneId) {
		if let Some(scene) = lock(&self.scenes).remove(&id) {
			let mut state = scene.lock();
			state.closed = true;

			for waiter in state.waiters.drain(..) {
				waiter.wake();
			}
		}
	}

	/// How many scenes the hub holds.
	pub fn scene_count(&self) -> usize {
		lock(&self.scenes).len()
	}

	/// Hands the scene's loads to the loader thread. `world` is the scene's,
	/// already locked by the caller.
	fn load(&self, scene: &Arc<Scene>, world: &mut World, loads: Vec<(AssetId, String)>) {
		if loads.is_empty() {
			return;
		}

		let source = lock(&self.source).clone();
		let mut loader = lock(&self.loader);

		for (asset, path) in loads {
			let Some(source) = source.clone() else {
				world.asset_loaded(asset, Err("the host has no asset source".into()));
				continue;
			};

			let job = Job {
				scene: Arc::downgrade(scene),
				asset,
				path,
				source,
			};
			let sender = loader.get_or_insert_with(spawn_loader);

			if let Err(mpsc::SendError(job)) = sender.send(job) {
				// The loader thread is gone; start another.
				let sender = loader.insert(spawn_loader());
				let _ = sender.send(job);
			}
		}
	}

	/// Shows a scene in a surface. The surface must come from
	/// [`Hub::instance`].
	pub fn attach(
		&'static self,
		scene: SceneId,
		surface: wgpu::Surface<'static>,
		width: u32,
		height: u32,
	) -> Result<View, Error> {
		let gpu = self.gpu(Some(&surface))?;
		let capabilities = surface.get_capabilities(gpu.adapter());
		let format = capabilities
			.formats
			.iter()
			.copied()
			.find(|format| {
				matches!(
					format,
					wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
				)
			})
			.or_else(|| capabilities.formats.first().copied())
			.ok_or(Error::Target("the surface supports no format"))?;
		let alpha_mode = crate::gpu::surface_alpha_mode(gpu.backend(), &capabilities.alpha_modes);
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			width: width.max(1),
			height: height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			desired_maximum_frame_latency: 2,
			alpha_mode,
			view_formats: Vec::new(),
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		surface.configure(gpu.device(), &config);
		let id = self
			.next_view
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		Ok(View {
			hub: self,
			id,
			scene,
			gpu,
			surface,
			config,
			configured: width > 0 && height > 0,
			waker: None,
			quarter_turns: 0,
			painter: None,
			shows_camera: false,
		})
	}
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
	mutex
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Assets loaded by path, shared while any scene holds them.
fn cache() -> &'static Mutex<HashMap<String, Weak<Asset>>> {
	static CACHE: OnceLock<Mutex<HashMap<String, Weak<Asset>>>> = OnceLock::new();
	CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reads and decodes an asset, or takes the copy another scene loaded.
fn load_asset(path: &str, source: &dyn AssetSource) -> Result<Arc<Asset>, String> {
	if let Some(asset) = lock(cache()).get(path).and_then(Weak::upgrade) {
		return Ok(asset);
	}

	let bytes = source.read(path)?;
	let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
	let asset = Asset::from_slice_with(&bytes, |relative| {
		let full = if folder.is_empty() {
			relative.to_owned()
		} else {
			format!("{folder}/{relative}")
		};

		source.read(&full)
	})
	.map_err(|error| format!("{path}: {error}"))?;
	let asset = Arc::new(asset);
	let mut cache = lock(cache());
	cache.retain(|_, asset| asset.strong_count() > 0);

	cache.insert(path.to_owned(), Arc::downgrade(&asset));
	Ok(asset)
}

/// One thread that loads assets in the order scenes ask for them, so
/// decoding never runs on the UI or guest thread.
fn spawn_loader() -> mpsc::Sender<Job> {
	let (sender, receiver) = mpsc::channel::<Job>();
	let spawned = std::thread::Builder::new()
		.name("leptoskit_3d assets".into())
		.spawn(move || {
			for job in receiver {
				let result = load_asset(&job.path, job.source.as_ref());
				let Some(scene) = job.scene.upgrade() else {
					continue;
				};
				let mut state = scene.lock();
				if state.closed {
					continue;
				}

				state.world_mut().asset_loaded(job.asset, result);
				state.notify();
			}
		});

	if spawned.is_err() {
		// Without a thread, jobs are dropped and their scenes keep waiting;
		// the next load tries to start one again.
		let (sender, _) = mpsc::channel();

		return sender;
	}

	sender
}

/// A scene shown in a native surface.
///
/// Call [`View::frame`] from the display link while it returns `true`, and
/// start the display link again when the waker set with
/// [`View::set_waker`] fires.
pub struct View {
	hub: &'static Hub,
	id: u64,
	scene: SceneId,
	gpu: Gpu,
	surface: wgpu::Surface<'static>,
	config: wgpu::SurfaceConfiguration,
	configured: bool,
	waker: Option<FrameWaker>,
	quarter_turns: u32,
	painter: Option<Box<dyn CameraPainter>>,
	shows_camera: bool,
}

impl View {
	/// The scene it shows.
	pub fn scene(&self) -> SceneId {
		self.scene
	}

	/// Calls `wake` (from any thread) whenever the scene changes, so the
	/// host can start its display link. It must only schedule work, never
	/// draw or re-enter the hub.
	pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
		let wake: FrameWaker = Arc::new(wake);
		self.waker = Some(wake.clone());

		if let Some(scene) = self.hub.scene(self.scene) {
			scene.lock().views.insert(self.id, wake);
		}
	}

	/// Resizes the drawing surface, in pixels.
	pub fn resize(&mut self, width: u32, height: u32) {
		if width == 0 || height == 0 {
			self.configured = false;

			return;
		}

		let limit = self.gpu.device().limits().max_texture_dimension_2d;
		self.config.width = width.min(limit);
		self.config.height = height.min(limit);
		self.surface.configure(self.gpu.device(), &self.config);
		self.configured = true;

		if let Some(wake) = &self.waker {
			wake();
		}
	}

	/// Tells the view how far the interface is turned from the device's
	/// natural orientation, in clockwise quarter turns (Android's
	/// `Surface.ROTATION_*`; on iOS, portrait 0, landscape right 1, upside
	/// down 2, landscape left 3). A camera feed needs it to turn its picture
	/// upright.
	pub fn set_rotation(&mut self, quarter_turns: u32) {
		let quarter_turns = quarter_turns % 4;

		if quarter_turns != self.quarter_turns {
			self.quarter_turns = quarter_turns;

			if let Some(wake) = &self.waker {
				wake();
			}
		}
	}

	/// Whether the last frame drew the camera feed's picture, which covers
	/// the whole view. A host whose views composite above the window (an
	/// Android `SurfaceView` on the media overlay layer) moves such a view
	/// below the window's own views.
	pub fn shows_camera(&self) -> bool {
		self.shows_camera
	}

	/// The camera feed, when the scene asks for the camera backdrop and one
	/// is installed.
	fn feed(&self, world: &World) -> Option<Arc<dyn CameraFeed>> {
		(world.backdrop() == Backdrop::Camera)
			.then(|| self.hub.camera_feed())
			.flatten()
	}

	/// Draws a frame for the display time `time` (seconds, from the display
	/// link). Returns whether the scene still animates and wants another
	/// frame.
	pub fn frame(&mut self, time: f64) -> Result<bool, Error> {
		let Some(scene) = self.hub.scene(self.scene) else {
			return Ok(false);
		};
		let mut state = scene.lock();

		if state.closed || !self.configured {
			return Ok(false);
		}

		state.renderer_on(&self.gpu);
		state.advance_to(time);

		let texture = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(texture)
			| wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
			wgpu::CurrentSurfaceTexture::Outdated => {
				log::debug!(
					target: leptoskit_log::target::GPU,
					"the surface is outdated; configuring it again"
				);
				self.surface.configure(self.gpu.device(), &self.config);
				return Ok(true);
			}
			wgpu::CurrentSurfaceTexture::Lost => {
				log::warn!(
					target: leptoskit_log::target::GPU,
					"the surface was lost; restoring it at {}×{}",
					self.config.width,
					self.config.height
				);
				self.surface.configure(self.gpu.device(), &self.config);
				return Ok(true);
			}
			_ => return Ok(true),
		};

		let view = texture
			.texture
			.create_view(&wgpu::TextureViewDescriptor::default());
		let renderer = state.renderer.as_mut().expect("the renderer was created");
		let feed = self.feed(renderer.world());

		if feed.is_none() {
			self.painter = None;
		}

		let painted = feed.as_ref().and_then(|feed| {
			self.painter.get_or_insert_with(|| feed.painter()).draw(
				renderer,
				&CameraTarget {
					color: &view,
					format: self.config.format,
					size: [self.config.width, self.config.height],
					quarter_turns: self.quarter_turns,
				},
			)
		});
		self.shows_camera = painted.is_some();

		match painted {
			Some(result) => result?,
			None => {
				renderer.render(&RenderTarget::new(
					&view,
					self.config.format,
					self.config.width,
					self.config.height,
				))?;
			}
		}

		self.gpu.queue().present(texture);
		let more = renderer.world().needs_frame() || feed.is_some_and(|feed| feed.live());

		if renderer.world().has_events() {
			for waiter in state.waiters.drain(..) {
				waiter.wake();
			}
		}

		Ok(more)
	}

	/// Whether the scene has changed or animates, so a display link should
	/// run.
	pub fn needs_frame(&self) -> bool {
		self.hub.scene(self.scene).is_some_and(|scene| {
			let state = scene.lock();
			let world = state.world();
			world.needs_frame() || self.feed(world).is_some_and(|feed| feed.live())
		})
	}
}

impl Drop for View {
	fn drop(&mut self) {
		if let Some(scene) = lock(&self.hub.scenes).get(&self.scene).cloned() {
			scene.lock().views.remove(&self.id);
		}
	}
}

fn scene_id(payload: &[u8]) -> Result<(SceneId, &[u8]), leptoskit_bridge::Error> {
	let (id, rest) = payload
		.split_first_chunk::<4>()
		.ok_or(leptoskit_bridge::Error::InvalidArgument)?;
	Ok((SceneId(u32::from_le_bytes(*id)), rest))
}

/// Waits for a scene's events, or the call's cancellation.
struct NextEvents {
	hub: &'static Hub,
	scene: SceneId,
	cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Future for NextEvents {
	type Output = Result<Reply, leptoskit_bridge::Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		if self.cancelled.as_mut().poll(context).is_ready() {
			return Poll::Ready(Err(leptoskit_bridge::Error::Cancelled));
		}

		self.hub.poll_events(self.scene, context).map(|mut events| {
			// Keep the reply within its limit; the rest wait for the next call.
			let mut bytes = encode_events(&events);
			while bytes.len() > MAX_EVENTS_REPLY && events.len() > 1 {
				let keep = events.len() / 2;
				let rest = events.split_off(keep);
				self.hub.requeue(self.scene, rest);
				bytes = encode_events(&events);
			}

			Ok(Reply::from(Bytes::from(bytes)))
		})
	}
}

impl Hub {
	/// Puts events back at the front of a scene's queue.
	fn requeue(&self, id: SceneId, events: Vec<Event>) {
		if let Some(scene) = lock(&self.scenes).get(&id) {
			scene.lock().world_mut().requeue_events(events);
		}
	}
}

/// The `leptoskit.scene3d` plugin, which hands a guest's ops to a [`Hub`].
#[derive(Clone)]
pub struct Scene3dPlugin {
	hub: &'static Hub,
}

impl Scene3dPlugin {
	/// The plugin for the process's hub.
	pub fn new() -> Self {
		Self::with_hub(Hub::global())
	}

	/// The plugin for `hub`.
	pub fn with_hub(hub: &'static Hub) -> Self {
		Self { hub }
	}
}

impl Default for Scene3dPlugin {
	fn default() -> Self {
		Self::new()
	}
}

impl AsyncPlugin for Scene3dPlugin {
	fn name(&self) -> &'static str {
		PLUGIN
	}

	fn methods(&self) -> &'static [MethodId] {
		&[APPLY, NEXT_EVENTS, CLOSE]
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, leptoskit_bridge::Error> {
		match method {
			APPLY | CLOSE => Ok(0),
			NEXT_EVENTS => Ok(MAX_EVENTS_REPLY),
			_ => Err(leptoskit_bridge::Error::Unavailable),
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		// Ops apply here, before the future is polled, so batches apply in the
		// order the guest sent them however the runtime schedules replies.
		let ready = |result: Result<Reply, leptoskit_bridge::Error>| -> PluginFuture<'_> {
			Box::pin(std::future::ready(result))
		};
		let empty = || Ok(Reply::from(Bytes::new()));

		match method {
			APPLY => {
				let result = scene_id(&payload).and_then(|(scene, batch)| {
					self.hub
						.apply(scene, batch)
						.map_err(|_| leptoskit_bridge::Error::InvalidArgument)
				});
				ready(result.and_then(|()| empty()))
			}
			NEXT_EVENTS => {
				match scene_id(&payload) {
					Ok((scene, _)) => {
						Box::pin(NextEvents {
							hub: self.hub,
							scene,
							cancelled: Box::pin(cancellation.cancelled_owned()),
						})
					}
					Err(error) => ready(Err(error)),
				}
			}
			CLOSE => {
				ready(scene_id(&payload).and_then(|(scene, _)| {
					self.hub.close(scene);
					empty()
				}))
			}
			_ => ready(Err(leptoskit_bridge::Error::Unavailable)),
		}
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(std::future::ready(()))
	}
}
