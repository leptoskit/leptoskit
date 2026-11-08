//! <!-- {=p_contactsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_contacts` as a direct dependency and select the
//! `contacts` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Contacts` rather than resolving
//! the plugin on every call. Status never prompts; request permission only
//! after the user chooses. Pick presents the system contact picker for
//! one bounded contact. Query returns at most 32 contacts with id, display
//! name, and optional primary phone/email. Purpose strings and manifest
//! permissions stay app-owned; denied access never invents contacts. Browsers
//! report unavailable; the Contacts Picker API remains later scope.
//!
//! <!-- {/p_contactsSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "contacts";
/// Empty request; exact four-byte [`Status`] response. Never prompts.
pub const STATUS: MethodId = MethodId(0);
/// Empty request; may prompt, then returns [`Status`].
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Empty request; presents the system picker and returns one [`Contact`].
pub const PICK: MethodId = MethodId(2);
/// One little-endian `u32` limit; returns a bounded [`Contact`] list.
pub const QUERY: MethodId = MethodId(3);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[STATUS, REQUEST_PERMISSION, PICK, QUERY];

/// Hard cap on contacts returned by one query.
pub const MAX_QUERY: u32 = 32;
/// Maximum UTF-8 bytes for a contact id.
pub const MAX_ID_BYTES: usize = 256;
/// Maximum UTF-8 bytes for a display name.
pub const MAX_NAME_BYTES: usize = 256;
/// Maximum UTF-8 bytes for a phone number field.
pub const MAX_PHONE_BYTES: usize = 64;
/// Maximum UTF-8 bytes for an email field.
pub const MAX_EMAIL_BYTES: usize = 320;

/// Foreground contacts authorization, distinct from picker availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// The platform cannot distinguish more specific non-granted states.
	Unknown = 0,
	/// No choice has been made; an explicit request can prompt.
	Prompt = 1,
	/// Access denied by the user or OS.
	Denied = 2,
	/// The OS has granted contacts access.
	Granted = 3,
	/// A platform restriction prevents the user granting access.
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

/// One bounded contact: id, display name, optional primary phone and email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
	id: String,
	display_name: String,
	phone: Option<String>,
	email: Option<String>,
}

impl Contact {
	/// Builds a validated contact. Id and display name are required and
	/// non-empty; phone and email may be omitted.
	pub fn new(
		id: impl Into<String>,
		display_name: impl Into<String>,
		phone: Option<impl Into<String>>,
		email: Option<impl Into<String>>,
	) -> Result<Self, Error> {
		Ok(Self {
			id: required_field(id.into(), MAX_ID_BYTES)?,
			display_name: required_field(display_name.into(), MAX_NAME_BYTES)?,
			phone: optional_field(phone.map(Into::into), MAX_PHONE_BYTES)?,
			email: optional_field(email.map(Into::into), MAX_EMAIL_BYTES)?,
		})
	}

	/// Stable contact identifier from the platform address book.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// User-visible display name.
	#[must_use]
	pub fn display_name(&self) -> &str {
		&self.display_name
	}

	/// Primary phone when the platform supplied one.
	#[must_use]
	pub fn phone(&self) -> Option<&str> {
		self.phone.as_deref()
	}

	/// Primary email when the platform supplied one.
	#[must_use]
	pub fn email(&self) -> Option<&str> {
		self.email.as_deref()
	}

	/// Wire: length-prefixed id, name, phone, email (empty optional fields).
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(
			16 + self.id.len()
				+ self.display_name.len()
				+ self.phone.as_ref().map_or(0, String::len)
				+ self.email.as_ref().map_or(0, String::len),
		);
		encoder.bytes(self.id.as_bytes())?;
		encoder.bytes(self.display_name.as_bytes())?;
		encoder.bytes(self.phone.as_deref().unwrap_or_default().as_bytes())?;
		encoder.bytes(self.email.as_deref().unwrap_or_default().as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted host/guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > max_contact_frame() {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let contact = Self::new(
			decoder.string()?.to_owned(),
			decoder.string()?.to_owned(),
			optional_decoded(decoder.string()?),
			optional_decoded(decoder.string()?),
		)?;
		decoder.finish()?;
		Ok(contact)
	}
}

/// Validated query limit in `1..=MAX_QUERY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
	limit: u32,
}

impl Query {
	/// Rejects zero and limits above the hard cap.
	pub fn new(limit: u32) -> Result<Self, Error> {
		if limit == 0 || limit > MAX_QUERY {
			return Err(Error::InvalidArgument);
		}

		Ok(Self { limit })
	}

	/// Maximum contacts the reply may contain.
	#[must_use]
	pub fn limit(self) -> u32 {
		self.limit
	}

