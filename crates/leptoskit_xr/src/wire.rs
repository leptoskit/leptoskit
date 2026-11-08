//! The binary layout every message uses: little-endian integers and floats,
//! counts before lists, and a strict reader that rejects non-finite numbers,
//! oversized lists, unknown enum values, and trailing bytes.

use leptoskit_bridge::Error;

use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Ray;
use crate::math::Vec3;
use crate::types::*;

/// Appends little-endian values to a buffer.
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
	/// An empty writer.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends a `u32`.
	pub fn u32(&mut self, value: u32) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a `u64`.
	pub fn u64(&mut self, value: u64) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends an `f32`.
	pub fn f32(&mut self, value: f32) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends an `f64`.
	pub fn f64(&mut self, value: f64) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a list's length.
	pub fn count(&mut self, count: usize) {
		self.u32(count as u32);
	}

	/// Appends a byte string, length first.
	pub fn bytes(&mut self, bytes: &[u8]) {
		self.count(bytes.len());
		self.0.extend_from_slice(bytes);
	}

	/// The bytes written.
	#[must_use]
	pub fn finish(self) -> Vec<u8> {
		self.0
	}
}

/// Reads little-endian values, failing with `InvalidArgument` on anything
/// malformed.
#[derive(Debug)]
pub struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
	/// A reader over `bytes`.
	#[must_use]
	pub fn new(bytes: &'a [u8]) -> Self {
		Self(bytes)
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		let (head, rest) = self
			.0
			.split_first_chunk::<N>()
			.ok_or(Error::InvalidArgument)?;
		self.0 = rest;
		Ok(*head)
	}

	/// Reads a `u32`.
	pub fn u32(&mut self) -> Result<u32, Error> {
		self.take().map(u32::from_le_bytes)
	}

	/// Reads a `u64`.
	pub fn u64(&mut self) -> Result<u64, Error> {
		self.take().map(u64::from_le_bytes)
	}

	/// Reads a finite `f32`.
	pub fn f32(&mut self) -> Result<f32, Error> {
		let value = f32::from_le_bytes(self.take()?);
		value
			.is_finite()
			.then_some(value)
			.ok_or(Error::InvalidArgument)
	}

	/// Reads a finite `f64`.
	pub fn f64(&mut self) -> Result<f64, Error> {
		let value = f64::from_le_bytes(self.take()?);
		value
			.is_finite()
			.then_some(value)
			.ok_or(Error::InvalidArgument)
	}

	/// Reads a boolean written as a `u32` of 0 or 1.
	pub fn bool(&mut self) -> Result<bool, Error> {
		match self.u32()? {
			0 => Ok(false),
			1 => Ok(true),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Reads a list's length, rejecting one above `max`.
	pub fn count(&mut self, max: usize) -> Result<usize, Error> {
		let count = self.u32()? as usize;
		(count <= max)
			.then_some(count)
			.ok_or(Error::InvalidArgument)
	}

	/// Reads a byte string of at most `max` bytes, length first.
	pub fn bytes(&mut self, max: usize) -> Result<&'a [u8], Error> {
		let len = self.count(max)?;
		let (head, rest) = self.0.split_at_checked(len).ok_or(Error::InvalidArgument)?;
		self.0 = rest;
		Ok(head)
	}

	/// Fails unless every byte was read.
	pub fn finish(self) -> Result<(), Error> {
		self.0
			.is_empty()
			.then_some(())
			.ok_or(Error::InvalidArgument)
	}
}

/// A value with a binary form.
pub trait Wire: Sized {
	/// Appends the value.
	fn write(&self, writer: &mut Writer);
	/// Reads one value.
	fn read(reader: &mut Reader<'_>) -> Result<Self, Error>;
}

/// Encodes one value as a complete message.
#[must_use]
pub fn encode<T: Wire>(value: &T) -> Vec<u8> {
	let mut writer = Writer::new();
	value.write(&mut writer);
	writer.finish()
}

/// Decodes a complete message holding exactly one value.
pub fn decode<T: Wire>(bytes: &[u8]) -> Result<T, Error> {
	let mut reader = Reader::new(bytes);
	let value = T::read(&mut reader)?;
	reader.finish()?;
	Ok(value)
}

fn option<'a, T>(
	reader: &mut Reader<'a>,
	read: impl FnOnce(&mut Reader<'a>) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
	if reader.bool()? {
		read(reader).map(Some)
	} else {
		Ok(None)
	}
}

fn write_option<T>(writer: &mut Writer, value: Option<&T>, write: impl FnOnce(&mut Writer, &T)) {
	writer.u32(u32::from(value.is_some()));

	if let Some(value) = value {
		write(writer, value);
	}
}

fn list<T: Wire>(reader: &mut Reader<'_>, max: usize) -> Result<Vec<T>, Error> {
	let count = reader.count(max)?;
	(0..count).map(|_| T::read(reader)).collect()
}

fn write_list<T: Wire>(writer: &mut Writer, values: &[T], max: usize) {
	let values = &values[..values.len().min(max)];
	writer.count(values.len());

	for value in values {
		value.write(writer);
	}
}

impl Wire for Vec3 {
	fn write(&self, writer: &mut Writer) {
		writer.f32(self.x);
		writer.f32(self.y);
		writer.f32(self.z);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self::new(reader.f32()?, reader.f32()?, reader.f32()?))
	}
}

