//! <!-- {=p_locationSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_location` as a direct dependency and select the
//! `location` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Location` rather than resolving
//! the plugin on every call. Native reads never prompt; request permission as a
//! separate, intentional action, and the app supplies its usage descriptions
//! and manifest permissions. Continuous updates run through `start_updates`
//! with an accuracy request, distance filter and interval, buffered into a
//! bounded batch stream by `next_updates`; background updates need the
//! `background` plugin's location-typed foreground service on Android and the
//! "Always" grant on iOS.
//!
//! <!-- {/p_locationSummary} -->

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Runtime registration name.
pub const NAME: &str = "location";
/// Empty request, eight-byte permission reply; never prompts.
pub const PERMISSION: MethodId = MethodId(0);
/// A u32 boolean requests fine accuracy where the OS supports that choice.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Twelve-byte options request, tagged fix/timeout reply.
pub const CURRENT: MethodId = MethodId(2);
/// Options followed by a previous Unix timestamp as f64; returns a newer fix.
pub const NEXT: MethodId = MethodId(3);
/// Empty request; upgrades "When In Use" to "Always" on iOS and requests
/// `ACCESS_BACKGROUND_LOCATION` on Android, each as its own system step.
pub const REQUEST_BACKGROUND_PERMISSION: MethodId = MethodId(4);
/// [`UpdateOptions`]; starts continuous updates into a bounded native buffer.
pub const START_UPDATES: MethodId = MethodId(5);
/// Empty request; stops every running update source. Buffered fixes stay
/// buffered for [`NEXT_UPDATES`].
pub const STOP_UPDATES: MethodId = MethodId(6);
/// Empty request; a cancellable wait for the fixes buffered since the last
/// call. An empty batch means no source is running and the buffer is empty.
pub const NEXT_UPDATES: MethodId = MethodId(7);
/// Empty request; iOS visit monitoring, delivered as fixes at arrival and
/// departure. Other platforms return `Unavailable`.
pub const START_VISITS: MethodId = MethodId(8);
/// Empty request; iOS significant-change monitoring, which relaunches a
/// killed app. Other platforms return `Unavailable`.
pub const START_SIGNIFICANT_CHANGES: MethodId = MethodId(9);
/// Runtime methods.
pub const METHODS: &[MethodId] = &[
	PERMISSION,
	REQUEST_PERMISSION,
	CURRENT,
	NEXT,
	REQUEST_BACKGROUND_PERMISSION,
	START_UPDATES,
	STOP_UPDATES,
	NEXT_UPDATES,
	START_VISITS,
	START_SIGNIFICANT_CHANGES,
];

/// A permission reply is two little-endian u32 fields.
const PERMISSION_BYTES: usize = 8;
/// A bounds request is three little-endian u32 fields.
const OPTIONS_BYTES: usize = 12;
/// A fix record is four tag fields followed by eight doubles.
const FIX_BYTES: usize = 80;
/// A tagged acquisition reply is a u32 tag followed by a fix record.
const RESPONSE_BYTES: usize = FIX_BYTES + 4;
/// A fix record carries eight doubles: four required and four optional.
const RECORD_DOUBLES: usize = 8;
const REQUIRED_FIELDS: usize = 4;
const OPTIONAL_FIELDS: usize = 4;
/// The four optional-value bits a fix record may set.
const OPTIONAL_BITS: u32 = 15;
/// Latitude and longitude bounds, in degrees.
const MAX_LATITUDE: f64 = 90.0;
const MAX_LONGITUDE: f64 = 180.0;
/// A heading stays below a full turn, in degrees.
const FULL_TURN: f64 = 360.0;
/// Longest acquisition timeout, in milliseconds.
const MAX_TIMEOUT_MS: u32 = 120_000;
/// Longest accepted cache tolerance, in milliseconds.
const MAX_AGE_MS: u32 = 3_600_000;
/// An update request is six little-endian fields: a u32 accuracy, an f64
/// distance filter, a u32 interval and three u32 flags.
const UPDATE_OPTIONS_BYTES: usize = 28;
/// Longest update interval, in milliseconds.
const MAX_INTERVAL_MS: u32 = 3_600_000;
/// Widest distance filter, in metres; zero disables filtering.
const MAX_DISTANCE_FILTER_M: f64 = 10_000.0;
/// Fixes a native update buffer keeps before dropping the oldest.
pub const UPDATE_BUFFER_CAPACITY: usize = 64;
/// An update batch header is two little-endian u32 fields.
const UPDATES_HEADER_BYTES: usize = 8;

