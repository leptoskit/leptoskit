//! <!-- {=p_calendarSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_calendar` as a direct dependency and select the
//! `calendar` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Calendar` rather than resolving
//! the plugin on every call. Status never prompts; request permission only
//! after the user chooses. List calendars (max 64), create an event, get one
//! event by id, or list events in a time window (max 64). Purpose strings and
//! manifest permissions stay app-owned; denied access never invents calendars
//! or events. Browsers and desktop hosts report unavailable; legacy Expo
//! Calendar APIs remain out of scope.
//!
//! <!-- {/p_calendarSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "calendar";
/// Empty request; exact four-byte [`Status`] response. Never prompts.
pub const STATUS: MethodId = MethodId(0);
/// Empty request; may prompt, then returns [`Status`].
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Empty request; returns a bounded [`CalendarInfo`] list.
pub const LIST_CALENDARS: MethodId = MethodId(2);
/// [`CreateEvent`] request; returns a length-prefixed event id.
pub const CREATE_EVENT: MethodId = MethodId(3);
/// Length-prefixed event id; returns one [`Event`].
pub const GET_EVENT: MethodId = MethodId(4);
/// [`ListEvents`] request; returns a bounded [`Event`] list.
pub const LIST_EVENTS: MethodId = MethodId(5);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[
	STATUS,
	REQUEST_PERMISSION,
	LIST_CALENDARS,
	CREATE_EVENT,
	GET_EVENT,
	LIST_EVENTS,
];

/// Hard cap on calendars returned by one list.
pub const MAX_CALENDARS: u32 = 64;
/// Hard cap on events returned by one list.
pub const MAX_EVENTS: u32 = 64;
/// Maximum UTF-8 bytes for a calendar or event id.
pub const MAX_ID_BYTES: usize = 256;
/// Maximum UTF-8 bytes for a calendar or event title.
pub const MAX_TITLE_BYTES: usize = 256;
/// Maximum UTF-8 bytes for event notes.
pub const MAX_NOTES_BYTES: usize = 4_096;
/// Maximum UTF-8 bytes for an event location.
pub const MAX_LOCATION_BYTES: usize = 512;
/// Longest accepted list-events window, in milliseconds (~366 days).
pub const MAX_WINDOW_MS: i64 = 31_622_400_000;

/// Foreground calendar authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// The platform cannot distinguish more specific non-granted states.
	Unknown = 0,
	/// No choice has been made; an explicit request can prompt.
	Prompt = 1,
	/// Access denied by the user or OS.
	Denied = 2,
	/// The OS has granted calendar read and write access.
	Granted = 3,
	/// A platform restriction prevents the user granting full access.
	Restricted = 4,
}

impl Status {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Prompt),
			2 => Ok(Self::Denied),
			3 => Ok(Self::Granted),
			4 => Ok(Self::Restricted),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// One bounded calendar: id, title, and whether the host may create events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
	id: String,
	title: String,
	allows_modifications: bool,
}

impl CalendarInfo {
	/// Builds a validated calendar row.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		allows_modifications: bool,
	) -> Result<Self, Error> {
		Ok(Self {
			id: required_field(id.into(), MAX_ID_BYTES)?,
			title: required_field(title.into(), MAX_TITLE_BYTES)?,
			allows_modifications,
		})
	}

	/// Stable calendar identifier from the platform store.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// User-visible calendar title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Whether the host may create events on this calendar.
	#[must_use]
	pub fn allows_modifications(&self) -> bool {
		self.allows_modifications
	}

	/// Wire: length-prefixed id and title, then a u32 modification flag.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(12 + self.id.len() + self.title.len());
		encoder.bytes(self.id.as_bytes())?;
		encoder.bytes(self.title.as_bytes())?;
		encoder.u32(u32::from(self.allows_modifications));
		Ok(encoder.finish())
	}

	/// Validates an untrusted host/guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > max_calendar_frame() {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let calendar = Self::decode_fields(&mut decoder)?;
		decoder.finish()?;
		Ok(calendar)
	}

	fn decode_fields(decoder: &mut Decoder<'_>) -> Result<Self, Error> {
		Self::new(
			decoder.string()?.to_owned(),
			decoder.string()?.to_owned(),
			flag(decoder.u32()?)?,
		)
	}
}

