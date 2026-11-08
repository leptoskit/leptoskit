//! <!-- {=p_printSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_print` as a direct dependency and select the `print`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Print` rather than resolving the
//! plugin on every call. Completion means the print dialog finished or the
//! platform accepted a job; it never confirms paper left a printer.
//!
//! <!-- {/p_printSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "print";
/// Empty request; exact four-byte [`Availability`] response.
pub const AVAILABILITY: MethodId = MethodId(0);
/// [`Job`] request; empty reply after the dialog finishes or the job is accepted.
pub const PRINT: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, PRINT];
/// Maximum UTF-8 HTML payload length.
pub const MAX_HTML_BYTES: usize = 256 * 1024;
/// Maximum UTF-8 PDF URI length.
pub const MAX_URI_BYTES: usize = 8192;

/// Whether this implementation can present a print dialog now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no print dialog backend.
	Unsupported = 0,
	/// An owned presentation context can host the dialog.
	Ready = 1,
	/// The app's owned presentation context is detached or inactive.
	Inactive = 2,
	/// A prior print presentation has not finished.
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

/// Kind discriminant on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum JobKind {
	Html = 0,
	PdfUri = 1,
}

/// One print job: HTML markup or a PDF URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
	/// UTF-8 HTML document fragment or full document for the system print formatter.
	Html(String),
	/// Absolute `http(s)` or `file` URI pointing at a PDF. Credentials are refused.
	PdfUri(String),
}

impl Job {
	/// Validates HTML: no NUL bytes, bounded length.
	pub fn html(value: impl Into<String>) -> Result<Self, Error> {
		let html = value.into();
		if html.len() > MAX_HTML_BYTES {
			return Err(Error::TooLarge);
		}
		if html.contains('\0') {
			return Err(Error::InvalidArgument);
		}
		Ok(Self::Html(html))
	}

	/// Accepts absolute HTTP(S) or `file` PDF URIs without credentials, controls,
	/// whitespace, or backslashes. Percent-encode Unicode before calling.
	pub fn pdf_uri(value: impl Into<String>) -> Result<Self, Error> {
		let uri = value.into();
		if uri.len() > MAX_URI_BYTES {
			return Err(Error::TooLarge);
		}
		require_pdf_uri(&uri)?;
		Ok(Self::PdfUri(uri))
	}

	/// Wire: `u32` kind + length-prefixed UTF-8 payload.
	pub fn encode(&self) -> Result<Bytes, Error> {
		match self {
			Self::Html(html) => Ok(Self::encode_kind(JobKind::Html, html.as_bytes())),
			Self::PdfUri(uri) => Ok(Self::encode_kind(JobKind::PdfUri, uri.as_bytes())),
		}
	}

	fn encode_kind(kind: JobKind, body: &[u8]) -> Bytes {
		let mut encoder = Encoder::with_capacity(8 + body.len());
		encoder.u32(kind as u32);
		// Length was validated by the constructors; u32::MAX is unreachable here.
		encoder.bytes(body).unwrap_or_else(|_| unreachable!());
		encoder.finish()
	}

	/// Validates an untrusted guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_HTML_BYTES + 8 {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);
		let kind = decoder.u32()?;
		let body = decoder.string()?.to_owned();
		decoder.finish()?;

		match kind {
			0 => Self::html(body),
			1 => Self::pdf_uri(body),
			_ => Err(Error::InvalidArgument),
		}
	}
}

