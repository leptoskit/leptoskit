//! The data an app sees: what a device can do, how a session is set up, and
//! what each frame reports.

use crate::math::Pose;
use crate::math::Projection;
use crate::math::Ray;
use crate::math::Vec3;

/// How an app experiences the real world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum Mode {
	/// Look around by turning the device: orientation only, from the
	/// rotation sensors, with no camera and no position. Every phone can do
	/// this; it is the fallback when AR is missing.
	Orientation = 0,
	/// Augmented reality through a phone's camera: position and rotation,
	/// the camera picture behind the scene, planes, and on some devices depth.
	PhoneAr = 1,
	/// A headset showing a virtual world: two views, head tracking,
	/// controllers and hands.
	ImmersiveVr = 2,
	/// A headset showing the room through its cameras (passthrough) with the
	/// scene on top.
	ImmersiveAr = 3,
}

impl Mode {
	/// Every mode, in the order an app usually prefers them last to first.
	pub const ALL: [Self; 4] = [
		Self::Orientation,
		Self::PhoneAr,
		Self::ImmersiveVr,
		Self::ImmersiveAr,
	];

	/// The mode with this wire value.
	#[must_use]
	pub fn from_u32(value: u32) -> Option<Self> {
		Self::ALL.get(value as usize).copied()
	}

	/// Whether the mode tracks position as well as rotation.
	#[must_use]
	pub fn tracks_position(self) -> bool {
		!matches!(self, Self::Orientation)
	}
}

/// A set of optional abilities a session can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Features(u32);

impl Features {
	/// Every feature.
	pub const ALL: Self = Self((1 << 10) - 1);
	/// Points pinned to the real world that stay put as tracking improves.
	pub const ANCHORS: Self = Self(1 << 5);
	/// The camera picture, drawn behind the scene by the host's renderer.
	pub const CAMERA_IMAGE: Self = Self(1 << 8);
	/// Hand-held controllers.
	pub const CONTROLLERS: Self = Self(1 << 6);
	/// A depth picture of the real world, for occlusion and room mapping.
	pub const DEPTH: Self = Self(1 << 2);
	/// Tracked hands with joints.
	pub const HANDS: Self = Self(1 << 7);
	/// Rays cast against the real world.
	pub const HIT_TEST: Self = Self(1 << 4);
	/// The real light's brightness and colour.
	pub const LIGHT_ESTIMATION: Self = Self(1 << 3);
	/// No features.
	pub const NONE: Self = Self(0);
	/// A headset's cameras show the room behind the scene.
	pub const PASSTHROUGH: Self = Self(1 << 9);
	/// Every plane orientation.
	pub const PLANES: Self = Self(Self::PLANES_HORIZONTAL.0 | Self::PLANES_VERTICAL.0);
	/// Floors, tables and other horizontal surfaces.
	pub const PLANES_HORIZONTAL: Self = Self(1);
	/// Walls and other vertical surfaces.
	pub const PLANES_VERTICAL: Self = Self(1 << 1);

	/// The set with these bits, ignoring bits no feature uses.
	#[must_use]
	pub const fn from_bits(bits: u32) -> Self {
		Self(bits & Self::ALL.0)
	}

	/// The raw bits.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Whether every feature of `other` is in this set.
	#[must_use]
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	/// Whether the sets share a feature.
	#[must_use]
	pub const fn intersects(self, other: Self) -> bool {
		self.0 & other.0 != 0
	}

	/// Both sets together.
	#[must_use]
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	/// The features in both sets.
	#[must_use]
	pub const fn intersection(self, other: Self) -> Self {
		Self(self.0 & other.0)
	}

	/// The features of this set missing from `other`.
	#[must_use]
	pub const fn difference(self, other: Self) -> Self {
		Self(self.0 & !other.0)
	}

	/// Whether the set is empty.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}
}

impl std::ops::BitOr for Features {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		self.union(other)
	}
}

/// What this device can do, mode by mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
	modes: [Option<Features>; 4],
}

impl Capabilities {
	/// A device that can do nothing.
	pub const NONE: Self = Self { modes: [None; 4] };