	/// Fixed four-byte request.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		self.limit.to_le_bytes()
	}

	/// Validates an untrusted query request.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(payload);
		let limit = decoder.u32()?;
		decoder.finish()?;
		Self::new(limit)
	}
}

/// Encodes a bounded contact list reply.
pub fn encode_contacts(contacts: &[Contact]) -> Result<Bytes, Error> {
	if contacts.len() > MAX_QUERY as usize {
		return Err(Error::TooLarge);
	}

	let mut encoder = Encoder::with_capacity(4 + contacts.len() * 32);
	encoder.u32(u32::try_from(contacts.len()).map_err(|_| Error::TooLarge)?);
	for contact in contacts {
		encoder.bytes(contact.id.as_bytes())?;
		encoder.bytes(contact.display_name.as_bytes())?;
		encoder.bytes(contact.phone.as_deref().unwrap_or_default().as_bytes())?;
		encoder.bytes(contact.email.as_deref().unwrap_or_default().as_bytes())?;
	}
	Ok(encoder.finish())
}

/// Decodes a bounded contact list, enforcing the caller's limit.
pub fn decode_contacts(payload: &[u8], query: Query) -> Result<Vec<Contact>, Error> {
	if payload.len() > 4 + max_contact_frame() * MAX_QUERY as usize {
		return Err(Error::TooLarge);
	}

	let mut decoder = Decoder::new(payload);
	let count = decoder.u32()? as usize;
	if count > query.limit() as usize || count > MAX_QUERY as usize {
		return Err(Error::TooLarge);
	}

	let mut contacts = Vec::with_capacity(count);
	for _ in 0..count {
		let id = decoder.string()?.to_owned();
		let display_name = decoder.string()?.to_owned();
		let phone = optional_decoded(decoder.string()?);
		let email = optional_decoded(decoder.string()?);
		contacts.push(Contact::new(id, display_name, phone, email)?);
	}
	decoder.finish()?;
	Ok(contacts)
}

