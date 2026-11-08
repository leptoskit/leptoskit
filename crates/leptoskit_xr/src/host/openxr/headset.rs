//! One OpenXR session: its lifecycle, its frame loop, and locating the
//! views, controllers, and hands each frame.

use std::sync::Arc;

use leptoskit_bridge::Error;
use openxr as xr;
use xr::sys::Handle as _;

use super::Headset;
use super::System;
use super::VIEW_CONFIGURATION;
use crate::host::XrFrame;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Vec3;
use crate::types::*;

/// The two hands, left first.
const HANDS: [(&str, Handedness); 2] = [("left", Handedness::Left), ("right", Handedness::Right)];

fn pose(pose: xr::Posef) -> Pose {
	let (p, o) = (pose.position, pose.orientation);
	Pose::new(
		Vec3::new(p.x, p.y, p.z),
		Quat::new(o.x, o.y, o.z, o.w)
			.normalized()
			.unwrap_or(Quat::IDENTITY),
	)
}

fn located(location: &xr::SpaceLocation) -> Option<Pose> {
	location
		.location_flags
		.contains(
			xr::SpaceLocationFlags::ORIENTATION_VALID | xr::SpaceLocationFlags::POSITION_VALID,
		)
		.then(|| pose(location.pose))
}

/// The controller actions, bound for Meta's Touch controllers and the
/// generic simple controller.
struct Input {
	set: xr::ActionSet,
	paths: [xr::Path; 2],
	aim: xr::Action<xr::Posef>,
	grip: xr::Action<xr::Posef>,
	trigger: xr::Action<f32>,
	squeeze: xr::Action<f32>,
	thumbstick: xr::Action<xr::Vector2f>,
	primary: xr::Action<bool>,
	secondary: xr::Action<bool>,
	aim_spaces: Vec<xr::Space>,
	grip_spaces: Vec<xr::Space>,
}

impl Input {
	fn new<G>(instance: &xr::Instance, session: &xr::Session<G>) -> xr::Result<Self> {
		let set = instance.create_action_set("leptoskit", "leptoskit", 0)?;
		let paths = [
			instance.string_to_path("/user/hand/left")?,
			instance.string_to_path("/user/hand/right")?,
		];
		let input = Self {
			aim: set.create_action("aim", "Aim", &paths)?,
			grip: set.create_action("grip", "Grip", &paths)?,
			trigger: set.create_action("trigger", "Trigger", &paths)?,
			squeeze: set.create_action("squeeze", "Squeeze", &paths)?,
			thumbstick: set.create_action("thumbstick", "Thumbstick", &paths)?,
			primary: set.create_action("primary", "Primary button", &paths)?,
			secondary: set.create_action("secondary", "Secondary button", &paths)?,
			aim_spaces: Vec::new(),
			grip_spaces: Vec::new(),
			set,
			paths,
		};
		input.suggest(instance)?;
		session.attach_action_sets(&[&input.set])?;
		let mut input = input;

		for path in paths {
			input
				.aim_spaces
				.push(input.aim.create_space(session, path, xr::Posef::IDENTITY)?);
			input.grip_spaces.push(
				input
					.grip
					.create_space(session, path, xr::Posef::IDENTITY)?,
			);
		}

		Ok(input)
	}

