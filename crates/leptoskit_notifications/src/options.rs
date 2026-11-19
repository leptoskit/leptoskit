//! How a send shapes the notification each platform shows: silent data
//! messages, priority, an image, a sound, a badge, a click link, and the
//! per-platform overrides Android, Apple, and the web each understand.
//!
//! Every field is optional and [`Default`] is exactly the behaviour the
//! service had before options existed: a visible notification at normal
//! presentation. A send that sets nothing serializes without an options
//! object at all — its intent hash, its stored spec, and every provider
//! payload it produces are byte for byte what they always were.
//!
//! The checks here run on both sides, like every shared check: an app's
//! server refuses a bad option without a round trip, and the service
//! refuses it all the same.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The longest `image` or `link` URL, in bytes: FCM's own bound for a
/// notification's image.
pub const MAX_OPTION_URL_LEN: usize = 1_024;

/// The longest sound name, in bytes.
pub const MAX_SOUND_LEN: usize = 64;

/// The longest Android channel id, in bytes.
pub const MAX_CHANNEL_ID_LEN: usize = 64;

/// The longest Android notification icon, in bytes: a resource name or a
/// URL.
pub const MAX_ANDROID_ICON_LEN: usize = 512;

/// The longest Android notification tag, in bytes.
pub const MAX_ANDROID_TAG_LEN: usize = 256;

/// The longest Apple subtitle, in bytes: the same bound a title has.
pub const MAX_SUBTITLE_LEN: usize = 200;

/// The longest Apple thread id, in bytes.
pub const MAX_THREAD_ID_LEN: usize = 64;

/// The longest Apple category, in bytes: the same bound a send's own
/// category has.
pub const MAX_APPLE_CATEGORY_LEN: usize = 64;

/// The longest web icon or badge icon, in bytes.
pub const MAX_WEB_ICON_LEN: usize = 1_024;

/// The most actions a web notification may carry: the platform's own
/// ceiling for a `showNotification` action set.
pub const MAX_WEB_ACTIONS: usize = 2;

/// The longest web action id, in bytes.
pub const MAX_ACTION_ID_LEN: usize = 64;

/// The longest web action title, in bytes.
pub const MAX_ACTION_TITLE_LEN: usize = 100;

/// The longest web language tag, in bytes: BCP 47's own ceiling.
pub const MAX_LANG_LEN: usize = 35;

/// The most bytes a send's options take as the JSON every one of its
/// rows stores. Each field's own bound passes options larger than this
/// together, so the whole is bounded too.
pub const MAX_OPTIONS_LEN: usize = 4_096;

/// How a send arrives on a device.
///
/// [`Notification`](Self::Notification) is the default: a visible
/// notification, shown by the platform from the payload. [`Data`](Self::Data)
/// is a **silent background message**: no banner anywhere — APNs answers
/// with `content-available` under `apns-push-type: background` at priority
/// 5, FCM carries the data map with no notification block, and the
/// receiver service worker shows nothing — while the send's `data` reaches
/// the app, which decides what to do with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
	/// A visible notification: the platform shows it from the payload.
	#[default]
	Notification,
	/// A silent background message: nothing is shown; the app's code
	/// receives the data. Data sends create no inbox row and never count
	/// toward a badge.
	Data,
}

impl MessageKind {
	/// The stored name.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Notification => "notification",
			Self::Data => "data",
		}
	}

	/// The kind a stored name stands for, or `None` for anything else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"notification" => Some(Self::Notification),
			"data" => Some(Self::Data),
			_ => None,
		}
	}
}

/// How urgently a send should wake a device: APNs `apns-priority` 10 or
/// 5, FCM `android.priority` HIGH or NORMAL, Web Push `Urgency` high or
/// normal. Unset keeps each platform's existing default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendPriority {
	/// The platform's most urgent delivery: wake the device now.
	High,
	/// The platform's ordinary delivery, saving battery where it can.
	Normal,
}