	/// Adds a supported mode with the features it offers.
	#[must_use]
	pub fn with(mut self, mode: Mode, features: Features) -> Self {
		self.modes[mode as usize] = Some(features);
		self
	}

	/// The features `mode` offers, or `None` when the device can't run it.
	#[must_use]
	pub fn features(&self, mode: Mode) -> Option<Features> {
		self.modes[mode as usize]
	}

	/// Whether the device can run `mode`.
	#[must_use]
	pub fn supports(&self, mode: Mode) -> bool {
		self.features(mode).is_some()
	}

	/// The richest supported mode from `preferred`, first match winning.
	#[must_use]
	pub fn first_of(&self, preferred: &[Mode]) -> Option<Mode> {
		preferred.iter().copied().find(|mode| self.supports(*mode))
	}

	/// Every supported mode.
	pub fn modes(&self) -> impl Iterator<Item = (Mode, Features)> + '_ {
		Mode::ALL
			.iter()
			.filter_map(|mode| Some((*mode, self.features(*mode)?)))
	}
}

/// Whether a mode can start right now, and if not, what stands in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// It can start.
	Ready = 0,
	/// The user hasn't allowed the camera yet. Ask with the `permissions`
	/// plugin, then check again.
	NeedsPermission = 1,
	/// The platform's AR service is missing or out of date (Google Play
	/// Services for AR). Asking with `install = true` opens the store.
	NeedsInstall = 2,
	/// The platform is still finding out; check again shortly.
	Checking = 3,
	/// This device can never run the mode.
	Unsupported = 4,
}

impl Availability {
	/// The value with this wire code.
	#[must_use]
	pub fn from_u32(value: u32) -> Option<Self> {
		[
			Self::Ready,
			Self::NeedsPermission,
			Self::NeedsInstall,
			Self::Checking,
			Self::Unsupported,
		]
		.get(value as usize)
		.copied()
	}
}

/// Where a session's origin is, and so what world coordinates mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum ReferenceSpace {
	/// The origin is where the device was when the session started, with +Y
	/// against gravity. Good for seated and handheld use.
	Local = 0,
	/// Like `Local`, but y = 0 is the floor. Headsets know the floor at once;
	/// phones place it when they first find a floor plane, and report y
	/// relative to the start until then.
	LocalFloor = 1,
	/// A world-scale space for walking around a room or further: the origin
	/// stays fixed in the room, and the platform corrects drift as it
	/// recognises places it has seen.
	#[default]
	World = 2,
}

impl ReferenceSpace {
	/// The value with this wire code.
	#[must_use]
	pub fn from_u32(value: u32) -> Option<Self> {
		[Self::Local, Self::LocalFloor, Self::World]
			.get(value as usize)
			.copied()
	}
}

/// How the guest wants a session set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
	/// The mode to run.
	pub mode: Mode,
	/// Features the session can't run without. Starting fails with
	/// `Unavailable` when one is missing.
	pub required: Features,
	/// Features to turn on when the device has them.
	pub optional: Features,
	/// What world coordinates mean.
	pub reference_space: ReferenceSpace,
	/// The most frames per second the guest receives, 1 to 120. Tracking and
	/// rendering on the host run at the display's own rate regardless; the
	/// guest sees the newest frame at most this often.
	pub frame_rate: u32,
}

impl SessionConfig {
	/// The slowest and fastest guest frame rates.
	pub const FRAME_RATES: std::ops::RangeInclusive<u32> = 1..=120;

	/// A session in `mode` with no features, in world space, at 30 frames a
	/// second.
	#[must_use]
	pub fn new(mode: Mode) -> Self {
		Self {
			mode,
			required: Features::NONE,
			optional: Features::NONE,
			reference_space: ReferenceSpace::World,
			frame_rate: 30,
		}
	}

	/// Requires `features`.
	#[must_use]
	pub fn require(mut self, features: Features) -> Self {
		self.required = self.required.union(features);
		self
	}

	/// Asks for `features` where the device has them.
	#[must_use]
	pub fn prefer(mut self, features: Features) -> Self {
		self.optional = self.optional.union(features);
		self
	}

