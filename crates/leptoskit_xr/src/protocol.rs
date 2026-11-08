//! The `xr` plugin's methods and their messages.
//!
//! Every request and reply is one [`Wire`](crate::wire::Wire) value. A host
//! answers a malformed request with `InvalidArgument`, a mode the device
//! can't run with `Unavailable`, a camera the user refused with
//! `PermissionDenied`, and a frame request after the session stopped with
//! `Cancelled`.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

use crate::math::Pose;
use crate::types::Mode;
use crate::wire::Reader;
use crate::wire::Wire;
use crate::wire::Writer;

/// The plugin's registration name.
pub const NAME: &str = "xr";

/// Empty request; replies with [`Capabilities`](crate::Capabilities).
pub const CAPABILITIES: MethodId = MethodId(0);
/// [`Prepare`] request; replies with
/// [`Availability`](crate::Availability).
pub const PREPARE: MethodId = MethodId(1);
/// [`SessionConfig`](crate::SessionConfig) request; replies with
/// [`SessionInfo`](crate::SessionInfo). Fails with `Busy` while a session runs.
pub const START: MethodId = MethodId(2);
/// [`NextFrame`] request; replies with the next [`Frame`](crate::Frame).
pub const NEXT_FRAME: MethodId = MethodId(3);
/// [`SessionConfig`](crate::SessionConfig) request for the running session;
/// replies with [`SessionInfo`](crate::SessionInfo). The mode can't change.
pub const CONFIGURE: MethodId = MethodId(4);
/// [`HitTestRequest`](crate::HitTestRequest); replies with a list of
/// [`Hit`](crate::Hit)s, nearest first.
pub const HIT_TEST: MethodId = MethodId(5);
/// [`AddAnchor`] request; replies with the anchor's `u64` id.
pub const ADD_ANCHOR: MethodId = MethodId(6);
/// A `u64` anchor id; empty reply.
pub const REMOVE_ANCHOR: MethodId = MethodId(7);
/// Empty request; empty reply. Pending frame requests fail with `Cancelled`.
pub const STOP: MethodId = MethodId(8);

/// Every method, in id order.
pub const METHODS: &[MethodId] = &[
	CAPABILITIES,
	PREPARE,
	START,
	NEXT_FRAME,
	CONFIGURE,
	HIT_TEST,
	ADD_ANCHOR,
	REMOVE_ANCHOR,
	STOP,
];

/// Asks whether `mode` can start, optionally fixing what can be fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prepare {
	/// The mode to check.
	pub mode: Mode,
	/// Opens the store to install or update the platform's AR service when
	/// it is missing (Android). Only ask after the user chose AR.
	pub install: bool,
}

impl Wire for Prepare {
	fn write(&self, writer: &mut Writer) {
		self.mode.write(writer);
		writer.u32(u32::from(self.install));
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			mode: Mode::read(reader)?,
			install: reader.bool()?,
		})
	}
}

/// Waits for a frame newer than one the guest already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NextFrame {
	/// The newest frame sequence the guest holds; 0 for none.
	pub after: u64,
	/// The plane revision the guest holds; the reply leaves the planes out
	/// when they haven't changed since. 0 for none.
	pub planes_revision: u64,
}

impl Wire for NextFrame {
	fn write(&self, writer: &mut Writer) {
		writer.u64(self.after);
		writer.u64(self.planes_revision);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			after: reader.u64()?,
			planes_revision: reader.u64()?,
		})
	}
}

/// Pins a point to the real world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddAnchor {
	/// Where, in world space.
	pub pose: Pose,
	/// The plane to attach it to, so it moves with the plane, where the
	/// platform supports that.
	pub plane: Option<u64>,
}

impl Wire for AddAnchor {
	fn write(&self, writer: &mut Writer) {
		self.pose.write(writer);
		writer.u32(u32::from(self.plane.is_some()));
		writer.u64(self.plane.unwrap_or(0));
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let pose = Pose::read(reader)?;
		let has_plane = reader.bool()?;
		let plane = reader.u64()?;

		if !has_plane && plane != 0 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			pose,
			plane: has_plane.then_some(plane),
		})
	}
}

/// Keeps the planes between frames, since a frame leaves them out when they
/// haven't changed since the revision the guest asked with.
#[derive(Debug, Clone, Default)]
pub struct PlaneCache {
	revision: u64,
	planes: Vec<crate::types::Plane>,
}

impl PlaneCache {
	/// The revision to ask with.
	#[must_use]
	pub fn revision(&self) -> u64 {
		self.revision
	}

	/// Takes a received frame's planes when they are new, and fills them in
	/// from the cache when it left them out.
	pub fn merge(&mut self, frame: &mut crate::types::Frame) {
		if frame.planes_revision == self.revision {
			frame.planes.clone_from(&self.planes);
		} else {
			self.revision = frame.planes_revision;
			self.planes.clone_from(&frame.planes);
		}
	}
}

impl Wire for u64 {
	fn write(&self, writer: &mut Writer) {
		writer.u64(*self);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		reader.u64()
	}
}

/// An empty message.
impl Wire for () {
	fn write(&self, _writer: &mut Writer) {}