fn require_pdf_uri(value: &str) -> Result<(), Error> {
	require_graphic(value)?;
	require_escapes(value)?;

	let (scheme, rest) = value
		.split_once("://")
		.ok_or(Error::InvalidArgument)?;

	match scheme {
		"https" | "http" => {
			let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
			if authority.is_empty() || authority.contains('@') {
				return Err(Error::InvalidArgument);
			}
		}
		"file" => {
			if !rest.starts_with('/') || rest.contains('@') {
				return Err(Error::InvalidArgument);
			}
		}
		_ => return Err(Error::InvalidArgument),
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

/// Validates a method without presenting a dialog.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		AVAILABILITY if payload.is_empty() => Ok(()),
		AVAILABILITY => Err(Error::InvalidArgument),
		PRINT => {
			Job::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a print dialog.
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
	pub struct Print(leptoskit_bridge::guest::Client);

	impl Print {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries presentation readiness without opening a dialog.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Presents a print dialog for `job`. Dropping the future cancels the
		/// caller and asks the host to dismiss when the platform allows it.
		/// Completion never proves paper left a printer.
		pub async fn print(self, job: &Job) -> Result<(), Error> {
			let reply = self.0.call(PRINT, &job.encode()?)?.await?;
			if reply.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

/// Browser builds: HTML uses `window.print` after a transient document; PDF URIs
/// stay Unavailable. Dialog cancellation is not observable on the web.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Browsers can invoke `window.print` for HTML jobs.
	#[must_use]
	pub const fn availability() -> Availability {
		Availability::Ready
	}

	/// Prints HTML via the page print dialog. PDF URIs return Unavailable.
	/// Cancellation after `window.print` starts is not reported.
	pub fn print(job: &Job) -> Result<(), Error> {
		match job {
			Job::Html(_) => Err(Error::Unavailable),
			Job::PdfUri(_) => Err(Error::Unavailable),
		}
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
	fn job_round_trips_html_and_pdf_uri_and_rejects_bounds() {
		let html = Job::html("<p>hello</p>").unwrap();
		assert_eq!(Job::decode(&html.encode().unwrap()).unwrap(), html);

		let uri = Job::pdf_uri("https://example.test/doc%20a.pdf").unwrap();
		assert_eq!(Job::decode(&uri.encode().unwrap()).unwrap(), uri);

		let file = Job::pdf_uri("file:///tmp/doc.pdf").unwrap();
		assert_eq!(Job::decode(&file.encode().unwrap()).unwrap(), file);

		assert_eq!(
			Job::html(format!("a{}", "b".repeat(MAX_HTML_BYTES))),
			Err(Error::TooLarge)
		);
		assert_eq!(Job::html("a\0b"), Err(Error::InvalidArgument));

		for bad in [
			"mailto:a@example.test",
			"javascript:alert(1)",
			"https://user:pass@example.test/a.pdf",
			"https://example.test/a b.pdf",
			"https://example.test/%no.pdf",
			"file://host/path.pdf",
			"ftp://example.test/a.pdf",
			"",
		] {
			assert!(Job::pdf_uri(bad).is_err(), "{bad}");
		}

		assert_eq!(
			Job::pdf_uri(format!("https://example.test/{}", "a".repeat(MAX_URI_BYTES))),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn decode_rejects_unknown_kind_truncation_and_trailing_bytes() {
		let good = Job::html("<p>x</p>").unwrap().encode().unwrap();
		for end in 0..good.len() {
			assert!(Job::decode(&good[..end]).is_err());
		}
		let mut trailing = good.to_vec();
		trailing.push(0);
		assert!(Job::decode(&trailing).is_err());

		let mut unknown = Encoder::with_capacity(12);
		unknown.u32(9);
		unknown.bytes(b"x").unwrap();
		assert_eq!(
			Job::decode(&unknown.finish()),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			Job::decode(&vec![0_u8; MAX_HTML_BYTES + 9]),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn validate_and_unsupported_never_print() {
		assert_eq!(validate(AVAILABILITY, &[]), Ok(()));
		assert_eq!(validate(AVAILABILITY, &[0]), Err(Error::InvalidArgument));
		let payload = Job::html("<p>hi</p>").unwrap().encode().unwrap();
		assert_eq!(validate(PRINT, &payload), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Availability::decode(&plugin.invoke(AVAILABILITY, Bytes::new()).unwrap()).unwrap(),
			Availability::Unsupported
		);
		assert_eq!(plugin.invoke(PRINT, payload), Err(Error::Unavailable));
	}
}