/// One bounded event: ids, title, start/end unix milliseconds, optional notes
/// and location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
	id: String,
	calendar_id: String,
	title: String,
	start_ms: i64,
	end_ms: i64,
	notes: Option<String>,
	location: Option<String>,
}

impl Event {
	/// Builds a validated event. End must be at or after start.
	pub fn new(
		id: impl Into<String>,
		calendar_id: impl Into<String>,
		title: impl Into<String>,
		start_ms: i64,
		end_ms: i64,
		notes: Option<impl Into<String>>,
		location: Option<impl Into<String>>,
	) -> Result<Self, Error> {
		if end_ms < start_ms {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			id: required_field(id.into(), MAX_ID_BYTES)?,
			calendar_id: required_field(calendar_id.into(), MAX_ID_BYTES)?,
			title: required_field(title.into(), MAX_TITLE_BYTES)?,
			start_ms,
			end_ms,
			notes: optional_field(notes.map(Into::into), MAX_NOTES_BYTES)?,
			location: optional_field(location.map(Into::into), MAX_LOCATION_BYTES)?,
		})
	}

	/// Stable event identifier from the platform store.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Calendar that owns the event.
	#[must_use]
	pub fn calendar_id(&self) -> &str {
		&self.calendar_id
	}

	/// User-visible title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Inclusive start instant as unix milliseconds.
	#[must_use]
	pub fn start_ms(&self) -> i64 {
		self.start_ms
	}

	/// Inclusive end instant as unix milliseconds.
	#[must_use]
	pub fn end_ms(&self) -> i64 {
		self.end_ms
	}

	/// Notes when the platform supplied them.
	#[must_use]
	pub fn notes(&self) -> Option<&str> {
		self.notes.as_deref()
	}

	/// Location when the platform supplied one.
	#[must_use]
	pub fn location(&self) -> Option<&str> {
		self.location.as_deref()
	}

	/// Wire: length-prefixed strings around two little-endian i64 instants.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(
			28 + self.id.len()
				+ self.calendar_id.len()
				+ self.title.len()
				+ self.notes.as_ref().map_or(0, String::len)
				+ self.location.as_ref().map_or(0, String::len),
		);
		Self::encode_fields(self, &mut encoder)?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted host/guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > max_event_frame() {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let event = Self::decode_fields(&mut decoder)?;
		decoder.finish()?;
		Ok(event)
	}

	fn encode_fields(&self, encoder: &mut Encoder) -> Result<(), Error> {
		encoder.bytes(self.id.as_bytes())?;
		encoder.bytes(self.calendar_id.as_bytes())?;
		encoder.bytes(self.title.as_bytes())?;
		write_i64(encoder, self.start_ms);
		write_i64(encoder, self.end_ms);
		encoder.bytes(self.notes.as_deref().unwrap_or_default().as_bytes())?;
		encoder.bytes(self.location.as_deref().unwrap_or_default().as_bytes())?;
		Ok(())
	}

	fn decode_fields(decoder: &mut Decoder<'_>) -> Result<Self, Error> {
		Self::new(
			decoder.string()?.to_owned(),
			decoder.string()?.to_owned(),
			decoder.string()?.to_owned(),
			read_i64(decoder)?,
			read_i64(decoder)?,
			optional_decoded(decoder.string()?),
			optional_decoded(decoder.string()?),
		)
	}
}

/// Create-event request: calendar, title, start/end, optional notes/location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateEvent {
	calendar_id: String,
	title: String,
	start_ms: i64,
	end_ms: i64,
	notes: Option<String>,
	location: Option<String>,
}