	fn read(_reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::Fov;
	use crate::math::Projection;
	use crate::math::Quat;
	use crate::math::Ray;
	use crate::math::Vec3;
	use crate::types::*;
	use crate::wire::decode;
	use crate::wire::encode;

	fn round_trip<T: Wire + PartialEq + std::fmt::Debug>(value: &T) {
		let bytes = encode(value);
		assert_eq!(&decode::<T>(&bytes).unwrap(), value);
		let mut longer = bytes.clone();
		longer.push(0);
		assert_eq!(decode::<T>(&longer), Err(Error::InvalidArgument));
		if !bytes.is_empty() {
			assert_eq!(
				decode::<T>(&bytes[..bytes.len() - 1]),
				Err(Error::InvalidArgument)
			);
		}
	}

	fn sample_frame() -> Frame {
		let pose = Pose::new(
			Vec3::new(0.1, 1.4, -0.3),
			Quat::from_axis_angle(Vec3::Y, 0.4),
		);
		let mut joints = Box::new([Joint::default(); HAND_JOINTS]);
		joints[3].tracked = true;
		joints[3].radius = 0.01;

		Frame {
			sequence: 7,
			time: 12.5,
			tracking: TrackingState::Limited(LimitedReason::ExcessiveMotion),
			viewer: pose,
			views: vec![
				View {
					pose,
					projection: Projection::Fov(Fov::symmetric(1.6, 0.9)),
				},
				View {
					pose,
					projection: Projection::Matrix(Fov::symmetric(1.0, 1.0).projection(0.1, 20.0)),
				},
			],
			planes: vec![Plane {
				id: 3,
				orientation: PlaneOrientation::Vertical,
				label: PlaneLabel::Wall,
				pose,
				extent: [2.0, 1.5],
				polygon: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
			}],
			planes_revision: 4,
			anchors: vec![Anchor {
				id: 9,
				pose,
				tracking: TrackingState::Normal,
			}],
			light: Some(LightEstimate {
				intensity: 0.8,
				color_temperature: 5000.0,
				main_light_direction: Some(Vec3::Y),
			}),
			inputs: vec![InputSource {
				id: 1,
				handedness: Handedness::Left,
				kind: InputKind::Hand,
				target_ray: pose,
				grip: Some(pose),
				buttons: vec![Button {
					pressed: true,
					touched: true,
					value: 1.0,
				}],
				axes: vec![0.5, -0.5],
				hand: Some(joints),
			}],
			depth: true,
		}
	}

	#[test]
	fn every_message_round_trips_and_rejects_extra_or_missing_bytes() {
		round_trip(&sample_frame());
		round_trip(
			&SessionConfig::new(Mode::ImmersiveAr)
				.require(Features::PASSTHROUGH)
				.prefer(Features::HANDS)
				.reference_space(ReferenceSpace::LocalFloor)
				.frame_rate(72),
		);
		round_trip(&SessionInfo {
			mode: Mode::PhoneAr,
			features: Features::PLANES | Features::DEPTH,
			reference_space: ReferenceSpace::World,
		});
		round_trip(
			&Capabilities::NONE
				.with(Mode::Orientation, Features::NONE)
				.with(Mode::PhoneAr, Features::ALL),
		);
		round_trip(&Availability::NeedsInstall);
		round_trip(&Prepare {
			mode: Mode::PhoneAr,
			install: true,
		});
		round_trip(&NextFrame {
			after: 3,
			planes_revision: 2,
		});
		round_trip(&AddAnchor {
			pose: Pose::IDENTITY,
			plane: Some(5),
		});
		round_trip(&HitTestRequest::screen(0.5, 0.25).max_results(4));
		round_trip(&HitTestRequest::ray(
			Ray::new(Vec3::ZERO, Vec3::FORWARD).unwrap(),
		));
		round_trip(&vec![Hit {
			pose: Pose::IDENTITY,
			distance: 1.25,
			kind: HitKind::Depth,
			plane: None,
		}]);
	}

	#[test]
	fn malformed_values_are_refused() {
		let mut config = encode(&SessionConfig::new(Mode::PhoneAr));
		let rate = config.len() - 4;
		config[rate..].copy_from_slice(&0_u32.to_le_bytes());
		assert_eq!(
			decode::<SessionConfig>(&config),
			Err(Error::InvalidArgument)
		);

		let mut nan = encode(&Pose::IDENTITY);
		nan[..4].copy_from_slice(&f32::NAN.to_le_bytes());
		assert_eq!(decode::<Pose>(&nan), Err(Error::InvalidArgument));

		let zero_rotation = [0_u8; 28];
		assert_eq!(decode::<Pose>(&zero_rotation), Err(Error::InvalidArgument));

		let off_screen = HitTestRequest::screen(1.5, 0.5);
		assert_eq!(
			decode::<HitTestRequest>(&encode(&off_screen)),
			Err(Error::InvalidArgument)
		);
		let too_many = HitTestRequest::screen(0.5, 0.5).max_results(99);
		assert_eq!(
			decode::<HitTestRequest>(&encode(&too_many)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode::<Mode>(&9_u32.to_le_bytes()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode::<Features>(&(1_u32 << 20).to_le_bytes()),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn oversized_lists_are_refused_before_allocation() {
		let mut frame = encode(&Frame {
			views: Vec::new(),
			planes: Vec::new(),
			anchors: Vec::new(),
			inputs: Vec::new(),
			light: None,
			..sample_frame()
		});
		// The views count follows sequence, time, tracking, and viewer.
		let views = 8 + 8 + 8 + 28;
		frame[views..views + 4].copy_from_slice(&u32::MAX.to_le_bytes());
		assert_eq!(decode::<Frame>(&frame), Err(Error::InvalidArgument));
	}

	#[test]
	fn writers_never_exceed_the_bounds() {
		let plane = sample_frame().planes[0].clone();
		let frame = Frame {
			planes: vec![plane; Frame::MAX_PLANES + 10],
			..sample_frame()
		};
		let decoded = decode::<Frame>(&encode(&frame)).unwrap();
		assert_eq!(decoded.planes.len(), Frame::MAX_PLANES);
	}
}