/// The sound a notification plays. Unset leaves the choice to the
/// platform and its system settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSound {
	/// The platform's default notification sound.
	Default,
	/// No sound at all: on Apple platforms and the web this is the same
	/// as sending no sound (they are silent without one); on Android the
	/// channel's own setting decides, because a channel is the only
	/// silence Android honors per notification.
	Silent,
	/// A named sound: `chime.aiff` on Apple platforms, the raw resource
	/// name on Android. Validated to 1–64 bytes of letters, digits,
	/// `-`, `_`, and `.`.
	Named(String),
}

/// What a send sets the app badge to. The badge is per **recipient**, so
/// [`Unread`](Self::Unread) is computed at push time for each person the
/// send reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BadgeSetting {
	/// That exact number. APNs `aps.badge` replaces whatever the app
	/// shows; `0` clears it.
	Set(u32),
	/// The recipient's live unread count, computed per delivery at push
	/// time: what [`unread_count`](crate::unread_count) would answer
	/// right then.
	Unread,
}

/// How interruptive Apple platforms treat a notification:
/// `aps.interruption-level`, which Focus modes and scheduling read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionLevel {
	/// Delivered quietly: no sound, no screen light-up, no vibration.
	Passive,
	/// The ordinary level: presented immediately if allowed.
	Active,
	/// Time sensitive: breaks through Focus when the person allows the
	/// category to.
	TimeSensitive,
	/// Critical: breaks through Focus and silence. Delivery requires an
	/// entitlement Apple grants separately.
	Critical,
}

impl InterruptionLevel {
	/// The `aps.interruption-level` wire name.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Passive => "passive",
			Self::Active => "active",
			Self::TimeSensitive => "time-sensitive",
			Self::Critical => "critical",
		}
	}
}

/// Which way a web notification's text reads: `showNotification`'s
/// `dir` option.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextDirection {
	/// Let the browser decide from the content.
	Auto,
	/// Left to right.
	Ltr,
	/// Right to left.
	Rtl,
}

impl TextDirection {
	/// The wire name the receiver passes through.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Auto => "auto",
			Self::Ltr => "ltr",
			Self::Rtl => "rtl",
		}
	}
}

/// One action button a web notification offers: an id the
/// `notificationclick` event carries, and the title on the button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebPushAction {
	/// The action's id, 1–64 bytes of printable text, as the click event
	/// answers it.
	pub action: String,
	/// The button's title, at most 100 bytes of printable text.
	pub title: String,
}

/// The Android-specific shape of one send: `android.notification`'s
/// placement and identity, as FCM's HTTP v1 API takes them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct AndroidOptions {
	/// The notification channel it posts through. The channel must exist
	/// (the local-notifications plugin configures one) or Android drops
	/// the post.
	pub channel_id: Option<String>,
	/// The accent color, `#RRGGBB` exactly.
	pub color: Option<String>,
	/// The small icon's resource name or URL. Unset uses the app's own.
	pub icon: Option<String>,
	/// Groups notifications on the device: the same tag replaces, a new
	/// one stacks. Independent of the send's own collapse id.
	pub tag: Option<String>,
	/// Keeps the notification on screen when the person dismisses the
	/// shade, the way an ongoing call does.
	pub sticky: Option<bool>,
}

/// The Apple-specific shape of one send: the alert's subtitle and the
/// `aps` keys iOS and macOS read for grouping and interruption.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct AppleOptions {
	/// A second line under the title, at most 200 bytes.
	pub subtitle: Option<String>,
	/// `aps.thread-id`: groups notifications in the inbox and collapses
	/// their stacks.
	pub thread_id: Option<String>,
	/// `aps.category`: the action category the app registered for its
	/// notifications.
	pub category: Option<String>,
	/// `aps.interruption-level`: how Focus modes treat it.
	pub interruption_level: Option<InterruptionLevel>,
	/// `aps.relevance-score`, 0 to 1: how the notification sorts in the
	/// system's own list.
	pub relevance_score: Option<f64>,
}

