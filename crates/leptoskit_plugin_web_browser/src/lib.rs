//! <!-- {=p_web_browserSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_web_browser` as a direct dependency and select the
//! `web-browser` plugin in
//! `native.plugins`; supported generated hosts discover it from Cargo metadata.
//! Generated Linux hosts register it where it has a Linux Rust implementation;
//! generated Windows and terminal hosts currently omit runtime plugins, so use
//! explicit registration in a custom host there. Guests enable the `guest`
//! feature and reuse `guest::WebBrowser` rather than resolving the plugin on
//! every call. This opens HTTP(S) in an in-app or system browser and answers
//! when it is dismissed. It is not `web-auth` (no OAuth return address) and not
//! `linking.open` (which does not wait for dismiss).
//!
//! <!-- {/p_web_browserSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "web-browser";
/// Empty request; exact four-byte [`Availability`] response.
pub const AVAILABILITY: MethodId = MethodId(0);
/// [`Open`] request; empty reply after the browser is dismissed.
pub const OPEN: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, OPEN];
/// Maximum ASCII encoded URL length.
pub const MAX_URL_BYTES: usize = 8192;

/// Whether this implementation can present an in-app browser now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no dismissible in-app browser backend.
	Unsupported = 0,
	/// An owned presentation context can host the browser.
	Ready = 1,
	/// The app's owned presentation context is detached or inactive.
	Inactive = 2,
	/// A prior browser presentation has not finished.
	Busy = 3,
}

impl Availability {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::Ready),
			2 => Ok(Self::Inactive),
			3 => Ok(Self::Busy),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Validated absolute HTTP(S) URL for one browser presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
	url: String,
}

impl Open {
	/// Accepts only absolute HTTP(S) URLs without credentials, controls,
	/// whitespace, or backslashes. Percent-encode Unicode before calling.
	pub fn url(value: impl Into<String>) -> Result<Self, Error> {
		let url = value.into();

		if url.len() > MAX_URL_BYTES {
			return Err(Error::TooLarge);
		}

		require_authority(&url)?;
		require_graphic(&url)?;
		require_escapes(&url)?;
		let parsed = url::Url::parse(&url).map_err(|_| Error::InvalidArgument)?;

		if parsed.host_str().is_none()
			|| !parsed.username().is_empty()
			|| parsed.password().is_some()
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self { url })
	}

	/// The validated URL string.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.url
	}

	/// Wire: length-prefixed UTF-8 URL.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(4 + self.url.len());
		encoder.bytes(self.url.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_URL_BYTES + 4 {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let url = Self::url(decoder.string()?.to_owned())?;
		decoder.finish()?;
		Ok(url)
	}
}

fn require_authority(value: &str) -> Result<(), Error> {
	let authority = value
		.strip_prefix("https://")
		.or_else(|| value.strip_prefix("http://"))
		.ok_or(Error::InvalidArgument)?
		.split(['/', '?', '#'])
		.next()
		.ok_or(Error::InvalidArgument)?;

	if authority.is_empty() || authority.contains('@') {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn require_graphic(value: &str) -> Result<(), Error> {
	if value
		.bytes()
		.any(|byte| !byte.is_ascii_graphic() || byte == b'\\')
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn require_escapes(value: &str) -> Result<(), Error> {
	for (index, byte) in value.bytes().enumerate() {
		if byte != b'%' {
			continue;
		}

		let pair = value.as_bytes().get(index + 1..index + 3);

		if !pair.is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit)) {
			return Err(Error::InvalidArgument);
		}
	}

	Ok(())
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Validates a method without opening a browser.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		AVAILABILITY if payload.is_empty() => Ok(()),
		AVAILABILITY => Err(Error::InvalidArgument),
		OPEN => {
			Open::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a dismissible browser.
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
	pub struct WebBrowser(leptoskit_bridge::guest::Client);

	impl WebBrowser {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries presentation readiness without opening a page.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Opens `url` and waits until the in-app browser is dismissed.
		/// Dropping the future cancels the caller and asks the host to dismiss
		/// when the platform allows it. This never establishes an OAuth return
		/// address; use `web-auth` for that.
		pub async fn open(self, open: &Open) -> Result<(), Error> {
			let reply = self.0.call(OPEN, &open.encode()?)?.await?;

			if reply.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

/// Browser builds navigate the page themselves; there is no separate sheet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Pages navigate themselves; there is no dismissible sheet API here.
	#[must_use]
	pub const fn availability() -> Availability {
		Availability::Unsupported
	}

	/// Always returns Unavailable without calling `window.open`.
	pub fn open(_open: &Open) -> Result<(), Error> {
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
			(1, Availability::Ready),
			(2, Availability::Inactive),
			(3, Availability::Busy),
		] {
			assert_eq!(Availability::decode(&u32::to_le_bytes(value)), Ok(expected));
		}

		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[4, 0, 0, 0]] {
			assert_eq!(Availability::decode(bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn open_accepts_http_urls_and_rejects_credentials_schemes_and_controls() {
		for good in [
			"https://example.test/a%20b?q=one#two",
			"http://127.0.0.1:8080/path",
		] {
			let open = Open::url(good).unwrap();
			assert_eq!(Open::decode(&open.encode().unwrap()).unwrap(), open);
			assert_eq!(open.as_str(), good);
		}

		for bad in [
			"file:///secret",
			"mailto:user@example.test",
			"javascript:alert(1)",
			"https://user:password@example.test",
			"https:///no-host",
			"https://example.test/a b",
			"https://example.test/%no",
			"https://example.test\\evil",
			"https://",
			"",
		] {
			assert!(Open::url(bad).is_err(), "{bad}");
		}

		assert_eq!(
			Open::url(format!(
				"https://example.test/{}",
				"a".repeat(MAX_URL_BYTES)
			)),
			Err(Error::TooLarge)
		);

		let open = Open::url("https://example.test/").unwrap();
		let bytes = open.encode().unwrap();
		for end in 0..bytes.len() {
			assert!(Open::decode(&bytes[..end]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Open::decode(&trailing).is_err());
	}

	#[test]
	fn validate_and_unsupported_never_open() {
		assert_eq!(validate(AVAILABILITY, &[]), Ok(()));
		assert_eq!(validate(AVAILABILITY, &[0]), Err(Error::InvalidArgument));
		let payload = Open::url("https://example.test/")
			.unwrap()
			.encode()
			.unwrap();
		assert_eq!(validate(OPEN, &payload), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(
			Availability::decode(&plugin.invoke(AVAILABILITY, Bytes::new()).unwrap()).unwrap(),
			Availability::Unsupported
		);
		assert_eq!(plugin.invoke(OPEN, payload), Err(Error::Unavailable));
	}
}