/// Foreground permission state, distinct from device service availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	Unknown = 0,
	Prompt = 1,
	Denied = 2,
	Granted = 3,
	Restricted = 4,
}
/// Permission's accuracy scope, not the measurement's horizontal error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AccuracyPermission {
	Unknown = 0,
	Reduced = 1,
	Full = 2,
}
/// Foreground authorization. Browser permission queries cannot report accuracy
/// scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
	/// Current authorization state.
	pub status: Status,
	/// Whether the OS grants approximate or precise access.
	pub accuracy: AccuracyPermission,
}
impl Permission {
	/// Decodes the exact native permission response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != PERMISSION_BYTES {
			return Err(Error::InvalidArgument);
		}

		let status = match u32_value(&bytes[..4])? {
			0 => Status::Unknown,
			1 => Status::Prompt,
			2 => Status::Denied,
			3 => Status::Granted,
			4 => Status::Restricted,
			_ => return Err(Error::InvalidArgument),
		};

		let accuracy = match u32_value(&bytes[4..])? {
			0 => AccuracyPermission::Unknown,
			1 => AccuracyPermission::Reduced,
			2 => AccuracyPermission::Full,
			_ => return Err(Error::InvalidArgument),
		};

		if status != Status::Granted && accuracy != AccuracyPermission::Unknown {
			return Err(Error::InvalidArgument);
		}

		Ok(Self { status, accuracy })
	}
}

/// The provider reported by Android. Other platforms explicitly return Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Provider {
	Unknown = 0,
	Gps = 1,
	Network = 2,
	Passive = 3,
	Fused = 4,
}
/// A measured coordinate. Optional fields stay unknown instead of becoming
/// zeros.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
	/// Unix timestamp in milliseconds, as provided by the platform.
	pub timestamp_ms: f64,
	/// Latitude in degrees, -90..=90.
	pub latitude: f64,
	/// Longitude in degrees, -180..=180.
	pub longitude: f64,
	/// Horizontal uncertainty in metres, not a promise of actual error.
	pub accuracy_m: f64,
	/// Altitude in metres. Vertical datum is platform-specific.
	pub altitude_m: Option<f64>,
	/// Vertical uncertainty in metres when reported.
	pub altitude_accuracy_m: Option<f64>,
	/// Course of travel clockwise from north, 0..<360 degrees; not compass
	/// heading.
	pub heading_degrees: Option<f64>,
	/// Speed in metres per second.
	pub speed_mps: Option<f64>,
	/// Provider only when reported by the platform.
	pub provider: Provider,
	/// Native software-simulated/mock flag. Unknown on browsers.
	pub simulated: Option<bool>,
}
impl Fix {
	/// Validates coordinates and optional values before serializing or using a
	/// fix.
	pub fn validate(self) -> Result<Self, Error> {
		self.validate_required()?;
		self.validate_optional()?;
		Ok(self)
	}

	/// The timestamp and the measurements every fix must carry.
	fn validate_required(self) -> Result<(), Error> {
		if !self.timestamp_ms.is_finite() || self.timestamp_ms < 0.0 {
			return Err(Error::InvalidArgument);
		}

		if !self.accuracy_m.is_finite() || self.accuracy_m < 0.0 {
			return Err(Error::InvalidArgument);
		}

		if !self.latitude.is_finite() || !(-MAX_LATITUDE..=MAX_LATITUDE).contains(&self.latitude) {
			return Err(Error::InvalidArgument);
		}

		if !self.longitude.is_finite()
			|| !(-MAX_LONGITUDE..=MAX_LONGITUDE).contains(&self.longitude)
		{
			return Err(Error::InvalidArgument);
		}

		Ok(())
	}

	/// The measurements a platform may omit; each must be finite and in range.
	fn validate_optional(self) -> Result<(), Error> {
		let omitted = [
			self.altitude_m,
			self.altitude_accuracy_m,
			self.heading_degrees,
			self.speed_mps,
		];

		if omitted
			.into_iter()
			.flatten()
			.any(|value| !value.is_finite())
		{
			return Err(Error::InvalidArgument);
		}

		if self.altitude_accuracy_m.is_some_and(|value| value < 0.0) {
			return Err(Error::InvalidArgument);
		}

		if self.speed_mps.is_some_and(|value| value < 0.0) {
			return Err(Error::InvalidArgument);
		}

		if self
			.heading_degrees
			.is_some_and(|value| !(0.0..FULL_TURN).contains(&value))
		{
			return Err(Error::InvalidArgument);
		}

		Ok(())
	}

	/// Fixed 80-byte little-endian wire representation, with explicit
	/// optional-value bits.
	pub fn encode(self) -> Result<[u8; FIX_BYTES], Error> {
		self.validate()?;
		let optional = [
			self.altitude_m,
			self.altitude_accuracy_m,
			self.heading_degrees,
			self.speed_mps,
		];
		let mask = optional
			.iter()
			.enumerate()
			.fold(0u32, |mask, (bit, value)| {
				mask | if value.is_some() { 1 << bit } else { 0 }
			});
		let mut bytes = [0; FIX_BYTES];

		bytes[..4].copy_from_slice(&mask.to_le_bytes());
		bytes[4..8].copy_from_slice(&(self.provider as u32).to_le_bytes());
		bytes[8..12].copy_from_slice(
			&self
				.simulated
				.map_or(0u32, |v| if v { 2 } else { 1 })
				.to_le_bytes(),
		);
		// Reserved field at 12..16 must remain zero.
		for (output, value) in bytes[16..].as_chunks_mut::<8>().0.iter_mut().zip(
			[
				self.timestamp_ms,
				self.latitude,
				self.longitude,
				self.accuracy_m,
			]
			.into_iter()
			.chain(optional.into_iter().map(|v| v.unwrap_or(0.0))),
		) {
			output.copy_from_slice(&value.to_le_bytes());
		}
		Ok(bytes)
	}