/// The web-specific shape of one send: the presentation options the
/// receiver service worker hands to `showNotification`. Only Web Push
/// endpoints read these.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct WebOptions {
	/// The notification's icon, at most 1024 bytes.
	pub icon: Option<String>,
	/// The badge — the compact shape the shelf or dock tray shows — at
	/// most 1024 bytes.
	pub badge_icon: Option<String>,
	/// Up to two action buttons.
	pub actions: Vec<WebPushAction>,
	/// Keeps the notification on screen until the person answers it,
	/// instead of auto-dismissing.
	pub require_interaction: Option<bool>,
	/// Which way the notification's text reads.
	pub dir: Option<TextDirection>,
	/// The notification's language tag, like `de` or `pt-BR`.
	pub lang: Option<String>,
}

/// How a send shapes the notification each platform shows. Everything is
/// optional; [`MessageOptions::is_empty`] is the send that had no options
/// at all, and it serializes — into an intent hash, a stored spec, or a
/// provider payload — exactly as sends did before this type existed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct MessageOptions {
	/// A visible notification or a silent data message. Data sends
	/// create no inbox row and never count toward a badge, but their
	/// delivery is durable and reported like any send's.
	#[serde(default, skip_serializing_if = "is_default_kind")]
	pub kind: MessageKind,
	/// How urgently to wake the device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub priority: Option<SendPriority>,
	/// An image the notification shows, `https`, at most
	/// [`MAX_OPTION_URL_LEN`] bytes. Apple platforms need a Notification
	/// Service Extension in the app to display it; Android and the web
	/// show it natively.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub image: Option<String>,
	/// The sound the notification plays.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sound: Option<NotificationSound>,
	/// What the send sets the recipient's app badge to.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub badge: Option<BadgeSetting>,
	/// Where a tap goes: an `https` URL or a path starting with `/`, at
	/// most [`MAX_OPTION_URL_LEN`] bytes, carried in every payload. The
	/// web receiver opens it on click (this origin only); native tap
	/// routing reads it from the payload.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub link: Option<String>,
	/// Android-only presentation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub android: Option<AndroidOptions>,
	/// Apple-only presentation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub apple: Option<AppleOptions>,
	/// Web-only presentation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub web: Option<WebOptions>,
}

impl MessageOptions {
	/// Whether every option is its default: the send that shaped nothing.
	/// Such an options value is not serialized at all, so a send without
	/// options hashes, stores, and pushes byte for byte as it always did.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.kind == MessageKind::Notification
			&& self.priority.is_none()
			&& self.image.is_none()
			&& self.sound.is_none()
			&& self.badge.is_none()
			&& self.link.is_none()
			&& self.android.is_none()
			&& self.apple.is_none()
			&& self.web.is_none()
	}

	/// Whether this is a silent data message.
	#[must_use]
	pub fn is_data(&self) -> bool {
		self.kind == MessageKind::Data
	}
}

/// `true` only for the explicit default kind, so a plain notification
/// serializes without a `kind` field.
#[allow(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_kind(kind: &MessageKind) -> bool {
	*kind == MessageKind::Notification
}

