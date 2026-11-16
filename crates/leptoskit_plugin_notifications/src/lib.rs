//! <!-- {=p_notificationsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_notifications` as a direct dependency and select the
//! `notifications` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::Notifications` rather than resolving the plugin on every call; web
//! builds keep the default `browser` feature, and interpreter guests turn it
//! off so their dependency tree stays free of browser imports. A successful
//! call means the OS accepted the request, not that a banner appeared; focus
//! modes and user settings can still suppress it. On iOS and macOS the
//! generated app delegate forwards APNs device tokens into the plugin's broker,
//! and on Android a push-enabled generated app forwards FCM registration tokens
//! into the same broker; `REMOTE_REGISTER` turns an explicit opt-in into a
//! bounded, typed registration answer, answers `unconfirmed` when the OS stays
//! silent rather than replaying a prior token, and carries refusal as stable
//! reason codes instead of the OS's own text. Every other platform reports
//! `Unavailable` for it.
//!
//! <!-- {/p_notificationsSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "notifications";
/// Empty payload; four-byte permission state.
pub const PERMISSION: MethodId = MethodId(0);
/// Empty payload; requests native permission and returns its current state.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Four length-prefixed strings: identifier, title, body, Android channel ID.
pub const SHOW: MethodId = MethodId(2);
/// One length-prefixed identifier; removes pending and delivered notifications.
pub const CANCEL: MethodId = MethodId(3);
/// Android only: identifier, name, description, importance u32, sound boolean
/// u32.
pub const CONFIGURE_CHANNEL: MethodId = MethodId(4);
/// Android only: one length-prefixed channel identifier.
pub const DELETE_CHANNEL: MethodId = MethodId(5);
/// Empty payload; dismisses only notifications owned by this plugin namespace.
pub const CANCEL_ALL: MethodId = MethodId(6);
/// iOS, macOS, and Android: empty payload; reads the latest
/// remote-registration answer without side effects. Other platforms report
/// `Unavailable`.
pub const REMOTE_TOKEN: MethodId = MethodId(7);
/// iOS, macOS, and Android: empty payload; asks the OS to register for
/// remote notifications and waits a bounded time for its answer, returned as
/// the new snapshot — or [`RemoteState::Unconfirmed`] when no answer arrived
/// in the bound, so silence can never masquerade as a stale token. Other
/// platforms report `Unavailable`.
pub const REMOTE_REGISTER: MethodId = MethodId(8);
/// Exported independently grantable operations.
pub const METHODS: &[MethodId] = &[
	PERMISSION,
	REQUEST_PERMISSION,
	SHOW,
	CANCEL,
	CONFIGURE_CHANNEL,
	DELETE_CHANNEL,
	CANCEL_ALL,
	REMOTE_TOKEN,
	REMOTE_REGISTER,
];

/// The longest APNs device token, in hex characters. Apple's tokens are 64
/// hex characters today; the bound leaves room for longer ones rather than
/// guessing a ceiling, and matches the notifications service's own validator
/// (`leptoskit_notifications::push::MAX_APNS_TOKEN_CHARS`).
pub const MAX_APNS_TOKEN_CHARS: usize = 200;

/// The shortest APNs device token, in hex characters. Never assume the
/// historical 32-byte length: the OS decides the token's length.
pub const MIN_APNS_TOKEN_CHARS: usize = 32;

/// The longest FCM registration token, in bytes, matching the notifications
/// service's own validator
/// (`leptoskit_notifications::push::MAX_FCM_TOKEN_LEN`). Real tokens are far
/// shorter; the bound keeps a hostile answer out without guessing Firebase's
/// shape.
pub const MAX_FCM_TOKEN_CHARS: usize = 4_096;

/// The longest registration-failure reason, in bytes. Anything longer is
/// truncated by the host before it reaches this contract.
pub const MAX_REMOTE_FAILURE_BYTES: usize = 256;