	/// Sets the reference space.
	#[must_use]
	pub fn reference_space(mut self, space: ReferenceSpace) -> Self {
		self.reference_space = space;
		self
	}

	/// Sets the guest's frame rate.
	#[must_use]
	pub fn frame_rate(mut self, frames_per_second: u32) -> Self {
		self.frame_rate = frames_per_second;
		self
	}

	/// The features this session runs with on a device offering `offered`,
	/// or `None` when a required one is missing.
	#[must_use]
	pub fn negotiate(&self, offered: Features) -> Option<Features> {
		offered
			.contains(self.required)
			.then(|| self.required.union(self.optional.intersection(offered)))
	}
}

/// The session the host started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionInfo {
	/// The mode it runs.
	pub mode: Mode,
	/// The features it has: every required one and the preferred ones the
	/// device offers.
	pub features: Features,
	/// What world coordinates mean.
	pub reference_space: ReferenceSpace,
}

/// Why tracking is limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum LimitedReason {
	/// The session has just started.
	Initializing = 0,
	/// The device is moving too fast.
	ExcessiveMotion = 1,
	/// The view has too little texture to track: a blank wall, darkness.
	InsufficientFeatures = 2,
	/// Too dark to see.
	InsufficientLight = 3,
	/// Tracking was interrupted and the device is finding its place again.
	Relocalizing = 4,
	/// The camera is in use elsewhere or was interrupted.
	CameraUnavailable = 5,
	/// No reason was given.
	Unknown = 6,
}

/// How well the device knows where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TrackingState {
	/// Not tracking: poses are stale or absent.
	#[default]
	NotAvailable,
	/// Tracking, but poses may be wrong or drift.
	Limited(LimitedReason),
	/// Tracking well.
	Normal,
}

impl TrackingState {
	/// The wire pair: state, then reason.
	#[must_use]
	pub fn to_wire(self) -> (u32, u32) {
		match self {
			Self::NotAvailable => (0, 0),
			Self::Limited(reason) => (1, reason as u32),
			Self::Normal => (2, 0),
		}
	}

	/// The state from its wire pair.
	#[must_use]
	pub fn from_wire(state: u32, reason: u32) -> Option<Self> {
		const REASONS: [LimitedReason; 7] = [
			LimitedReason::Initializing,
			LimitedReason::ExcessiveMotion,
			LimitedReason::InsufficientFeatures,
			LimitedReason::InsufficientLight,
			LimitedReason::Relocalizing,
			LimitedReason::CameraUnavailable,
			LimitedReason::Unknown,
		];

		match (state, reason) {
			(0, 0) => Some(Self::NotAvailable),
			(1, reason) => REASONS.get(reason as usize).copied().map(Self::Limited),
			(2, 0) => Some(Self::Normal),
			_ => None,
		}
	}
}

/// Which way a plane faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PlaneOrientation {
	/// Facing up: a floor or a table top.
	HorizontalUp = 0,
	/// Facing down: a ceiling.
	HorizontalDown = 1,
	/// Upright: a wall or a door.
	Vertical = 2,
}

/// What the platform thinks a plane is. Only ARKit (on devices with a neural
/// engine) and headsets with a scene model label planes; elsewhere every plane
/// is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PlaneLabel {
	/// Not labelled.
	Unknown = 0,
	/// A floor.
	Floor = 1,
	/// A wall.
	Wall = 2,
	/// A ceiling.
	Ceiling = 3,
	/// A table.
	Table = 4,
	/// A seat.
	Seat = 5,
	/// A door.
	Door = 6,
	/// A window.
	Window = 7,
}

/// A flat real-world surface the device found.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
	/// Stable while the plane exists. A plane merged into another disappears
	/// and the survivor keeps its own id.
	pub id: u64,
	/// Which way it faces.
	pub orientation: PlaneOrientation,
	/// What it is, where the platform says.
	pub label: PlaneLabel,
	/// The plane's centre. Its local +Y is the surface normal; local x and z
	/// lie in the surface.
	pub pose: Pose,
	/// The size of its bounding rectangle along local x and z, in metres.
	pub extent: [f32; 2],
	/// The boundary as points in the plane's local x–z, in order. Empty when
	/// the platform reports only the rectangle.
	pub polygon: Vec<[f32; 2]>,
}

