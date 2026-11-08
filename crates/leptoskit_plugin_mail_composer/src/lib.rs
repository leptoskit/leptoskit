//! <!-- {=p_mail_composerSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_mail_composer` as a direct dependency and select the
//! `mail-composer` plugin in
//! `native.plugins`; supported generated hosts discover it from Cargo metadata.
//! Generated Linux hosts register it where it has a Linux Rust implementation;
//! generated Windows and terminal hosts currently omit runtime plugins, so use
//! explicit registration in a custom host there. Guests enable the `guest`
//! feature and reuse `guest::MailComposer` rather than resolving the plugin on
//! every call. Completion means the OS finished its composer callback; it never
//! confirms delivery. Attachments are deferred in this MVP. Outbound `mailto:`
//! URLs remain `leptoskit_plugin_linking`.
//!
//! <!-- {/p_mail_composerSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "mail-composer";
/// Empty request; exact four-byte [`Availability`] response.
pub const AVAILABILITY: MethodId = MethodId(0);
/// [`Compose`] request; empty reply after the OS composer callback.
pub const PRESENT: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, PRESENT];

/// Maximum addresses in each of to, cc and bcc.
pub const MAX_ADDRESSES: usize = 32;
/// Maximum UTF-8 bytes per address.
pub const MAX_ADDRESS_BYTES: usize = 320;
/// Maximum UTF-8 subject size.
pub const MAX_SUBJECT_BYTES: usize = 998;
/// Maximum UTF-8 body size.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Whether this implementation can present a mail composer now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no mail composer backend.
	Unsupported = 0,
	/// The device or OS reports that mail composition is not possible.
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

/// Validated to/cc/bcc, subject and body for one composer presentation.
///
/// Attachments are intentionally omitted from this MVP; see the crate readme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compose {
	to: Vec<String>,
	cc: Vec<String>,
	bcc: Vec<String>,
	subject: String,
	body: String,
}

impl Compose {
	/// Builds a validated compose request. Address lists may be empty so the
	/// user can fill them in.
	pub fn new(
		to: impl IntoIterator<Item = impl Into<String>>,
		cc: impl IntoIterator<Item = impl Into<String>>,
		bcc: impl IntoIterator<Item = impl Into<String>>,
		subject: impl Into<String>,
		body: impl Into<String>,
	) -> Result<Self, Error> {
		Ok(Self {
			to: addresses(to)?,
			cc: addresses(cc)?,
			bcc: addresses(bcc)?,
			subject: bounded_text(subject.into(), MAX_SUBJECT_BYTES)?,
			body: bounded_text(body.into(), MAX_BODY_BYTES)?,
		})
	}

	/// Prefill To addresses.
	#[must_use]
	pub fn to(&self) -> &[String] {
		&self.to
	}

	/// Prefill Cc addresses.
	#[must_use]
	pub fn cc(&self) -> &[String] {
		&self.cc
	}

	/// Prefill Bcc addresses.
	#[must_use]
	pub fn bcc(&self) -> &[String] {
		&self.bcc
	}

	/// Prefill subject.
	#[must_use]
	pub fn subject(&self) -> &str {
		&self.subject
	}