/// Current OS notification authorization, without guessing prompt history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Permission {
	/// Android cannot distinguish an initial denial from a non-promptable
	/// denial.
	Unknown = 0,
	/// The platform explicitly reports that permission has not been requested.
	NotDetermined = 1,
	/// Notifications are disabled or authorization was denied.
	Denied = 2,
	/// Authorized, subject to per-channel settings and OS delivery policy.
	Granted = 3,
	/// Apple provisional authorization, typically quiet delivery.
	Provisional = 4,
	/// Apple temporary App Clip authorization.
	Ephemeral = 5,
}
impl Permission {
	/// Validates the exact four-byte response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?) {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::NotDetermined),
			2 => Ok(Self::Denied),
			3 => Ok(Self::Granted),
			4 => Ok(Self::Provisional),
			5 => Ok(Self::Ephemeral),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// A validated APNs device token: exactly the lower-case hex bytes the OS
/// handed the app, with no assumed length. Secret-ish material that names a
/// device: [`Debug`](std::fmt::Debug) shows its length alone, never its
/// bytes, and logs and error messages show none of it either.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceToken(String);
impl DeviceToken {
	/// Validates a token's shape: an even count of
	/// [`MIN_APNS_TOKEN_CHARS`]..=[`MAX_APNS_TOKEN_CHARS`] lower-case hex
	/// characters, whole bytes, whatever length the OS chose. The host
	/// lower-cases Apple's bytes before they cross the bridge; anything else
	/// is refused here.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();

		if value.len() < MIN_APNS_TOKEN_CHARS
			|| value.len() > MAX_APNS_TOKEN_CHARS
			|| value.len() % 2 != 0
			|| !value
				.bytes()
				.all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self(value))
	}

	/// Borrows the hex characters.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}
impl std::fmt::Debug for DeviceToken {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// The token names a device; its bytes never reach a log or a panic.
		write!(f, "DeviceToken(<{} hex chars>)", self.0.len())
	}
}

/// A validated FCM registration token: exactly the printable-ASCII bytes
/// Google's SDK handed the app, of whatever length it chose. Secret-ish
/// material that names a device: [`Debug`](std::fmt::Debug) shows its length
/// alone, never its bytes, and logs and error messages show none of it
/// either.
#[derive(Clone, PartialEq, Eq)]
pub struct FcmToken(String);
impl FcmToken {
	/// Validates a token's shape: 1..=[`MAX_FCM_TOKEN_CHARS`] bytes of
	/// printable ASCII with no spaces — the shape Firebase issues, not hex
	/// and not an assumed length. Anything else is refused here.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();

		if value.is_empty()
			|| value.len() > MAX_FCM_TOKEN_CHARS
			|| !value.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self(value))
	}

	/// Borrows the token's characters.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}
impl std::fmt::Debug for FcmToken {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// The token names a device; its bytes never reach a log or a panic.
		write!(f, "FcmToken(<{} chars>)", self.0.len())
	}
}

/// The host's latest remote-notification registration answer, or the
/// absence of one. [`Debug`](std::fmt::Debug) never prints a token's bytes.
#[derive(Clone, PartialEq, Eq)]
pub enum RemoteState {
	/// No answer has arrived since the host process started. A token the OS
	/// issued for a previous process run is not remembered: hosts hand
	/// tokens out again on launch.
	None,
	/// The OS issued an APNs device token, as lower-case hex. iOS and macOS
	/// answer this; Android never does.
	Token(DeviceToken),
	/// The OS issued an FCM registration token, as printable ASCII. Android
	/// answers this; iOS and macOS never do.
	FcmToken(FcmToken),
	/// The OS refused registration. The reason is one of the contract's
	/// stable [`remote_failure`] codes, bounded and control-free by
	/// construction, never the OS's own error text and never a token.
	Failed(String),
	/// The host asked the OS to register and no new answer arrived within
	/// its wait bound. Only `REMOTE_REGISTER` answers this; the broker's
	/// last answer, if any, is still readable through `REMOTE_TOKEN`. A
	/// caller must not treat a previous token as fresh because of it.
	Unconfirmed,
}
impl std::fmt::Debug for RemoteState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::None => f.write_str("None"),
			Self::Token(token) => f.debug_tuple("Token").field(token).finish(),
			Self::FcmToken(token) => f.debug_tuple("FcmToken").field(token).finish(),
			Self::Failed(reason) => f.debug_tuple("Failed").field(reason).finish(),
			Self::Unconfirmed => f.write_str("Unconfirmed"),
		}
	}
}

