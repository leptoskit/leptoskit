//! Scenes in the browser: each `<canvas>` a `Scene3d` view renders draws its
//! scene with WebGPU, or WebGL2 where the browser has no WebGPU, paced by
//! `requestAnimationFrame` only while the scene changes or animates.
//!
//! A `Scene3d` view renders as `<canvas data-lk-scene3d="id">`. The first
//! ops a scene applies start a `MutationObserver` that draws into such
//! canvases as they appear and lets them go as they leave, so the page's
//! interface code never links this module.
//!
//! Everything runs on the page's one thread. Assets are fetched relative to
//! [`set_asset_base`] (`/assets/` by default, where `leptoskit build` copies
//! the app's `assets/` folder) and must be `.glb` files or `.gltf` files with
//! embedded data.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;
use std::task::Poll;
use std::task::Waker;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::Asset;
use crate::Backend;
use crate::Event;
use crate::Gpu;
use crate::RenderTarget;
use crate::Renderer;
use crate::World;
use crate::protocol::*;
use crate::sanitize::*;

struct PageScene {
	world: Option<World>,
	renderer: Option<Renderer>,
	waiters: Vec<Waker>,
	canvases: Vec<Weak<Canvas>>,
	time: Option<f64>,
}

impl PageScene {
	fn world(&self) -> &World {
		match (&self.renderer, &self.world) {
			(Some(renderer), _) => renderer.world(),
			(None, Some(world)) => world,
			(None, None) => unreachable!("a scene always has a world or a renderer"),
		}
	}

	fn world_mut(&mut self) -> &mut World {
		match (&mut self.renderer, &mut self.world) {
			(Some(renderer), _) => renderer.world_mut(),
			(None, Some(world)) => world,
			(None, None) => unreachable!("a scene always has a world or a renderer"),
		}
	}
}

#[derive(Default)]
struct Page {
	scenes: HashMap<SceneId, PageScene>,
	base: Option<String>,
	backend: Backend,
	instance: Option<wgpu::Instance>,
	/// The page's WebGPU device, shared by every canvas. WebGL2 contexts
	/// cannot share resources, so under WebGL2 each canvas has its own.
	gpu: Option<Gpu>,
	cache: HashMap<String, std::sync::Weak<Asset>>,
}

thread_local! {
	static PAGE: RefCell<Page> = RefCell::new(Page::default());
}

/// Where relative asset paths resolve: a URL ending in `/`. The default is
/// `/assets/`.
pub fn set_asset_base(url: impl Into<String>) {
	PAGE.with_borrow_mut(|page| page.base = Some(url.into()));
}

/// The graphics API to draw with: [`Backend::Auto`] (WebGPU, falling back to
/// WebGL2), [`Backend::WebGpu`], or [`Backend::Gl`]. It applies before the
/// first canvas draws.
pub fn set_backend(backend: Backend) {
	PAGE.with_borrow_mut(|page| {
		if page.instance.is_none() {
			page.backend = backend;
		}
	});
}

fn with_scene<T>(id: SceneId, action: impl FnOnce(&mut PageScene) -> T) -> Option<T> {
	PAGE.with_borrow_mut(|page| {
		if !page.scenes.contains_key(&id) && page.scenes.len() >= MAX_SCENES {
			return None;
		}
		let scene = page.scenes.entry(id).or_insert_with(|| {
			PageScene {
				world: Some(World::new()),
				renderer: None,
				waiters: Vec::new(),
				canvases: Vec::new(),
				time: None,
			}
		});
		Some(action(scene))
	})
}