/// Checks a send's options, on both sides of the wire.
///
/// # Errors
///
/// A message naming the field and what's wrong with it.
pub fn check_options(options: &MessageOptions) -> Result<(), String> {
	if let Some(image) = options.image.as_deref() {
		check_https_url(image, "options.image")?;
	}

	if let Some(link) = options.link.as_deref() {
		check_link(link)?;
	}

	if let Some(NotificationSound::Named(name)) = options.sound.as_ref()
		&& (name.is_empty()
			|| name.len() > MAX_SOUND_LEN
			|| !name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
	{
		return Err(format!(
			"options.sound names a sound of 1 to {MAX_SOUND_LEN} bytes of letters, digits, `-`, \
			 `_`, and `.`"
		));
	}

	if let Some(android) = options.android.as_ref() {
		check_printable(
			android.channel_id.as_deref(),
			MAX_CHANNEL_ID_LEN,
			"options.android.channel_id",
		)?;
		check_printable(
			android.icon.as_deref(),
			MAX_ANDROID_ICON_LEN,
			"options.android.icon",
		)?;
		check_printable(
			android.tag.as_deref(),
			MAX_ANDROID_TAG_LEN,
			"options.android.tag",
		)?;

		if let Some(color) = android.color.as_deref() {
			let hex = color.len() == 7
				&& color.as_bytes()[0] == b'#'
				&& color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit);

			if !hex {
				return Err("options.android.color is `#RRGGBB`".into());
			}
		}
	}

	if let Some(apple) = options.apple.as_ref() {
		check_printable(
			apple.subtitle.as_deref(),
			MAX_SUBTITLE_LEN,
			"options.apple.subtitle",
		)?;
		check_printable(
			apple.thread_id.as_deref(),
			MAX_THREAD_ID_LEN,
			"options.apple.thread_id",
		)?;
		check_printable(
			apple.category.as_deref(),
			MAX_APPLE_CATEGORY_LEN,
			"options.apple.category",
		)?;

		if let Some(score) = apple.relevance_score
			&& !(0.0..=1.0).contains(&score)
		{
			return Err("options.apple.relevance_score is a number from 0 to 1".into());
		}
	}

	if let Some(web) = options.web.as_ref() {
		check_display_url(web.icon.as_deref(), "options.web.icon")?;
		check_display_url(web.badge_icon.as_deref(), "options.web.badge_icon")?;

		if web.actions.len() > MAX_WEB_ACTIONS {
			return Err(format!(
				"options.web.actions carries at most {MAX_WEB_ACTIONS} actions"
			));
		}

		for action in &web.actions {
			check_printable(
				Some(action.action.as_str()),
				MAX_ACTION_ID_LEN,
				"options.web.actions.action",
			)?;
			check_printable(
				Some(action.title.as_str()),
				MAX_ACTION_TITLE_LEN,
				"options.web.actions.title",
			)?;
		}

		if let Some(lang) = web.lang.as_deref()
			&& (lang.is_empty()
				|| lang.len() > MAX_LANG_LEN
				|| !lang
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
		{
			return Err(format!(
				"options.web.lang is a language tag of 1 to {MAX_LANG_LEN} bytes, like `de` or \
				 `pt-BR`"
			));
		}
	}

	if crate::stored_len(options) > MAX_OPTIONS_LEN {
		return Err(format!(
			"options are at most {MAX_OPTIONS_LEN} bytes once stored as JSON: drop or shorten \
			 some of them"
		));
	}

	Ok(())
}

/// An `https` URL with no credentials and no fragment, within
/// [`MAX_OPTION_URL_LEN`].
///
/// # Errors
///
/// A message naming `field`.
fn check_https_url(value: &str, field: &str) -> Result<(), String> {
	if value.starts_with('/') {
		return Err(format!(
			"{field} is an absolute https URL, not a path starting with \"/\""
		));
	}

	let parsed = parse_display_url(value, field)?;

	if parsed.scheme() != "https" {
		return Err(format!("{field} is https"));
	}

	Ok(())
}

/// An `https` URL or an app-relative path — the two shapes an option the
/// receiver displays may take, resolved against its own origin when it is
/// a path.
///
/// # Errors
///
/// A message naming `field`.
fn check_display_url(value: Option<&str>, field: &str) -> Result<(), String> {
	let Some(value) = value else {
		return Ok(());
	};

	if value.starts_with('/') {
		if value.starts_with("//") {
			return Err(format!(
				"{field} is on this origin: it may not name another authority"
			));
		}

		if value.len() > MAX_OPTION_URL_LEN {
			return Err(format!("{field} is at most {MAX_OPTION_URL_LEN} bytes"));
		}

		if value.contains('\\') || value.chars().any(char::is_control) {
			return Err(format!(
				"{field} is a path of printable text without a backslash"
			));
		}

		return Ok(());
	}

	let parsed = parse_display_url(value, field)?;

	if parsed.scheme() != "https" {
		return Err(format!("{field} is https or a path starting with \"/\""));
	}

	Ok(())
}

/// The parsed URL both display checks share, once the shape is known to
/// be absolute: no credentials, no fragment, a host, and within bound.
///
/// # Errors
///
/// A message naming `field`.
fn parse_display_url(value: &str, field: &str) -> Result<url::Url, String> {
	if value.len() > MAX_OPTION_URL_LEN {
		return Err(format!("{field} is at most {MAX_OPTION_URL_LEN} bytes"));
	}

	let parsed = url::Url::parse(value)
		.map_err(|_| format!("{field} is an absolute https URL or a path starting with \"/\""))?;

	if !parsed.username().is_empty() || parsed.password().is_some() {
		return Err(format!("{field} carries no credentials in its URL"));
	}

	if parsed.fragment().is_some() {
		return Err(format!("{field} has no fragment"));
	}

	if parsed.host_str().is_none_or(str::is_empty) {
		return Err(format!("{field} names a host"));
	}

	Ok(parsed)
}

/// A click `link`: an `https` URL or a path on the app's own origin.
///
/// # Errors
///
/// A message naming the field.
fn check_link(value: &str) -> Result<(), String> {
	if value.len() > MAX_OPTION_URL_LEN {
		return Err(format!(
			"options.link is at most {MAX_OPTION_URL_LEN} bytes"
		));
	}

	if value.starts_with('/') {
		if value.starts_with("//") {
			return Err(
				"options.link is on your own origin: it may not name another authority".into(),
			);
		}

		if value.contains('\\') {
			return Err("options.link has no backslash".into());
		}

		if value.chars().any(char::is_control) {
			return Err("options.link has no control characters".into());
		}

		return Ok(());
	}

	check_https_url(value, "options.link")
}

/// Optional printable text within `max` bytes, no control characters.
///
/// # Errors
///
/// A message naming `field`.
fn check_printable(value: Option<&str>, max: usize, field: &str) -> Result<(), String> {
	let Some(value) = value else {
		return Ok(());
	};

	if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
		return Err(format!("{field} is 1 to {max} bytes of printable text"));
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::AndroidOptions;
	use super::AppleOptions;
	use super::BadgeSetting;
	use super::InterruptionLevel;
	use super::MessageKind;
	use super::MessageOptions;
	use super::NotificationSound;
	use super::SendPriority;
	use super::WebOptions;
	use super::WebPushAction;
	use super::check_options;

	fn options() -> MessageOptions {
		MessageOptions {
			priority: Some(SendPriority::High),
			image: Some("https://cdn.example.test/paid.png".into()),
			sound: Some(NotificationSound::Named("chime.aiff".into())),
			badge: Some(BadgeSetting::Unread),
			link: Some("/invoice/42".into()),
			android: Some(AndroidOptions {
				channel_id: Some("inbox".into()),
				color: Some("#10b981".into()),
				icon: Some("ic_income".into()),
				tag: Some("billing".into()),
				sticky: Some(true),
			}),
			apple: Some(AppleOptions {
				subtitle: Some("Invoice #42".into()),
				thread_id: Some("billing".into()),
				category: Some("BILLING".into()),
				interruption_level: Some(InterruptionLevel::TimeSensitive),
				relevance_score: Some(1.0),
			}),
			web: Some(WebOptions {
				icon: Some("https://cdn.example.test/icon.png".into()),
				badge_icon: Some("/badge.png".into()),
				actions: vec![WebPushAction {
					action: "open".into(),
					title: "Open invoice".into(),
				}],
				require_interaction: Some(true),
				dir: None,
				lang: Some("en".into()),
			}),
			..MessageOptions::default()
		}
	}

	/// The kitchen-sink options pass, and each shape's refusal names the
	/// field it gave up on.
	#[test]
	fn options_validate_with_precise_errors() {
		assert!(check_options(&options()).is_ok());
		assert!(check_options(&MessageOptions::default()).is_ok());

		let refusals = [
			(
				MessageOptions {
					image: Some("http://cdn.example.test/p.png".into()),
					..MessageOptions::default()
				},
				"options.image is https",
			),
			(
				MessageOptions {
					image: Some("https://cdn.example.test/p.png#frag".into()),
					..MessageOptions::default()
				},
				"options.image has no fragment",
			),
			(
				MessageOptions {
					image: Some(format!(
						"https://cdn.example.test/{}.png",
						"x".repeat(1_100)
					)),
					..MessageOptions::default()
				},
				"options.image is at most 1024 bytes",
			),
			(
				MessageOptions {
					link: Some("invoice/42".into()),
					..MessageOptions::default()
				},
				"options.link is an absolute https URL or a path",
			),
			(
				MessageOptions {
					link: Some("//evil.test/invoice".into()),
					..MessageOptions::default()
				},
				"options.link is on your own origin",
			),
			(
				MessageOptions {
					sound: Some(NotificationSound::Named("no spaces".into())),
					..MessageOptions::default()
				},
				"options.sound names a sound",
			),
			(
				MessageOptions {
					android: Some(AndroidOptions {
						color: Some("10b981".into()),
						..AndroidOptions::default()
					}),
					..MessageOptions::default()
				},
				"options.android.color is `#RRGGBB`",
			),
			(
				MessageOptions {
					apple: Some(AppleOptions {
						relevance_score: Some(1.5),
						..AppleOptions::default()
					}),
					..MessageOptions::default()
				},
				"options.apple.relevance_score",
			),
			(
				MessageOptions {
					web: Some(WebOptions {
						actions: vec![
							WebPushAction {
								action: "a".into(),
								title: "A".into(),
							},
							WebPushAction {
								action: "b".into(),
								title: "B".into(),
							},
							WebPushAction {
								action: "c".into(),
								title: "C".into(),
							},
						],
						..WebOptions::default()
					}),
					..MessageOptions::default()
				},
				"options.web.actions carries at most 2",
			),
			(
				MessageOptions {
					web: Some(WebOptions {
						lang: Some("de_AT".into()),
						..WebOptions::default()
					}),
					..MessageOptions::default()
				},
				"options.web.lang",
			),
		];

		for (value, complaint) in refusals {
			let error = check_options(&value).unwrap_err();
			assert!(
				error.contains(complaint),
				"{complaint} names the refusal: {error}"
			);
		}
	}

	/// Options with only defaults serialize to nothing: a send that set
	/// nothing is byte for byte the sends that came before, in its intent
	/// hash and its stored spec alike.
	#[test]
	fn empty_options_serialize_to_nothing() {
		let plain = serde_json::to_value(MessageOptions::default()).unwrap();
		assert_eq!(plain, serde_json::json!({}));

		let explicit = serde_json::to_value(&MessageOptions {
			kind: MessageKind::Notification,
			..MessageOptions::default()
		})
		.unwrap();
		assert_eq!(plain, explicit, "an explicit default kind is still empty");

		let set = serde_json::to_value(options()).unwrap();
		assert_eq!(
			set["priority"], "high",
			"a set option serializes in snake case: {set}"
		);
		assert_eq!(set["apple"]["interruption_level"], "time_sensitive");
		assert_eq!(set["web"]["badge_icon"], "/badge.png");
		assert!(
			set.get("kind").is_none(),
			"the default kind never serializes: {set}"
		);

		// And it round trips.
		let back: MessageOptions = serde_json::from_value(set).unwrap();
		assert_eq!(back, options());
		assert!(!options().is_empty());
		assert!(MessageOptions::default().is_empty());
	}
}
