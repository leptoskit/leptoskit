//! The session runtime: one backend, one session, and the guest's bounded
//! frame stream.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge::Error;

use super::Backend;
use super::CameraFrame;
use super::DepthEstimator;
use super::Started;
use super::Viewport;
use super::XrFrame;
use crate::math::Pose;
use crate::protocol::NextFrame;
use crate::types::*;

/// Anchor ids the runtime keeps itself, for backends without anchors of their
/// own, have the top bit set so they never clash with a backend's.
const RUNTIME_ANCHOR: u64 = 1 << 63;
/// How often the built-in driver polls a running session: often enough not
/// to miss a 60 Hz camera frame.
const DRIVE_PERIOD: Duration = Duration::from_micros(16_667);
/// How often the built-in driver checks for a session while none runs.
const IDLE_PERIOD: Duration = Duration::from_millis(50);
/// Frames arriving this much early still count as due, so a 60 Hz source
/// delivering at 59.9 Hz does not halve a 60 Hz guest rate.
const RATE_SLACK: f64 = 0.002;

/// One backend and at most one session, shared by the plugin that serves the
/// guest and the renderer that draws the frames.
///
/// Something must call [`XrRuntime::update`] once per display frame while a
/// session runs: the renderer's frame loop, or the [`Driver`] thread from
/// [`XrRuntime::drive`] when nothing renders.
pub struct XrRuntime {
	state: Mutex<State>,
	external_driver: AtomicBool,
	clock: Instant,
	listener: Mutex<Option<Listener>>,
}

/// Called after a session starts, changes, or stops.
type Listener = Arc<dyn Fn() + Send + Sync>;

impl std::fmt::Debug for XrRuntime {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("XrRuntime").finish_non_exhaustive()
	}
}

struct State {
	backend: Box<dyn Backend>,
	session: Option<Session>,
	generation: u64,
	waiters: Vec<Waker>,
	estimator: Option<Box<dyn DepthEstimator>>,
	presented: Option<u32>,
}

struct Session {
	generation: u64,
	config: SessionConfig,
	info: SessionInfo,
	latest: Option<Arc<XrFrame>>,
	published: Option<(u64, Arc<XrFrame>)>,
	sequence: u64,
	last_publish: Option<f64>,
	planes: Arc<[Plane]>,
	planes_revision: u64,
	anchors: Vec<Anchor>,
	next_anchor: u64,
	/// Whether the depth estimator fills in the depth the backend lacks.
	estimate_depth: bool,
}

impl Session {
	fn due(&self, time: f64) -> bool {
		let period = 1.0 / f64::from(self.config.frame_rate);
		self.last_publish
			.is_none_or(|last| time - last >= period - RATE_SLACK || time < last)
	}
}

impl XrRuntime {
	/// A runtime over `backend`, with no session running.
	pub fn new(backend: impl Backend + 'static) -> Arc<Self> {
		Arc::new(Self {
			state: Mutex::new(State {
				backend: Box::new(backend),
				session: None,
				generation: 0,
				waiters: Vec::new(),
				estimator: None,
				presented: None,
			}),
			external_driver: AtomicBool::new(false),
			clock: Instant::now(),
			listener: Mutex::new(None),
		})
	}