	fn suggest(&self, instance: &xr::Instance) -> xr::Result<()> {
		let path = |text: &str| instance.string_to_path(text);
		let touch = [
			xr::Binding::new(&self.aim, path("/user/hand/left/input/aim/pose")?),
			xr::Binding::new(&self.aim, path("/user/hand/right/input/aim/pose")?),
			xr::Binding::new(&self.grip, path("/user/hand/left/input/grip/pose")?),
			xr::Binding::new(&self.grip, path("/user/hand/right/input/grip/pose")?),
			xr::Binding::new(&self.trigger, path("/user/hand/left/input/trigger/value")?),
			xr::Binding::new(&self.trigger, path("/user/hand/right/input/trigger/value")?),
			xr::Binding::new(&self.squeeze, path("/user/hand/left/input/squeeze/value")?),
			xr::Binding::new(&self.squeeze, path("/user/hand/right/input/squeeze/value")?),
			xr::Binding::new(&self.thumbstick, path("/user/hand/left/input/thumbstick")?),
			xr::Binding::new(&self.thumbstick, path("/user/hand/right/input/thumbstick")?),
			xr::Binding::new(&self.primary, path("/user/hand/left/input/x/click")?),
			xr::Binding::new(&self.primary, path("/user/hand/right/input/a/click")?),
			xr::Binding::new(&self.secondary, path("/user/hand/left/input/y/click")?),
			xr::Binding::new(&self.secondary, path("/user/hand/right/input/b/click")?),
		];
		instance.suggest_interaction_profile_bindings(
			path("/interaction_profiles/oculus/touch_controller")?,
			&touch,
		)?;
		let simple = [
			xr::Binding::new(&self.aim, path("/user/hand/left/input/aim/pose")?),
			xr::Binding::new(&self.aim, path("/user/hand/right/input/aim/pose")?),
			xr::Binding::new(&self.grip, path("/user/hand/left/input/grip/pose")?),
			xr::Binding::new(&self.grip, path("/user/hand/right/input/grip/pose")?),
			xr::Binding::new(&self.trigger, path("/user/hand/left/input/select/click")?),
			xr::Binding::new(&self.trigger, path("/user/hand/right/input/select/click")?),
		];
		instance.suggest_interaction_profile_bindings(
			path("/interaction_profiles/khr/simple_controller")?,
			&simple,
		)
	}

	/// Each active controller's poses, buttons, and stick.
	fn sources<G>(
		&self,
		session: &xr::Session<G>,
		base: &xr::Space,
		time: xr::Time,
	) -> Vec<InputSource> {
		if session.sync_actions(&[(&self.set).into()]).is_err() {
			return Vec::new();
		}

		(0..2)
			.filter_map(|index| self.source(session, base, time, index))
			.collect()
	}

	fn source<G>(
		&self,
		session: &xr::Session<G>,
		base: &xr::Space,
		time: xr::Time,
		index: usize,
	) -> Option<InputSource> {
		let path = self.paths[index];

		if !self.aim.is_active(session, path).unwrap_or(false) {
			return None;
		}

		let target_ray = located(&self.aim_spaces[index].locate(base, time).ok()?)?;
		let grip = self.grip_spaces[index]
			.locate(base, time)
			.ok()
			.as_ref()
			.and_then(located);
		let value =
			|action: &xr::Action<f32>| action.state(session, path).map_or(0.0, |s| s.current_state);
		let pressed =
			|action: &xr::Action<bool>| action.state(session, path).is_ok_and(|s| s.current_state);
		let stick = self
			.thumbstick
			.state(session, path)
			.map_or(xr::Vector2f::default(), |state| state.current_state);
		let analog = |value: f32| {
			Button {
				pressed: value > 0.5,
				touched: value > 0.0,
				value,
			}
		};
		let digital = |pressed: bool| {
			Button {
				pressed,
				touched: pressed,
				value: if pressed { 1.0 } else { 0.0 },
			}
		};

		Some(InputSource {
			id: index as u32,
			handedness: HANDS[index].1,
			kind: InputKind::Controller,
			target_ray,
			grip,
			buttons: vec![
				analog(value(&self.trigger)),
				analog(value(&self.squeeze)),
				digital(pressed(&self.primary)),
				digital(pressed(&self.secondary)),
			],
			axes: vec![stick.x, -stick.y],
			hand: None,
		})
	}
}

