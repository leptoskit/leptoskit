//! The browser build: the same sessions and frames as [`crate::guest`],
//! through WebXR, or the `deviceorientation` event for looking around.
//!
//! Start a session from a user gesture, such as a button's click: browsers
//! refuse immersive sessions and, on iOS, motion permission otherwise. The
//! browser composites the camera (phones) or passthrough (headsets) itself,
//! so frames carry no camera picture.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::math::Pose;
use crate::protocol::PlaneCache;
use crate::types::*;

mod frame;
mod inputs;
mod js;
mod motion;

/// The WebXR modes, in the order this crate tries them.
const XR_MODES: [(&str, Mode); 3] = [
	("immersive-ar", Mode::PhoneAr),
	("immersive-ar", Mode::ImmersiveAr),
	("immersive-vr", Mode::ImmersiveVr),
];

/// The browser's AR and VR. Make it once and keep it.
#[derive(Debug, Clone)]
pub struct Xr {
	system: JsValue,
	headset: bool,
}

impl Xr {
	/// The page's `navigator.xr`, if any; orientation sessions work without
	/// it.
	pub fn new() -> Result<Self, Error> {
		let window = web_sys::window().ok_or(Error::Unavailable)?;
		let navigator = window.navigator();
		let agent = navigator.user_agent().unwrap_or_default();

		Ok(Self {
			system: js::get(&navigator, "xr"),
			// WebXR calls the camera passthrough of a headset and a phone's
			// AR both `immersive-ar`; the browser says which it is.
			headset: agent.contains("OculusBrowser") || agent.contains("Quest"),
		})
	}

	fn mode_name(&self, mode: Mode) -> Option<&'static str> {
		XR_MODES
			.iter()
			.find(|(_, candidate)| *candidate == mode)
			.map(|(name, _)| *name)
			.filter(|_| {
				match mode {
					Mode::PhoneAr => !self.headset,
					Mode::ImmersiveAr => self.headset,
					_ => true,
				}
			})
	}

	async fn supported(&self, name: &str) -> bool {
		!self.system.is_undefined()
			&& js::await_call(&self.system, "isSessionSupported", &[name.into()])
				.await
				.is_ok_and(|supported| supported.as_bool() == Some(true))
	}

	/// What the browser can do. Depth, planes, and light are asked for when
	/// a session starts and granted where the browser has them.
	pub async fn capabilities(&self) -> Result<Capabilities, Error> {
		let mut capabilities = Capabilities::NONE;

		if motion::available() {
			capabilities =
				capabilities.with(Mode::Orientation, Features::HIT_TEST | Features::ANCHORS);
		}

		for (name, mode) in XR_MODES {
			if self.mode_name(mode).is_some() && self.supported(name).await {
				capabilities = capabilities.with(mode, offered(mode));
			}
		}

		Ok(capabilities)
	}

	/// Whether `mode` can start. The browser asks for the camera itself when
	/// the session starts, so this never reports `NeedsPermission`.
	pub async fn prepare(&self, mode: Mode, _install: bool) -> Result<Availability, Error> {
		Ok(if self.capabilities().await?.supports(mode) {
			Availability::Ready
		} else {
			Availability::Unsupported
		})
	}

	/// Starts a session. Call it from a user gesture.
	pub async fn start(&self, config: SessionConfig) -> Result<Session, Error> {
		if !SessionConfig::FRAME_RATES.contains(&config.frame_rate) {
			return Err(Error::InvalidArgument);
		}

		let state = if config.mode == Mode::Orientation {
			motion::start(&config).await?
		} else {
			let name = self.mode_name(config.mode).ok_or(Error::Unavailable)?;
			frame::start(&self.system, name, &config).await?
		};
		let info = state.borrow().info;

		Ok(Session {
			state,
			info,
			after: 0,
			planes: PlaneCache::default(),
		})
	}
}

/// What each WebXR mode can offer, before the browser says what it grants.
fn offered(mode: Mode) -> Features {
	let tracked = Features::HIT_TEST | Features::ANCHORS;

	match mode {
		Mode::PhoneAr => tracked | Features::PLANES | Features::DEPTH | Features::LIGHT_ESTIMATION,
		Mode::ImmersiveAr => {
			tracked
				| Features::PLANES
				| Features::DEPTH
				| Features::CONTROLLERS
				| Features::HANDS
				| Features::PASSTHROUGH
		}
		Mode::ImmersiveVr => tracked | Features::CONTROLLERS | Features::HANDS,
		Mode::Orientation => tracked,
	}
}