	/// Strictly decodes the 80-byte fix payload, including reserved fields.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != FIX_BYTES {
			return Err(Error::InvalidArgument);
		}

		let mask = u32_value(&bytes[..4])?;

		if mask > OPTIONAL_BITS || u32_value(&bytes[12..16])? != 0 {
			return Err(Error::InvalidArgument);
		}

		let provider = read_provider(u32_value(&bytes[4..8])?)?;
		let simulated = read_simulated(u32_value(&bytes[8..12])?)?;
		let values = read_values(&bytes[16..]);
		let optional = read_optional(&values, mask)?;

		Self {
			timestamp_ms: values[0],
			latitude: values[1],
			longitude: values[2],
			accuracy_m: values[3],
			altitude_m: optional[0],
			altitude_accuracy_m: optional[1],
			heading_degrees: optional[2],
			speed_mps: optional[3],
			provider,
			simulated,
		}
		.validate()
	}
}

/// Rejects a provider code the wire format does not define.
fn read_provider(code: u32) -> Result<Provider, Error> {
	match code {
		0 => Ok(Provider::Unknown),
		1 => Ok(Provider::Gps),
		2 => Ok(Provider::Network),
		3 => Ok(Provider::Passive),
		4 => Ok(Provider::Fused),
		_ => Err(Error::InvalidArgument),
	}
}

/// Reads the mock flag, which is absent when the platform does not report it.
fn read_simulated(code: u32) -> Result<Option<bool>, Error> {
	match code {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}

/// Reads the eight little-endian doubles of one fix record.
fn read_values(bytes: &[u8]) -> [f64; RECORD_DOUBLES] {
	let mut values = [0.0; RECORD_DOUBLES];
	let (chunks, _) = bytes.as_chunks::<8>();

	for (value, input) in values.iter_mut().zip(chunks) {
		*value = f64::from_le_bytes(*input);
	}

	values
}

/// Reads the optional values, rejecting a cleared bit that still carries bytes.
fn read_optional(
	values: &[f64; RECORD_DOUBLES],
	mask: u32,
) -> Result<[Option<f64>; OPTIONAL_FIELDS], Error> {
	let mut optional = [None; OPTIONAL_FIELDS];

	for (index, value) in values[REQUIRED_FIELDS..].iter().enumerate() {
		if mask & (1 << index) != 0 {
			optional[index] = Some(*value);
		} else if value.to_bits() != 0 {
			return Err(Error::InvalidArgument);
		}
	}

	Ok(optional)
}
/// Validated acquisition bounds. Maximum age is an explicit cache tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
	high_accuracy: bool,
	timeout_ms: u32,
	maximum_age_ms: u32,
}
impl Options {
	/// Timeout is 1..=120,000ms and maximum cached age is 0..=3,600,000ms.
	pub fn new(high_accuracy: bool, timeout_ms: u32, maximum_age_ms: u32) -> Result<Self, Error> {
		if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS || maximum_age_ms > MAX_AGE_MS {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			high_accuracy,
			timeout_ms,
			maximum_age_ms,
		})
	}

	/// Requests the best available precision without overriding approximate
	/// permission.
	pub fn high_accuracy(self) -> bool {
		self.high_accuracy
	}

	/// Total acquisition time budget in milliseconds.
	pub fn timeout_ms(self) -> u32 {
		self.timeout_ms
	}

	/// Maximum accepted cached fix age. Zero requests a fresh measurement.
	pub fn maximum_age_ms(self) -> u32 {
		self.maximum_age_ms
	}

	/// Exact request representation.
	pub fn encode(self) -> [u8; OPTIONS_BYTES] {
		let mut bytes = [0; OPTIONS_BYTES];
		for (slot, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip([
			u32::from(self.high_accuracy),
			self.timeout_ms,
			self.maximum_age_ms,
		]) {
			slot.copy_from_slice(&value.to_le_bytes());
		}
		bytes
	}

	/// Validates options from a complete native request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != OPTIONS_BYTES {
			return Err(Error::InvalidArgument);
		}

		let accuracy = u32_value(&bytes[..4])?;

		if accuracy > 1 {
			return Err(Error::InvalidArgument);
		}

		Self::new(
			accuracy == 1,
			u32_value(&bytes[4..8])?,
			u32_value(&bytes[8..])?,
		)
	}
}
/// The accuracy a continuous update stream asks the platform for; it is a
/// request, not a promise, and never overrides an approximate grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum UpdateAccuracy {
	/// Best the platform offers.
	Best = 0,
	/// Highest accuracy, for active navigation guidance.
	Navigation = 1,
	/// Lower power; less precise fixes are acceptable.
	Balanced = 2,
}