impl CreateEvent {
	/// Builds a validated create request. End must be at or after start.
	pub fn new(
		calendar_id: impl Into<String>,
		title: impl Into<String>,
		start_ms: i64,
		end_ms: i64,
		notes: Option<impl Into<String>>,
		location: Option<impl Into<String>>,
	) -> Result<Self, Error> {
		if end_ms < start_ms {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			calendar_id: required_field(calendar_id.into(), MAX_ID_BYTES)?,
			title: required_field(title.into(), MAX_TITLE_BYTES)?,
			start_ms,
			end_ms,
			notes: optional_field(notes.map(Into::into), MAX_NOTES_BYTES)?,
			location: optional_field(location.map(Into::into), MAX_LOCATION_BYTES)?,
		})
	}

	/// Target calendar id.
	#[must_use]
	pub fn calendar_id(&self) -> &str {
		&self.calendar_id
	}

	/// Event title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Start instant as unix milliseconds.
	#[must_use]
	pub fn start_ms(&self) -> i64 {
		self.start_ms
	}

	/// End instant as unix milliseconds.
	#[must_use]
	pub fn end_ms(&self) -> i64 {
		self.end_ms
	}

	/// Optional notes.
	#[must_use]
	pub fn notes(&self) -> Option<&str> {
		self.notes.as_deref()
	}

	/// Optional location.
	#[must_use]
	pub fn location(&self) -> Option<&str> {
		self.location.as_deref()
	}

	/// Wire encoding of the create request.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(
			24 + self.calendar_id.len()
				+ self.title.len()
				+ self.notes.as_ref().map_or(0, String::len)
				+ self.location.as_ref().map_or(0, String::len),
		);
		encoder.bytes(self.calendar_id.as_bytes())?;
		encoder.bytes(self.title.as_bytes())?;
		write_i64(&mut encoder, self.start_ms);
		write_i64(&mut encoder, self.end_ms);
		encoder.bytes(self.notes.as_deref().unwrap_or_default().as_bytes())?;
		encoder.bytes(self.location.as_deref().unwrap_or_default().as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted create request.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > max_create_frame() {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let request = Self::new(
			decoder.string()?.to_owned(),
			decoder.string()?.to_owned(),
			read_i64(&mut decoder)?,
			read_i64(&mut decoder)?,
			optional_decoded(decoder.string()?),
			optional_decoded(decoder.string()?),
		)?;
		decoder.finish()?;
		Ok(request)
	}
}

/// List-events request: inclusive start/end window and a hard row cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListEvents {
	start_ms: i64,
	end_ms: i64,
	limit: u32,
}

impl ListEvents {
	/// Rejects inverted or oversize windows and zero/oversize limits.
	pub fn new(start_ms: i64, end_ms: i64, limit: u32) -> Result<Self, Error> {
		if end_ms < start_ms {
			return Err(Error::InvalidArgument);
		}

		let span = end_ms.checked_sub(start_ms).ok_or(Error::InvalidArgument)?;
		if span > MAX_WINDOW_MS {
			return Err(Error::InvalidArgument);
		}

		if limit == 0 || limit > MAX_EVENTS {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			start_ms,
			end_ms,
			limit,
		})
	}

	/// Window start as unix milliseconds.
	#[must_use]
	pub fn start_ms(self) -> i64 {
		self.start_ms
	}

	/// Window end as unix milliseconds.
	#[must_use]
	pub fn end_ms(self) -> i64 {
		self.end_ms
	}

	/// Maximum events the reply may contain.
	#[must_use]
	pub fn limit(self) -> u32 {
		self.limit
	}

	/// Fixed twenty-byte request.
	#[must_use]
	pub fn encode(self) -> [u8; 20] {
		let mut bytes = [0_u8; 20];
		bytes[..8].copy_from_slice(&self.start_ms.to_le_bytes());
		bytes[8..16].copy_from_slice(&self.end_ms.to_le_bytes());
		bytes[16..].copy_from_slice(&self.limit.to_le_bytes());
		bytes
	}

	/// Validates an untrusted list request.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(payload);
		let start_ms = read_i64(&mut decoder)?;
		let end_ms = read_i64(&mut decoder)?;
		let limit = decoder.u32()?;
		decoder.finish()?;
		Self::new(start_ms, end_ms, limit)
	}
}

/// Encodes a length-prefixed event id reply.
pub fn encode_event_id(id: &str) -> Result<Bytes, Error> {
	let id = required_field(id.to_owned(), MAX_ID_BYTES)?;
	let mut encoder = Encoder::with_capacity(4 + id.len());
	encoder.bytes(id.as_bytes())?;
	Ok(encoder.finish())
}

/// Decodes a length-prefixed event id reply.
pub fn decode_event_id(payload: &[u8]) -> Result<String, Error> {
	if payload.len() > 4 + MAX_ID_BYTES {
		return Err(Error::TooLarge);
	}

	let mut decoder = Decoder::new(payload);
	let id = required_field(decoder.string()?.to_owned(), MAX_ID_BYTES)?;
	decoder.finish()?;
	Ok(id)
}

