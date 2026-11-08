//! <!-- {=p_mapsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_maps` as a direct dependency and select the `maps`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Maps` rather than resolving the plugin
//! on every call. Create a session, bind it with `Map(session)`, then set the
//! camera and a bounded marker list. iOS and macOS use `MapKit`; Android uses
//! Google Maps (`MapView`); Windows, Linux, HarmonyOS, and the browser return
//! `Unavailable`. User location, polygons, and clustering remain later scope.
//!
//! <!-- {/p_mapsSummary} -->
//!
//! Maps wire v1: create returns a session id; set-camera takes session + pose;
//! set-markers takes session + ≤32 markers; status replies with pose, marker
//! count, and active; release drops the host session. Coordinates are IEEE-754
//! little-endian f64 degrees; zoom is a finite f64 in `[0, 22]`.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "maps";
/// Empty request → session id.
pub const CREATE: MethodId = MethodId(0);
/// [`Camera`] request → empty reply.
pub const SET_CAMERA: MethodId = MethodId(1);
/// [`SetMarkers`] request → empty reply.
pub const SET_MARKERS: MethodId = MethodId(2);
/// Session id → encoded [`Status`].
pub const STATUS: MethodId = MethodId(3);
/// Session id → empty reply; drops the host session.
pub const RELEASE: MethodId = MethodId(4);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[CREATE, SET_CAMERA, SET_MARKERS, STATUS, RELEASE];

/// Wire protocol version for status replies.
const WIRE_VERSION: u32 = 1;
/// Hard cap on markers per session update.
pub const MAX_MARKERS: u32 = 32;
/// Maximum UTF-8 bytes for a marker title.
pub const MAX_TITLE_BYTES: usize = 128;
/// Inclusive latitude bound in degrees.
const MAX_LATITUDE: f64 = 90.0;
/// Inclusive longitude bound in degrees.
const MAX_LONGITUDE: f64 = 180.0;
/// Inclusive zoom bound (`MapKit` / Google Maps common range).
const MAX_ZOOM: f64 = 22.0;

/// A host-owned map session. Identifiers are never reused within one plugin
/// instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

impl SessionId {
	/// Encodes the id as four little-endian bytes.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(4);
		output.u32(self.0);
		output.finish()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		input.finish()?;
		Ok(Self(id))
	}
}

/// Map camera centre and zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
	/// Latitude in degrees, finite and within ±90.
	pub latitude: f64,
	/// Longitude in degrees, finite and within ±180.
	pub longitude: f64,
	/// Zoom level, finite and within `[0, 22]`.
	pub zoom: f64,
}

impl Camera {
	/// Rejects non-finite values and out-of-range coordinates or zoom.
	pub fn new(latitude: f64, longitude: f64, zoom: f64) -> Result<Self, Error> {
		validate_latitude(latitude)?;
		validate_longitude(longitude)?;
		validate_zoom(zoom)?;
		Ok(Self {
			latitude,
			longitude,
			zoom,
		})
	}

	/// Default camera: null island at a mid-city zoom.
	#[must_use]
	pub fn default_pose() -> Self {
		Self {
			latitude: 0.0,
			longitude: 0.0,
			zoom: 2.0,
		}
	}

	/// Encodes three little-endian f64 values.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(24);
		write_f64(&mut output, self.latitude);
		write_f64(&mut output, self.longitude);
		write_f64(&mut output, self.zoom);
		output.finish()
	}

	/// Decodes through [`Camera::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let camera = read_camera(&mut input)?;
		input.finish()?;
		Ok(camera)
	}
}

/// Session-scoped camera update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetCamera {
	/// Target session.
	pub session: SessionId,
	/// New camera pose.
	pub camera: Camera,
}

impl SetCamera {
	/// Encodes session id then camera pose.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(28);
		output.u32(self.session.0);
		write_f64(&mut output, self.camera.latitude);
		write_f64(&mut output, self.camera.longitude);
		write_f64(&mut output, self.camera.zoom);
		output.finish()
	}

	/// Decodes session id and camera, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let session = SessionId(input.u32()?);
		let camera = read_camera(&mut input)?;
		input.finish()?;
		Ok(Self { session, camera })
	}
}