	fn lock(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Calls `listener` after a session starts, changes, or stops, on the
	/// thread that made the change and outside the runtime's lock, so it may
	/// call the runtime. A renderer uses it to start drawing the session.
	pub fn set_listener(&self, listener: Option<Arc<dyn Fn() + Send + Sync>>) {
		*self.listener.lock().unwrap_or_else(PoisonError::into_inner) = listener;
	}

	fn changed(&self) {
		let listener = self
			.listener
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone();

		if let Some(listener) = listener {
			listener();
		}
	}

	/// Fills in depth from camera pictures for sessions whose platform has
	/// none, from the next session on: with an estimator, every mode that
	/// offers the camera picture also offers depth. `None` removes it.
	pub fn set_depth_estimator(&self, estimator: Option<Box<dyn DepthEstimator>>) {
		self.lock().estimator = estimator;
	}

	/// What the device can do.
	pub fn capabilities(&self) -> Capabilities {
		let state = self.lock();
		estimated(state.backend.capabilities(), state.estimator.is_some())
	}

	/// Whether `mode` can start now; with `install`, opens the store for a
	/// missing AR service.
	pub fn prepare(&self, mode: Mode, install: bool) -> Availability {
		self.lock().backend.prepare(mode, install)
	}

	/// Starts a session. Fails with `Busy` while one runs, and `Unavailable`
	/// when the device lacks the mode or a required feature.
	pub fn start(&self, config: SessionConfig) -> Result<SessionInfo, Error> {
		let info = self.start_locked(config)?;
		self.changed();
		Ok(info)
	}

	fn start_locked(&self, config: SessionConfig) -> Result<SessionInfo, Error> {
		let mut state = self.lock();

		if state.session.is_some() {
			return Err(Error::Busy);
		}

		let (info, estimate_depth) = begin(&mut state, &config, false)?;
		state.generation += 1;
		state.session = Some(Session {
			generation: state.generation,
			config,
			info,
			latest: None,
			published: None,
			sequence: 0,
			last_publish: None,
			planes: Arc::from([]),
			planes_revision: 0,
			anchors: Vec::new(),
			next_anchor: 1,
			estimate_depth,
		});
		Ok(info)
	}

	/// Changes the running session's features, reference space, or frame
	/// rate. The mode can't change: stop and start again for that.
	pub fn configure(&self, config: SessionConfig) -> Result<SessionInfo, Error> {
		let info = self.configure_locked(config)?;
		self.changed();
		Ok(info)
	}

	fn configure_locked(&self, config: SessionConfig) -> Result<SessionInfo, Error> {
		let mut state = self.lock();

		match &state.session {
			None => return Err(Error::Unavailable),
			Some(session) if session.config.mode != config.mode => {
				return Err(Error::InvalidArgument);
			}
			Some(_) => {}
		}

		let (info, estimate_depth) = begin(&mut state, &config, true)?;
		let session = state.session.as_mut().ok_or(Error::Unavailable)?;
		session.config = config;
		session.info = info;
		session.estimate_depth = estimate_depth;
		Ok(info)
	}

	/// The running session, if any.
	pub fn session(&self) -> Option<SessionInfo> {
		self.lock().session.as_ref().map(|session| session.info)
	}

	/// Polls the backend for a new frame and hands it to the guest when its
	/// frame rate allows. Returns the new frame for the renderer, or `None`
	/// when nothing changed or no session runs. `time` is the caller's
	/// monotonic clock in seconds.
	pub fn update(&self, time: f64) -> Result<Option<Arc<XrFrame>>, Error> {
		let mut state = self.lock();
		let state = &mut *state;
		let Some(session) = state.session.as_mut() else {
			return Ok(None);
		};
		let Some(mut frame) = state.backend.update(time)? else {
			return Ok(None);
		};

		if *frame.planes != *session.planes {
			session.planes = Arc::clone(&frame.planes);
			session.planes_revision += 1;
		}

		frame.planes_revision = session.planes_revision;
		frame.anchors.extend(session.anchors.iter().copied());

		if session.estimate_depth
			&& frame.depth.is_none()
			&& let Some(estimator) = state.estimator.as_mut()
		{
			if estimator.ready()
				&& let Some(camera) = CameraFrame::from_frame(&frame, estimator.max_side())
			{
				estimator.submit(Arc::new(camera));
			}

			if let Some(estimate) = estimator.latest() {
				frame.depth = Some(estimate.depth);
				frame.depth_view = Some(estimate.view);
			}
		}

		session.sequence += 1;
		let frame = Arc::new(frame);
		session.latest = Some(Arc::clone(&frame));

		if session.due(frame.time) {
			session.last_publish = Some(frame.time);
			session.published = Some((session.sequence, Arc::clone(&frame)));
			wake(&mut state.waiters);
		}

		Ok(Some(frame))
	}

	/// The newest frame, for a renderer that draws on its own schedule.
	pub fn latest_frame(&self) -> Option<Arc<XrFrame>> {
		self.lock()
			.session
			.as_ref()
			.and_then(|session| session.latest.clone())
	}

	/// Waits for a frame newer than `request.after`, as the guest receives it.
	/// Resolves with `Cancelled` when the session stops first.
	pub fn next_frame(self: &Arc<Self>, request: NextFrame) -> NextGuestFrame {
		let generation = self
			.lock()
			.session
			.as_ref()
			.map(|session| session.generation);

		NextGuestFrame {
			runtime: Arc::clone(self),
			request,
			generation,
		}
	}

	/// Casts a ray against the real world as of the newest frame.
	///
	/// Depth an estimator read from an older picture is cast against from
	/// the view it was seen from ([`XrFrame::depth_view`]), so a ray from
	/// where the viewer is now meets the room where it was; the backend
	/// answers the other targets.
	pub fn hit_test(&self, request: &HitTestRequest) -> Result<Vec<Hit>, Error> {
		let mut state = self.lock();
		let state = &mut *state;
		let session = state.session.as_ref().ok_or(Error::Unavailable)?;
		let Some(frame) = session.latest.clone() else {
			return Ok(Vec::new());
		};
		let estimated = frame
			.depth
			.as_ref()
			.zip(frame.depth_view.as_ref())
			.filter(|_| request.targets.contains(HitTargets::DEPTH));

		let mut hits = match estimated {
			Some((depth, seen)) => {
				let others = HitTestRequest {
					targets: HitTargets::from_bits(
						request.targets.bits() & !HitTargets::DEPTH.bits(),
					),
					..*request
				};
				let mut hits = state.backend.hit_test(&others, &frame);

				if let Some(ray) = crate::hit_test::request_ray(request.origin, &frame.views) {
					hits.extend(crate::hit_test::hit_depth(ray, seen, depth));
				}

				hits
			}
			None => state.backend.hit_test(request, &frame),
		};
		crate::hit_test::sort_and_truncate(&mut hits, request.max_results as usize);
		Ok(hits)
	}

	/// Pins a point to the real world, with the platform's anchors where it
	/// has them and a fixed point otherwise.
	pub fn add_anchor(&self, pose: Pose, plane: Option<u64>) -> Result<u64, Error> {
		let mut state = self.lock();
		let state = &mut *state;
		let session = state.session.as_mut().ok_or(Error::Unavailable)?;

		match state.backend.add_anchor(pose, plane) {
			Err(Error::Unavailable) => {}
			result => return result,
		}

		if session.anchors.len() >= Frame::MAX_ANCHORS {
			return Err(Error::Busy);
		}

		let id = RUNTIME_ANCHOR | session.next_anchor;
		session.next_anchor += 1;
		session.anchors.push(Anchor {
			id,
			pose,
			tracking: TrackingState::Normal,
		});
		Ok(id)
	}

	/// Releases an anchor.
	pub fn remove_anchor(&self, id: u64) -> Result<(), Error> {
		let mut state = self.lock();
		let state = &mut *state;
		let session = state.session.as_mut().ok_or(Error::Unavailable)?;

		if id & RUNTIME_ANCHOR == 0 {
			return state.backend.remove_anchor(id);
		}

		let before = session.anchors.len();
		session.anchors.retain(|anchor| anchor.id != id);

		if session.anchors.len() == before {
			Err(Error::InvalidArgument)
		} else {
			Ok(())
		}
	}

	/// Chooses the `leptoskit_3d` scene a headset renderer draws, by its id,
	/// or none. It holds until the session stops.
	pub fn present(&self, scene: Option<u32>) {
		self.lock().presented = scene;
	}

	/// The scene a headset renderer draws, if any.
	pub fn presented(&self) -> Option<u32> {
		self.lock().presented
	}

	/// Tells a phone backend the screen it projects for.
	pub fn set_viewport(&self, viewport: Viewport) {
		self.lock().backend.set_viewport(viewport);
	}

	/// Ends the session. Waiting frame requests resolve with `Cancelled`.
	pub fn stop(&self) {
		let mut state = self.lock();
		let stopped = state.session.take().is_some();

		if stopped {
			state.backend.stop();
			state.presented = None;

			if let Some(estimator) = state.estimator.as_mut() {
				estimator.reset();
			}
		}

		wake(&mut state.waiters);
		drop(state);

		if stopped {
			self.changed();
		}
	}

	/// Whether a renderer calls [`XrRuntime::update`] itself. While it does,
	/// the built-in [`Driver`] leaves the session alone.
	pub fn set_external_driver(&self, external: bool) {
		self.external_driver.store(external, Ordering::Release);
	}

	/// Seconds since this runtime was made, on a monotonic clock.
	pub fn now(&self) -> f64 {
		self.clock.elapsed().as_secs_f64()
	}

	/// Starts a thread that updates running sessions about 60 times a second,
	/// for hosts where nothing renders the session (the guest still receives
	/// frames). It stops when the [`Driver`] is dropped.
	pub fn drive(self: &Arc<Self>) -> std::io::Result<Driver> {
		let stop = Arc::new(AtomicBool::new(false));
		let runtime = Arc::downgrade(self);
		let flag = Arc::clone(&stop);
		let thread = std::thread::Builder::new()
			.name("leptoskit-xr".into())
			.spawn(move || drive(&runtime, &flag))?;

		Ok(Driver {
			stop,
			thread: Some(thread),
		})
	}
}

/// `capabilities` with depth added to every mode that offers the camera
/// picture, when a depth estimator can make it.
fn estimated(capabilities: Capabilities, estimating: bool) -> Capabilities {
	if !estimating {
		return capabilities;
	}

	capabilities
		.modes()
		.filter(|(_, features)| features.contains(Features::CAMERA_IMAGE))
		.fold(capabilities, |capabilities, (mode, features)| {
			capabilities.with(mode, features | Features::DEPTH)
		})
}

/// Starts (or, with `configure`, reconfigures) the backend for `config`,
/// asking it for the camera picture when the depth estimator stands in for
/// depth it lacks. Returns the session's info and whether the estimator
/// fills in depth.
fn begin(
	state: &mut State,
	config: &SessionConfig,
	configure: bool,
) -> Result<(SessionInfo, bool), Error> {
	let offered = state
		.backend
		.capabilities()
		.features(config.mode)
		.unwrap_or_default();
	let mut info = negotiate(
		&estimated(state.backend.capabilities(), state.estimator.is_some()),
		config,
	)?;
	let estimating = state.estimator.is_some()
		&& info.features.contains(Features::DEPTH)
		&& offered.contains(Features::CAMERA_IMAGE);
	let mut asked = info.features;

	if estimating {
		asked = asked | Features::CAMERA_IMAGE;

		if !offered.contains(Features::DEPTH) {
			asked = asked.difference(Features::DEPTH);
		}
	}

	let mut started = if configure {
		state.backend.configure(config, asked)?
	} else {
		state.backend.start(config, asked)?
	};
	let estimate_depth = estimating
		&& !started.features.contains(Features::DEPTH)
		&& started.features.contains(Features::CAMERA_IMAGE);

	if estimate_depth {
		started.features = started.features | Features::DEPTH;
	}

	if let Err(error) = accept(&mut info, config, started) {
		if !configure {
			state.backend.stop();
		}

		return Err(error);
	}

	Ok((info, estimate_depth))
}

fn negotiate(capabilities: &Capabilities, config: &SessionConfig) -> Result<SessionInfo, Error> {
	if !SessionConfig::FRAME_RATES.contains(&config.frame_rate) {
		return Err(Error::InvalidArgument);
	}

	let offered = capabilities
		.features(config.mode)
		.ok_or(Error::Unavailable)?;
	let features = config.negotiate(offered).ok_or(Error::Unavailable)?;

	Ok(SessionInfo {
		mode: config.mode,
		features,
		reference_space: config.reference_space,
	})
}

/// Records what the backend started, refusing a session that lost a required
/// feature on the way.
fn accept(info: &mut SessionInfo, config: &SessionConfig, started: Started) -> Result<(), Error> {
	if !started.features.contains(config.required) {
		return Err(Error::Unavailable);
	}

	info.reference_space = started.reference_space;
	info.features = info.features.intersection(started.features);
	Ok(())
}

fn wake(waiters: &mut Vec<Waker>) {
	for waiter in waiters.drain(..) {
		waiter.wake();
	}
}

fn drive(runtime: &Weak<XrRuntime>, stop: &AtomicBool) {
	while !stop.load(Ordering::Acquire) {
		let Some(runtime) = runtime.upgrade() else {
			return;
		};
		let active =
			!runtime.external_driver.load(Ordering::Acquire) && runtime.session().is_some();

		if active && runtime.update(runtime.now()).is_err() {
			runtime.stop();
		}

		drop(runtime);

		std::thread::sleep(if active { DRIVE_PERIOD } else { IDLE_PERIOD });
	}
}

/// The thread [`XrRuntime::drive`] started. Dropping it stops the thread.
#[derive(Debug)]
pub struct Driver {
	stop: Arc<AtomicBool>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Driver {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);

		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

/// A wait for the guest's next frame; see [`XrRuntime::next_frame`].
#[derive(Debug)]
pub struct NextGuestFrame {
	runtime: Arc<XrRuntime>,
	request: NextFrame,
	generation: Option<u64>,
}

impl Future for NextGuestFrame {
	type Output = Result<Frame, Error>;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		let mut state = self.runtime.lock();
		let state = &mut *state;
		let Some(session) = state
			.session
			.as_ref()
			.filter(|session| Some(session.generation) == self.generation)
		else {
			return Poll::Ready(Err(Error::Cancelled));
		};

		if let Some((sequence, frame)) = &session.published
			&& *sequence > self.request.after
		{
			let include_planes = frame.planes_revision != self.request.planes_revision;

			return Poll::Ready(Ok(frame.to_guest(*sequence, include_planes)));
		}

		if !state
			.waiters
			.iter()
			.any(|waiter| waiter.will_wake(context.waker()))
		{
			state.waiters.push(context.waker().clone());
		}

		Poll::Pending
	}
}