/// A running OpenXR session over graphics API `G`.
///
/// The renderer that made the session keeps a reference too: after each
/// [`Headset::update`] that returns a frame, it renders into its swapchains
/// and calls [`HeadsetSession::end_frame`] with its layers. A frame the
/// renderer skips is ended empty on the next update.
pub struct HeadsetSession<G: xr::Graphics> {
	system: Arc<System>,
	session: xr::Session<G>,
	waiter: xr::FrameWaiter,
	stream: xr::FrameStream<G>,
	space: xr::Space,
	view_space: xr::Space,
	input: Input,
	hands: Vec<(Handedness, xr::HandTracker)>,
	passthrough: Option<(xr::Passthrough, xr::PassthroughLayerFB)>,
	blend_mode: xr::EnvironmentBlendMode,
	events: xr::EventDataBuffer,
	running: bool,
	frame: Option<xr::FrameState>,
	begun: bool,
}

// SAFETY: OpenXR handles may be used from any thread, one call at a time;
// the session is always behind the shared headset's mutex.
unsafe impl<G: xr::Graphics> Send for HeadsetSession<G> {}

impl<G: xr::Graphics> HeadsetSession<G> {
	/// Takes over a session the renderer created, with its frame waiter and
	/// stream, and sets up its spaces, controllers, hands, and passthrough
	/// for `config` and `features`.
	pub fn new(
		system: Arc<System>,
		session: xr::Session<G>,
		waiter: xr::FrameWaiter,
		stream: xr::FrameStream<G>,
		config: &SessionConfig,
		features: Features,
	) -> Result<Self, Error> {
		let space = reference_space(&system, &session, config.reference_space)?;
		let view_space = session
			.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
			.map_err(|_| Error::Native)?;
		let input = Input::new(&system.instance, &session).map_err(|_| Error::Native)?;
		let hands = if features.contains(Features::HANDS) && system.hand_tracking {
			[
				(xr::Hand::LEFT, Handedness::Left),
				(xr::Hand::RIGHT, Handedness::Right),
			]
			.into_iter()
			.filter_map(|(hand, handedness)| {
				Some((handedness, session.create_hand_tracker(hand).ok()?))
			})
			.collect()
		} else {
			Vec::new()
		};
		let (passthrough, blend_mode) = passthrough(&system, &session, config.mode)?;

		Ok(Self {
			system,
			session,
			waiter,
			stream,
			space,
			view_space,
			input,
			hands,
			passthrough,
			blend_mode,
			events: xr::EventDataBuffer::new(),
			running: false,
			frame: None,
			begun: false,
		})
	}

	/// The session, for creating swapchains.
	pub fn session(&self) -> &xr::Session<G> {
		&self.session
	}

	/// The space every pose is reported in.
	pub fn space(&self) -> &xr::Space {
		&self.space
	}

	/// The frame being rendered: its display time and whether to draw.
	pub fn frame_state(&self) -> Option<xr::FrameState> {
		self.frame.filter(|_| self.begun)
	}

	/// Meta's passthrough layer, to submit beneath the projection layer in
	/// an immersive AR session.
	pub fn passthrough_layer(&self) -> Option<&xr::PassthroughLayerFB> {
		self.passthrough.as_ref().map(|(_, layer)| layer)
	}