/// Wakes the scene's event waiters (when it has events) and asks its
/// canvases for a frame. Runs after the page is released, since waking can
/// schedule work that borrows it.
fn notify(id: SceneId) {
	let (waiters, canvases) = PAGE.with_borrow_mut(|page| {
		let Some(scene) = page.scenes.get_mut(&id) else {
			return (Vec::new(), Vec::new());
		};
		let waiters = if scene.world().has_events() {
			std::mem::take(&mut scene.waiters)
		} else {
			Vec::new()
		};
		scene.canvases.retain(|canvas| canvas.strong_count() > 0);
		let canvases: Vec<Rc<Canvas>> = scene.canvases.iter().filter_map(Weak::upgrade).collect();
		(waiters, canvases)
	});
	for waiter in waiters {
		waiter.wake();
	}
	for canvas in canvases {
		request_frame(&canvas);
	}
}

/// The canvases `Scene3d` views rendered, and what draws into each.
struct Observed {
	_observer: web_sys::MutationObserver,
	_callback: Closure<dyn FnMut()>,
	canvases: Vec<(web_sys::HtmlCanvasElement, SceneId, Attachment)>,
}

thread_local! {
	static OBSERVED: RefCell<Option<Observed>> = const { RefCell::new(None) };
}

/// The scene a canvas names, if it is a `Scene3d` view's.
fn named_scene(canvas: &web_sys::HtmlCanvasElement) -> Option<SceneId> {
	canvas
		.get_attribute("data-lk-scene3d")?
		.parse()
		.ok()
		.map(SceneId)
}

/// Attaches canvases that appeared and detaches ones that left the page or
/// changed scene.
fn rescan() {
	let Some(document) = web_sys::window().and_then(|window| window.document()) else {
		return;
	};
	let Ok(found) = document.query_selector_all("canvas[data-lk-scene3d]") else {
		return;
	};
	let mut dropped = Vec::new();
	OBSERVED.with_borrow_mut(|observed| {
		let Some(observed) = observed else {
			return;
		};
		let mut kept = Vec::with_capacity(observed.canvases.len());
		for (canvas, scene, attachment) in observed.canvases.drain(..) {
			if canvas.is_connected() && named_scene(&canvas) == Some(scene) {
				kept.push((canvas, scene, attachment));
			} else {
				dropped.push(attachment);
			}
		}
		observed.canvases = kept;
		for index in 0..found.length() {
			let Some(canvas) = found
				.item(index)
				.and_then(|node| node.dyn_into::<web_sys::HtmlCanvasElement>().ok())
			else {
				continue;
			};
			let Some(scene) = named_scene(&canvas) else {
				continue;
			};
			if observed.canvases.iter().any(|(known, ..)| *known == canvas) {
				continue;
			}
			let attachment = attach(canvas.clone(), scene);
			observed.canvases.push((canvas, scene, attachment));
		}
	});
	// Dropping outside the borrow: a drop may wake work that rescans.
	drop(dropped);
}

/// Starts watching the page for `Scene3d` canvases. Applying a scene's
/// first ops does this; call it to draw scenes made elsewhere.
pub fn observe() {
	let started = OBSERVED.with_borrow(Option::is_some);
	if started {
		return;
	}
	let Some(document) = web_sys::window().and_then(|window| window.document()) else {
		return;
	};
	let callback = Closure::<dyn FnMut()>::new(rescan);
	let Ok(observer) = web_sys::MutationObserver::new(callback.as_ref().unchecked_ref()) else {
		return;
	};
	let options = web_sys::MutationObserverInit::new();
	options.set_child_list(true);
	options.set_subtree(true);
	options.set_attributes(true);
	options.set_attribute_filter(&js_sys::Array::of1(&"data-lk-scene3d".into()));
	if let Some(root) = document.document_element()
		&& observer.observe_with_options(&root, &options).is_ok()
	{
		OBSERVED.with_borrow_mut(|observed| {
			*observed = Some(Observed {
				_observer: observer,
				_callback: callback,
				canvases: Vec::new(),
			});
		});
		rescan();
	}
}