impl UpdateAccuracy {
	/// Decodes the accuracy field of an update request.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Ok(match code {
			0 => Self::Best,
			1 => Self::Navigation,
			2 => Self::Balanced,
			_ => return Err(Error::InvalidArgument),
		})
	}
}

/// Validated configuration for one continuous update stream.
///
/// `pauses_automatically` and `show_background_indicator` are iOS settings the
/// other platforms decode and ignore. Background updates need the
/// platform's own support: the `background` plugin's location-typed
/// foreground service on Android, and "Always" authorization on iOS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateOptions {
	accuracy: UpdateAccuracy,
	distance_filter_m: f64,
	interval_ms: u32,
	background: bool,
	pauses_automatically: bool,
	show_background_indicator: bool,
}

impl UpdateOptions {
	/// Best-accuracy updates every second, no distance filter,
	/// foreground-only, with iOS's automatic pausing on.
	pub fn new(accuracy: UpdateAccuracy) -> Self {
		Self {
			accuracy,
			distance_filter_m: 0.0,
			interval_ms: 1000,
			background: false,
			pauses_automatically: true,
			show_background_indicator: false,
		}
	}

	/// Minimum distance, in metres, between delivered fixes; 0..=10,000,
	/// where zero delivers every platform fix.
	pub fn with_distance_filter_m(mut self, metres: f64) -> Result<Self, Error> {
		if !metres.is_finite() || !(0.0..=MAX_DISTANCE_FILTER_M).contains(&metres) {
			return Err(Error::InvalidArgument);
		}

		self.distance_filter_m = metres;
		Ok(self)
	}

	/// Longest requested interval, in milliseconds; 1..=3,600,000. The
	/// platform remains free to deliver fixes sooner.
	pub fn with_interval_ms(mut self, milliseconds: u32) -> Result<Self, Error> {
		(1..=MAX_INTERVAL_MS)
			.contains(&milliseconds)
			.then_some(())
			.ok_or(Error::InvalidArgument)?;
		self.interval_ms = milliseconds;
		Ok(self)
	}

	/// Keeps updates running while the app is in the background.
	#[must_use]
	pub fn with_background(mut self, background: bool) -> Self {
		self.background = background;
		self
	}

	/// iOS only: lets the system pause updates, for example when the device
	/// stops moving. Ignored elsewhere.
	#[must_use]
	pub fn with_pauses_automatically(mut self, pauses: bool) -> Self {
		self.pauses_automatically = pauses;
		self
	}

	/// iOS only: shows the background location indicator while updates run
	/// without the app being visible. Ignored elsewhere.
	#[must_use]
	pub fn with_show_background_indicator(mut self, visible: bool) -> Self {
		self.show_background_indicator = visible;
		self
	}

	/// The accuracy the stream asked for.
	#[must_use]
	pub const fn accuracy(&self) -> UpdateAccuracy {
		self.accuracy
	}

	/// Minimum metres between fixes; zero delivers every fix.
	#[must_use]
	pub const fn distance_filter_m(&self) -> f64 {
		self.distance_filter_m
	}

	/// The requested interval, in milliseconds.
	#[must_use]
	pub const fn interval_ms(&self) -> u32 {
		self.interval_ms
	}

	/// Whether updates must keep running in the background.
	#[must_use]
	pub const fn background(&self) -> bool {
		self.background
	}

	/// Whether iOS may pause updates automatically.
	#[must_use]
	pub const fn pauses_automatically(&self) -> bool {
		self.pauses_automatically
	}

	/// Whether iOS shows its background location indicator.
	#[must_use]
	pub const fn show_background_indicator(&self) -> bool {
		self.show_background_indicator
	}

	/// Exact request representation: six little-endian fields.
	pub fn encode(self) -> [u8; UPDATE_OPTIONS_BYTES] {
		let mut bytes = [0; UPDATE_OPTIONS_BYTES];

		bytes[..4].copy_from_slice(&(self.accuracy as u32).to_le_bytes());
		bytes[4..12].copy_from_slice(&self.distance_filter_m.to_le_bytes());
		bytes[12..16].copy_from_slice(&self.interval_ms.to_le_bytes());
		bytes[16..20].copy_from_slice(&u32::from(self.background).to_le_bytes());
		bytes[20..24].copy_from_slice(&u32::from(self.pauses_automatically).to_le_bytes());
		bytes[24..].copy_from_slice(&u32::from(self.show_background_indicator).to_le_bytes());
		bytes
	}