/// The stable failure codes a host may answer for remote registration.
/// Hosts never forward the OS's own error text: it can carry device or
/// provisioning detail an app has no business logging, and its shape varies
/// between OS releases. These codes are the whole contract.
pub mod remote_failure {
	/// The OS refused registration and the host could not narrow it further.
	/// Most often a missing or mismatched `aps-environment` entitlement on
	/// an Apple platform; the app's own messaging decides what to say.
	pub const OS_REFUSED: &str = "os-refused";
	/// The OS refused registration naming the `aps-environment` entitlement
	/// specifically: the app's signing does not carry the push entitlement
	/// registration needs. Apple platforms only; Firebase never answers it.
	pub const APS_ENVIRONMENT: &str = "aps-environment";
	/// The OS handed the host a token outside the contract's bounds. Its
	/// bytes are refused, not shortened or padded.
	pub const TOKEN_SHAPE: &str = "token-shape";
	/// Firebase is not usable in this app: the `firebase-messaging` classes
	/// are absent — `[native.android] push` never turned on — or the app has
	/// no Firebase initialization values, the `google-services.json` the app
	/// author installs. Nothing was asked and nothing was refused by an OS.
	pub const FIREBASE_NOT_CONFIGURED: &str = "firebase-not-configured";
}

/// A revisioned snapshot of the host's remote-registration state. Only the
/// latest answer is kept: a queue could hand a caller a token the OS has
/// already replaced, so observers compare [`RemoteRegistration::revision`]
/// and re-read instead of replaying history.
///
/// The revision is a full `u64` and `0` means no answer has arrived; every
/// answered state carries a revision of at least `1`, and the unanswered
/// [`RemoteState::Unconfirmed`] is `0` by definition: it is the absence of a
/// new answer, not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRegistration {
	revision: u64,
	state: RemoteState,
}
impl RemoteRegistration {
	/// Builds a snapshot. Revision `0` is reserved for "nothing answered":
	/// [`RemoteState::None`] and [`RemoteState::Unconfirmed`] take it alone,
	/// and every answered state takes a revision of at least `1`.
	pub fn new(revision: u64, state: RemoteState) -> Result<Self, Error> {
		let answered = !matches!(state, RemoteState::None | RemoteState::Unconfirmed);

		if answered != (revision != 0) {
			return Err(Error::InvalidArgument);
		}

		if let RemoteState::Failed(reason) = &state
			&& (reason.is_empty()
				|| reason.len() > MAX_REMOTE_FAILURE_BYTES
				|| !reason.bytes().all(|byte| byte.is_ascii_graphic()))
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self { revision, state })
	}

	/// The empty snapshot a host answers before any callback.
	#[must_use]
	pub fn none() -> Self {
		Self {
			revision: 0,
			state: RemoteState::None,
		}
	}

	/// The snapshot `REMOTE_REGISTER` answers when its wait bound passed
	/// with no new answer from the OS.
	#[must_use]
	pub fn unconfirmed() -> Self {
		Self {
			revision: 0,
			state: RemoteState::Unconfirmed,
		}
	}

	/// Monotonic answer counter; `0` means no answer has arrived.
	#[must_use]
	pub fn revision(&self) -> u64 {
		self.revision
	}

	/// The latest answer.
	#[must_use]
	pub fn state(&self) -> &RemoteState {
		&self.state
	}

	/// The device token, when the latest answer is one, of either kind.
	#[must_use]
	pub fn token(&self) -> Option<&str> {
		match &self.state {
			RemoteState::Token(token) => Some(token.as_str()),
			RemoteState::FcmToken(token) => Some(token.as_str()),
			_ => None,
		}
	}

	/// Encodes a `REMOTE_TOKEN`/`REMOTE_REGISTER` reply.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(16 + MAX_FCM_TOKEN_CHARS);
		output.u32((self.revision >> 32) as u32);
		output.u32(self.revision as u32);

		match &self.state {
			RemoteState::None => {
				output.u32(0);
				output.bytes(b"")?;
			}
			RemoteState::Token(token) => {
				output.u32(1);
				output.bytes(token.0.as_bytes())?;
			}
			RemoteState::FcmToken(token) => {
				output.u32(4);
				output.bytes(token.0.as_bytes())?;
			}
			RemoteState::Failed(reason) => {
				output.u32(2);
				output.bytes(reason.as_bytes())?;
			}
			RemoteState::Unconfirmed => {
				output.u32(3);
				output.bytes(b"")?;
			}
		}

		Ok(output.finish())
	}

	/// Validates a complete snapshot reply, including trailing bytes, under
	/// the same rules as [`RemoteRegistration::new`].
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 16 + MAX_FCM_TOKEN_CHARS.max(MAX_REMOTE_FAILURE_BYTES) {
			return Err(Error::InvalidArgument);
		}

		let mut input = Decoder::new(payload);
		let revision = u64::from(input.u32()?) << 32 | u64::from(input.u32()?);
		let state = match input.u32()? {
			0 => {
				let text = input.string()?;

				if !text.is_empty() {
					return Err(Error::InvalidArgument);
				}

				RemoteState::None
			}
			1 => RemoteState::Token(DeviceToken::new(input.string()?)?),
			2 => {
				let reason = input.string()?.to_owned();

				if reason.is_empty()
					|| reason.len() > MAX_REMOTE_FAILURE_BYTES
					|| !reason.bytes().all(|byte| byte.is_ascii_graphic())
				{
					return Err(Error::InvalidArgument);
				}

				RemoteState::Failed(reason)
			}
			3 => {
				let text = input.string()?;

				if !text.is_empty() {
					return Err(Error::InvalidArgument);
				}

				RemoteState::Unconfirmed
			}
			4 => RemoteState::FcmToken(FcmToken::new(input.string()?)?),
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;

		Self::new(revision, state)
	}
}