	/// Ends the frame [`Headset::update`] began, showing `layers`.
	pub fn end_frame(&mut self, layers: &[&xr::CompositionLayerBase<'_, G>]) -> Result<(), Error> {
		let state = self.frame.ok_or(Error::InvalidCall)?;

		if !std::mem::take(&mut self.begun) {
			return Err(Error::InvalidCall);
		}

		self.stream
			.end(state.predicted_display_time, self.blend_mode, layers)
			.map_err(|_| Error::Native)
	}

	/// Ends the frame with one projection layer of `views`, over Meta's
	/// passthrough layer when the session shows the room.
	pub fn end_projection(
		&mut self,
		views: &[xr::CompositionLayerProjectionView<'_, G>],
	) -> Result<(), Error> {
		let state = self.frame.ok_or(Error::InvalidCall)?;

		if !std::mem::take(&mut self.begun) {
			return Err(Error::InvalidCall);
		}

		let projection = xr::CompositionLayerProjection::new()
			.layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
			.space(&self.space)
			.views(views);
		// `openxr` keeps its passthrough layer builder private, so the layer
		// is written out; its header is what `xrEndFrame` reads.
		let passthrough = self.passthrough.as_ref().map(|(_, layer)| {
			xr::sys::CompositionLayerPassthroughFB {
				ty: xr::sys::CompositionLayerPassthroughFB::TYPE,
				next: std::ptr::null(),
				flags: xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA,
				space: xr::sys::Space::from_raw(0),
				layer_handle: *layer.inner(),
			}
		});
		let time = state.predicted_display_time;
		let result = match &passthrough {
			Some(passthrough) => {
				// SAFETY: `CompositionLayerBase` is a transparent wrapper over
				// the layer header that begins every layer struct, and the
				// layer lives until the call returns.
				let base = unsafe {
					&*std::ptr::from_ref(passthrough).cast::<xr::CompositionLayerBase<'_, G>>()
				};
				self.stream.end(time, self.blend_mode, &[base, &projection])
			}
			None => self.stream.end(time, self.blend_mode, &[&projection]),
		};

		result.map_err(|_| Error::Native)
	}

	fn poll_events(&mut self) -> Result<(), Error> {
		loop {
			let state = match self
				.system
				.instance
				.poll_event(&mut self.events)
				.map_err(|_| Error::Native)?
			{
				None => return Ok(()),
				Some(xr::Event::SessionStateChanged(change)) => change.state(),
				Some(xr::Event::InstanceLossPending(_)) => return Err(Error::Cancelled),
				Some(_) => continue,
			};

			self.change_state(state)?;
		}
	}

	fn change_state(&mut self, state: xr::SessionState) -> Result<(), Error> {
		match state {
			xr::SessionState::READY => {
				self.session
					.begin(VIEW_CONFIGURATION)
					.map_err(|_| Error::Native)?;
				self.running = true;
			}
			xr::SessionState::STOPPING => {
				self.session.end().map_err(|_| Error::Native)?;
				self.running = false;
			}
			xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
				self.running = false;
				return Err(Error::Cancelled);
			}
			_ => {}
		}

		Ok(())
	}

	fn locate(&self, time: xr::Time) -> XrFrame {
		let (flags, views) = self
			.session
			.locate_views(VIEW_CONFIGURATION, time, &self.space)
			.unwrap_or((xr::ViewStateFlags::EMPTY, Vec::new()));
		let viewer = self.view_space.locate(&self.space, time).ok();
		let tracking = if flags
			.contains(xr::ViewStateFlags::POSITION_VALID | xr::ViewStateFlags::ORIENTATION_VALID)
		{
			TrackingState::Normal
		} else if flags.contains(xr::ViewStateFlags::ORIENTATION_VALID) {
			TrackingState::Limited(LimitedReason::Unknown)
		} else {
			TrackingState::NotAvailable
		};
		let mut inputs = self.input.sources(&self.session, &self.space, time);
		inputs.extend(self.hands(time));

		XrFrame {
			time: time.as_nanos() as f64 / 1e9,
			tracking,
			viewer: viewer.map_or(Pose::IDENTITY, |location| pose(location.pose)),
			views: views
				.iter()
				.map(|view| {
					View {
						pose: pose(view.pose),
						projection: Projection::Fov(Fov {
							left: view.fov.angle_left,
							right: view.fov.angle_right,
							up: view.fov.angle_up,
							down: view.fov.angle_down,
						}),
					}
				})
				.collect(),
			inputs,
			..XrFrame::default()
		}
	}

	/// Tracked hands, aiming from the palm along the index finger.
	fn hands(&self, time: xr::Time) -> Vec<InputSource> {
		self.hands
			.iter()
			.enumerate()
			.filter_map(|(index, (handedness, tracker))| {
				let joints = self.space.locate_hand_joints(tracker, time).ok()??;
				let mut hand = Box::new([Joint::default(); HAND_JOINTS]);

				for (joint, location) in hand.iter_mut().zip(joints.iter()) {
					*joint = Joint {
						pose: pose(location.pose),
						radius: location.radius,
						tracked: location
							.location_flags
							.contains(xr::SpaceLocationFlags::POSITION_VALID),
					};
				}

				let palm = hand[xr::HandJoint::PALM.into_raw() as usize].pose;

				Some(InputSource {
					id: 2 + index as u32,
					handedness: *handedness,
					kind: InputKind::Hand,
					target_ray: palm,
					grip: Some(palm),
					buttons: Vec::new(),
					axes: Vec::new(),
					hand: Some(hand),
				})
			})
			.collect()
	}
}

impl<G: xr::Graphics> Headset for HeadsetSession<G> {
	fn update(&mut self) -> Result<Option<XrFrame>, Error> {
		self.poll_events()?;

		if !self.running {
			return Ok(None);
		}

		if self.begun {
			// The renderer skipped the last frame; show nothing for it.
			self.end_frame(&[])?;
		}

		let state = self.waiter.wait().map_err(|_| Error::Native)?;
		self.stream.begin().map_err(|_| Error::Native)?;
		self.frame = Some(state);
		self.begun = true;
		Ok(Some(self.locate(state.predicted_display_time)))
	}