impl Plane {
	/// The most boundary points a plane carries; longer outlines are thinned.
	pub const MAX_POLYGON: usize = 64;

	/// Whether a point in the plane's local x–z is on the plane: inside its
	/// outline, or inside its rectangle when it has none.
	#[must_use]
	pub fn contains_local(&self, x: f32, z: f32) -> bool {
		if self.polygon.len() < 3 {
			return x.abs() <= self.extent[0] * 0.5 && z.abs() <= self.extent[1] * 0.5;
		}

		let mut inside = false;
		let mut previous = self.polygon[self.polygon.len() - 1];

		for &point in &self.polygon {
			if (point[1] > z) != (previous[1] > z)
				&& x < (previous[0] - point[0]) * (z - point[1]) / (previous[1] - point[1])
					+ point[0]
			{
				inside = !inside;
			}

			previous = point;
		}

		inside
	}
}

/// Thins a boundary to at most [`Plane::MAX_POLYGON`] points by keeping
/// evenly spaced ones, so a plane's message stays bounded.
#[must_use]
pub fn thin_polygon(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
	if points.len() <= Plane::MAX_POLYGON {
		return points.to_vec();
	}

	(0..Plane::MAX_POLYGON)
		.map(|index| points[index * points.len() / Plane::MAX_POLYGON])
		.collect()
}

/// The real light around the device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightEstimate {
	/// Ambient brightness, where 1 is a well-lit room. ARKit's 1000 lumens
	/// and ARCore's pixel intensity of 0.5 both map to 1.
	pub intensity: f32,
	/// The light's colour temperature in kelvin; 6500 is neutral daylight.
	pub color_temperature: f32,
	/// The direction the strongest light comes from, towards the light, where
	/// the platform estimates one (ARCore's environmental HDR).
	pub main_light_direction: Option<Vec3>,
}

/// A point pinned to the real world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
	/// The id `add_anchor` returned.
	pub id: u64,
	/// Where it is now; it moves as the platform corrects its map.
	pub pose: Pose,
	/// Whether the platform still knows where it is.
	pub tracking: TrackingState,
}

/// Which hand an input belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Handedness {
	/// Neither: a touch on a phone screen, or gaze.
	None = 0,
	/// The left hand.
	Left = 1,
	/// The right hand.
	Right = 2,
}

/// What kind of input a source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum InputKind {
	/// A hand-held controller.
	Controller = 0,
	/// A tracked hand.
	Hand = 1,
	/// A touch on a phone's screen, which aims along the camera ray.
	Screen = 2,
	/// The viewer's gaze.
	Gaze = 3,
}

/// One button or trigger on a controller, or a pinch on a hand.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Button {
	/// Held down.
	pub pressed: bool,
	/// A finger rests on it.
	pub touched: bool,
	/// How far it is pressed, 0 to 1.
	pub value: f32,
}

/// The joints of a tracked hand, in OpenXR's and WebXR's order: the palm and
/// wrist, then the thumb's four, then four or five for each finger.
pub const HAND_JOINTS: usize = 26;

/// One joint of a tracked hand.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Joint {
	/// Where the joint is.
	pub pose: Pose,
	/// Half the finger's thickness there, in metres.
	pub radius: f32,
	/// Whether the platform knows this joint right now.
	pub tracked: bool,
}

/// A controller, hand, touch, or gaze.
#[derive(Debug, Clone, PartialEq)]
pub struct InputSource {
	/// Stable while the source is present.
	pub id: u32,
	/// Which hand.
	pub handedness: Handedness,
	/// What it is.
	pub kind: InputKind,
	/// Where it points: aim along its −Z axis.
	pub target_ray: Pose,
	/// Where it is held, for drawing a controller or a held object.
	pub grip: Option<Pose>,
	/// Buttons, trigger first, then squeeze, then the platform's own order.
	/// At most [`InputSource::MAX_BUTTONS`].
	pub buttons: Vec<Button>,
	/// Thumbstick and touchpad axes, x then y, −1 to 1. At most
	/// [`InputSource::MAX_AXES`].
	pub axes: Vec<f32>,
	/// The joints of a tracked hand.
	pub hand: Option<Box<[Joint; HAND_JOINTS]>>,
}