impl Wire for Quat {
	fn write(&self, writer: &mut Writer) {
		writer.f32(self.x);
		writer.f32(self.y);
		writer.f32(self.z);
		writer.f32(self.w);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Self::new(reader.f32()?, reader.f32()?, reader.f32()?, reader.f32()?)
			.normalized()
			.ok_or(Error::InvalidArgument)
	}
}

impl Wire for Pose {
	fn write(&self, writer: &mut Writer) {
		self.position.write(writer);
		self.orientation.write(writer);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self::new(Vec3::read(reader)?, Quat::read(reader)?))
	}
}

impl Wire for Ray {
	fn write(&self, writer: &mut Writer) {
		self.origin.write(writer);
		self.direction.write(writer);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Self::new(Vec3::read(reader)?, Vec3::read(reader)?).ok_or(Error::InvalidArgument)
	}
}

impl Wire for Projection {
	fn write(&self, writer: &mut Writer) {
		match self {
			Self::Fov(fov) => {
				writer.u32(0);
				for angle in [fov.left, fov.right, fov.up, fov.down] {
					writer.f32(angle);
				}
			}
			Self::Matrix(matrix) => {
				writer.u32(1);
				for value in matrix {
					writer.f32(*value);
				}
			}
		}
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		match reader.u32()? {
			0 => {
				Ok(Self::Fov(Fov {
					left: reader.f32()?,
					right: reader.f32()?,
					up: reader.f32()?,
					down: reader.f32()?,
				}))
			}
			1 => {
				let mut matrix = [0.0; 16];
				for value in &mut matrix {
					*value = reader.f32()?;
				}

				Ok(Self::Matrix(matrix))
			}
			_ => Err(Error::InvalidArgument),
		}
	}
}

impl Wire for Features {
	fn write(&self, writer: &mut Writer) {
		writer.u32(self.bits());
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let bits = reader.u32()?;
		let features = Self::from_bits(bits);
		(features.bits() == bits)
			.then_some(features)
			.ok_or(Error::InvalidArgument)
	}
}

impl Wire for Mode {
	fn write(&self, writer: &mut Writer) {
		writer.u32(*self as u32);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Self::from_u32(reader.u32()?).ok_or(Error::InvalidArgument)
	}
}

/// Marks a mode as supported in the capability words.
const SUPPORTED: u32 = 1 << 31;

impl Wire for Capabilities {
	fn write(&self, writer: &mut Writer) {
		for mode in Mode::ALL {
			writer.u32(self.features(mode).map_or(0, |f| f.bits() | SUPPORTED));
		}
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let mut capabilities = Self::NONE;

		for mode in Mode::ALL {
			let word = reader.u32()?;

			if word & SUPPORTED != 0 {
				let features = Features::from_bits(word & !SUPPORTED);

				if features.bits() != word & !SUPPORTED {
					return Err(Error::InvalidArgument);
				}

				capabilities = capabilities.with(mode, features);
			} else if word != 0 {
				return Err(Error::InvalidArgument);
			}
		}

		Ok(capabilities)
	}
}

impl Wire for Availability {
	fn write(&self, writer: &mut Writer) {
		writer.u32(*self as u32);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Self::from_u32(reader.u32()?).ok_or(Error::InvalidArgument)
	}
}

impl Wire for SessionConfig {
	fn write(&self, writer: &mut Writer) {
		self.mode.write(writer);
		self.required.write(writer);
		self.optional.write(writer);
		writer.u32(self.reference_space as u32);
		writer.u32(self.frame_rate);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let config = Self {
			mode: Mode::read(reader)?,
			required: Features::read(reader)?,
			optional: Features::read(reader)?,
			reference_space: ReferenceSpace::from_u32(reader.u32()?)
				.ok_or(Error::InvalidArgument)?,
			frame_rate: reader.u32()?,
		};

		Self::FRAME_RATES
			.contains(&config.frame_rate)
			.then_some(config)
			.ok_or(Error::InvalidArgument)
	}
}

