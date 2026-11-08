//! <!-- {=p_android_intentSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_android_intent` as a direct dependency and select the
//! `android_intent` plugin in `native.plugins`; generated Android hosts
//! discover it from Cargo metadata. Guests enable the `guest` feature and reuse
//! `guest::AndroidIntent` rather than resolving the plugin on every call. It
//! launches, broadcasts and resolves arbitrary Android intents with typed
//! extras, the way Flutter's `android_intent_plus` does. Only Android registers
//! it: on the web, iOS, macOS, Windows, Linux, the terminal, HarmonyOS, Wear OS
//! and watchOS, `guest::AndroidIntent::new` returns `Unavailable`. Guest code
//! can change over the air, so the plugin refuses URI-permission grant flags,
//! `file:` URIs and parcelable extras.
//!
//! <!-- {/p_android_intentSummary} -->
//!
//! # Wire format
//!
//! Integers are little-endian; a string is a u32 byte length and UTF-8
//! bytes, and an optional string is empty when absent. An intent is:
//!
//! ```text
//! u32 version (1) · u32 flags · action · data · mime type · package ·
//! component class · u32 category count, categories · extras bundle
//! ```
//!
//! A bundle is a u32 entry count followed by entries of key, u32 tag and
//! value, with tags from [`Extra::tag`]. Booleans are u32 `0`/`1`, doubles
//! are IEEE 754 bits, arrays and lists are a u32 count and their elements.
//! The native plugin is the trust boundary and applies every rule below
//! again, in the same order, because a guest can send any bytes it likes.
//!
//! # Security
//!
//! Guest code can be replaced over the air, so the plugin treats every
//! request as untrusted:
//!
//! - The four `FLAG_GRANT_*_URI_PERMISSION` flags are refused with
//!   `PermissionDenied`. With them an intent could hand any app read or write
//!   access to this app's own content URIs, such as files behind its
//!   `FileProvider`.
//! - `file:` data URIs are refused with `PermissionDenied`.
//! - Extras are primitives, strings, arrays and nested bundles only. No URI,
//!   intent, pending intent or other parcelable can be sent, so a request
//!   cannot carry a capability.
//! - `guest::AndroidIntent::parse_and_launch` handles `intent:` URIs the way a
//!   browser does: it refuses a component or a selector, refuses grant flags
//!   and adds `CATEGORY_BROWSABLE`, so a URI from a web page cannot open this
//!   app's private activities.
//! - Every field, count, depth and the whole payload is bounded.
//!
//! What remains is the app's own authority: a guest can start any exported
//! activity, service or receiver, and any of this app's own components,
//! with the permissions the app holds. Grant the plugin's methods only to
//! guests that need them.

mod constants;
mod extras;
mod intent;
mod reply;
mod validate;
mod wire;

pub use constants::action;
pub use constants::category;
pub use constants::extra;
pub use constants::flag;
pub use extras::Extra;
pub use extras::Extras;
pub use intent::Chooser;
pub use intent::Intent;
pub use intent::IntentUri;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
pub use reply::ResolvedActivity;
pub use reply::decode_bool;
pub use reply::encode_bool;

/// Stable plugin registration name.
pub const NAME: &str = "android_intent";

/// [`Intent`] -> empty. `startActivity`.
pub const LAUNCH: MethodId = MethodId(0);
/// [`Chooser`] -> empty. `startActivity(Intent.createChooser(…))`.
pub const LAUNCH_CHOOSER: MethodId = MethodId(1);
/// [`Intent`] -> empty. `sendBroadcast`.
pub const SEND_BROADCAST: MethodId = MethodId(2);
/// Explicit [`Intent`] -> empty. `startService`.
pub const SEND_SERVICE: MethodId = MethodId(3);
/// [`Intent`] -> u32 boolean. Whether an activity resolves.
pub const CAN_RESOLVE_ACTIVITY: MethodId = MethodId(4);
/// [`Intent`] -> optional [`ResolvedActivity`].
pub const RESOLVED_ACTIVITY: MethodId = MethodId(5);
/// UTF-8 [`IntentUri`] -> empty. `Intent.parseUri` then `startActivity`.
pub const PARSE_AND_LAUNCH: MethodId = MethodId(6);
/// Every method, each independently grantable by the host.
pub const METHODS: &[MethodId] = &[
	LAUNCH,
	LAUNCH_CHOOSER,
	SEND_BROADCAST,
	SEND_SERVICE,
	CAN_RESOLVE_ACTIVITY,
	RESOLVED_ACTIVITY,
	PARSE_AND_LAUNCH,
];