/// Applies ops to a scene, creating it on first use.
pub fn apply(id: SceneId, ops: Vec<Op>) {
	observe();
	let loads = with_scene(id, |scene| {
		let world = scene.world_mut();
		world.apply(ops);
		world.take_loads()
	});
	for (asset, path) in loads.unwrap_or_default() {
		wasm_bindgen_futures::spawn_local(load(id, asset, path));
	}
	notify(id);
}

/// Forgets a scene; its canvases draw nothing more.
pub fn close(id: SceneId) {
	let waiters = PAGE.with_borrow_mut(|page| {
		page.scenes
			.remove(&id)
			.map(|scene| scene.waiters)
			.unwrap_or_default()
	});
	for waiter in waiters {
		waiter.wake();
	}
}

/// Waits for a scene's next events.
pub async fn next_events(id: SceneId) -> Vec<Event> {
	std::future::poll_fn(|context| {
		PAGE.with_borrow_mut(|page| {
			let Some(scene) = page.scenes.get_mut(&id) else {
				return Poll::Ready(Vec::new());
			};
			let events = scene.world_mut().take_events();
			if events.is_empty() {
				if !scene
					.waiters
					.iter()
					.any(|waiter| waiter.will_wake(context.waker()))
				{
					scene.waiters.push(context.waker().clone());
				}
				Poll::Pending
			} else {
				Poll::Ready(events)
			}
		})
	})
	.await
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
	let window = web_sys::window().ok_or("there is no window")?;
	let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
		.await
		.map_err(|error| format!("{url} could not be fetched: {error:?}"))?;
	let response: web_sys::Response = response
		.dyn_into()
		.map_err(|_| "the fetch did not return a response".to_owned())?;
	if !response.ok() {
		return Err(format!("{url} answered {}", response.status()));
	}
	let buffer = response
		.array_buffer()
		.map_err(|error| format!("{url} has no body: {error:?}"))?;
	let buffer = wasm_bindgen_futures::JsFuture::from(buffer)
		.await
		.map_err(|error| format!("{url} could not be read: {error:?}"))?;
	let bytes = js_sys::Uint8Array::new(&buffer);
	if bytes.length() as usize > MAX_ASSET_BYTES {
		return Err(format!("{url} is larger than 128 MiB"));
	}
	Ok(bytes.to_vec())
}

async fn load(id: SceneId, asset: AssetId, path: String) {
	let (base, cached) = PAGE.with_borrow(|page| {
		(
			page.base.clone().unwrap_or_else(|| "/assets/".into()),
			page.cache.get(&path).and_then(std::sync::Weak::upgrade),
		)
	});
	let result = if let Some(asset) = cached {
		Ok(asset)
	} else {
		let url = format!("{base}{path}");
		fetch(&url).await.and_then(|bytes| {
			Asset::from_slice(&bytes)
				.map(Arc::new)
				.map_err(|error| format!("{path}: {error}"))
		})
	};
	if let Ok(asset) = &result {
		PAGE.with_borrow_mut(|page| {
			page.cache.retain(|_, asset| asset.strong_count() > 0);
			page.cache.insert(path.clone(), Arc::downgrade(asset));
		});
	}
	PAGE.with_borrow_mut(|page| {
		if let Some(scene) = page.scenes.get_mut(&id) {
			scene.world_mut().asset_loaded(asset, result);
		}
	});
	notify(id);
}

type FrameCallback = Closure<dyn FnMut(f64)>;
type Resizes = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// A canvas drawing a scene.
struct Canvas {
	element: web_sys::HtmlCanvasElement,
	scene: SceneId,
	surface: RefCell<Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration, Gpu)>>,
	scheduled: Cell<bool>,
	closed: Cell<bool>,
	callback: RefCell<Option<FrameCallback>>,
	observer: RefCell<Option<Resizes>>,
}

/// Keeps a canvas drawing; dropping it stops the frames and lets the
/// surface go.
pub struct Attachment(Rc<Canvas>);

impl Drop for Attachment {
	fn drop(&mut self) {
		self.0.closed.set(true);
		if let Some((observer, _)) = self.0.observer.borrow_mut().take() {
			observer.disconnect();
		}
		self.0.callback.borrow_mut().take();
		self.0.surface.borrow_mut().take();
	}
}