impl Wire for SessionInfo {
	fn write(&self, writer: &mut Writer) {
		self.mode.write(writer);
		self.features.write(writer);
		writer.u32(self.reference_space as u32);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			mode: Mode::read(reader)?,
			features: Features::read(reader)?,
			reference_space: ReferenceSpace::from_u32(reader.u32()?)
				.ok_or(Error::InvalidArgument)?,
		})
	}
}

impl Wire for TrackingState {
	fn write(&self, writer: &mut Writer) {
		let (state, reason) = self.to_wire();
		writer.u32(state);
		writer.u32(reason);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Self::from_wire(reader.u32()?, reader.u32()?).ok_or(Error::InvalidArgument)
	}
}

fn plane_orientation(value: u32) -> Result<PlaneOrientation, Error> {
	[
		PlaneOrientation::HorizontalUp,
		PlaneOrientation::HorizontalDown,
		PlaneOrientation::Vertical,
	]
	.get(value as usize)
	.copied()
	.ok_or(Error::InvalidArgument)
}

fn plane_label(value: u32) -> Result<PlaneLabel, Error> {
	[
		PlaneLabel::Unknown,
		PlaneLabel::Floor,
		PlaneLabel::Wall,
		PlaneLabel::Ceiling,
		PlaneLabel::Table,
		PlaneLabel::Seat,
		PlaneLabel::Door,
		PlaneLabel::Window,
	]
	.get(value as usize)
	.copied()
	.ok_or(Error::InvalidArgument)
}

impl Wire for Plane {
	fn write(&self, writer: &mut Writer) {
		writer.u64(self.id);
		writer.u32(self.orientation as u32);
		writer.u32(self.label as u32);
		self.pose.write(writer);
		writer.f32(self.extent[0]);
		writer.f32(self.extent[1]);
		let polygon = &self.polygon[..self.polygon.len().min(Self::MAX_POLYGON)];
		writer.count(polygon.len());

		for point in polygon {
			writer.f32(point[0]);
			writer.f32(point[1]);
		}
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let id = reader.u64()?;
		let orientation = plane_orientation(reader.u32()?)?;
		let label = plane_label(reader.u32()?)?;
		let pose = Pose::read(reader)?;
		let extent = [reader.f32()?, reader.f32()?];
		let count = reader.count(Self::MAX_POLYGON)?;
		let polygon = (0..count)
			.map(|_| Ok([reader.f32()?, reader.f32()?]))
			.collect::<Result<_, Error>>()?;

		Ok(Self {
			id,
			orientation,
			label,
			pose,
			extent,
			polygon,
		})
	}
}

impl Wire for LightEstimate {
	fn write(&self, writer: &mut Writer) {
		writer.f32(self.intensity);
		writer.f32(self.color_temperature);
		write_option(writer, self.main_light_direction.as_ref(), |w, v| {
			v.write(w);
		});
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			intensity: reader.f32()?,
			color_temperature: reader.f32()?,
			main_light_direction: option(reader, Vec3::read)?,
		})
	}
}

impl Wire for Anchor {
	fn write(&self, writer: &mut Writer) {
		writer.u64(self.id);
		self.pose.write(writer);
		self.tracking.write(writer);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			id: reader.u64()?,
			pose: Pose::read(reader)?,
			tracking: TrackingState::read(reader)?,
		})
	}
}

impl Wire for Button {
	fn write(&self, writer: &mut Writer) {
		writer.u32(u32::from(self.pressed) | (u32::from(self.touched) << 1));
		writer.f32(self.value);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let flags = reader.u32()?;

		if flags > 0b11 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			pressed: flags & 1 != 0,
			touched: flags & 0b10 != 0,
			value: reader.f32()?,
		})
	}
}

impl Wire for Joint {
	fn write(&self, writer: &mut Writer) {
		self.pose.write(writer);
		writer.f32(self.radius);
		writer.u32(u32::from(self.tracked));
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			pose: Pose::read(reader)?,
			radius: reader.f32()?,
			tracked: reader.bool()?,
		})
	}
}

impl Wire for f32 {
	fn write(&self, writer: &mut Writer) {
		writer.f32(*self);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		reader.f32()
	}
}

fn handedness(value: u32) -> Result<Handedness, Error> {
	[Handedness::None, Handedness::Left, Handedness::Right]
		.get(value as usize)
		.copied()
		.ok_or(Error::InvalidArgument)
}

fn input_kind(value: u32) -> Result<InputKind, Error> {
	[
		InputKind::Controller,
		InputKind::Hand,
		InputKind::Screen,
		InputKind::Gaze,
	]
	.get(value as usize)
	.copied()
	.ok_or(Error::InvalidArgument)
}