	/// Validates options from a complete native request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != UPDATE_OPTIONS_BYTES {
			return Err(Error::InvalidArgument);
		}

		let accuracy = UpdateAccuracy::from_code(u32_value(&bytes[..4])?)?;
		let distance = f64::from_le_bytes(
			bytes[4..12]
				.try_into()
				.map_err(|_| Error::InvalidArgument)?,
		);

		if !distance.is_finite() || !(0.0..=MAX_DISTANCE_FILTER_M).contains(&distance) {
			return Err(Error::InvalidArgument);
		}

		for field in [16, 20, 24] {
			if u32_value(&bytes[field..field + 4])? > 1 {
				return Err(Error::InvalidArgument);
			}
		}

		Ok(Self::new(accuracy)
			.with_distance_filter_m(distance)?
			.with_interval_ms(u32_value(&bytes[12..16])?)?
			.with_background(u32_value(&bytes[16..20])? == 1)
			.with_pauses_automatically(u32_value(&bytes[20..24])? == 1)
			.with_show_background_indicator(u32_value(&bytes[24..])? == 1))
	}
}
/// One [`NEXT_UPDATES`] batch: the fixes buffered since the previous call,
/// plus how many older fixes the bounded native buffer dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Updates {
	/// Fixes the buffer discarded, oldest first, since the last batch.
	pub dropped: u32,
	/// Buffered fixes, oldest first, at most [`UPDATE_BUFFER_CAPACITY`].
	pub fixes: Vec<Fix>,
}