/// Starts drawing `scene` into `canvas`, sized by its CSS box.
pub fn attach(element: web_sys::HtmlCanvasElement, scene: SceneId) -> Attachment {
	let canvas = Rc::new(Canvas {
		element,
		scene,
		surface: RefCell::new(None),
		scheduled: Cell::new(false),
		closed: Cell::new(false),
		callback: RefCell::new(None),
		observer: RefCell::new(None),
	});
	with_scene(scene, |state| state.canvases.push(Rc::downgrade(&canvas)));
	// Resizing asks for a frame, which fits the drawing buffer to the box.
	let weak = Rc::downgrade(&canvas);
	let resized = Closure::<dyn FnMut()>::new(move || {
		if let Some(canvas) = weak.upgrade() {
			request_frame(&canvas);
		}
	});
	if let Ok(observer) = web_sys::ResizeObserver::new(resized.as_ref().unchecked_ref()) {
		observer.observe(&canvas.element);
		*canvas.observer.borrow_mut() = Some((observer, resized));
	}
	let pending = canvas.clone();
	wasm_bindgen_futures::spawn_local(async move {
		if let Err(error) = connect(&pending).await {
			web_sys::console::error_1(&format!("leptoskit_3d: {error}").into());
			return;
		}
		request_frame(&pending);
	});
	Attachment(canvas)
}

/// Creates the canvas's surface and finds a device for it.
async fn connect(canvas: &Rc<Canvas>) -> Result<(), String> {
	let existing = PAGE.with_borrow(|page| page.instance.clone());
	let instance = if let Some(instance) = existing {
		instance
	} else {
		let backend = PAGE.with_borrow(|page| page.backend);
		let instance = Gpu::instance(backend).await;
		PAGE.with_borrow_mut(|page| page.instance.get_or_insert(instance).clone())
	};
	let surface = instance
		.create_surface(wgpu::SurfaceTarget::Canvas(canvas.element.clone()))
		.map_err(|error| error.to_string())?;
	let shared = PAGE.with_borrow(|page| page.gpu.clone());
	let gpu = if let Some(gpu) = shared {
		gpu
	} else {
		let gpu = Gpu::request_on(instance, Some(&surface))
			.await
			.map_err(|error| error.to_string())?;
		if gpu.backend() == wgpu::Backend::Gl {
			gpu
		} else {
			PAGE.with_borrow_mut(|page| page.gpu.get_or_insert(gpu.clone()).clone())
		}
	};
	let capabilities = surface.get_capabilities(gpu.adapter());
	let format = capabilities
		.formats
		.iter()
		.copied()
		.find(|format| !format.is_srgb())
		.or_else(|| capabilities.formats.first().copied())
		.ok_or("the canvas supports no format")?;
	let alpha_mode = if capabilities
		.alpha_modes
		.contains(&wgpu::CompositeAlphaMode::PreMultiplied)
	{
		wgpu::CompositeAlphaMode::PreMultiplied
	} else {
		wgpu::CompositeAlphaMode::Auto
	};
	let config = wgpu::SurfaceConfiguration {
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
		format,
		width: canvas.element.width().max(1),
		height: canvas.element.height().max(1),
		present_mode: wgpu::PresentMode::Fifo,
		desired_maximum_frame_latency: 2,
		alpha_mode,
		view_formats: Vec::new(),
		color_space: wgpu::SurfaceColorSpace::Auto,
	};
	if !canvas.closed.get() {
		*canvas.surface.borrow_mut() = Some((surface, config, gpu));
	}
	Ok(())
}