/// One map pin.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
	/// Latitude in degrees.
	pub latitude: f64,
	/// Longitude in degrees.
	pub longitude: f64,
	/// Optional title; empty means untitled.
	pub title: String,
}

impl Marker {
	/// Rejects bad coordinates, NULs, and oversized titles.
	pub fn new(latitude: f64, longitude: f64, title: impl Into<String>) -> Result<Self, Error> {
		validate_latitude(latitude)?;
		validate_longitude(longitude)?;
		let title = title.into();
		if title.contains('\0') || title.len() > MAX_TITLE_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			latitude,
			longitude,
			title,
		})
	}

	fn encode_into(&self, output: &mut Encoder) -> Result<(), Error> {
		write_f64(output, self.latitude);
		write_f64(output, self.longitude);
		output.bytes(self.title.as_bytes())?;
		Ok(())
	}

	fn decode_from(input: &mut Decoder<'_>) -> Result<Self, Error> {
		let latitude = read_f64(input)?;
		let longitude = read_f64(input)?;
		let title = input.string()?.to_owned();
		Self::new(latitude, longitude, title)
	}
}

/// Replace the marker set for one session.
#[derive(Debug, Clone, PartialEq)]
pub struct SetMarkers {
	/// Target session.
	pub session: SessionId,
	/// Markers to show; at most [`MAX_MARKERS`].
	pub markers: Vec<Marker>,
}

impl SetMarkers {
	/// Rejects oversized lists.
	pub fn new(session: SessionId, markers: Vec<Marker>) -> Result<Self, Error> {
		if markers.len() as u32 > MAX_MARKERS {
			return Err(Error::TooLarge);
		}
		Ok(Self { session, markers })
	}

	/// Encodes session, count, then each marker.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8 + self.markers.len() * 48);
		output.u32(self.session.0);
		output.u32(self.markers.len() as u32);
		for marker in &self.markers {
			marker.encode_into(&mut output)?;
		}
		Ok(output.finish())
	}

	/// Decodes through [`SetMarkers::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let session = SessionId(input.u32()?);
		let count = input.u32()?;
		if count > MAX_MARKERS {
			return Err(Error::TooLarge);
		}
		let mut markers = Vec::with_capacity(count as usize);
		for _ in 0..count {
			markers.push(Marker::decode_from(&mut input)?);
		}
		input.finish()?;
		Self::new(session, markers)
	}
}

/// A transport that names one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommand {
	/// Target session.
	pub session: SessionId,
}

impl SessionCommand {
	/// Encodes the session id alone.
	#[must_use]
	pub fn encode(self) -> Bytes {
		self.session.encode()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Ok(Self {
			session: SessionId::decode(bytes)?,
		})
	}
}

/// Snapshot of one map session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
	/// Current camera pose.
	pub camera: Camera,
	/// Number of markers currently set.
	pub marker_count: u32,
	/// Whether the session is open.
	pub active: bool,
}

impl Status {
	/// Encodes version, camera, marker count, and active flag.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(36);
		output.u32(WIRE_VERSION);
		write_f64(&mut output, self.camera.latitude);
		write_f64(&mut output, self.camera.longitude);
		write_f64(&mut output, self.camera.zoom);
		output.u32(self.marker_count);
		output.u32(u32::from(self.active));
		output.finish()
	}

	/// Strictly validates a fixed-width status reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let camera = read_camera(&mut input)?;
		let marker_count = input.u32()?;
		if marker_count > MAX_MARKERS {
			return Err(Error::InvalidArgument);
		}
		let active = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self {
			camera,
			marker_count,
			active,
		})
	}
}