/// What a session shares between its frame callback and its owner.
pub(crate) struct State {
	info: SessionInfo,
	frame_rate: u32,
	session: JsValue,
	space: JsValue,
	callback: Option<Closure<dyn FnMut(f64, JsValue)>>,
	listener: Option<Closure<dyn FnMut(web_sys::Event)>>,
	published: Option<Frame>,
	last_publish: Option<f64>,
	sequence: u64,
	planes_revision: u64,
	planes: Vec<Plane>,
	plane_ids: js_sys::Map,
	next_plane: u64,
	depth: Option<crate::image::DepthImage>,
	anchors: Vec<(u64, JsValue)>,
	fixed_anchors: Vec<Anchor>,
	next_anchor: u64,
	hit_sources: Vec<(JsValue, Rc<RefCell<HitSlot>>)>,
	waker: Option<Waker>,
	ended: bool,
}

/// A hit test waiting for the next frame's results.
#[derive(Default)]
struct HitSlot {
	hits: Option<Vec<Hit>>,
	waker: Option<Waker>,
}

impl State {
	fn new(info: SessionInfo, frame_rate: u32, session: JsValue, space: JsValue) -> Self {
		Self {
			info,
			frame_rate,
			session,
			space,
			callback: None,
			listener: None,
			published: None,
			last_publish: None,
			sequence: 0,
			planes_revision: 0,
			planes: Vec::new(),
			plane_ids: js_sys::Map::new(),
			next_plane: 0,
			depth: None,
			anchors: Vec::new(),
			fixed_anchors: Vec::new(),
			next_anchor: 0,
			hit_sources: Vec::new(),
			waker: None,
			ended: false,
		}
	}

	/// Hands a frame to the waiting session when its frame rate allows.
	fn publish(&mut self, mut frame: Frame) {
		self.sequence += 1;

		if frame.planes != self.planes {
			self.planes.clone_from(&frame.planes);
			self.planes_revision += 1;
		}

		let period = 1000.0 / f64::from(self.frame_rate);
		let now = frame.time * 1000.0;

		if self
			.last_publish
			.is_some_and(|last| now - last < period - 2.0 && now >= last)
		{
			return;
		}

		self.last_publish = Some(now);
		frame.sequence = self.sequence;
		frame.planes_revision = self.planes_revision;
		frame.anchors.extend(self.fixed_anchors.iter().copied());
		self.published = Some(frame);

		if let Some(waker) = self.waker.take() {
			waker.wake();
		}
	}

	fn end(&mut self) {
		self.ended = true;

		// The browser may still hold these closures, and the end listener
		// may be the one running now, so they are leaked rather than dropped
		// under it. The orientation listener is removed first.
		if let Some(listener) = self.listener.take() {
			if self.session.is_undefined()
				&& let Some(window) = web_sys::window()
			{
				let _ = window.remove_event_listener_with_callback(
					"deviceorientation",
					listener.as_ref().unchecked_ref(),
				);
			}
			listener.forget();
		}

		if let Some(callback) = self.callback.take() {
			callback.forget();
		}

		if let Some(waker) = self.waker.take() {
			waker.wake();
		}

		for (_, slot) in self.hit_sources.drain(..) {
			if let Some(waker) = slot.borrow_mut().waker.take() {
				waker.wake();
			}
		}
	}
}

/// A running session. Dropping it doesn't end it; call [`Session::stop`].
pub struct Session {
	state: Rc<RefCell<State>>,
	info: SessionInfo,
	after: u64,
	planes: PlaneCache,
}

impl std::fmt::Debug for Session {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Session")
			.field("info", &self.info)
			.finish_non_exhaustive()
	}
}

/// Waits for a frame newer than `after`.
struct NextFrame<'a>(&'a RefCell<State>, u64);

impl Future for NextFrame<'_> {
	type Output = Result<Frame, Error>;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		let mut state = self.0.borrow_mut();

		if state.ended {
			return Poll::Ready(Err(Error::Cancelled));
		}

		match &state.published {
			Some(frame) if frame.sequence > self.1 => Poll::Ready(Ok(frame.clone())),
			_ => {
				state.waker = Some(context.waker().clone());
				Poll::Pending
			}
		}
	}
}

/// Waits for a hit test's results.
struct NextHits(Rc<RefCell<HitSlot>>, Rc<RefCell<State>>);

