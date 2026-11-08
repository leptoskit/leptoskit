//! <!-- {=p_smsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_sms` as a direct dependency and select the `sms`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Sms` rather than resolving the plugin
//! on every call. Completion means the OS finished its composer callback; it
//! never confirms delivery. Outbound `sms:` URLs remain
//! `leptoskit_plugin_linking`.
//!
//! <!-- {/p_smsSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "sms";
/// Empty request; exact four-byte [`Availability`] response.
pub const AVAILABILITY: MethodId = MethodId(0);
/// [`Compose`] request; empty reply after the OS composer callback.
pub const PRESENT: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, PRESENT];

/// Maximum recipients in one compose request.
pub const MAX_RECIPIENTS: usize = 32;
/// Maximum UTF-8 bytes per recipient.
pub const MAX_RECIPIENT_BYTES: usize = 64;
/// Maximum UTF-8 body size.
pub const MAX_BODY_BYTES: usize = 8 * 1024;

/// Whether this implementation can present an SMS composer now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no SMS composer backend.
	Unsupported = 0,
	/// The device or OS reports that SMS composition is not possible.
	Absent = 1,
	/// An owned presentation context can host the composer.
	Ready = 2,
	/// The app's owned presentation context is detached or inactive.
	Inactive = 3,
	/// A prior composer flow has not finished.
	Busy = 4,
}

impl Availability {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::Absent),
			2 => Ok(Self::Ready),
			3 => Ok(Self::Inactive),
			4 => Ok(Self::Busy),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Validated recipients and optional body for one composer presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compose {
	recipients: Vec<String>,
	body: String,
}

impl Compose {
	/// Builds a validated compose request. Recipients may be empty so the
	/// user can fill them in; each recipient is a non-empty phone address.
	pub fn new(
		recipients: impl IntoIterator<Item = impl Into<String>>,
		body: impl Into<String>,
	) -> Result<Self, Error> {
		let recipients = recipients
			.into_iter()
			.map(|recipient| validate_recipient(recipient.into()))
			.collect::<Result<Vec<_>, _>>()?;

		if recipients.len() > MAX_RECIPIENTS {
			return Err(Error::TooLarge);
		}

		let body = validate_body(body.into())?;
		Ok(Self { recipients, body })
	}

	/// Prefill recipients; never a delivery destination confirmation.
	#[must_use]
	pub fn recipients(&self) -> &[String] {
		&self.recipients
	}

	/// Prefill body text.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Wire: recipient count, length-prefixed recipients, length-prefixed body.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(
			8 + self
				.recipients
				.iter()
				.map(|recipient| 4 + recipient.len())
				.sum::<usize>()
				+ self.body.len(),
		);
		encoder.u32(u32::try_from(self.recipients.len()).map_err(|_| Error::TooLarge)?);

		for recipient in &self.recipients {
			encoder.bytes(recipient.as_bytes())?;
		}

		encoder.bytes(self.body.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_BODY_BYTES + MAX_RECIPIENTS * (MAX_RECIPIENT_BYTES + 4) + 8 {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let count = decoder.u32()? as usize;

		if count > MAX_RECIPIENTS {
			return Err(Error::TooLarge);
		}

		let mut recipients = Vec::with_capacity(count);

		for _ in 0..count {
			recipients.push(validate_recipient(decoder.string()?.to_owned())?);
		}

		let body = validate_body(decoder.string()?.to_owned())?;
		decoder.finish()?;
		Ok(Self { recipients, body })
	}
}