/// The intent wire version.
pub const WIRE_VERSION: u32 = 1;
/// Largest encoded intent. Android's binder buffer is 1 MiB shared by the
/// whole process; this stays far below it.
pub const MAX_INTENT_BYTES: usize = 128 * 1024;
/// Longest action, category, package, component class or extra key.
pub const MAX_NAME_BYTES: usize = 512;
/// Longest MIME type.
pub const MAX_MIME_BYTES: usize = 255;
/// Longest data URI or `intent:` URI.
pub const MAX_URI_BYTES: usize = 8192;
/// Longest string extra, string array element or byte array.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
/// Most elements in one array or list extra.
pub const MAX_ARRAY_LEN: usize = 1024;
/// Most entries in one bundle.
pub const MAX_EXTRAS: usize = 64;
/// Most entries across a bundle and every bundle nested in it.
pub const MAX_TOTAL_EXTRAS: usize = 256;
/// Deepest bundle nesting; the intent's own extras are depth 1.
pub const MAX_DEPTH: usize = 4;
/// Most categories on one intent.
pub const MAX_CATEGORIES: usize = 8;
/// Longest chooser title.
pub const MAX_TITLE_BYTES: usize = 1024;
/// Longest activity label in a [`ResolvedActivity`] reply; the native side
/// truncates longer labels at a character boundary.
pub const MAX_LABEL_BYTES: usize = 1024;
/// `FLAG_GRANT_READ_URI_PERMISSION | FLAG_GRANT_WRITE_URI_PERMISSION |
/// FLAG_GRANT_PERSISTABLE_URI_PERMISSION | FLAG_GRANT_PREFIX_URI_PERMISSION`,
/// always refused.
pub const GRANT_URI_PERMISSION_FLAGS: u32 = 0x01 | 0x02 | 0x40 | 0x80;

/// One `Intent` flag bit. The [`flag`] module names the ones upstream does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flag(u32);

impl Flag {
	/// A single flag bit, refusing zero and combined values as upstream's
	/// `convertFlags` does.
	pub const fn new(bits: u32) -> Result<Self, Error> {
		if !bits.is_power_of_two() {
			return Err(Error::InvalidArgument);
		}

		Ok(Self(bits))
	}

	/// The flag's bit.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}
}

/// A plugin for hosts that are not Android: macOS, Windows, Linux, the
/// terminal and direct browser callers. Payloads are validated exactly as
/// Android would validate them, then every method reports
/// [`Error::Unavailable`], because nothing outside Android has intents.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(
		&mut self,
		method: MethodId,
		payload: leptoskit_bridge::Bytes,
	) -> Result<leptoskit_bridge::Bytes, Error> {
		decode_request(method, &payload)?;
		Err(Error::Unavailable)
	}
}

/// A decoded, validated request for one method.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
	/// [`LAUNCH`].
	Launch(Intent),
	/// [`LAUNCH_CHOOSER`].
	LaunchChooser(Chooser),
	/// [`SEND_BROADCAST`].
	SendBroadcast(Intent),
	/// [`SEND_SERVICE`].
	SendService(Intent),
	/// [`CAN_RESOLVE_ACTIVITY`].
	CanResolveActivity(Intent),
	/// [`RESOLVED_ACTIVITY`].
	ResolvedActivity(Intent),
	/// [`PARSE_AND_LAUNCH`].
	ParseAndLaunch(IntentUri),
}

/// Decodes one untrusted request the way the native plugin does. An unknown
/// method is [`Error::Unavailable`].
pub fn decode_request(method: MethodId, payload: &[u8]) -> Result<Request, Error> {
	let request = match method {
		LAUNCH => Request::Launch(Intent::decode(payload)?),
		LAUNCH_CHOOSER => Request::LaunchChooser(Chooser::decode(payload)?),
		SEND_BROADCAST => Request::SendBroadcast(Intent::decode(payload)?),
		SEND_SERVICE => Request::SendService(explicit(Intent::decode(payload)?)?),
		CAN_RESOLVE_ACTIVITY => Request::CanResolveActivity(Intent::decode(payload)?),
		RESOLVED_ACTIVITY => Request::ResolvedActivity(Intent::decode(payload)?),
		PARSE_AND_LAUNCH => Request::ParseAndLaunch(IntentUri::decode(payload)?),
		_ => return Err(Error::Unavailable),
	};

	Ok(request)
}