/// Stable application-chosen notification or channel identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier(String);
impl Identifier {
	/// Accepts 1..=64 ASCII letters, digits, hyphens, periods or underscores.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_id(&value)?;
		Ok(Self(value))
	}

	/// Borrows the identifier.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Encodes a cancellation/channel deletion payload.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.0.len());
		output.bytes(self.0.as_bytes())?;
		Ok(output.finish())
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > 64
		|| !value
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_text(
	value: &str,
	maximum: usize,
	multiline: bool,
	required: bool,
) -> Result<(), Error> {
	if value.len() > maximum
		|| (required && value.trim().is_empty())
		|| value
			.chars()
			.any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

/// Validated, plain-text local notification. A repeated ID replaces that app's
/// prior notification where supported; Android requires a configured channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
	id: Identifier,
	title: String,
	body: String,
	channel: Identifier,
}
impl Notification {
	/// Constructs bounded content, rejecting NUL/control injection.
	pub fn new(
		id: Identifier,
		title: impl Into<String>,
		body: impl Into<String>,
		channel: Identifier,
	) -> Result<Self, Error> {
		let title = title.into();
		let body = body.into();
		validate_text(&title, 256, false, true)?;
		validate_text(&body, 4096, true, false)?;
		Ok(Self {
			id,
			title,
			body,
			channel,
		})
	}

	/// Application-chosen replacement/cancellation ID.
	#[must_use]
	pub fn id(&self) -> &Identifier {
		&self.id
	}

	/// Plain-text heading.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Plain-text body.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Android channel identifier; other platforms do not use channel settings.
	#[must_use]
	pub fn channel(&self) -> &Identifier {
		&self.channel
	}

	/// Encodes content without a JSON intermediary.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(
			16 + self.id.0.len() + self.title.len() + self.body.len() + self.channel.0.len(),
		);
		for value in [
			self.id.as_str(),
			&self.title,
			&self.body,
			self.channel.as_str(),
		] {
			output.bytes(value.as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Validates the complete native control message, including trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 4496 {
			return Err(Error::InvalidArgument);
		}

		let mut input = Decoder::new(payload);
		let result = Self::new(
			Identifier::new(input.string()?)?,
			input.string()?,
			input.string()?,
			Identifier::new(input.string()?)?,
		)?;
		input.finish()?;
		Ok(result)
	}
}

/// Android channel importance. OS/user settings can override the initial value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Importance {
	/// Quiet notification without interrupting the user.
	Low = 1,
	/// Standard notification importance.
	Default = 2,
	/// High importance; visual interruption remains OS-controlled.
	High = 3,
}

/// Initial Android channel configuration. Existing channel importance/sound
/// are user-owned and cannot be changed by recreating the same channel ID.
#[derive(Debug, Clone)]
pub struct Channel {
	id: Identifier,
	name: String,
	description: String,
	importance: Importance,
	sound: bool,
}
impl Channel {
	/// Validates channel display metadata and explicit sound preference.
	pub fn new(
		id: Identifier,
		name: impl Into<String>,
		description: impl Into<String>,
		importance: Importance,
		sound: bool,
	) -> Result<Self, Error> {
		let name = name.into();
		let description = description.into();
		validate_text(&name, 128, false, true)?;
		validate_text(&description, 1024, true, false)?;
		Ok(Self {
			id,
			name,
			description,
			importance,
			sound,
		})
	}