fn read_hand(reader: &mut Reader<'_>) -> Result<Box<[Joint; HAND_JOINTS]>, Error> {
	let mut joints = Box::new([Joint::default(); HAND_JOINTS]);

	for joint in &mut *joints {
		*joint = Joint::read(reader)?;
	}

	Ok(joints)
}

impl Wire for InputSource {
	fn write(&self, writer: &mut Writer) {
		writer.u32(self.id);
		writer.u32(self.handedness as u32);
		writer.u32(self.kind as u32);
		self.target_ray.write(writer);
		write_option(writer, self.grip.as_ref(), |w, v| v.write(w));
		write_list(writer, &self.buttons, Self::MAX_BUTTONS);
		write_list(writer, &self.axes, Self::MAX_AXES);
		write_option(writer, self.hand.as_ref(), |w, joints| {
			for joint in &**joints {
				joint.write(w);
			}
		});
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			id: reader.u32()?,
			handedness: handedness(reader.u32()?)?,
			kind: input_kind(reader.u32()?)?,
			target_ray: Pose::read(reader)?,
			grip: option(reader, Pose::read)?,
			buttons: list(reader, Self::MAX_BUTTONS)?,
			axes: list(reader, Self::MAX_AXES)?,
			hand: option(reader, read_hand)?,
		})
	}
}

impl Wire for View {
	fn write(&self, writer: &mut Writer) {
		self.pose.write(writer);
		self.projection.write(writer);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			pose: Pose::read(reader)?,
			projection: Projection::read(reader)?,
		})
	}
}

impl Wire for Frame {
	fn write(&self, writer: &mut Writer) {
		writer.u64(self.sequence);
		writer.f64(self.time);
		self.tracking.write(writer);
		self.viewer.write(writer);
		write_list(writer, &self.views, Self::MAX_VIEWS);
		writer.u64(self.planes_revision);
		write_list(writer, &self.planes, Self::MAX_PLANES);
		write_list(writer, &self.anchors, Self::MAX_ANCHORS);

		write_option(writer, self.light.as_ref(), |w, v| v.write(w));
		write_list(writer, &self.inputs, Self::MAX_INPUTS);
		writer.u32(u32::from(self.depth));
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			sequence: reader.u64()?,
			time: reader.f64()?,
			tracking: TrackingState::read(reader)?,
			viewer: Pose::read(reader)?,
			views: list(reader, Self::MAX_VIEWS)?,
			planes_revision: reader.u64()?,
			planes: list(reader, Self::MAX_PLANES)?,
			anchors: list(reader, Self::MAX_ANCHORS)?,
			light: option(reader, LightEstimate::read)?,
			inputs: list(reader, Self::MAX_INPUTS)?,
			depth: reader.bool()?,
		})
	}
}

impl Wire for HitTestRequest {
	fn write(&self, writer: &mut Writer) {
		match self.origin {
			HitOrigin::Ray(ray) => {
				writer.u32(0);
				ray.write(writer);
			}
			HitOrigin::Screen { x, y } => {
				writer.u32(1);
				writer.f32(x);
				writer.f32(y);
			}
		}

		writer.u32(self.targets.bits());
		writer.u32(self.max_results);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let origin = match reader.u32()? {
			0 => HitOrigin::Ray(Ray::read(reader)?),
			1 => {
				let (x, y) = (reader.f32()?, reader.f32()?);
				if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
					return Err(Error::InvalidArgument);
				}

				HitOrigin::Screen { x, y }
			}
			_ => return Err(Error::InvalidArgument),
		};

		let bits = reader.u32()?;
		let targets = HitTargets::from_bits(bits);
		let max_results = reader.u32()?;

		if targets.bits() != bits || bits == 0 || !(1..=Self::MAX_RESULTS).contains(&max_results) {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			origin,
			targets,
			max_results,
		})
	}
}

fn hit_kind(value: u32) -> Result<HitKind, Error> {
	[
		HitKind::Plane,
		HitKind::InfinitePlane,
		HitKind::Depth,
		HitKind::Estimated,
	]
	.get(value as usize)
	.copied()
	.ok_or(Error::InvalidArgument)
}

impl Wire for Hit {
	fn write(&self, writer: &mut Writer) {
		self.pose.write(writer);
		writer.f32(self.distance);
		writer.u32(self.kind as u32);
		write_option(writer, self.plane.as_ref(), |w, id| w.u64(*id));
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			pose: Pose::read(reader)?,
			distance: reader.f32()?,
			kind: hit_kind(reader.u32()?)?,
			plane: option(reader, Reader::u64)?,
		})
	}
}

impl Wire for Vec<Hit> {
	fn write(&self, writer: &mut Writer) {
		write_list(writer, self, HitTestRequest::MAX_RESULTS as usize);
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		list(reader, HitTestRequest::MAX_RESULTS as usize)
	}
}