fn request_frame(canvas: &Rc<Canvas>) {
	if canvas.closed.get() || canvas.scheduled.get() || canvas.surface.borrow().is_none() {
		return;
	}
	let Some(window) = web_sys::window() else {
		return;
	};
	let weak = Rc::downgrade(canvas);
	let callback = Closure::<dyn FnMut(f64)>::new(move |time: f64| {
		if let Some(canvas) = weak.upgrade() {
			canvas.scheduled.set(false);
			if frame(&canvas, time / 1000.0) {
				request_frame(&canvas);
			}
		}
	});
	if window
		.request_animation_frame(callback.as_ref().unchecked_ref())
		.is_ok()
	{
		canvas.scheduled.set(true);
		*canvas.callback.borrow_mut() = Some(callback);
	}
}

/// Fits the drawing buffer to the canvas's box at the display's pixel
/// ratio, returning its size.
fn fit(
	canvas: &Canvas,
	config: &mut wgpu::SurfaceConfiguration,
	surface: &wgpu::Surface<'_>,
	gpu: &Gpu,
) -> (u32, u32) {
	let ratio = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio());
	let limit = gpu.device().limits().max_texture_dimension_2d;
	let width = ((f64::from(canvas.element.client_width()) * ratio).round() as u32).min(limit);
	let height = ((f64::from(canvas.element.client_height()) * ratio).round() as u32).min(limit);
	if width > 0 && height > 0 && (width != config.width || height != config.height) {
		canvas.element.set_width(width);
		canvas.element.set_height(height);
		config.width = width;
		config.height = height;
		surface.configure(gpu.device(), config);
	}
	(width, height)
}

/// Draws one frame, returning whether the scene wants another.
fn frame(canvas: &Rc<Canvas>, time: f64) -> bool {
	let mut surface = canvas.surface.borrow_mut();
	let Some((surface, config, gpu)) = surface.as_mut() else {
		return false;
	};
	let (width, height) = fit(canvas, config, surface, gpu);
	if width == 0 || height == 0 {
		return false;
	}
	let texture = match surface.get_current_texture() {
		wgpu::CurrentSurfaceTexture::Success(texture)
		| wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
		wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
			surface.configure(gpu.device(), config);
			return true;
		}
		_ => return true,
	};
	let view = texture
		.texture
		.create_view(&wgpu::TextureViewDescriptor::default());
	let target = RenderTarget::new(&view, config.format, config.width, config.height);
	let outcome = with_scene(canvas.scene, |scene| {
		if scene.renderer.is_none() {
			let world = scene.world.take().unwrap_or_default();
			scene.renderer = Some(Renderer::with_world(gpu, world));
		}
		let elapsed = match scene.time {
			Some(last) if time > last => (time - last) as f32,
			_ => 0.0,
		};
		let renderer = scene.renderer.as_mut().expect("the renderer was created");
		if scene.time.is_none_or(|last| time > last) {
			scene.time = Some(time);
			renderer.advance(elapsed);
		}
		// A renderer is bound to the device it was made on; under WebGL2 only
		// the first canvas showing a scene can draw it.
		if !renderer.gpu().same_device(gpu) {
			return Ok(false);
		}
		renderer.render(&target)?;
		Ok::<_, crate::Error>(renderer.world().needs_frame())
	});
	gpu.queue().present(texture);
	match outcome {
		Some(Ok(more)) => {
			// Finished clips and raycasts queue events while drawing.
			let id = canvas.scene;
			wasm_bindgen_futures::spawn_local(async move { notify_events(id) });
			more
		}
		Some(Err(error)) => {
			web_sys::console::error_1(&format!("leptoskit_3d: {error}").into());
			false
		}
		None => false,
	}
}

/// Wakes event waiters only; the canvas schedules its own frames.
fn notify_events(id: SceneId) {
	let waiters = PAGE.with_borrow_mut(|page| {
		page.scenes
			.get_mut(&id)
			.filter(|scene| scene.world().has_events())
			.map(|scene| std::mem::take(&mut scene.waiters))
			.unwrap_or_default()
	});
	for waiter in waiters {
		waiter.wake();
	}
}