/// Encodes a bounded calendar list reply.
pub fn encode_calendars(calendars: &[CalendarInfo]) -> Result<Bytes, Error> {
	if calendars.len() > MAX_CALENDARS as usize {
		return Err(Error::TooLarge);
	}

	let mut encoder = Encoder::with_capacity(4 + calendars.len() * 32);
	encoder.u32(u32::try_from(calendars.len()).map_err(|_| Error::TooLarge)?);
	for calendar in calendars {
		encoder.bytes(calendar.id.as_bytes())?;
		encoder.bytes(calendar.title.as_bytes())?;
		encoder.u32(u32::from(calendar.allows_modifications));
	}
	Ok(encoder.finish())
}

/// Decodes a bounded calendar list.
pub fn decode_calendars(payload: &[u8]) -> Result<Vec<CalendarInfo>, Error> {
	if payload.len() > 4 + max_calendar_frame() * MAX_CALENDARS as usize {
		return Err(Error::TooLarge);
	}

	let mut decoder = Decoder::new(payload);
	let count = decoder.u32()? as usize;
	if count > MAX_CALENDARS as usize {
		return Err(Error::TooLarge);
	}

	let mut calendars = Vec::with_capacity(count);
	for _ in 0..count {
		calendars.push(CalendarInfo::decode_fields(&mut decoder)?);
	}
	decoder.finish()?;
	Ok(calendars)
}

/// Encodes a bounded event list reply.
pub fn encode_events(events: &[Event]) -> Result<Bytes, Error> {
	if events.len() > MAX_EVENTS as usize {
		return Err(Error::TooLarge);
	}

	let mut encoder = Encoder::with_capacity(4 + events.len() * 48);
	encoder.u32(u32::try_from(events.len()).map_err(|_| Error::TooLarge)?);
	for event in events {
		Event::encode_fields(event, &mut encoder)?;
	}
	Ok(encoder.finish())
}

/// Decodes a bounded event list, enforcing the caller's limit.
pub fn decode_events(payload: &[u8], query: ListEvents) -> Result<Vec<Event>, Error> {
	if payload.len() > 4 + max_event_frame() * MAX_EVENTS as usize {
		return Err(Error::TooLarge);
	}

	let mut decoder = Decoder::new(payload);
	let count = decoder.u32()? as usize;
	if count > query.limit() as usize || count > MAX_EVENTS as usize {
		return Err(Error::TooLarge);
	}

	let mut events = Vec::with_capacity(count);
	for _ in 0..count {
		events.push(Event::decode_fields(&mut decoder)?);
	}
	decoder.finish()?;
	Ok(events)
}

/// Validates a method without opening the calendar store.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS | REQUEST_PERMISSION | LIST_CALENDARS if payload.is_empty() => Ok(()),
		STATUS | REQUEST_PERMISSION | LIST_CALENDARS => Err(Error::InvalidArgument),
		CREATE_EVENT => {
			CreateEvent::decode(payload)?;
			Ok(())
		}
		GET_EVENT => {
			decode_event_id(payload)?;
			Ok(())
		}
		LIST_EVENTS => {
			ListEvents::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a calendar backend.
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
	pub struct Calendar(leptoskit_bridge::guest::Client);

	impl Calendar {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads authorization without prompting.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Requests calendar access after user intent.
		pub async fn request_permission(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
		}

		/// Returns up to [`MAX_CALENDARS`] calendars. Denied access never
		/// invents rows; the host returns [`Error::PermissionDenied`].
		pub async fn list_calendars(self) -> Result<Vec<CalendarInfo>, Error> {
			decode_calendars(&self.0.call(LIST_CALENDARS, &[])?.await?)
		}

		/// Creates one event and returns its id.
		pub async fn create_event(self, request: &CreateEvent) -> Result<String, Error> {
			decode_event_id(&self.0.call(CREATE_EVENT, &request.encode()?)?.await?)
		}

		/// Loads one event by id.
		pub async fn get_event(self, id: &str) -> Result<Event, Error> {
			Event::decode(&self.0.call(GET_EVENT, &encode_event_id(id)?)?.await?)
		}

		/// Returns up to `query.limit()` events in the time window.
		pub async fn list_events(self, query: ListEvents) -> Result<Vec<Event>, Error> {
			decode_events(&self.0.call(LIST_EVENTS, &query.encode())?.await?, query)
		}
	}
}

