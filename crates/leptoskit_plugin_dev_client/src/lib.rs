//! Connect a development phone to the guest `leptoskit dev` serves.
//!
//! The address in the terminal's QR code is
//! `http://<host>:<port>/_leptoskit/dev`. [`guest_url`] turns that, or the
//! same address pasted by hand, into the platform's guest URL. Only a local
//! network host is accepted.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "dev-client";
/// Empty request → four-byte status.
pub const STATUS: MethodId = MethodId(0);
/// Canonical guest URL → empty reply. The host starts following it.
pub const CONNECT: MethodId = MethodId(1);
/// Empty request → the text a QR reader saw.
pub const SCAN: MethodId = MethodId(2);
/// Supported operations.
pub const METHODS: &[MethodId] = &[STATUS, CONNECT, SCAN];

const AVAILABLE: u32 = 1;
const UNAVAILABLE: u32 = 2;

/// Why a scanned or pasted address is not a development guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
	/// Nothing was entered.
	Empty,
	/// The phone platform is not `android` or `ios`.
	Platform,
	/// The address is not `http` or `https`.
	Scheme,
	/// A username or password was included.
	Credentials,
	/// A query or fragment was included.
	Query,
	/// The host is not on the local network.
	Host,
	/// The path is not the development guest.
	Path,
	/// The port is missing its digits or is zero.
	Port,
}

impl std::fmt::Display for SessionError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Empty => "Enter the address from leptoskit dev",
			Self::Platform => "This build cannot open that development guest",
			Self::Scheme => "The address must start with http:// or https://",
			Self::Credentials => "The address cannot include a username or password",
			Self::Query => "The address cannot include a query or a fragment",
			Self::Host => "The address must be on this network",
			Self::Path => "The address is not a leptoskit dev server",
			Self::Port => "The address needs a valid port",
		})
	}
}

impl std::error::Error for SessionError {}

/// The URL a QR code should carry: the development server, before the platform.
///
/// Returns `None` when `host` is not a local-network address or `port` is 0.
#[must_use]
pub fn session_link(host: IpAddr, port: u16) -> Option<String> {
	if port == 0 || !dev_ip(host) {
		return None;
	}

	let host = match host {
		IpAddr::V4(address) => address.to_string(),
		IpAddr::V6(address) => format!("[{address}]"),
	};
	Some(format!("http://{host}:{port}/_leptoskit/dev"))
}

/// The guest URL for `platform` (`android` or `ios`).
///
/// `scanned` may be the QR text or the guest URL itself.
///
/// # Errors
///
/// Returns [`SessionError`] when the address is empty, not a local `http` or
/// `https` URL, or not this platform's development guest.
pub fn guest_url(scanned: &str, platform: &str) -> Result<String, SessionError> {
	if platform != "android" && platform != "ios" {
		return Err(SessionError::Platform);
	}

	let text = scanned.trim();

	if text.is_empty() {
		return Err(SessionError::Empty);
	}

	if text.contains(['?', '#']) {
		return Err(SessionError::Query);
	}

	if text.chars().any(char::is_whitespace) {
		return Err(SessionError::Path);
	}

	let (scheme, rest) = if let Some(rest) = text.strip_prefix("http://") {
		("http", rest)
	} else if let Some(rest) = text.strip_prefix("https://") {
		("https", rest)
	} else {
		return Err(SessionError::Scheme);
	};

	if rest.contains('@') {
		return Err(SessionError::Credentials);
	}

	let (authority, path) = match rest.find('/') {
		Some(index) => (&rest[..index], &rest[index..]),
		None => (rest, ""),
	};
	let (host, port) = split_host(authority, scheme)?;
	let path = path.trim_end_matches('/');
	let guest = format!("/_leptoskit/dev/{platform}/guest");

	if !(path.is_empty() || path == "/_leptoskit/dev" || path == guest) {
		return Err(SessionError::Path);
	}

	Ok(format!("{scheme}://{host}:{port}{guest}"))
}

/// Accepts a guest URL for either phone. A base address becomes the Android
/// guest.
///
/// # Errors
///
/// Same as [`guest_url`].
pub fn accept_guest_url(text: &str) -> Result<String, SessionError> {
	match guest_url(text, "android") {
		Err(SessionError::Path) => guest_url(text, "ios"),
		other => other,
	}
}