/// Decodes a tagged native update batch. `None` means no source is running
/// and nothing is buffered: the stream has ended.
pub fn decode_updates(bytes: &[u8]) -> Result<Option<Updates>, Error> {
	if bytes.len() < UPDATES_HEADER_BYTES {
		return Err(Error::InvalidArgument);
	}

	let dropped = u32_value(&bytes[..4])?;
	let count = u32_value(&bytes[4..UPDATES_HEADER_BYTES])?;

	if count == 0 {
		if dropped != 0 || bytes.len() != UPDATES_HEADER_BYTES {
			return Err(Error::InvalidArgument);
		}

		return Ok(None);
	}

	if count as usize > UPDATE_BUFFER_CAPACITY
		|| bytes.len() != UPDATES_HEADER_BYTES + count as usize * FIX_BYTES
	{
		return Err(Error::InvalidArgument);
	}

	let fixes = (0..count as usize)
		.map(|index| {
			let start = UPDATES_HEADER_BYTES + index * FIX_BYTES;
			Fix::decode(&bytes[start..start + FIX_BYTES])
		})
		.collect::<Result<Vec<_>, _>>()?;

	Ok(Some(Updates { dropped, fixes }))
}
/// Acquisition failure. Timeout is distinct from permission denial or
/// unavailability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
	Bridge(Error),
	TimedOut,
}
impl From<Error> for LocationError {
	fn from(value: Error) -> Self {
		Self::Bridge(value)
	}
}
impl std::fmt::Display for LocationError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::TimedOut => formatter.write_str("location acquisition timed out"),
			Self::Bridge(error) => write!(formatter, "location bridge error: {error}"),
		}
	}
}
impl std::error::Error for LocationError {}
/// Decodes a tagged native acquisition response.
pub fn decode_response(bytes: &[u8]) -> Result<Fix, LocationError> {
	if bytes.len() == 4 && bytes == 1u32.to_le_bytes() {
		return Err(LocationError::TimedOut);
	}

	if bytes.len() != RESPONSE_BYTES || u32_value(&bytes[..4])? != 0 {
		return Err(Error::InvalidArgument.into());
	}

	Ok(Fix::decode(&bytes[4..])?)
}
fn u32_value(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Reactive location fixes over this plugin's sources.
#[cfg(feature = "hooks")]
pub mod signals;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached native guest client; one outstanding acquisition per plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Foreground location capability.
	#[derive(Debug, Clone, Copy)]
	pub struct Location(leptoskit_bridge::guest::Client);
	impl Location {
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		pub async fn permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
		}

		/// Requests foreground access; Android may grant approximate access
		/// only.
		pub async fn request_permission(self, precise: bool) -> Result<Permission, Error> {
			Permission::decode(
				&self
					.0
					.call(REQUEST_PERMISSION, &u32::from(precise).to_le_bytes())?
					.await?,
			)
		}

		/// Returns a fresh or explicitly age-tolerated fix. Reads never prompt.
		pub async fn current(self, options: Options) -> Result<Fix, LocationError> {
			decode_response(&self.0.call(CURRENT, &options.encode())?.await?)
		}

		/// Waits for a strictly newer timestamp. Dropping the call stops native
		/// tracking. Repeating this bounded operation is a foreground
		/// watch, not a durable path log.
		pub async fn next(
			self,
			options: Options,
			after_timestamp_ms: f64,
		) -> Result<Fix, LocationError> {
			if !after_timestamp_ms.is_finite() || after_timestamp_ms < 0.0 {
				return Err(Error::InvalidArgument.into());
			}

			let mut payload = [0; 20];
			payload[..12].copy_from_slice(&options.encode());
			payload[12..].copy_from_slice(&after_timestamp_ms.to_le_bytes());
			decode_response(&self.0.call(NEXT, &payload)?.await?)
		}

		/// Upgrades "When In Use" to "Always" on iOS, or requests
		/// `ACCESS_BACKGROUND_LOCATION` on Android. Both platforms require the
		/// foreground grant first; calling this before
		/// [`Self::request_permission`] fails. Cancelling the call cannot
		/// dismiss the system screen.
		pub async fn request_background_permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(REQUEST_BACKGROUND_PERMISSION, &[])?.await?)
		}

		/// Starts continuous updates. On Android, `background` needs the
		/// `background` plugin's location-typed foreground service running;
		/// the call fails without it. On iOS it needs "Always" authorization.
		pub async fn start_updates(self, options: UpdateOptions) -> Result<(), Error> {
			self.empty(START_UPDATES, &options.encode()).await
		}

		/// Stops every running update source. Fixes already buffered stay
		/// buffered for [`Self::next_updates`].
		pub async fn stop_updates(self) -> Result<(), Error> {
			self.empty(STOP_UPDATES, &[]).await
		}

		/// Waits for the fixes buffered since the last call, oldest first.
		/// The wait is cancellable: dropping the future abandons it without
		/// losing buffered fixes. A `None` reply means no source is running
		/// and nothing is buffered, so the stream has ended. `dropped`
		/// counts fixes the bounded native buffer discarded, oldest first,
		/// since the previous batch.
		pub async fn next_updates(self) -> Result<Option<Updates>, Error> {
			decode_updates(&self.0.call(NEXT_UPDATES, &[])?.await?)
		}

		/// Starts iOS visit monitoring. Each visit report is buffered as one
		/// fix at the visit's coordinate: timestamped at arrival while the
		/// visit is ongoing, and at departure once iOS reports that it ended.
		/// Other platforms return `Unavailable`.
		pub async fn start_visits(self) -> Result<(), Error> {
			self.empty(START_VISITS, &[]).await
		}

		/// Starts iOS significant-change monitoring, a low-power source that
		/// relaunches a killed app on the next event. After a relaunch the
		/// app must call this and [`Self::start_updates`] again at startup;
		/// iOS then delivers the event that woke it to the new listener.
		/// Other platforms return `Unavailable`.
		pub async fn start_significant_changes(self) -> Result<(), Error> {
			self.empty(START_SIGNIFICANT_CHANGES, &[]).await
		}

		async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
			if self.0.call(method, payload)?.await?.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A timeout a real acquisition would use.
	const SAMPLE_TIMEOUT_MS: u32 = 30_000;
	fn fix() -> Fix {
		Fix {
			timestamp_ms: 1_800_000_000_000.0,
			latitude: 37.0,
			longitude: -122.0,
			accuracy_m: 5.0,
			altitude_m: None,
			altitude_accuracy_m: None,
			heading_degrees: None,
			speed_mps: None,
			provider: Provider::Unknown,
			simulated: None,
		}
	}
	#[test]
	fn fix_roundtrip_preserves_unknown_and_zero_measurements() {
		let unknown = fix();
		assert_eq!(Fix::decode(&unknown.encode().unwrap()), Ok(unknown));
		let zero = Fix {
			altitude_m: Some(0.0),
			speed_mps: Some(0.0),
			heading_degrees: Some(0.0),
			altitude_accuracy_m: Some(0.0),
			simulated: Some(false),
			..unknown
		};
		assert_eq!(Fix::decode(&zero.encode().unwrap()), Ok(zero));
		assert_ne!(unknown.encode().unwrap(), zero.encode().unwrap());
	}
	#[test]
	fn invalid_wire_and_coordinate_data_are_rejected() {
		for latitude in [
			-MAX_LATITUDE - 1.0,
			MAX_LATITUDE + 1.0,
			f64::NAN,
			f64::INFINITY,
		] {
			assert_eq!(
				Fix { latitude, ..fix() }.encode(),
				Err(Error::InvalidArgument)
			);
		}

		for heading in [-1.0, FULL_TURN, f64::NAN] {
			assert_eq!(
				Fix {
					heading_degrees: Some(heading),
					..fix()
				}
				.encode(),
				Err(Error::InvalidArgument)
			);
		}

		let encoded = fix().encode().unwrap();
		assert_eq!(
			Fix::decode(&encoded[..FIX_BYTES - 1]),
			Err(Error::InvalidArgument)
		);

		for index in [0, 4, 8, 12, 79] {
			let mut corrupt = encoded;
			corrupt[index] = 255;
			assert_eq!(Fix::decode(&corrupt), Err(Error::InvalidArgument));
		}

		assert_eq!(decode_response(&[1, 0, 0, 0]), Err(LocationError::TimedOut));
		assert_eq!(
			decode_response(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument.into())
		);
	}
	#[test]
	fn request_bounds_and_authorization_scope_are_strict() {
		assert!(Options::new(true, 0, 0).is_err());
		assert!(Options::new(false, MAX_TIMEOUT_MS + 1, 0).is_err());
		assert!(Options::new(false, 1000, MAX_AGE_MS + 1).is_err());
		let options = Options::new(true, SAMPLE_TIMEOUT_MS, 0).unwrap();
		assert_eq!(Options::decode(&options.encode()), Ok(options));
		assert!(Permission::decode(&[2, 0, 0, 0, 2, 0, 0, 0]).is_err());

		assert_eq!(
			Permission::decode(&[3, 0, 0, 0, 1, 0, 0, 0])
				.unwrap()
				.accuracy,
			AccuracyPermission::Reduced
		);
	}
	#[test]
	fn every_permission_status_and_scope_decodes_exactly() {
		let pair = |status: u32, accuracy: u32| {
			let mut bytes = status.to_le_bytes().to_vec();
			bytes.extend(accuracy.to_le_bytes());

			Permission::decode(&bytes)
		};

		for (value, status) in (0..).zip([
			Status::Unknown,
			Status::Prompt,
			Status::Denied,
			Status::Granted,
			Status::Restricted,
		]) {
			assert_eq!(
				pair(value, 0),
				Ok(Permission {
					status,
					accuracy: AccuracyPermission::Unknown
				})
			);
		}
		assert_eq!(pair(3, 2).unwrap().accuracy, AccuracyPermission::Full);
		assert_eq!(pair(5, 0), Err(Error::InvalidArgument));
		assert_eq!(pair(3, 3), Err(Error::InvalidArgument));
		assert_eq!(
			Permission::decode(&[3, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn every_provider_and_simulation_flag_roundtrips() {
		for provider in [
			Provider::Unknown,
			Provider::Gps,
			Provider::Network,
			Provider::Passive,
			Provider::Fused,
		] {
			for simulated in [None, Some(false), Some(true)] {
				let measured = Fix {
					provider,
					simulated,
					altitude_m: Some(-12.5),
					speed_mps: Some(3.0),
					..fix()
				};

				assert_eq!(Fix::decode(&measured.encode().unwrap()), Ok(measured));
			}
		}
		let mut unknown_provider = fix().encode().unwrap();
		unknown_provider[4] = 5;
		assert_eq!(Fix::decode(&unknown_provider), Err(Error::InvalidArgument));

		let mut unknown_simulation = fix().encode().unwrap();
		unknown_simulation[8] = 3;
		assert_eq!(
			Fix::decode(&unknown_simulation),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn options_accessors_and_strict_decoding() {
		let options = Options::new(false, 1, MAX_AGE_MS).unwrap();
		assert!(!options.high_accuracy());
		assert_eq!(options.timeout_ms(), 1);
		assert_eq!(options.maximum_age_ms(), MAX_AGE_MS);
		assert_eq!(Options::decode(&options.encode()), Ok(options));

		let mut invalid_accuracy = options.encode();
		invalid_accuracy[0] = 2;
		assert_eq!(
			Options::decode(&invalid_accuracy),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			Options::decode(&options.encode()[..OPTIONS_BYTES - 1]),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn tagged_responses_carry_fixes_or_typed_failures() {
		let mut response = 0u32.to_le_bytes().to_vec();
		response.extend(fix().encode().unwrap());
		assert_eq!(decode_response(&response), Ok(fix()));

		response[0] = 2;
		assert_eq!(
			decode_response(&response),
			Err(LocationError::Bridge(Error::InvalidArgument))
		);
		assert_eq!(
			LocationError::TimedOut.to_string(),
			"location acquisition timed out"
		);
		assert_eq!(
			LocationError::from(Error::Unavailable).to_string(),
			format!("location bridge error: {}", Error::Unavailable)
		);
	}
	#[test]
	fn update_options_carry_defaults_and_round_trip() {
		let options = UpdateOptions::new(UpdateAccuracy::Best);
		assert_eq!(options.distance_filter_m(), 0.0);
		assert_eq!(options.interval_ms(), 1000);
		assert!(!options.background());
		assert!(options.pauses_automatically());
		assert!(!options.show_background_indicator());

		let tuned = UpdateOptions::new(UpdateAccuracy::Navigation)
			.with_distance_filter_m(12.5)
			.unwrap()
			.with_interval_ms(5_000)
			.unwrap()
			.with_background(true)
			.with_pauses_automatically(false)
			.with_show_background_indicator(true);
		assert_eq!(UpdateOptions::decode(&tuned.encode()), Ok(tuned));

		let balanced = UpdateOptions::new(UpdateAccuracy::Balanced).with_background(true);
		assert_eq!(UpdateOptions::decode(&balanced.encode()), Ok(balanced));
		for accuracy in [
			UpdateAccuracy::Best,
			UpdateAccuracy::Navigation,
			UpdateAccuracy::Balanced,
		] {
			let options = UpdateOptions::new(accuracy);
			assert_eq!(UpdateOptions::decode(&options.encode()), Ok(options));
		}
	}
	#[test]
	fn update_options_reject_out_of_bounds_and_bad_wire() {
		for metres in [-1.0, MAX_DISTANCE_FILTER_M + 1.0, f64::NAN, f64::INFINITY] {
			assert_eq!(
				UpdateOptions::new(UpdateAccuracy::Best).with_distance_filter_m(metres),
				Err(Error::InvalidArgument)
			);
		}

		for interval in [0, MAX_INTERVAL_MS + 1] {
			assert_eq!(
				UpdateOptions::new(UpdateAccuracy::Best).with_interval_ms(interval),
				Err(Error::InvalidArgument)
			);
		}

		assert_eq!(UpdateAccuracy::from_code(3), Err(Error::InvalidArgument));

		let encoded = UpdateOptions::new(UpdateAccuracy::Best)
			.with_background(true)
			.encode();
		assert_eq!(
			UpdateOptions::decode(&encoded[..UPDATE_OPTIONS_BYTES - 1]),
			Err(Error::InvalidArgument)
		);

		for index in [0, 16, 20, 24] {
			let mut invalid = encoded;
			invalid[index] = if index == 0 { 3 } else { 2 };
			assert_eq!(UpdateOptions::decode(&invalid), Err(Error::InvalidArgument));
		}

		let mut nonfinite = encoded;
		nonfinite[4..12].copy_from_slice(&f64::NAN.to_le_bytes());
		assert_eq!(
			UpdateOptions::decode(&nonfinite),
			Err(Error::InvalidArgument)
		);
	}
	/// Builds a native batch from fixes, the way the handlers do.
	fn batch(dropped: u32, fixes: &[Fix]) -> Vec<u8> {
		let mut bytes = dropped.to_le_bytes().to_vec();
		bytes.extend_from_slice(&(fixes.len() as u32).to_le_bytes());

		for fix in fixes {
			bytes.extend_from_slice(&fix.encode().unwrap());
		}

		bytes
	}
	#[test]
	fn update_batches_round_trip_and_end_the_stream() {
		let fixes: Vec<_> = (0..3)
			.map(|index| {
				Fix {
					timestamp_ms: 1_800_000_000_000.0 + f64::from(index),
					latitude: 37.0 + f64::from(index),
					..fix()
				}
			})
			.collect();
		let payload = batch(7, &fixes);
		assert_eq!(
			decode_updates(&payload),
			Ok(Some(Updates {
				dropped: 7,
				fixes: fixes.clone()
			}))
		);
		assert_eq!(decode_updates(&payload).unwrap().unwrap().fixes, fixes);

		// An empty batch ends the stream, and only without unreported drops.
		assert_eq!(decode_updates(&batch(0, &[])), Ok(None));
		assert_eq!(decode_updates(&batch(3, &[])), Err(Error::InvalidArgument));
	}
	#[test]
	fn update_batches_reject_bad_framing_and_invalid_fixes() {
		let fixes: Vec<_> = (0..2).map(|_| Fix { ..fix() }).collect();
		let payload = batch(0, &fixes);

		for length in 0..payload.len() {
			assert!(decode_updates(&payload[..length]).is_err());
		}

		let mut oversized = (fixes.len() as u32 + 1).to_le_bytes().to_vec();
		oversized.extend_from_slice(&(UPDATE_BUFFER_CAPACITY as u32 + 1).to_le_bytes());
		oversized.resize(UPDATES_HEADER_BYTES, 0);
		assert_eq!(decode_updates(&oversized), Err(Error::InvalidArgument));

		let mut invalid_fix = payload.clone();
		invalid_fix[UPDATES_HEADER_BYTES + 16..UPDATES_HEADER_BYTES + 24]
			.copy_from_slice(&f64::NAN.to_le_bytes());
		assert_eq!(decode_updates(&invalid_fix), Err(Error::InvalidArgument));

		let mut full = batch(0, &[]);
		let count = UPDATE_BUFFER_CAPACITY as u32;
		full[4..8].copy_from_slice(&count.to_le_bytes());
		full.resize(
			UPDATES_HEADER_BYTES + UPDATE_BUFFER_CAPACITY * FIX_BYTES - FIX_BYTES,
			0,
		);
		assert_eq!(decode_updates(&full), Err(Error::InvalidArgument));
	}
	#[test]
	fn update_batches_hold_a_full_buffer_of_valid_fixes() {
		let fixes: Vec<_> = (0..UPDATE_BUFFER_CAPACITY)
			.map(|index| {
				Fix {
					timestamp_ms: index as f64,
					..fix()
				}
			})
			.collect();
		let payload = batch(UPDATE_BUFFER_CAPACITY as u32, &fixes);
		assert_eq!(
			decode_updates(&payload),
			Ok(Some(Updates {
				dropped: UPDATE_BUFFER_CAPACITY as u32,
				fixes
			}))
		);
	}
}