/// Browser builds have no calendar backend in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unavailable; never invents calendars.
	pub fn status() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; no synthetic consent is invented.
	pub fn request_permission() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; never invents calendars.
	pub fn list_calendars() -> Result<Vec<CalendarInfo>, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; never invents events.
	pub fn create_event(_request: &CreateEvent) -> Result<String, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; never invents events.
	pub fn get_event(_id: &str) -> Result<Event, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; never invents events.
	pub fn list_events(_query: ListEvents) -> Result<Vec<Event>, Error> {
		Err(Error::Unavailable)
	}
}

fn max_calendar_frame() -> usize {
	12 + MAX_ID_BYTES + MAX_TITLE_BYTES
}

fn max_event_frame() -> usize {
	28 + 2 * MAX_ID_BYTES + MAX_TITLE_BYTES + MAX_NOTES_BYTES + MAX_LOCATION_BYTES
}

fn max_create_frame() -> usize {
	24 + MAX_ID_BYTES + MAX_TITLE_BYTES + MAX_NOTES_BYTES + MAX_LOCATION_BYTES
}

fn optional_decoded(value: &str) -> Option<String> {
	(!value.is_empty()).then(|| value.to_owned())
}

fn required_field(value: String, max: usize) -> Result<String, Error> {
	if value.is_empty() || value.len() > max || value.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

fn optional_field(value: Option<String>, max: usize) -> Result<Option<String>, Error> {
	match value {
		None => Ok(None),
		Some(value) if value.is_empty() => Err(Error::InvalidArgument),
		Some(value) if value.len() > max || value.contains('\0') => Err(Error::InvalidArgument),
		Some(value) => Ok(Some(value)),
	}
}

fn flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

fn write_i64(encoder: &mut Encoder, value: i64) {
	let bits = value as u64;
	encoder.u32(bits as u32);
	encoder.u32((bits >> 32) as u32);
}

fn read_i64(decoder: &mut Decoder<'_>) -> Result<i64, Error> {
	let low = u64::from(decoder.u32()?);
	let high = u64::from(decoder.u32()?);
	Ok(((high << 32) | low) as i64)
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	fn sample_calendar() -> CalendarInfo {
		CalendarInfo::new("cal-1", "Work", true).unwrap()
	}

	fn sample_event() -> Event {
		Event::new(
			"evt-1",
			"cal-1",
			"Standup",
			1_700_000_000_000,
			1_700_000_360_000,
			Some("daily"),
			Some("Room A"),
		)
		.unwrap()
	}

	fn sample_create() -> CreateEvent {
		CreateEvent::new(
			"cal-1",
			"Standup",
			1_700_000_000_000,
			1_700_000_360_000,
			Some("daily"),
			Some("Room A"),
		)
		.unwrap()
	}

	#[test]
	fn status_rejects_unknown_values_truncation_and_trailing_bytes() {
		for (value, expected) in [
			(0, Status::Unknown),
			(1, Status::Prompt),
			(2, Status::Denied),
			(3, Status::Granted),
			(4, Status::Restricted),
		] {
			assert_eq!(Status::decode(&u32::to_le_bytes(value)), Ok(expected));
			assert_eq!(expected.encode(), u32::to_le_bytes(value));
		}

		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[5, 0, 0, 0]] {
			assert_eq!(Status::decode(bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn calendar_round_trips_and_rejects_nul_empty_and_oversize() {
		let calendar = sample_calendar();
		assert_eq!(
			CalendarInfo::decode(&calendar.encode().unwrap()).unwrap(),
			calendar
		);
		assert_eq!(calendar.id(), "cal-1");
		assert_eq!(calendar.title(), "Work");
		assert!(calendar.allows_modifications());

		assert_eq!(
			CalendarInfo::new("", "Work", false),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CalendarInfo::new("id", "", false),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CalendarInfo::new("id\0", "Work", false),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CalendarInfo::new("id", "ti\0tle", false),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CalendarInfo::new("i".repeat(MAX_ID_BYTES + 1), "Work", false),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CalendarInfo::new("id", "t".repeat(MAX_TITLE_BYTES + 1), false),
			Err(Error::InvalidArgument)
		);

		let mut encoder = Encoder::with_capacity(32);
		encoder.bytes(b"id").unwrap();
		encoder.bytes(b"title").unwrap();
		encoder.u32(2);
		assert_eq!(
			CalendarInfo::decode(&encoder.finish()),
			Err(Error::InvalidArgument)
		);

		let bytes = calendar.encode().unwrap();
		for end in 0..bytes.len() {
			assert!(CalendarInfo::decode(&bytes[..end]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(CalendarInfo::decode(&trailing).is_err());
		assert_eq!(
			CalendarInfo::decode(&vec![0_u8; max_calendar_frame() + 1]),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn event_and_create_round_trips_and_reject_bad_fields() {
		let event = sample_event();
		assert_eq!(Event::decode(&event.encode().unwrap()).unwrap(), event);
		assert_eq!(event.id(), "evt-1");
		assert_eq!(event.calendar_id(), "cal-1");
		assert_eq!(event.title(), "Standup");
		assert_eq!(event.notes(), Some("daily"));
		assert_eq!(event.location(), Some("Room A"));
		assert_eq!(event.start_ms(), 1_700_000_000_000);
		assert_eq!(event.end_ms(), 1_700_000_360_000);

		let minimal = Event::new("a", "b", "c", 1, 1, None::<String>, None::<String>).unwrap();
		assert_eq!(minimal.notes(), None);
		assert_eq!(minimal.location(), None);
		assert_eq!(Event::decode(&minimal.encode().unwrap()).unwrap(), minimal);

		assert_eq!(
			Event::new("a", "b", "c", 2, 1, None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Event::new("a\0", "b", "c", 1, 2, None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Event::new("a", "b", "c", 1, 2, Some(""), None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Event::new(
				"a",
				"b",
				"c",
				1,
				2,
				Some("n".repeat(MAX_NOTES_BYTES + 1)),
				None::<String>
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Event::decode(&vec![0_u8; max_event_frame() + 1]),
			Err(Error::TooLarge)
		);

		let create = sample_create();
		assert_eq!(
			CreateEvent::decode(&create.encode().unwrap()).unwrap(),
			create
		);
		assert_eq!(create.calendar_id(), "cal-1");
		assert_eq!(create.title(), "Standup");
		assert_eq!(create.start_ms(), 1_700_000_000_000);
		assert_eq!(create.end_ms(), 1_700_000_360_000);
		assert_eq!(create.notes(), Some("daily"));
		assert_eq!(create.location(), Some("Room A"));
		assert_eq!(
			CreateEvent::new("a", "b", 2, 1, None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		let mut inverted = Encoder::with_capacity(32);
		inverted.bytes(b"cal").unwrap();
		inverted.bytes(b"title").unwrap();
		write_i64(&mut inverted, 2);
		write_i64(&mut inverted, 1);
		inverted.bytes(b"").unwrap();
		inverted.bytes(b"").unwrap();
		assert_eq!(
			CreateEvent::decode(&inverted.finish()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			CreateEvent::decode(&vec![0_u8; max_create_frame() + 1]),
			Err(Error::TooLarge)
		);

		let id = encode_event_id("evt-9").unwrap();
		assert_eq!(decode_event_id(&id).unwrap(), "evt-9");
		assert_eq!(decode_event_id(b""), Err(Error::InvalidArgument));
		assert_eq!(
			decode_event_id(&vec![0_u8; 5 + MAX_ID_BYTES]),
			Err(Error::TooLarge)
		);
		assert_eq!(encode_event_id(""), Err(Error::InvalidArgument));
		assert_eq!(
			encode_event_id(&"x".repeat(MAX_ID_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn list_codecs_reject_trailers_caps_and_oversize_windows() {
		assert_eq!(ListEvents::new(2, 1, 1), Err(Error::InvalidArgument));
		assert_eq!(
			ListEvents::new(0, MAX_WINDOW_MS + 1, 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(ListEvents::new(0, 1, 0), Err(Error::InvalidArgument));
		assert_eq!(
			ListEvents::new(0, 1, MAX_EVENTS + 1),
			Err(Error::InvalidArgument)
		);
		let query = ListEvents::new(10, 20, 3).unwrap();
		assert_eq!(query.start_ms(), 10);
		assert_eq!(query.end_ms(), 20);
		assert_eq!(query.limit(), 3);
		assert_eq!(ListEvents::decode(&query.encode()), Ok(query));
		assert_eq!(ListEvents::decode(&[1, 0]), Err(Error::InvalidArgument));
		let mut trailing = query.encode().to_vec();
		trailing.push(0);
		assert_eq!(ListEvents::decode(&trailing), Err(Error::InvalidArgument));

		let calendars = vec![
			sample_calendar(),
			CalendarInfo::new("2", "Home", false).unwrap(),
		];
		let encoded = encode_calendars(&calendars).unwrap();
		assert_eq!(decode_calendars(&encoded).unwrap(), calendars);
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(decode_calendars(&trailing).is_err());
		for end in 0..encoded.len() {
			assert!(decode_calendars(&encoded[..end]).is_err());
		}
		let too_many = (0..=MAX_CALENDARS)
			.map(|index| CalendarInfo::new(index.to_string(), "n", false).unwrap())
			.collect::<Vec<_>>();
		assert_eq!(encode_calendars(&too_many), Err(Error::TooLarge));
		assert_eq!(
			decode_calendars(&vec![
				0_u8;
				5 + max_calendar_frame() * MAX_CALENDARS as usize
			]),
			Err(Error::TooLarge)
		);
		let mut over_count = (MAX_CALENDARS + 1).to_le_bytes().to_vec();
		assert_eq!(decode_calendars(&over_count), Err(Error::TooLarge));
		over_count = 1u32.to_le_bytes().to_vec();
		over_count.extend_from_slice(&sample_calendar().encode().unwrap());
		over_count.push(0);
		assert!(decode_calendars(&over_count).is_err());

		let events = vec![
			sample_event(),
			Event::new("2", "cal-1", "Lunch", 1, 2, None::<String>, None::<String>).unwrap(),
		];
		let encoded = encode_events(&events).unwrap();
		assert_eq!(
			decode_events(&encoded, ListEvents::new(0, 10, 2).unwrap()).unwrap(),
			events
		);
		assert_eq!(
			decode_events(&encoded, ListEvents::new(0, 10, 1).unwrap()),
			Err(Error::TooLarge)
		);
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(decode_events(&trailing, ListEvents::new(0, 10, 2).unwrap()).is_err());
		for end in 0..encoded.len() {
			assert!(decode_events(&encoded[..end], ListEvents::new(0, 10, 2).unwrap()).is_err());
		}
		let too_many = (0..=MAX_EVENTS)
			.map(|index| {
				Event::new(
					index.to_string(),
					"c",
					"t",
					0,
					1,
					None::<String>,
					None::<String>,
				)
				.unwrap()
			})
			.collect::<Vec<_>>();
		assert_eq!(encode_events(&too_many), Err(Error::TooLarge));
		assert_eq!(
			decode_events(
				&vec![0_u8; 5 + max_event_frame() * MAX_EVENTS as usize],
				ListEvents::new(0, 1, MAX_EVENTS).unwrap()
			),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn validate_and_unsupported_never_invent_events() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(REQUEST_PERMISSION, &[]), Ok(()));
		assert_eq!(validate(LIST_CALENDARS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(REQUEST_PERMISSION, &[0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(validate(LIST_CALENDARS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(CREATE_EVENT, &sample_create().encode().unwrap()),
			Ok(())
		);
		assert_eq!(validate(CREATE_EVENT, &[]), Err(Error::InvalidArgument));
		assert_eq!(validate(GET_EVENT, &encode_event_id("1").unwrap()), Ok(()));
		assert_eq!(validate(GET_EVENT, &[]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(LIST_EVENTS, &ListEvents::new(0, 1, 8).unwrap().encode()),
			Ok(())
		);
		assert_eq!(validate(LIST_EVENTS, &[]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		for method in METHODS {
			let payload = match *method {
				CREATE_EVENT => sample_create().encode().unwrap(),
				GET_EVENT => encode_event_id("1").unwrap(),
				LIST_EVENTS => Bytes::copy_from_slice(&ListEvents::new(0, 1, 1).unwrap().encode()),
				_ => Bytes::new(),
			};
			assert_eq!(plugin.invoke(*method, payload), Err(Error::Unavailable));
		}
		assert_eq!(NAME, "calendar");
		assert_eq!(MAX_CALENDARS, 64);
		assert_eq!(MAX_EVENTS, 64);
	}

	#[test]
	fn event_decode_rejects_nul_inside_valid_framing() {
		let mut encoder = Encoder::with_capacity(64);
		encoder.bytes(b"a\0b").unwrap();
		encoder.bytes(b"cal").unwrap();
		encoder.bytes(b"title").unwrap();
		write_i64(&mut encoder, 1);
		write_i64(&mut encoder, 2);
		encoder.bytes(b"").unwrap();
		encoder.bytes(b"").unwrap();
		assert_eq!(
			Event::decode(&encoder.finish()),
			Err(Error::InvalidArgument)
		);
	}
}