fn split_host(authority: &str, scheme: &str) -> Result<(String, u16), SessionError> {
	if authority.is_empty() {
		return Err(SessionError::Host);
	}

	let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
		let end = rest.find(']').ok_or(SessionError::Host)?;
		let host = &rest[..end];
		let after = &rest[end + 1..];
		let port = if after.is_empty() {
			None
		} else {
			Some(parse_port(
				after.strip_prefix(':').ok_or(SessionError::Port)?,
			)?)
		};
		(host, port)
	} else if let Some((host, port)) = authority.rsplit_once(':') {
		if host.is_empty() || host.contains(':') {
			return Err(SessionError::Host);
		}

		(host, Some(parse_port(port)?))
	} else {
		(authority, None)
	};

	if host.is_empty() || !dev_host(host) {
		return Err(SessionError::Host);
	}

	let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
	let rendered = if host.contains(':') {
		format!("[{host}]")
	} else {
		host.to_owned()
	};
	Ok((rendered, port))
}

fn parse_port(text: &str) -> Result<u16, SessionError> {
	let port: u16 = text.parse().map_err(|_| SessionError::Port)?;

	if port == 0 {
		Err(SessionError::Port)
	} else {
		Ok(port)
	}
}

fn dev_host(host: &str) -> bool {
	if host.eq_ignore_ascii_case("localhost") {
		return true;
	}

	let lower = host.to_ascii_lowercase();

	if let Some(name) = lower.strip_suffix(".local") {
		return !name.is_empty()
			&& name.split('.').all(|label| {
				(1..=63).contains(&label.len())
					&& !label.starts_with('-')
					&& !label.ends_with('-')
					&& label
						.chars()
						.all(|letter| letter.is_ascii_alphanumeric() || letter == '-')
			});
	}

	host.parse::<IpAddr>().is_ok_and(dev_ip)
}

/// Whether `ip` is a loopback, private, link-local, or unique-local address.
#[must_use]
pub fn dev_ip(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(address) => dev_v4(address),
		IpAddr::V6(address) => dev_v6(address),
	}
}

fn dev_v4(address: Ipv4Addr) -> bool {
	!address.is_unspecified()
		&& !address.is_broadcast()
		&& !address.is_multicast()
		&& (address.is_loopback() || address.is_private() || address.is_link_local())
}

fn dev_v6(address: Ipv6Addr) -> bool {
	if address.is_unspecified() || address.is_multicast() {
		return false;
	}

	if address.is_loopback() {
		return true;
	}

	let [first, second, ..] = address.octets();
	// fc00::/7 unique local, fe80::/10 link local.
	first & 0xfe == 0xfc || (first == 0xfe && second & 0xc0 == 0x80)
}

/// Follows a development guest when `follow` returns true.
pub struct DevClient<F> {
	follow: F,
	closed: bool,
}

impl<F> std::fmt::Debug for DevClient<F> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DevClient")
			.field("closed", &self.closed)
			.finish_non_exhaustive()
	}
}

impl<F> DevClient<F>
where
	F: FnMut(&str) -> bool,
{
	/// Calls `follow` with each accepted guest URL.
	pub fn new(follow: F) -> Self {
		Self {
			follow,
			closed: false,
		}
	}

	/// Refuses later calls. The host drops the plugin after this.
	pub fn close(&mut self) {
		self.closed = true;
	}
}