impl InputSource {
	/// The most axes a source carries.
	pub const MAX_AXES: usize = 4;
	/// The most buttons a source carries.
	pub const MAX_BUTTONS: usize = 8;
}

/// One view of the world: the phone's screen, or one eye of a headset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
	/// Where the view's camera or eye is, looking down its −Z axis.
	pub pose: Pose,
	/// How it projects the world.
	pub projection: Projection,
}

impl View {
	/// The ray through a point on this view, where `(0, 0)` is the top-left
	/// corner and `(1, 1)` the bottom-right.
	#[must_use]
	pub fn ray(&self, x: f32, y: f32) -> Option<Ray> {
		let direction = self.pose.transform_vector(self.projection.direction(x, y));

		Ray::new(self.pose.position, direction)
	}
}

/// What a hit test casts against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HitTargets(u32);

impl HitTargets {
	/// Every target.
	pub const ALL: Self = Self(0b1111);
	/// The depth picture: any real surface, flat or not.
	pub const DEPTH: Self = Self(1 << 2);
	/// Surfaces the platform guesses before it has found a plane (ARKit's
	/// estimated planes, ARCore's instant placement).
	pub const ESTIMATED: Self = Self(1 << 3);
	/// Planes extended without limit, to place something past a plane's
	/// detected edge.
	pub const INFINITE_PLANES: Self = Self(1 << 1);
	/// Planes found so far, inside their outlines.
	pub const PLANES: Self = Self(1);

	/// The set with these bits, ignoring unused ones.
	#[must_use]
	pub const fn from_bits(bits: u32) -> Self {
		Self(bits & Self::ALL.0)
	}

	/// The raw bits.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Whether every target of `other` is in this set.
	#[must_use]
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

impl std::ops::BitOr for HitTargets {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

/// Where a hit test's ray comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitOrigin {
	/// A ray in world space, for example a controller's target ray.
	Ray(Ray),
	/// A point on the first view, where `(0, 0)` is its top-left corner and
	/// `(1, 1)` its bottom-right: where the user tapped on a phone.
	Screen {
		/// Across, 0 to 1.
		x: f32,
		/// Down, 0 to 1.
		y: f32,
	},
}

/// A ray to cast against the real world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitTestRequest {
	/// Where the ray comes from.
	pub origin: HitOrigin,
	/// What it can hit.
	pub targets: HitTargets,
	/// The most hits to return, nearest first, 1 to
	/// [`HitTestRequest::MAX_RESULTS`].
	pub max_results: u32,
}

impl HitTestRequest {
	/// The most hits one test returns.
	pub const MAX_RESULTS: u32 = 16;

	/// A test through a point on the screen against planes and depth,
	/// returning the nearest hit.
	#[must_use]
	pub fn screen(x: f32, y: f32) -> Self {
		Self {
			origin: HitOrigin::Screen { x, y },
			targets: HitTargets::PLANES | HitTargets::DEPTH,
			max_results: 1,
		}
	}

	/// A test along a world-space ray against planes and depth, returning the
	/// nearest hit.
	#[must_use]
	pub fn ray(ray: Ray) -> Self {
		Self {
			origin: HitOrigin::Ray(ray),
			targets: HitTargets::PLANES | HitTargets::DEPTH,
			max_results: 1,
		}
	}

	/// Sets the targets.
	#[must_use]
	pub fn targets(mut self, targets: HitTargets) -> Self {
		self.targets = targets;
		self
	}

	/// Sets the most hits returned.
	#[must_use]
	pub fn max_results(mut self, count: u32) -> Self {
		self.max_results = count;
		self
	}
}

/// What a hit test hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HitKind {
	/// A detected plane, inside its outline.
	Plane = 0,
	/// A detected plane, extended past its outline.
	InfinitePlane = 1,
	/// The depth picture.
	Depth = 2,
	/// A surface the platform guessed.
	Estimated = 3,
}