/// Android refuses implicit service intents.
fn explicit(intent: Intent) -> Result<Intent, Error> {
	if !intent.is_explicit() {
		return Err(Error::InvalidArgument);
	}

	Ok(intent)
}

/// Cached native bridge client for an interpreted Wasm guest.
///
/// Only Android registers this plugin. On the web, iOS, macOS, Windows, Linux,
/// the terminal, HarmonyOS, Wear OS and watchOS, [`guest::AndroidIntent::new`]
/// returns [`Error::Unavailable`], so check for it once and hide Android-only
/// features.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct AndroidIntent(leptoskit_bridge::guest::Client);

	impl AndroidIntent {
		/// Resolves the native plugin. [`Error::Unavailable`] everywhere but
		/// Android.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Starts the activity the intent resolves to. Android shows its own
		/// chooser when several apps match and none is the default.
		///
		/// [`Error::Unavailable`] when no activity handles it,
		/// [`Error::PermissionDenied`] when Android refuses (for example a
		/// non-exported activity in another app, or a permission this app
		/// lacks). Success means Android accepted the request, not that
		/// the activity finished anything.
		pub async fn launch(self, intent: &Intent) -> Result<(), Error> {
			empty(&self.0.call(LAUNCH, &intent.encode()?)?.await?)
		}

		/// Starts the intent through Android's chooser with `title`, or the
		/// system's default title when `title` is empty.
		pub async fn launch_chooser(self, intent: &Intent, title: &str) -> Result<(), Error> {
			empty(
				&self
					.0
					.call(LAUNCH_CHOOSER, &Chooser::encode(intent, title)?)?
					.await?,
			)
		}

		/// Sends the intent as a broadcast. Without a package, any app's
		/// registered receiver for the action can read the extras.
		pub async fn send_broadcast(self, intent: &Intent) -> Result<(), Error> {
			empty(&self.0.call(SEND_BROADCAST, &intent.encode()?)?.await?)
		}

		/// Starts a service. The intent must name a package or component;
		/// [`Error::Unavailable`] when no service matches or Android
		/// refuses a start from the background.
		pub async fn send_service(self, intent: &Intent) -> Result<(), Error> {
			let intent = explicit(intent.clone())?;
			empty(&self.0.call(SEND_SERVICE, &intent.encode()?)?.await?)
		}

		/// Whether an activity would handle the intent. On Android 11 and
		/// later the answer is `false` for apps the manifest's `<queries>`
		/// does not make visible.
		pub async fn can_resolve_activity(self, intent: &Intent) -> Result<bool, Error> {
			decode_bool(
				&self
					.0
					.call(CAN_RESOLVE_ACTIVITY, &intent.encode()?)?
					.await?,
			)
		}

		/// The activity that would handle the intent, subject to the same
		/// package visibility as [`Self::can_resolve_activity`].
		pub async fn resolved_activity(
			self,
			intent: &Intent,
		) -> Result<Option<ResolvedActivity>, Error> {
			ResolvedActivity::decode(&self.0.call(RESOLVED_ACTIVITY, &intent.encode()?)?.await?)
		}

		/// Parses an `intent:` URI with `Intent.parseUri(uri,
		/// URI_INTENT_SCHEME)` and starts it, with the browser-like policy
		/// described in the crate documentation. There is no
		/// `browser_fallback_url` handling: on [`Error::Unavailable`], open
		/// the fallback yourself.
		pub async fn parse_and_launch(self, uri: &str) -> Result<(), Error> {
			let uri = IntentUri::parse(uri)?;
			empty(
				&self
					.0
					.call(PARSE_AND_LAUNCH, uri.as_str().as_bytes())?
					.await?,
			)
		}
	}

	fn empty(reply: &[u8]) -> Result<(), Error> {
		if reply.is_empty() {
			return Ok(());
		}

		Err(Error::InvalidArgument)
	}
}

#[cfg(test)]
mod tests;