fn validate_recipient(value: String) -> Result<String, Error> {
	if value.is_empty() || value.len() > MAX_RECIPIENT_BYTES {
		return Err(Error::InvalidArgument);
	}

	if !value
		.chars()
		.all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '-' | ' ' | '(' | ')'))
	{
		return Err(Error::InvalidArgument);
	}

	if !value.chars().any(|ch| ch.is_ascii_digit()) {
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

fn validate_body(value: String) -> Result<String, Error> {
	if value.len() > MAX_BODY_BYTES {
		return Err(Error::TooLarge);
	}

	if value.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Validates a method without opening a composer.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		AVAILABILITY if payload.is_empty() => Ok(()),
		AVAILABILITY => Err(Error::InvalidArgument),
		PRESENT => {
			Compose::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without an SMS composer.
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
		if method == AVAILABILITY {
			Ok(Bytes::copy_from_slice(&Availability::Unsupported.encode()))
		} else {
			Err(Error::Unavailable)
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Sms(leptoskit_bridge::guest::Client);

	impl Sms {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries composer readiness without presenting UI.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Presents the system SMS composer. An empty reply means the OS
		/// finished its callback, never that a message was delivered.
		pub async fn present(self, compose: &Compose) -> Result<(), Error> {
			let reply = self.0.call(PRESENT, &compose.encode()?)?.await?;

			if reply.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

/// Browser builds have no SMS composer.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Browsers have no supported SMS composer.
	#[must_use]
	pub const fn availability() -> Availability {
		Availability::Unsupported
	}

	/// Always returns Unavailable without opening a page.
	pub fn present(_compose: &Compose) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn availability_rejects_unknown_values_truncation_and_trailing_bytes() {
		for (value, expected) in [
			(0, Availability::Unsupported),
			(1, Availability::Absent),
			(2, Availability::Ready),
			(3, Availability::Inactive),
			(4, Availability::Busy),
		] {
			assert_eq!(Availability::decode(&u32::to_le_bytes(value)), Ok(expected));
			assert_eq!(expected.encode(), u32::to_le_bytes(value));
		}

		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[5, 0, 0, 0]] {
			assert_eq!(Availability::decode(bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn compose_round_trips_and_rejects_bad_recipients_and_bodies() {
		let compose = Compose::new(["+1 (555) 010-0200", "5550100"], "hello\n世界").unwrap();
		assert_eq!(
			Compose::decode(&compose.encode().unwrap()).unwrap(),
			compose
		);
		assert_eq!(compose.recipients().len(), 2);
		assert_eq!(compose.body(), "hello\n世界");

		let empty = Compose::new(None::<String>, "").unwrap();
		assert_eq!(empty.recipients().len(), 0);
		assert_eq!(Compose::decode(&empty.encode().unwrap()).unwrap(), empty);

		for bad in [
			"",
			"abc",
			"+-",
			"555\0",
			&"1".repeat(MAX_RECIPIENT_BYTES + 1),
		] {
			assert_eq!(
				Compose::new([bad], ""),
				Err(Error::InvalidArgument),
				"{bad}"
			);
		}

		assert_eq!(
			Compose::new(["1"; MAX_RECIPIENTS + 1], ""),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Compose::new(["1"], "a".repeat(MAX_BODY_BYTES + 1)),
			Err(Error::TooLarge)
		);
		assert_eq!(Compose::new(["1"], "a\0b"), Err(Error::InvalidArgument));

		let bytes = compose.encode().unwrap();
		for end in 0..bytes.len() {
			assert!(Compose::decode(&bytes[..end]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Compose::decode(&trailing).is_err());
	}

	#[test]
	fn validate_and_unsupported_never_present() {
		assert_eq!(validate(AVAILABILITY, &[]), Ok(()));
		assert_eq!(validate(AVAILABILITY, &[0]), Err(Error::InvalidArgument));
		let payload = Compose::new(["555"], "hi").unwrap().encode().unwrap();
		assert_eq!(validate(PRESENT, &payload), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Availability::decode(&plugin.invoke(AVAILABILITY, Bytes::new()).unwrap()).unwrap(),
			Availability::Unsupported
		);
		assert_eq!(plugin.invoke(PRESENT, payload), Err(Error::Unavailable));
	}

	#[test]
	fn decode_rejects_oversized_frames_and_recipient_counts() {
		let oversized = vec![0_u8; MAX_BODY_BYTES + MAX_RECIPIENTS * (MAX_RECIPIENT_BYTES + 4) + 9];
		assert_eq!(Compose::decode(&oversized), Err(Error::TooLarge));

		let mut too_many = (MAX_RECIPIENTS as u32 + 1).to_le_bytes().to_vec();
		too_many.extend_from_slice(&0_u32.to_le_bytes());
		assert_eq!(Compose::decode(&too_many), Err(Error::TooLarge));
	}
}