impl Future for NextHits {
	type Output = Result<Vec<Hit>, Error>;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		if self.1.borrow().ended {
			return Poll::Ready(Err(Error::Cancelled));
		}

		let mut slot = self.0.borrow_mut();

		if let Some(hits) = slot.hits.take() {
			Poll::Ready(Ok(hits))
		} else {
			slot.waker = Some(context.waker().clone());
			Poll::Pending
		}
	}
}

impl Session {
	/// The mode, features, and reference space the browser granted.
	pub fn info(&self) -> SessionInfo {
		self.info
	}

	/// The `XRSession`, for a renderer to set its `XRWebGLLayer` on, or
	/// `undefined` for an orientation session.
	pub fn xr_session(&self) -> JsValue {
		self.state.borrow().session.clone()
	}

	/// Waits for the next frame. Fails with `Cancelled` once the session
	/// ends, including when the user leaves it from the browser.
	pub async fn next_frame(&mut self) -> Result<Frame, Error> {
		let mut frame = NextFrame(&self.state, self.after).await?;
		self.after = frame.sequence;
		self.planes.merge(&mut frame);
		Ok(frame)
	}

	/// Casts a ray against the real world: the browser's hit test for planes
	/// and estimated surfaces, the depth picture where the browser shares it,
	/// and planes extended past their outlines in software.
	pub async fn hit_test(&self, request: HitTestRequest) -> Result<Vec<Hit>, Error> {
		let (views, session, space) = {
			let state = self.state.borrow();
			let views = state
				.published
				.as_ref()
				.map(|frame| frame.views.clone())
				.unwrap_or_default();
			(views, state.session.clone(), state.space.clone())
		};
		let ray = crate::hit_test::request_ray(request.origin, &views).ok_or(Error::Unavailable)?;
		let mut hits = {
			let state = self.state.borrow();
			let mut hits =
				crate::hit_test::hit_planes(ray, &state.planes, HitTargets::INFINITE_PLANES);
			if request.targets.contains(HitTargets::DEPTH)
				&& let (Some(depth), Some(view)) = (&state.depth, views.first())
			{
				hits.extend(crate::hit_test::hit_depth(ray, view, depth));
			}
			hits
		};

		if !session.is_undefined() && request.targets.contains(HitTargets::PLANES) {
			let slot = frame::request_hits(&session, &space, ray, &self.state).await?;
			hits.extend(NextHits(slot, Rc::clone(&self.state)).await?);
		}

		crate::hit_test::sort_and_truncate(&mut hits, request.max_results as usize);
		Ok(hits)
	}

	/// Pins a point to the real world. Browsers with WebXR anchors keep it in
	/// place as their map improves; elsewhere it stays where it was put.
	pub async fn add_anchor(&self, pose: Pose, _plane: Option<u64>) -> Result<u64, Error> {
		frame::add_anchor(&self.state, pose).await
	}

	/// Releases an anchor. It returns a future, as the guest client's does,
	/// so shared code awaits it on either.
	pub fn remove_anchor(&self, id: u64) -> std::future::Ready<Result<(), Error>> {
		std::future::ready(self.remove(id))
	}

	fn remove(&self, id: u64) -> Result<(), Error> {
		let mut state = self.state.borrow_mut();
		let before = state.anchors.len() + state.fixed_anchors.len();

		state.anchors.retain(|(anchor, handle)| {
			if *anchor == id {
				let _ = js::call(handle, "delete", &[]);
			}
			*anchor != id
		});
		state.fixed_anchors.retain(|anchor| anchor.id != id);

		if state.anchors.len() + state.fixed_anchors.len() == before {
			Err(Error::InvalidArgument)
		} else {
			Ok(())
		}
	}

	/// Ends the session.
	pub async fn stop(self) -> Result<(), Error> {
		let session = self.state.borrow().session.clone();
		self.state.borrow_mut().end();

		if !session.is_undefined() {
			js::await_call(&session, "end", &[]).await?;
		}

		Ok(())
	}
}

/// Ends the shared state when the browser ends the session.
fn on_end(session: &JsValue, state: &Rc<RefCell<State>>) {
	let weak = Rc::downgrade(state);
	let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
		if let Some(state) = weak.upgrade() {
			state.borrow_mut().end();
		}
	});
	let _ = js::call(
		session,
		"addEventListener",
		&[
			"end".into(),
			listener.as_ref().unchecked_ref::<JsValue>().clone(),
		],
	);
	state.borrow_mut().listener = Some(listener);
}