fn max_contact_frame() -> usize {
	16 + MAX_ID_BYTES + MAX_NAME_BYTES + MAX_PHONE_BYTES + MAX_EMAIL_BYTES
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

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Validates a method without opening the address book or picker.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS | REQUEST_PERMISSION | PICK if payload.is_empty() => Ok(()),
		STATUS | REQUEST_PERMISSION | PICK => Err(Error::InvalidArgument),
		QUERY => {
			Query::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a contacts backend.
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
	pub struct Contacts(leptoskit_bridge::guest::Client);

	impl Contacts {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads authorization without prompting.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Requests contacts access after user intent.
		pub async fn request_permission(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
		}

		/// Presents the system contact picker for one contact. Cancel is
		/// honest: dismiss without a selection returns [`Error::Cancelled`].
		pub async fn pick(self) -> Result<Contact, Error> {
			Contact::decode(&self.0.call(PICK, &[])?.await?)
		}

		/// Returns up to `query.limit()` contacts. Denied access never invents
		/// rows; the host returns [`Error::PermissionDenied`].
		pub async fn query(self, query: Query) -> Result<Vec<Contact>, Error> {
			decode_contacts(&self.0.call(QUERY, &query.encode())?.await?, query)
		}
	}
}

/// Browser builds have no contacts backend in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unavailable; Contacts Picker API remains later scope.
	pub fn status() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; no synthetic consent is invented.
	pub fn request_permission() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable without opening a picker.
	pub fn pick() -> Result<Contact, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable; never invents contacts.
	pub fn query(_query: Query) -> Result<Vec<Contact>, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	fn sample() -> Contact {
		Contact::new(
			"id-1",
			"Ada Lovelace",
			Some("+1 555 0100"),
			Some("ada@example.com"),
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
	fn contact_round_trips_and_rejects_nul_empty_and_oversize_fields() {
		let contact = sample();
		assert_eq!(
			Contact::decode(&contact.encode().unwrap()).unwrap(),
			contact
		);
		assert_eq!(contact.id(), "id-1");
		assert_eq!(contact.display_name(), "Ada Lovelace");
		assert_eq!(contact.phone(), Some("+1 555 0100"));
		assert_eq!(contact.email(), Some("ada@example.com"));

		let minimal = Contact::new("a", "b", None::<String>, None::<String>).unwrap();
		assert_eq!(minimal.phone(), None);
		assert_eq!(minimal.email(), None);
		assert_eq!(
			Contact::decode(&minimal.encode().unwrap()).unwrap(),
			minimal
		);

		assert_eq!(
			Contact::new("", "name", None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "", None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "name", Some(""), None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "name", None::<String>, Some("")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id\0", "name", None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "na\0me", None::<String>, None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "name", Some("55\0x"), None::<String>),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new("id", "name", None::<String>, Some("a\0b")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new(
				"i".repeat(MAX_ID_BYTES + 1),
				"name",
				None::<String>,
				None::<String>
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new(
				"id",
				"n".repeat(MAX_NAME_BYTES + 1),
				None::<String>,
				None::<String>
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new(
				"id",
				"name",
				Some("1".repeat(MAX_PHONE_BYTES + 1)),
				None::<String>
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Contact::new(
				"id",
				"name",
				None::<String>,
				Some("e".repeat(MAX_EMAIL_BYTES + 1))
			),
			Err(Error::InvalidArgument)
		);

		let bytes = contact.encode().unwrap();
		for end in 0..bytes.len() {
			assert!(Contact::decode(&bytes[..end]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Contact::decode(&trailing).is_err());
	}

	#[test]
	fn contact_decode_rejects_oversized_frames() {
		let oversized = vec![0_u8; max_contact_frame() + 1];
		assert_eq!(Contact::decode(&oversized), Err(Error::TooLarge));
	}

	#[test]
	fn contact_decode_rejects_nul_inside_valid_framing() {
		let mut encoder = Encoder::with_capacity(32);
		encoder.bytes(b"a\0b").unwrap();
		encoder.bytes(b"name").unwrap();
		encoder.bytes(b"").unwrap();
		encoder.bytes(b"").unwrap();
		assert_eq!(
			Contact::decode(&encoder.finish()),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn query_bounds_and_list_codec_reject_trailers_and_oversize() {
		assert_eq!(Query::new(0), Err(Error::InvalidArgument));
		assert_eq!(Query::new(MAX_QUERY + 1), Err(Error::InvalidArgument));
		let query = Query::new(3).unwrap();
		assert_eq!(query.limit(), 3);
		assert_eq!(Query::decode(&query.encode()), Ok(query));
		assert_eq!(Query::decode(&[1, 0]), Err(Error::InvalidArgument));
		assert_eq!(
			Query::decode(&{
				let mut trailing = query.encode().to_vec();
				trailing.push(0);
				trailing
			}),
			Err(Error::InvalidArgument)
		);

		let contacts = vec![
			sample(),
			Contact::new("2", "Bob", None::<String>, None::<String>).unwrap(),
		];
		let encoded = encode_contacts(&contacts).unwrap();
		assert_eq!(
			decode_contacts(&encoded, Query::new(2).unwrap()).unwrap(),
			contacts
		);
		assert_eq!(
			decode_contacts(&encoded, Query::new(1).unwrap()),
			Err(Error::TooLarge)
		);

		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(decode_contacts(&trailing, Query::new(2).unwrap()).is_err());

		for end in 0..encoded.len() {
			assert!(decode_contacts(&encoded[..end], Query::new(2).unwrap()).is_err());
		}

		let too_many = (0..=MAX_QUERY)
			.map(|index| {
				Contact::new(index.to_string(), "n", None::<String>, None::<String>).unwrap()
			})
			.collect::<Vec<_>>();
		assert_eq!(encode_contacts(&too_many), Err(Error::TooLarge));

		let oversized = vec![0_u8; 5 + max_contact_frame() * MAX_QUERY as usize];
		assert_eq!(
			decode_contacts(&oversized, Query::new(MAX_QUERY).unwrap()),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn validate_and_unsupported_never_invent_contacts() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(REQUEST_PERMISSION, &[]), Ok(()));
		assert_eq!(validate(PICK, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(REQUEST_PERMISSION, &[0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(validate(PICK, &[0]), Err(Error::InvalidArgument));
		assert_eq!(validate(QUERY, &Query::new(8).unwrap().encode()), Ok(()));
		assert_eq!(validate(QUERY, &[]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		for method in METHODS {
			let payload = if *method == QUERY {
				Bytes::copy_from_slice(&Query::new(1).unwrap().encode())
			} else {
				Bytes::new()
			};
			assert_eq!(plugin.invoke(*method, payload), Err(Error::Unavailable));
		}
		assert_eq!(NAME, "contacts");
		assert_eq!(MAX_QUERY, 32);
	}

	#[test]
	fn decode_contacts_rejects_count_above_hard_cap() {
		let mut payload = (MAX_QUERY + 1).to_le_bytes().to_vec();
		assert_eq!(
			decode_contacts(&payload, Query::new(MAX_QUERY).unwrap()),
			Err(Error::TooLarge)
		);
		payload = 1u32.to_le_bytes().to_vec();
		payload.extend_from_slice(&sample().encode().unwrap());
		payload.push(0);
		assert!(decode_contacts(&payload, Query::new(1).unwrap()).is_err());
	}
}