impl<F> leptoskit_bridge::Plugin for DevClient<F>
where
	F: FnMut(&str) -> bool,
{
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if self.closed {
			return Err(Error::Cancelled);
		}

		match method {
			STATUS => {
				finish(&payload)?;
				Ok(status(AVAILABLE))
			}
			CONNECT => {
				let url = decode_text(&payload)?;
				let url = accept_guest_url(&url).map_err(|_| Error::InvalidArgument)?;

				if (self.follow)(&url) {
					Ok(Bytes::new())
				} else {
					Err(Error::Unavailable)
				}
			}
			SCAN => {
				finish(&payload)?;
				Err(Error::Unavailable)
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// A host that does not follow development guests.
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
		match method {
			STATUS => {
				finish(&payload)?;
				Ok(status(UNAVAILABLE))
			}
			CONNECT | SCAN => {
				if method == CONNECT {
					decode_text(&payload)?;
				} else {
					finish(&payload)?;
				}
				Err(Error::Unavailable)
			}
			_ => Err(Error::Unavailable),
		}
	}
}

fn status(value: u32) -> Bytes {
	Bytes::copy_from_slice(&value.to_le_bytes())
}

fn finish(payload: &[u8]) -> Result<(), Error> {
	Decoder::new(payload).finish()
}

fn decode_text(payload: &[u8]) -> Result<String, Error> {
	let mut input = Decoder::new(payload);
	let text = input.string()?.to_owned();
	input.finish()?;
	Ok(text)
}

/// Length-prefixed UTF-8, the same framing the phone plugins use.
pub fn encode_text(text: &str) -> Result<Bytes, Error> {
	let mut output = Encoder::with_capacity(4 + text.len());
	output.bytes(text.as_bytes())?;
	Ok(output.finish())
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::Error;
	use leptoskit_bridge::codec::Decoder;

	use super::CONNECT;
	use super::SCAN;
	use super::STATUS;
	use super::encode_text;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct DevClient(leptoskit_bridge::guest::Client);

	impl DevClient {
		/// Resolves a registered native plugin.
		///
		/// # Errors
		///
		/// Returns an error when the host did not register `dev-client`.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(super::NAME).map(Self)
		}

		/// Whether this host can follow a development guest.
		///
		/// # Errors
		///
		/// Returns an error when the host rejects the call.
		pub async fn available(self) -> Result<bool, Error> {
			let bytes = self.0.call(STATUS, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let value = input.u32()?;
			input.finish()?;
			Ok(value == 1)
		}

		/// Asks the host to follow `url`.
		///
		/// # Errors
		///
		/// Returns an error when `url` is not a local development guest or the
		/// host cannot follow it.
		pub async fn connect(self, url: &str) -> Result<(), Error> {
			let bytes = self.0.call(CONNECT, &encode_text(url)?)?.await?;
			Decoder::new(&bytes).finish()
		}

		/// Reads one QR code.
		///
		/// # Errors
		///
		/// Returns an error when the host has no reader, or the person cancels.
		pub async fn scan(self) -> Result<String, Error> {
			let bytes = self.0.call(SCAN, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let text = input.string()?.to_owned();
			input.finish()?;
			Ok(text)
		}
	}
}

#[cfg(test)]
mod tests {
	use std::net::Ipv4Addr;
	use std::net::Ipv6Addr;
	use std::sync::Mutex;

	use leptoskit_bridge::Plugin;

	use super::*;

	fn address(text: &str, platform: &str) -> String {
		guest_url(text, platform).unwrap_or_else(|error| panic!("{text}: {error}"))
	}

	#[test]
	fn a_local_address_becomes_the_phone_guest() {
		assert_eq!(
			address("  http://127.0.0.1:9/_leptoskit/dev  ", "android"),
			"http://127.0.0.1:9/_leptoskit/dev/android/guest"
		);
		assert_eq!(
			address(
				"http://10.0.2.2:43123/_leptoskit/dev/android/guest",
				"android"
			),
			"http://10.0.2.2:43123/_leptoskit/dev/android/guest"
		);
		assert_eq!(
			address("http://192.168.0.10", "ios"),
			"http://192.168.0.10:80/_leptoskit/dev/ios/guest"
		);
		assert_eq!(
			address("https://dev.local/_leptoskit/dev/", "ios"),
			"https://dev.local:443/_leptoskit/dev/ios/guest"
		);
		assert_eq!(
			address("http://[::1]:3000/_leptoskit/dev/android/guest/", "android"),
			"http://[::1]:3000/_leptoskit/dev/android/guest"
		);
		assert_eq!(
			address("http://localhost:3000/_leptoskit/dev", "ios"),
			"http://localhost:3000/_leptoskit/dev/ios/guest"
		);
		assert_eq!(
			address("http://169.254.1.1:1", "android"),
			"http://169.254.1.1:1/_leptoskit/dev/android/guest"
		);
		assert_eq!(
			address("http://[fc00::1]:4/_leptoskit/dev", "android"),
			"http://[fc00::1]:4/_leptoskit/dev/android/guest"
		);
		assert_eq!(
			address("http://[fe80::1]:4/_leptoskit/dev", "ios"),
			"http://[fe80::1]:4/_leptoskit/dev/ios/guest"
		);
		assert_eq!(
			accept_guest_url("http://172.16.0.1:5/_leptoskit/dev/ios/guest").unwrap(),
			"http://172.16.0.1:5/_leptoskit/dev/ios/guest"
		);
		assert_eq!(
			accept_guest_url("http://10.1.1.1:5/_leptoskit/dev").unwrap(),
			"http://10.1.1.1:5/_leptoskit/dev/android/guest"
		);
	}

	#[test]
	fn a_public_or_broken_address_is_refused() {
		let cases = [
			("", "android", SessionError::Empty),
			("   ", "android", SessionError::Empty),
			("http://127.0.0.1:1", "web", SessionError::Platform),
			("ftp://127.0.0.1:1", "android", SessionError::Scheme),
			("127.0.0.1:1", "android", SessionError::Scheme),
			(
				"http://user:pw@127.0.0.1:1/_leptoskit/dev",
				"android",
				SessionError::Credentials,
			),
			(
				"http://127.0.0.1:1/_leptoskit/dev?x=1",
				"android",
				SessionError::Query,
			),
			(
				"http://127.0.0.1:1/_leptoskit/dev#top",
				"android",
				SessionError::Query,
			),
			(
				"http://8.8.8.8:1/_leptoskit/dev",
				"android",
				SessionError::Host,
			),
			(
				"http://evil.com:1/_leptoskit/dev",
				"android",
				SessionError::Host,
			),
			("http://0.0.0.0:1", "android", SessionError::Host),
			("http://255.255.255.255:1", "android", SessionError::Host),
			("http://224.0.0.1:1", "android", SessionError::Host),
			("http://[::]:1", "android", SessionError::Host),
			("http://", "android", SessionError::Host),
			("http://[::1", "android", SessionError::Host),
			("http://[]:1", "android", SessionError::Host),
			("http://:80", "android", SessionError::Host),
			("http://127.0.0.1:1/other", "android", SessionError::Path),
			(
				"http://127.0.0.1:1/_leptoskit/dev/ios/guest",
				"android",
				SessionError::Path,
			),
			(
				"http://127.0.0.1:1 has spaces",
				"android",
				SessionError::Path,
			),
			("http://127.0.0.1:0", "android", SessionError::Port),
			("http://127.0.0.1:", "android", SessionError::Port),
			("http://127.0.0.1:99999", "android", SessionError::Port),
			("http://[::1]:", "android", SessionError::Port),
			("http://[::1]x", "android", SessionError::Port),
			("http://-.local:1", "android", SessionError::Host),
			("http://.local:1", "android", SessionError::Host),
		];

		for (text, platform, error) in cases {
			assert_eq!(guest_url(text, platform), Err(error), "{text}");
		}

		assert_eq!(
			accept_guest_url("http://8.8.8.8:1"),
			Err(SessionError::Host)
		);
		for error in [
			SessionError::Empty,
			SessionError::Platform,
			SessionError::Scheme,
			SessionError::Credentials,
			SessionError::Query,
			SessionError::Host,
			SessionError::Path,
			SessionError::Port,
		] {
			assert_ne!(error.to_string(), "");
		}
	}

	#[test]
	fn the_qr_text_is_the_server_before_the_platform() {
		assert_eq!(
			session_link(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 43123).unwrap(),
			"http://192.168.1.20:43123/_leptoskit/dev"
		);
		assert_eq!(
			session_link(IpAddr::V6(Ipv6Addr::LOCALHOST), 9).unwrap(),
			"http://[::1]:9/_leptoskit/dev"
		);
		assert!(session_link(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 9).is_none());
		assert!(session_link(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).is_none());
		assert!(dev_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
		assert!(!dev_ip("2001:db8::1".parse().unwrap()));
	}

	#[test]
	fn connecting_follows_a_local_guest_and_refuses_the_rest() {
		let seen = Mutex::new(String::new());
		let mut plugin = DevClient::new(|url: &str| {
			*seen.lock().unwrap() = url.to_owned();
			url.contains("192.168")
		});
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		let status = plugin.invoke(STATUS, Bytes::new()).unwrap();
		assert_eq!(&status[..], &1_u32.to_le_bytes());
		let url = "http://192.168.1.9:4/_leptoskit/dev/android/guest";
		plugin.invoke(CONNECT, encode_text(url).unwrap()).unwrap();
		assert_eq!(seen.lock().unwrap().as_str(), url);
		assert_eq!(
			plugin.invoke(
				CONNECT,
				encode_text("http://10.0.0.1:4/_leptoskit/dev/ios/guest").unwrap()
			),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(CONNECT, encode_text("http://evil.com").unwrap()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(CONNECT, Bytes::from(vec![1, 0, 0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(plugin.invoke(SCAN, Bytes::new()), Err(Error::Unavailable));
		assert_eq!(
			plugin.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
		plugin.close();
		assert_eq!(plugin.invoke(STATUS, Bytes::new()), Err(Error::Cancelled));
		assert!(format!("{plugin:?}").contains("closed"));
	}

	#[test]
	fn an_unsupported_host_answers_unavailable() {
		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		let status = plugin.invoke(STATUS, Bytes::new()).unwrap();
		assert_eq!(&status[..], &2_u32.to_le_bytes());
		assert_eq!(
			plugin.invoke(STATUS, Bytes::from(vec![1])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(
				CONNECT,
				encode_text("http://127.0.0.1:1/_leptoskit/dev/android/guest").unwrap()
			),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(CONNECT, Bytes::new()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(plugin.invoke(SCAN, Bytes::new()), Err(Error::Unavailable));
		assert_eq!(
			plugin.invoke(SCAN, Bytes::from(vec![1])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