	/// Encodes the Android-only channel operation.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output =
			Encoder::with_capacity(20 + self.id.0.len() + self.name.len() + self.description.len());

		for value in [self.id.as_str(), &self.name, &self.description] {
			output.bytes(value.as_bytes())?;
		}

		output.u32(self.importance as u32);
		output.u32(u32::from(self.sound));
		Ok(output.finish())
	}

	/// Validates an Android channel wire message.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 1236 {
			return Err(Error::InvalidArgument);
		}

		let mut input = Decoder::new(payload);
		let id = Identifier::new(input.string()?)?;
		let name = input.string()?;
		let description = input.string()?;

		let importance = match input.u32()? {
			1 => Importance::Low,
			2 => Importance::Default,
			3 => Importance::High,
			_ => return Err(Error::InvalidArgument),
		};

		let sound = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};

		input.finish()?;
		Self::new(id, name, description, importance, sound)
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub mod linux;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn bounded_messages_roundtrip_unicode_and_reject_ambiguous_input() {
		let note = Notification::new(
			Identifier::new("job-1").unwrap(),
			"Finished ✓",
			"Two\nlines",
			Identifier::new("general").unwrap(),
		)
		.unwrap();
		let bytes = note.encode().unwrap();
		assert_eq!(Notification::decode(&bytes).unwrap(), note);

		for length in 0..bytes.len() {
			assert!(Notification::decode(&bytes[..length]).is_err());
		}

		let mut trailing = bytes.to_vec();

		trailing.push(0);
		assert!(Notification::decode(&trailing).is_err());

		for id in ["", "../a/b", "bad\0id", "é", &"a".repeat(65)] {
			assert!(Identifier::new(id).is_err());
		}

		assert!(Notification::new(note.id.clone(), "\t", "", note.channel.clone()).is_err());
		assert!(Notification::new(note.id, "Valid", "bad\0body", note.channel).is_err());
	}
	#[test]
	fn channel_enums_and_permissions_are_strict() {
		let channel = Channel::new(
			Identifier::new("messages").unwrap(),
			"Messages",
			"",
			Importance::Low,
			false,
		)
		.unwrap();
		let mut bytes = channel.encode().unwrap().to_vec();
		assert!(Channel::decode(&bytes).is_ok());
		let last = bytes.len() - 4;
		bytes[last] = 2;
		assert!(Channel::decode(&bytes).is_err());

		assert_eq!(
			Permission::decode(&[4, 0, 0, 0]),
			Ok(Permission::Provisional)
		);

		for value in [&[][..], &[3], &[6, 0, 0, 0], &[3, 0, 0, 0, 0]] {
			assert!(Permission::decode(value).is_err());
		}
	}
	#[test]
	fn every_permission_state_decodes_exactly() {
		let expected = [
			Permission::Unknown,
			Permission::NotDetermined,
			Permission::Denied,
			Permission::Granted,
			Permission::Provisional,
			Permission::Ephemeral,
		];

		for (value, permission) in (0u32..).zip(expected) {
			assert_eq!(Permission::decode(&value.to_le_bytes()), Ok(permission));
		}
	}
	#[test]
	fn identifiers_and_notification_accessors_preserve_content() {
		let id = Identifier::new("backup.done_2").unwrap();
		assert_eq!(id.as_str(), "backup.done_2");
		let mut expected = Encoder::default();
		expected.bytes(b"backup.done_2").unwrap();
		assert_eq!(id.encode().unwrap(), expected.finish());
		let channel = Identifier::new("general").unwrap();

		let note = Notification::new(id.clone(), "Backup", "Done\tok", channel.clone()).unwrap();
		assert_eq!(note.id(), &id);
		assert_eq!(note.title(), "Backup");
		assert_eq!(note.body(), "Done\tok");
		assert_eq!(note.channel(), &channel);
		assert!(Notification::new(id.clone(), "x".repeat(257), "", channel.clone()).is_err());
		assert!(Notification::new(id, "Title\nInjected", "", channel).is_err());
	}
	#[test]
	fn notification_decode_rejects_oversized_and_invalid_channel_messages() {
		assert_eq!(
			Notification::decode(&[0; 4497]),
			Err(Error::InvalidArgument)
		);

		let mut bad_channel = Encoder::default();

		for value in ["job", "Title", "", "no/slash"] {
			bad_channel.bytes(value.as_bytes()).unwrap();
		}

		assert_eq!(
			Notification::decode(&bad_channel.finish()),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn channel_decode_accepts_every_importance_and_sound_preference() {
		for (importance, sound) in [
			(Importance::Low, true),
			(Importance::Default, false),
			(Importance::High, true),
		] {
			let channel = Channel::new(
				Identifier::new("alerts").unwrap(),
				"Alerts",
				"Line one\nline two",
				importance,
				sound,
			)
			.unwrap();
			let encoded = channel.encode().unwrap();
			let decoded = Channel::decode(&encoded).unwrap();
			assert_eq!(decoded.importance, importance);
			assert_eq!(decoded.sound, sound);
			assert_eq!(decoded.encode().unwrap(), encoded);
			let mut bad_importance = encoded.to_vec();
			let offset = bad_importance.len() - 8;
			bad_importance[offset] = 4;
			assert_eq!(
				Channel::decode(&bad_importance).unwrap_err(),
				Error::InvalidArgument
			);
		}
		assert_eq!(
			Channel::decode(&[0; 1237]).unwrap_err(),
			Error::InvalidArgument
		);
		assert!(
			Channel::new(
				Identifier::new("alerts").unwrap(),
				" ",
				"",
				Importance::Low,
				false
			)
			.is_err()
		);
	}
	/// A 64-hex-character token, the length Apple issues today, and a
	/// deliberately different one: the contract must not assume 32 bytes.
	fn sample_tokens() -> [String; 2] {
		[
			"a1b2c3d4e5f67890".repeat(4),
			format!("{}{}", "f".repeat(31), "0".repeat(35)),
		]
	}
	/// An FCM-shaped registration token: URL-safe base64, neither hex nor a
	/// length anyone assumes — 131 characters, in Firebase's own alphabet.
	/// Fake material.
	fn fcm_sample_token() -> FcmToken {
		let token = format!("{}:{}", "e".repeat(100), "Az09_-".repeat(5));
		FcmToken::new(token).expect("the fake token is in bounds")
	}
	#[test]
	fn device_tokens_validate_shape_without_assuming_a_length() {
		for token in sample_tokens() {
			assert_eq!(DeviceToken::new(token.clone()).unwrap().as_str(), token);
		}

		// Uppercase, too short, too long, odd-length, and non-hex are all
		// refused: the host lower-cases Apple's whole bytes before they
		// cross the bridge, so a lone nibble is corruption, not a token.
		assert!(DeviceToken::new(sample_tokens()[0].to_ascii_uppercase()).is_err());
		assert!(DeviceToken::new("a".repeat(MIN_APNS_TOKEN_CHARS - 1)).is_err());
		assert!(DeviceToken::new("a".repeat(MAX_APNS_TOKEN_CHARS + 1)).is_err());
		assert!(DeviceToken::new(format!("{}g", "a".repeat(MIN_APNS_TOKEN_CHARS - 1))).is_err());
		assert!(DeviceToken::new("a".repeat(MIN_APNS_TOKEN_CHARS + 1)).is_err());
		assert!(DeviceToken::new("").is_err());
	}

	/// An FCM registration token is printable ASCII of no assumed length —
	/// never hex — and the service's own bound is the contract's.
	#[test]
	fn fcm_tokens_validate_shape_without_assuming_hex() {
		let token = "WebView-Help:AZerty0-9_".repeat(8); // gitleaks:allow (a fake FCM token a test builds)
		assert_eq!(FcmToken::new(token.clone()).unwrap().as_str(), token);

		// Short, printable, unusual: Firebase's own alphabet decides the
		// shape, so nothing here demands a length or a hex nibble.
		assert!(FcmToken::new("a".repeat(MAX_FCM_TOKEN_CHARS)).is_ok());
		assert!(FcmToken::new("").is_err());
		assert!(FcmToken::new("a".repeat(MAX_FCM_TOKEN_CHARS + 1)).is_err());
		assert!(FcmToken::new("a b").is_err());
		assert!(FcmToken::new("a\tb").is_err());
		assert!(FcmToken::new("héllo").is_err());
		assert!(FcmToken::new("a\0b").is_err());
	}

	/// The token names a device: no `Debug` output — token, snapshot, or
	/// state — carries its bytes, so a logged or panicked value leaks
	/// nothing.
	#[test]
	fn debug_output_never_carries_token_bytes() {
		let token = DeviceToken::new(sample_tokens()[0].clone()).unwrap();
		let token_debug = format!("{token:?}");
		assert_eq!(token_debug, "DeviceToken(<64 hex chars>)");
		assert!(!token_debug.contains(&sample_tokens()[0]));

		let snapshot = RemoteRegistration::new(7, RemoteState::Token(token)).unwrap();
		for debug in [
			format!("{:?}", snapshot.state()),
			format!("{snapshot:?}"),
			format!("{:?}", RemoteState::None),
			format!("{:?}", RemoteRegistration::unconfirmed()),
		] {
			assert!(!debug.contains(&sample_tokens()[0]), "{debug}");
		}

		// The FCM token redacts the same way: a length, never bytes.
		let fcm = fcm_sample_token();
		let fcm_bytes = fcm.as_str().to_owned();
		let fcm_debug = format!("{fcm:?}");
		assert_eq!(fcm_debug, "FcmToken(<131 chars>)", "{fcm_debug}");
		assert!(!fcm_debug.contains(&fcm_bytes));
		let fcm_snapshot = RemoteRegistration::new(1, RemoteState::FcmToken(fcm)).unwrap();
		for debug in [
			format!("{:?}", fcm_snapshot.state()),
			format!("{fcm_snapshot:?}"),
		] {
			assert!(!debug.contains(&fcm_bytes), "{debug}");
		}
	}

	#[test]
	fn remote_snapshots_round_trip_every_state() {
		let none = RemoteRegistration::none();
		assert_eq!(none.revision(), 0);
		assert_eq!(none.state(), &RemoteState::None);
		assert_eq!(
			RemoteRegistration::decode(&none.encode().unwrap()),
			Ok(none)
		);

		let token = RemoteRegistration::new(
			7,
			RemoteState::Token(DeviceToken::new(sample_tokens()[1].clone()).unwrap()),
		)
		.unwrap();
		assert_eq!(token.token(), Some(sample_tokens()[1].as_str()));
		assert_eq!(
			RemoteRegistration::decode(&token.encode().unwrap()),
			Ok(token)
		);

		// An FCM token rides the same snapshot with its own state, and the
		// two token kinds never answer for each other.
		let fcm = fcm_sample_token();
		let fcm_snapshot = RemoteRegistration::new(4, RemoteState::FcmToken(fcm.clone())).unwrap();
		assert_eq!(fcm_snapshot.token(), Some(fcm.as_str()));
		assert_eq!(
			RemoteRegistration::decode(&fcm_snapshot.encode().unwrap()),
			Ok(fcm_snapshot)
		);

		let failed = RemoteRegistration::new(
			9,
			RemoteState::Failed(remote_failure::APS_ENVIRONMENT.into()),
		)
		.unwrap();
		assert!(failed.token().is_none());
		assert_eq!(
			RemoteRegistration::decode(&failed.encode().unwrap()),
			Ok(failed)
		);

		let unconfirmed = RemoteRegistration::unconfirmed();
		assert_eq!(unconfirmed.revision(), 0);
		assert_eq!(unconfirmed.state(), &RemoteState::Unconfirmed);
		assert!(unconfirmed.token().is_none());
		assert_eq!(
			RemoteRegistration::decode(&unconfirmed.encode().unwrap()),
			Ok(unconfirmed)
		);

		// The revision is a full u64: the wire carries both halves, and the
		// constructor refuses nothing but the answered/revision-0 mismatch.
		let huge = RemoteRegistration::new(
			u64::MAX,
			RemoteState::Token(DeviceToken::new(sample_tokens()[0].clone()).unwrap()),
		)
		.unwrap();
		assert_eq!(huge.revision(), u64::MAX);
		assert_eq!(
			RemoteRegistration::decode(&huge.encode().unwrap()),
			Ok(huge)
		);
	}
	#[test]
	fn remote_snapshot_decode_rejects_broken_payloads() {
		let token = RemoteRegistration::new(
			3,
			RemoteState::Token(DeviceToken::new(sample_tokens()[0].clone()).unwrap()),
		)
		.unwrap();
		let bytes = token.encode().unwrap();

		for length in 0..bytes.len() {
			assert!(RemoteRegistration::decode(&bytes[..length]).is_err());
		}

		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(RemoteRegistration::decode(&trailing).is_err());
		assert_eq!(
			RemoteRegistration::decode(&[0; 17 + MAX_FCM_TOKEN_CHARS]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			RemoteRegistration::decode(&[0; 17 + MAX_REMOTE_FAILURE_BYTES]),
			Err(Error::InvalidArgument)
		);

		// State 5 is nothing: the wire's token states are 1 (APNs hex) and
		// 4 (FCM printable ASCII) alone.
		let mut unknown_state = Encoder::default();
		unknown_state.u32(0);
		unknown_state.u32(1);
		unknown_state.u32(5);
		unknown_state.bytes(b"").unwrap();
		assert!(RemoteRegistration::decode(&unknown_state.finish()).is_err());

		// Revision 0 means "no answer": an answered state with it, and an
		// unanswered state without it, are both refused — on the constructor
		// and the decoder alike.
		assert_eq!(
			RemoteRegistration::new(
				0,
				RemoteState::Token(DeviceToken::new(sample_tokens()[0].clone()).unwrap())
			)
			.unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(
			RemoteRegistration::new(0, RemoteState::Failed(remote_failure::OS_REFUSED.into()))
				.unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(
			RemoteRegistration::new(1, RemoteState::None).unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(
			RemoteRegistration::new(1, RemoteState::Unconfirmed).unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(
			RemoteRegistration::new(0, RemoteState::None).unwrap(),
			RemoteRegistration::none()
		);
		assert_eq!(
			RemoteRegistration::new(0, RemoteState::Unconfirmed).unwrap(),
			RemoteRegistration::unconfirmed()
		);

		let mut zero_answered = Encoder::default();
		zero_answered.u32(0);
		zero_answered.u32(0);
		zero_answered.u32(1);
		zero_answered.bytes(sample_tokens()[0].as_bytes()).unwrap();
		assert!(RemoteRegistration::decode(&zero_answered.finish()).is_err());

		let mut bad_state = Encoder::default();
		bad_state.u32(0);
		bad_state.u32(1);
		bad_state.u32(3);
		bad_state.bytes(b"").unwrap();
		assert!(RemoteRegistration::decode(&bad_state.finish()).is_err());

		// A token-state string must be a valid token; a failure-state string
		// must be one of the contract's codes: non-empty, printable, bounded.
		let mut upper = Encoder::default();
		upper.u32(0);
		upper.u32(1);
		upper.u32(1);
		upper
			.bytes(sample_tokens()[0].to_ascii_uppercase().as_bytes())
			.unwrap();
		assert!(RemoteRegistration::decode(&upper.finish()).is_err());

		let mut control = Encoder::default();
		control.u32(0);
		control.u32(1);
		control.u32(2);
		control.bytes(b"bad\nreason").unwrap();
		assert!(RemoteRegistration::decode(&control.finish()).is_err());

		let mut long_reason = Encoder::default();
		long_reason.u32(0);
		long_reason.u32(1);
		long_reason.u32(2);
		long_reason
			.bytes(&vec![b'x'; MAX_REMOTE_FAILURE_BYTES + 1])
			.unwrap();
		assert!(RemoteRegistration::decode(&long_reason.finish()).is_err());

		for reason in ["", "\t", "a b"] {
			assert_eq!(
				RemoteRegistration::new(1, RemoteState::Failed(reason.into())).unwrap_err(),
				Error::InvalidArgument,
				"{reason:?}"
			);
		}
		assert!(
			RemoteRegistration::new(1, RemoteState::Failed("x".repeat(MAX_REMOTE_FAILURE_BYTES)))
				.is_ok()
		);
		for code in [
			remote_failure::OS_REFUSED,
			remote_failure::APS_ENVIRONMENT,
			remote_failure::TOKEN_SHAPE,
		] {
			assert!(RemoteRegistration::new(1, RemoteState::Failed(code.into())).is_ok());
		}
	}
}