/// Where a ray met the real world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
	/// The point, with local +Y along the surface normal: a ready-made pose
	/// for something standing on the surface.
	pub pose: Pose,
	/// How far along the ray, in metres.
	pub distance: f32,
	/// What was hit.
	pub kind: HitKind,
	/// The plane hit, for plane hits.
	pub plane: Option<u64>,
}

/// One frame as the guest receives it: everything but the pictures.
///
/// The camera image and the depth picture stay on the host, where the
/// renderer uses them; the guest learns whether depth exists and asks the host
/// to hit-test against it.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
	/// Increases with every frame the guest receives; frames it skipped
	/// still advance it.
	pub sequence: u64,
	/// When the frame was captured, in seconds on the platform's monotonic
	/// clock. Only differences between frames mean anything.
	pub time: f64,
	/// How well the device knows where it is.
	pub tracking: TrackingState,
	/// Where the viewer is: the phone, or the point between a headset's eyes.
	pub viewer: Pose,
	/// The views a renderer draws: one for a phone, two for a headset.
	pub views: Vec<View>,
	/// Every plane found so far.
	pub planes: Vec<Plane>,
	/// Increases whenever the planes change. A guest that already holds this
	/// revision's planes receives an empty `planes` list and keeps its own.
	pub planes_revision: u64,
	/// Pinned points.
	pub anchors: Vec<Anchor>,
	/// The real light, when estimated.
	pub light: Option<LightEstimate>,
	/// Controllers, hands, touches, and gaze.
	pub inputs: Vec<InputSource>,
	/// Whether the host has a depth picture this frame, so depth hit tests
	/// and occlusion work.
	pub depth: bool,
}

impl Frame {
	/// The most anchors a frame carries.
	pub const MAX_ANCHORS: usize = 64;
	/// The most input sources a frame carries.
	pub const MAX_INPUTS: usize = 8;
	/// The most planes a frame carries; further planes are left out, the
	/// largest first kept.
	pub const MAX_PLANES: usize = 64;
	/// The most views a frame carries.
	pub const MAX_VIEWS: usize = 4;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn negotiation_requires_and_prefers() {
		let config = SessionConfig::new(Mode::PhoneAr)
			.require(Features::PLANES_HORIZONTAL)
			.prefer(Features::DEPTH | Features::LIGHT_ESTIMATION);
		let offered = Features::PLANES | Features::LIGHT_ESTIMATION;
		assert_eq!(
			config.negotiate(offered),
			Some(Features::PLANES_HORIZONTAL | Features::LIGHT_ESTIMATION)
		);
		assert_eq!(config.negotiate(Features::DEPTH), None);
	}

	#[test]
	fn a_polygon_contains_its_inside_only() {
		let plane = Plane {
			id: 1,
			orientation: PlaneOrientation::HorizontalUp,
			label: PlaneLabel::Floor,
			pose: Pose::IDENTITY,
			extent: [2.0, 2.0],
			polygon: vec![[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]],
		};
		assert!(plane.contains_local(0.0, 0.0));
		assert!(!plane.contains_local(0.9, 0.9));
		let rectangle = Plane {
			polygon: Vec::new(),
			..plane
		};
		assert!(rectangle.contains_local(0.9, 0.9));
		assert!(!rectangle.contains_local(1.1, 0.0));
	}

	#[test]
	fn long_outlines_are_thinned_to_the_bound() {
		let points: Vec<[f32; 2]> = (0..1000).map(|i| [i as f32, 0.0]).collect();
		assert_eq!(thin_polygon(&points).len(), Plane::MAX_POLYGON);
		assert_eq!(thin_polygon(&points[..10]).len(), 10);
	}

	#[test]
	fn tracking_states_round_trip() {
		for state in [
			TrackingState::NotAvailable,
			TrackingState::Normal,
			TrackingState::Limited(LimitedReason::Relocalizing),
		] {
			let (code, reason) = state.to_wire();
			assert_eq!(TrackingState::from_wire(code, reason), Some(state));
		}
		assert_eq!(TrackingState::from_wire(1, 99), None);
		assert_eq!(TrackingState::from_wire(0, 1), None);
	}
}