fn validate_latitude(value: f64) -> Result<(), Error> {
	if !value.is_finite() || !(-MAX_LATITUDE..=MAX_LATITUDE).contains(&value) {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

fn validate_longitude(value: f64) -> Result<(), Error> {
	if !value.is_finite() || !(-MAX_LONGITUDE..=MAX_LONGITUDE).contains(&value) {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

fn validate_zoom(value: f64) -> Result<(), Error> {
	if !value.is_finite() || !(0.0..=MAX_ZOOM).contains(&value) {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

fn write_f64(output: &mut Encoder, value: f64) {
	let bits = value.to_bits().to_le_bytes();
	output.u32(u32::from_le_bytes(bits[..4].try_into().unwrap()));
	output.u32(u32::from_le_bytes(bits[4..].try_into().unwrap()));
}

fn read_f64(input: &mut Decoder<'_>) -> Result<f64, Error> {
	let low = input.u32()?.to_le_bytes();
	let high = input.u32()?.to_le_bytes();
	let mut bits = [0_u8; 8];
	bits[..4].copy_from_slice(&low);
	bits[4..].copy_from_slice(&high);
	Ok(f64::from_le_bytes(bits))
}

fn read_camera(input: &mut Decoder<'_>) -> Result<Camera, Error> {
	Camera::new(read_f64(input)?, read_f64(input)?, read_f64(input)?)
}

fn decode_flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Validates method payloads without invoking a host.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		CREATE => Decoder::new(payload).finish(),
		SET_CAMERA => SetCamera::decode(payload).map(|_| ()),
		SET_MARKERS => SetMarkers::decode(payload).map(|_| ()),
		STATUS | RELEASE => SessionCommand::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a map surface.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		validate(method, &payload)?;
		Err(Error::Unavailable)
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Maps(leptoskit_bridge::guest::Client);

	impl Maps {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Opens a map session and returns its id.
		pub async fn create(self) -> Result<SessionId, Error> {
			SessionId::decode(&self.0.call(CREATE, &[])?.await?)
		}

		/// Moves the camera for `session`.
		pub async fn set_camera(self, request: SetCamera) -> Result<(), Error> {
			require_empty(&self.0.call(SET_CAMERA, &request.encode())?.await?)
		}

		/// Replaces markers for `session`.
		pub async fn set_markers(self, request: &SetMarkers) -> Result<(), Error> {
			require_empty(&self.0.call(SET_MARKERS, &request.encode()?)?.await?)
		}

		/// Snapshots camera, marker count, and active state.
		pub async fn status(self, session: SessionId) -> Result<Status, Error> {
			Status::decode(
				&self
					.0
					.call(STATUS, &SessionCommand { session }.encode())?
					.await?,
			)
		}

		/// Releases the host session; bound `Map` views show nothing.
		pub async fn release(self, session: SessionId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(RELEASE, &SessionCommand { session }.encode())?
					.await?,
			)
		}
	}

	fn require_empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}

/// Browsers have no first-party map host in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unavailable without inventing a session.
	pub fn create() -> Result<SessionId, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn set_camera(_request: SetCamera) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn set_markers(_request: &SetMarkers) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn status(_session: SessionId) -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn release(_session: SessionId) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn session_and_camera_codecs_reject_bounds_and_trailers() {
		assert_eq!(
			SessionId::decode(&SessionId(42).encode()),
			Ok(SessionId(42))
		);
		let mut trailing = SessionId(1).encode().to_vec();
		trailing.push(0);
		assert_eq!(SessionId::decode(&trailing), Err(Error::InvalidArgument));
		assert!(SessionId::decode(&[]).is_err());

		let camera = Camera::new(37.77, -122.42, 12.0).unwrap();
		assert_eq!(Camera::decode(&camera.encode()), Ok(camera));
		assert_eq!(Camera::default_pose().zoom, 2.0);
		assert_eq!(
			Camera::new(91.0, 0.0, 1.0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Camera::new(0.0, 181.0, 1.0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Camera::new(0.0, 0.0, 23.0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Camera::new(f64::NAN, 0.0, 1.0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Camera::new(0.0, f64::INFINITY, 1.0),
			Err(Error::InvalidArgument)
		);
		let mut trailing = camera.encode().to_vec();
		trailing.push(0);
		assert_eq!(Camera::decode(&trailing), Err(Error::InvalidArgument));
		assert!(Camera::decode(&camera.encode()[..8]).is_err());

		let set = SetCamera {
			session: SessionId(3),
			camera,
		};
		assert_eq!(SetCamera::decode(&set.encode()), Ok(set));
		let mut trailing = set.encode().to_vec();
		trailing.push(1);
		assert_eq!(SetCamera::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn markers_status_and_unsupported_never_invent_sessions() {
		let marker = Marker::new(1.0, 2.0, "Pin").unwrap();
		assert_eq!(
			Marker::new(1.0, 2.0, "a\0b"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Marker::new(1.0, 2.0, "x".repeat(MAX_TITLE_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
		let markers = SetMarkers::new(SessionId(1), vec![marker.clone()]).unwrap();
		assert_eq!(SetMarkers::decode(&markers.encode().unwrap()), Ok(markers));
		assert_eq!(
			SetMarkers::new(
				SessionId(1),
				(0..=MAX_MARKERS)
					.map(|index| Marker::new(0.0, 0.0, index.to_string()).unwrap())
					.collect()
			),
			Err(Error::TooLarge)
		);
		assert_eq!(
			SetMarkers::decode(&{
				let mut over = Encoder::with_capacity(8);
				over.u32(1);
				over.u32(MAX_MARKERS + 1);
				over.finish()
			}),
			Err(Error::TooLarge)
		);
		assert_eq!(
			SetMarkers::decode(&SetMarkers::new(SessionId(1), vec![marker]).unwrap().encode().unwrap()[..4]),
			Err(Error::InvalidArgument)
		);

		let status = Status {
			camera: Camera::default_pose(),
			marker_count: 1,
			active: true,
		};
		assert_eq!(Status::decode(&status.encode()), Ok(status));
		let mut bad_version = status.encode().to_vec();
		bad_version[0] = 9;
		assert_eq!(Status::decode(&bad_version), Err(Error::InvalidArgument));
		let mut bad_count = status.encode().to_vec();
		bad_count[28..32].copy_from_slice(&(MAX_MARKERS + 1).to_le_bytes());
		assert_eq!(Status::decode(&bad_count), Err(Error::InvalidArgument));
		let mut bad_flag = status.encode().to_vec();
		bad_flag[32] = 2;
		assert_eq!(Status::decode(&bad_flag), Err(Error::InvalidArgument));
		let mut trailing = status.encode().to_vec();
		trailing.push(0);
		assert_eq!(Status::decode(&trailing), Err(Error::InvalidArgument));

		assert_eq!(validate(CREATE, &[]), Ok(()));
		assert_eq!(validate(CREATE, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(
				SET_CAMERA,
				&SetCamera {
					session: SessionId(1),
					camera: Camera::default_pose(),
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				SET_MARKERS,
				&SetMarkers::new(SessionId(1), vec![Marker::new(0.0, 0.0, "").unwrap()])
					.unwrap()
					.encode()
					.unwrap()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				STATUS,
				&SessionCommand {
					session: SessionId(1)
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				RELEASE,
				&SessionCommand {
					session: SessionId(1)
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		assert_eq!(METHODS.len(), 5);
		assert_eq!(NAME, "maps");

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(CREATE, Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(
				SET_CAMERA,
				SetCamera {
					session: SessionId(1),
					camera: Camera::default_pose(),
				}
				.encode()
			),
			Err(Error::Unavailable)
		);
	}

	#[test]
	fn empty_reply_and_flag_helpers() {
		assert!(Decoder::new(&[]).finish().is_ok());
		assert_eq!(Decoder::new(&[0]).finish(), Err(Error::InvalidArgument));
		assert_eq!(decode_flag(0), Ok(false));
		assert_eq!(decode_flag(1), Ok(true));
		assert_eq!(decode_flag(2), Err(Error::InvalidArgument));
		assert_eq!(
			SessionCommand::decode(
				&SessionCommand {
					session: SessionId(9)
				}
				.encode()
			),
			Ok(SessionCommand {
				session: SessionId(9)
			})
		);
	}
}