	/// Prefill body.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Wire: to, cc, bcc lists, then subject and body strings.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(estimate(self));
		encode_list(&mut encoder, &self.to)?;
		encode_list(&mut encoder, &self.cc)?;
		encode_list(&mut encoder, &self.bcc)?;
		encoder.bytes(self.subject.as_bytes())?;
		encoder.bytes(self.body.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len()
			> MAX_BODY_BYTES + MAX_SUBJECT_BYTES + 3 * MAX_ADDRESSES * (MAX_ADDRESS_BYTES + 4) + 20
		{
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let to = decode_list(&mut decoder)?;
		let cc = decode_list(&mut decoder)?;
		let bcc = decode_list(&mut decoder)?;
		let subject = bounded_text(decoder.string()?.to_owned(), MAX_SUBJECT_BYTES)?;
		let body = bounded_text(decoder.string()?.to_owned(), MAX_BODY_BYTES)?;
		decoder.finish()?;
		Ok(Self {
			to,
			cc,
			bcc,
			subject,
			body,
		})
	}
}

fn addresses(values: impl IntoIterator<Item = impl Into<String>>) -> Result<Vec<String>, Error> {
	let addresses = values
		.into_iter()
		.map(|value| validate_address(value.into()))
		.collect::<Result<Vec<_>, _>>()?;

	if addresses.len() > MAX_ADDRESSES {
		return Err(Error::TooLarge);
	}

	Ok(addresses)
}

fn validate_address(value: String) -> Result<String, Error> {
	if value.is_empty() || value.len() > MAX_ADDRESS_BYTES || value.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	let (local, domain) = value.split_once('@').ok_or(Error::InvalidArgument)?;

	if local.is_empty()
		|| domain.is_empty()
		|| domain.contains('@')
		|| !value.is_ascii()
		|| value.bytes().any(|byte| byte.is_ascii_whitespace())
	{
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

fn bounded_text(value: String, max: usize) -> Result<String, Error> {
	if value.len() > max {
		return Err(Error::TooLarge);
	}

	if value.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

fn encode_list(encoder: &mut Encoder, values: &[String]) -> Result<(), Error> {
	encoder.u32(u32::try_from(values.len()).map_err(|_| Error::TooLarge)?);

	for value in values {
		encoder.bytes(value.as_bytes())?;
	}

	Ok(())
}

fn decode_list(decoder: &mut Decoder<'_>) -> Result<Vec<String>, Error> {
	let count = decoder.u32()? as usize;

	if count > MAX_ADDRESSES {
		return Err(Error::TooLarge);
	}

	let mut values = Vec::with_capacity(count);

	for _ in 0..count {
		values.push(validate_address(decoder.string()?.to_owned())?);
	}

	Ok(values)
}

fn estimate(compose: &Compose) -> usize {
	12 + compose.subject.len()
		+ compose.body.len()
		+ compose
			.to
			.iter()
			.chain(compose.cc.iter())
			.chain(compose.bcc.iter())
			.map(|value| 4 + value.len())
			.sum::<usize>()
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

/// Explicit unavailable handler for hosts without a mail composer.
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
	pub struct MailComposer(leptoskit_bridge::guest::Client);

	impl MailComposer {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries composer readiness without presenting UI.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Presents the system mail composer. An empty reply means the OS
		/// finished its callback, never that mail was delivered. Attachments
		/// are not part of this MVP.
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

/// Browser builds have no mail composer.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Browsers have no supported mail composer.
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
		}

		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[5, 0, 0, 0]] {
			assert_eq!(Availability::decode(bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn compose_round_trips_and_rejects_bad_addresses_and_fields() {
		let compose = Compose::new(
			["a@example.test", "b@example.test"],
			["c@example.test"],
			None::<String>,
			"Subject line",
			"Body\n世界",
		)
		.unwrap();
		assert_eq!(
			Compose::decode(&compose.encode().unwrap()).unwrap(),
			compose
		);
		assert_eq!(compose.to().len(), 2);
		assert_eq!(compose.cc(), ["c@example.test"]);
		assert_eq!(compose.bcc().len(), 0);
		assert_eq!(compose.subject(), "Subject line");
		assert_eq!(compose.body(), "Body\n世界");

		let empty = Compose::new(None::<String>, None::<String>, None::<String>, "", "").unwrap();
		assert_eq!(Compose::decode(&empty.encode().unwrap()).unwrap(), empty);

		for bad in ["", "no-at", "@domain", "local@", "a@b@c", "a b@c", "a\0@b"] {
			assert_eq!(
				Compose::new([bad], None::<String>, None::<String>, "", ""),
				Err(Error::InvalidArgument),
				"{bad}"
			);
		}

		assert_eq!(
			Compose::new(
				["a@b"; MAX_ADDRESSES + 1],
				None::<String>,
				None::<String>,
				"",
				""
			),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Compose::new(
				None::<String>,
				None::<String>,
				None::<String>,
				"s".repeat(MAX_SUBJECT_BYTES + 1),
				""
			),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Compose::new(
				None::<String>,
				None::<String>,
				None::<String>,
				"",
				"b".repeat(MAX_BODY_BYTES + 1)
			),
			Err(Error::TooLarge)
		);

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
		let payload = Compose::new(["a@b.test"], None::<String>, None::<String>, "s", "b")
			.unwrap()
			.encode()
			.unwrap();
		assert_eq!(validate(PRESENT, &payload), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(
			Availability::decode(&plugin.invoke(AVAILABILITY, Bytes::new()).unwrap()).unwrap(),
			Availability::Unsupported
		);
		assert_eq!(plugin.invoke(PRESENT, payload), Err(Error::Unavailable));
	}
}