	fn stop(&mut self) {
		if self.begun {
			let _ = self.end_frame(&[]);
		}

		let _ = self.session.request_exit();
	}
}

/// The space a reference space request maps to: the floor-level space where
/// the headset has one.
fn reference_space<G>(
	system: &System,
	session: &xr::Session<G>,
	space: ReferenceSpace,
) -> Result<xr::Space, Error> {
	let available = session.enumerate_reference_spaces().unwrap_or_default();
	let floor = if system.extensions.ext_local_floor
		&& available.contains(&xr::ReferenceSpaceType::LOCAL_FLOOR_EXT)
	{
		xr::ReferenceSpaceType::LOCAL_FLOOR_EXT
	} else {
		xr::ReferenceSpaceType::STAGE
	};
	let kind = match space {
		ReferenceSpace::LocalFloor if available.contains(&floor) => floor,
		ReferenceSpace::World if available.contains(&xr::ReferenceSpaceType::STAGE) => {
			xr::ReferenceSpaceType::STAGE
		}
		_ => xr::ReferenceSpaceType::LOCAL,
	};

	session
		.create_reference_space(kind, xr::Posef::IDENTITY)
		.map_err(|_| Error::Native)
}

type PassthroughParts = Option<(xr::Passthrough, xr::PassthroughLayerFB)>;

/// For immersive AR: the display's own blend where it sees through, or Meta's
/// passthrough layer under an opaque one.
fn passthrough<G: xr::Graphics>(
	system: &System,
	session: &xr::Session<G>,
	mode: Mode,
) -> Result<(PassthroughParts, xr::EnvironmentBlendMode), Error> {
	if mode != Mode::ImmersiveAr {
		return Ok((None, xr::EnvironmentBlendMode::OPAQUE));
	}

	for blend in [
		xr::EnvironmentBlendMode::ALPHA_BLEND,
		xr::EnvironmentBlendMode::ADDITIVE,
	] {
		if system.blend_modes.contains(&blend) {
			return Ok((None, blend));
		}
	}

	if !system.extensions.fb_passthrough {
		return Err(Error::Unavailable);
	}

	let passthrough = session
		.create_passthrough(xr::PassthroughFlagsFB::IS_RUNNING_AT_CREATION)
		.map_err(|_| Error::Native)?;
	let layer = session
		.create_passthrough_layer(
			&passthrough,
			xr::PassthroughFlagsFB::IS_RUNNING_AT_CREATION,
			xr::PassthroughLayerPurposeFB::RECONSTRUCTION,
		)
		.map_err(|_| Error::Native)?;

	Ok((Some((passthrough, layer)), xr::EnvironmentBlendMode::OPAQUE))
}
