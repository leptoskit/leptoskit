//! Platform-native mobile OS integrations shared across iOS/iPadOS and Android
//! as well as unique OS surfaces:
//!
//! - **App Intents / Siri Shortcuts & Android App Actions**
//!   ([`AppIntentDefinition`])
//! - **System Search Indexing** (`CoreSpotlight` / Android `AppSearch`:
//!   [`SpotlightItem`])
//! - **Apple Handoff / Continuity** (`NSUserActivity`: [`UserActivity`])
//! - **Incoming Share Extensions / Android Share Targets** ([`IncomingShare`])
//! - **Mobile System Chrome & Platform Capabilities**: status bar style, native
//!   sheet detents, Android Material You dynamic color roles, Android
//!   conversation bubbles, and iPadOS multi-scene / Apple Pencil preferences.

use leptoskit_bridge::Error;

const MAX_ID_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_PHRASES: usize = 8;
const MAX_KEYWORDS: usize = 16;
const MAX_ATTACHMENTS: usize = 32;

/// Declarative intent donated to Siri / App Intents (iOS/macOS) or Google
/// Assistant App Actions (Android).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIntentDefinition {
	id: String,
	title: String,
	short_title: String,
	phrases: Vec<String>,
	open_app_when_run: bool,
	symbol_name: Option<String>,
	deep_link: Option<String>,
}

impl AppIntentDefinition {
	/// Validates and builds an App Intent / voice shortcut definition.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] or [`Error::TooLarge`] when fields
	/// exceed limits or contain control characters.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		short_title: impl Into<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let title = title.into();
		let short_title = short_title.into();
		validate_id(&id)?;
		validate_text(&title, MAX_TITLE_BYTES, false)?;
		validate_text(&short_title, MAX_TITLE_BYTES, false)?;
		Ok(Self {
			id,
			title,
			short_title,
			phrases: Vec::new(),
			open_app_when_run: true,
			symbol_name: None,
			deep_link: None,
		})
	}

	/// Adds up to 8 voice phrases (for Siri Shortcuts / App Shortcuts).
	///
	/// # Errors
	///
	/// Returns [`Error::TooLarge`] above 8 phrases or
	/// [`Error::InvalidArgument`] on blank/control text.
	pub fn phrases(
		mut self,
		phrases: impl IntoIterator<Item = impl Into<String>>,
	) -> Result<Self, Error> {
		let mut out = Vec::new();
		for phrase in phrases {
			let phrase = phrase.into();
			validate_text(&phrase, MAX_TITLE_BYTES, false)?;
			out.push(phrase);
			if out.len() > MAX_PHRASES {
				return Err(Error::TooLarge);
			}
		}
		self.phrases = out;
		Ok(self)
	}

	/// Controls whether running the intent brings the app to the foreground.
	#[must_use]
	pub const fn open_app_when_run(mut self, open: bool) -> Self {
		self.open_app_when_run = open;
		self
	}

	/// Sets an optional deep link opened when the intent runs.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] for unsafe or malformed URLs.
	pub fn deep_link(mut self, url: impl Into<String>) -> Result<Self, Error> {
		let url = url.into();
		validate_safe_url(&url)?;
		self.deep_link = Some(url);
		Ok(self)
	}

	/// Sets an optional platform symbol name (SF Symbol on Apple, drawable on
	/// Android).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if the symbol name is invalid.
	pub fn symbol_name(mut self, symbol: impl Into<String>) -> Result<Self, Error> {
		let symbol = symbol.into();
		validate_id(&symbol)?;
		self.symbol_name = Some(symbol);
		Ok(self)
	}

	/// Optional deep link.
	#[must_use]
	pub fn deep_link_url(&self) -> Option<&str> {
		self.deep_link.as_deref()
	}

	/// Stable intent identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Whether invoking the intent foregrounds the app.
	#[must_use]
	pub const fn opens_app(&self) -> bool {
		self.open_app_when_run
	}

	/// Registered voice phrases.
	#[must_use]
	pub fn phrase_list(&self) -> &[String] {
		&self.phrases
	}

	/// Short launcher/shortcut title.
	#[must_use]
	pub fn short_title(&self) -> &str {
		&self.short_title
	}

	/// Optional symbol name.
	#[must_use]
	pub fn symbol(&self) -> Option<&str> {
		self.symbol_name.as_deref()
	}

	/// Full intent title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}
}

/// Content indexed in Apple `CoreSpotlight` or Android `AppSearch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotlightItem {
	id: String,
	domain: String,
	title: String,
	description: String,
	keywords: Vec<String>,
	deep_link: String,
	expiration_epoch_secs: Option<u64>,
}

impl SpotlightItem {
	/// Validates and constructs an on-device search index item.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] or [`Error::TooLarge`] on invalid
	/// fields.
	pub fn new(
		id: impl Into<String>,
		domain: impl Into<String>,
		title: impl Into<String>,
		description: impl Into<String>,
		deep_link: impl Into<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let domain = domain.into();
		let title = title.into();
		let description = description.into();
		let deep_link = deep_link.into();
		validate_id(&id)?;
		validate_id(&domain)?;
		validate_text(&title, MAX_TITLE_BYTES, false)?;
		validate_text(&description, MAX_TEXT_BYTES, true)?;
		validate_safe_url(&deep_link)?;
		Ok(Self {
			id,
			domain,
			title,
			description,
			keywords: Vec::new(),
			deep_link,
			expiration_epoch_secs: None,
		})
	}

	/// Sets up to 16 search keywords.
	///
	/// # Errors
	///
	/// Returns [`Error::TooLarge`] above 16 keywords or
	/// [`Error::InvalidArgument`] on invalid text.
	pub fn keywords(
		mut self,
		keywords: impl IntoIterator<Item = impl Into<String>>,
	) -> Result<Self, Error> {
		let mut out = Vec::new();
		for keyword in keywords {
			let keyword = keyword.into();
			validate_text(&keyword, MAX_ID_BYTES, false)?;
			out.push(keyword);
			if out.len() > MAX_KEYWORDS {
				return Err(Error::TooLarge);
			}
		}
		self.keywords = out;
		Ok(self)
	}

	/// Sets an optional Unix expiration timestamp in seconds.
	#[must_use]
	pub const fn expires_at(mut self, epoch_secs: u64) -> Self {
		self.expiration_epoch_secs = Some(epoch_secs);
		self
	}

	/// Deep link opened when the user selects this search result.
	#[must_use]
	pub fn deep_link(&self) -> &str {
		&self.deep_link
	}

	/// Search description snippet.
	#[must_use]
	pub fn description(&self) -> &str {
		&self.description
	}

	/// Logical domain identifier (for batch deletion by domain).
	#[must_use]
	pub fn domain(&self) -> &str {
		&self.domain
	}

	/// Optional expiration timestamp.
	#[must_use]
	pub const fn expiration_epoch_secs(&self) -> Option<u64> {
		self.expiration_epoch_secs
	}

	/// Unique item identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Indexed keywords.
	#[must_use]
	pub fn keyword_list(&self) -> &[String] {
		&self.keywords
	}

	/// Visible search title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}
}

/// Apple Handoff / Continuity user activity (`NSUserActivity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserActivity {
	activity_type: String,
	title: String,
	webpage_url: Option<String>,
	eligible_for_handoff: bool,
	eligible_for_search: bool,
	eligible_for_prediction: bool,
}

impl UserActivity {
	/// Validates a reverse-DNS `activity_type` (for example
	/// `"com.example.notes.view"`) and title.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] on invalid type or title.
	pub fn new(activity_type: impl Into<String>, title: impl Into<String>) -> Result<Self, Error> {
		let activity_type = activity_type.into();
		let title = title.into();
		validate_id(&activity_type)?;
		validate_text(&title, MAX_TITLE_BYTES, false)?;
		Ok(Self {
			activity_type,
			title,
			webpage_url: None,
			eligible_for_handoff: true,
			eligible_for_search: false,
			eligible_for_prediction: false,
		})
	}

	/// Fallback HTTPS URL for Universal Links / browser Handoff.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] unless `url` starts with `https://`.
	pub fn webpage_url(mut self, url: impl Into<String>) -> Result<Self, Error> {
		let url = url.into();
		if !url.starts_with("https://") || url.len() > MAX_URL_BYTES {
			return Err(Error::InvalidArgument);
		}
		self.webpage_url = Some(url);
		Ok(self)
	}

	/// Marks whether the activity can hand off to another Apple device.
	#[must_use]
	pub const fn handoff(mut self, enabled: bool) -> Self {
		self.eligible_for_handoff = enabled;
		self
	}

	/// Marks whether Spotlight can index this activity.
	#[must_use]
	pub const fn search(mut self, enabled: bool) -> Self {
		self.eligible_for_search = enabled;
		self
	}

	/// Marks whether Siri Suggestions can predict this activity.
	#[must_use]
	pub const fn prediction(mut self, enabled: bool) -> Self {
		self.eligible_for_prediction = enabled;
		self
	}

	/// Reverse-DNS activity type.
	#[must_use]
	pub fn activity_type(&self) -> &str {
		&self.activity_type
	}

	/// Whether Handoff is enabled.
	#[must_use]
	pub const fn is_eligible_for_handoff(&self) -> bool {
		self.eligible_for_handoff
	}

	/// Whether Siri prediction is enabled.
	#[must_use]
	pub const fn is_eligible_for_prediction(&self) -> bool {
		self.eligible_for_prediction
	}

	/// Whether Spotlight search is enabled.
	#[must_use]
	pub const fn is_eligible_for_search(&self) -> bool {
		self.eligible_for_search
	}

	/// User-visible title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Optional HTTPS webpage fallback.
	#[must_use]
	pub fn webpage(&self) -> Option<&str> {
		self.webpage_url.as_deref()
	}
}

/// One attachment received by an iOS Share Extension or Android `ACTION_SEND`
/// share target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedAttachment {
	/// MIME type (such as `"image/png"` or `"application/pdf"`).
	pub mime_type: String,
	/// Optional display file name.
	pub file_name: Option<String>,
	/// Size in bytes.
	pub byte_len: u64,
}

/// Inbound content shared into the app from another app's share sheet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncomingShare {
	/// Shared plain text, if any.
	pub text: Option<String>,
	/// Shared URL, if any.
	pub url: Option<String>,
	/// Optional subject line.
	pub subject: Option<String>,
	/// Shared file attachments (bounded to 32).
	pub attachments: Vec<SharedAttachment>,
}

impl IncomingShare {
	/// Validates attachment bounds and safe URL schemes.
	///
	/// # Errors
	///
	/// Returns [`Error::TooLarge`] above 32 attachments or
	/// [`Error::InvalidArgument`] for unsafe URLs.
	pub fn validate(&self) -> Result<(), Error> {
		if self.attachments.len() > MAX_ATTACHMENTS {
			return Err(Error::TooLarge);
		}
		if let Some(url) = self.url.as_deref() {
			validate_safe_url(url)?;
		}
		Ok(())
	}
}

/// Mobile system status-bar foreground style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StatusBarStyle {
	/// Follow system appearance / root view background.
	#[default]
	Auto,
	/// Light status-bar icons/text (for dark backgrounds).
	LightContent,
	/// Dark status-bar icons/text (for light backgrounds).
	DarkContent,
}

/// Mobile system status bar configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusBarConfig {
	/// Foreground icon/text style.
	pub style: StatusBarStyle,
	/// Hide the system status bar when `true`.
	pub hidden: bool,
	/// Enable edge-to-edge drawing behind system bars (Android / iOS).
	pub edge_to_edge: bool,
}

/// Native iOS `UISheetPresentationController.Detent` / Android
/// `BottomSheetBehavior` height stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SheetDetent {
	/// Half-height medium sheet.
	Medium,
	/// Full-height sheet.
	Large,
	/// Custom fractional height (`0.1..=1.0`).
	Fraction(f32),
}

impl SheetDetent {
	/// Validates custom fraction bounds (`0.1..=1.0`).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] when outside `0.1..=1.0` or
	/// nonfinite.
	pub fn validate(self) -> Result<Self, Error> {
		if let Self::Fraction(ratio) = self
			&& (!ratio.is_finite() || !(0.1..=1.0).contains(&ratio))
		{
			return Err(Error::InvalidArgument);
		}
		Ok(self)
	}
}

/// Android Material You (Monet) dynamic color roles extracted from the user's
/// wallpaper on Android 12+.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamicColorRole {
	/// Primary brand accent.
	Primary,
	/// Foreground color on primary surfaces.
	OnPrimary,
	/// Primary tonal container.
	PrimaryContainer,
	/// Secondary accent.
	Secondary,
	/// Tertiary highlight accent.
	Tertiary,
	/// Main canvas surface.
	Surface,
	/// Elevated card/sheet surface container.
	SurfaceContainer,
	/// Primary foreground text on surfaces.
	OnSurface,
}

/// Android conversation bubble configuration (`Notification.BubbleMetadata`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BubbleConfig {
	/// Conversation shortcut ID.
	pub shortcut_id: String,
	/// Desired expanded bubble height in dp (`200..=1200`).
	pub desired_height_dp: u16,
	/// Automatically expand the bubble when posted while foregrounded.
	pub auto_expand: bool,
	/// Suppress the regular shade notification while the bubble is active.
	pub suppress_notification: bool,
}

impl BubbleConfig {
	/// Validates shortcut ID and bubble height bounds (`200..=1200` dp).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] on invalid fields.
	pub fn validate(&self) -> Result<(), Error> {
		validate_id(&self.shortcut_id)?;
		if !(200..=1200).contains(&self.desired_height_dp) {
			return Err(Error::InvalidArgument);
		}
		Ok(())
	}
}

/// iPadOS Apple Pencil double-tap / squeeze preference (`UIPencilInteraction`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PencilPreference {
	/// Follow system Pencil settings.
	#[default]
	SystemDefault,
	/// Switch between current tool and eraser.
	SwitchEraser,
	/// Switch to previous tool.
	SwitchPrevious,
	/// Show the color palette.
	ShowColorPalette,
	/// Ignore Pencil hover/squeeze gestures.
	Ignore,
}

/// iPadOS multi-window (`UIApplicationSupportsMultipleScenes`) configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MultiSceneConfig {
	/// Allow Stage Manager and Split View multiple scenes.
	pub supports_multiple_scenes: bool,
	/// Enable Apple Pencil hover / squeeze handling.
	pub pencil_preference: PencilPreference,
}

/// Host contract for mobile OS integrations (intents, Spotlight/AppSearch,
/// Handoff, and incoming share intake).
pub trait MobileIntegrationHost {
	/// Registers or updates an App Intent / Siri Shortcut / Android App Action.
	fn register_intent(&mut self, intent: AppIntentDefinition) -> Result<(), Error>;
	/// Indexes an item in `CoreSpotlight` / Android `AppSearch`.
	fn index_spotlight(&mut self, item: SpotlightItem) -> Result<(), Error>;
	/// Publishes an active Handoff `NSUserActivity`.
	fn publish_user_activity(&mut self, activity: UserActivity) -> Result<(), Error>;
	/// Drains the next incoming share payload, if any.
	fn take_incoming_share(&mut self) -> Option<IncomingShare>;
}

/// Recording [`MobileIntegrationHost`] for portable tests and simulation.
#[derive(Debug, Clone, Default)]
pub struct MemoryMobileIntegrationHost {
	intents: Vec<AppIntentDefinition>,
	spotlight: Vec<SpotlightItem>,
	activities: Vec<UserActivity>,
	incoming_shares: Vec<IncomingShare>,
}

impl MemoryMobileIntegrationHost {
	/// Creates an empty in-memory mobile integration host.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Simulates an incoming share arriving from another app.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] or [`Error::TooLarge`] if invalid.
	pub fn push_incoming_share(&mut self, share: IncomingShare) -> Result<(), Error> {
		share.validate()?;
		self.incoming_shares.push(share);
		Ok(())
	}

	/// Published Handoff activities.
	#[must_use]
	pub fn activities(&self) -> &[UserActivity] {
		&self.activities
	}

	/// Registered App Intents.
	#[must_use]
	pub fn intents(&self) -> &[AppIntentDefinition] {
		&self.intents
	}

	/// Indexed Spotlight / `AppSearch` items.
	#[must_use]
	pub fn spotlight_items(&self) -> &[SpotlightItem] {
		&self.spotlight
	}
}

impl MobileIntegrationHost for MemoryMobileIntegrationHost {
	fn register_intent(&mut self, intent: AppIntentDefinition) -> Result<(), Error> {
		self.intents.push(intent);
		Ok(())
	}

	fn index_spotlight(&mut self, item: SpotlightItem) -> Result<(), Error> {
		self.spotlight.push(item);
		Ok(())
	}

	fn publish_user_activity(&mut self, activity: UserActivity) -> Result<(), Error> {
		self.activities.push(activity);
		Ok(())
	}

	fn take_incoming_share(&mut self) -> Option<IncomingShare> {
		if self.incoming_shares.is_empty() {
			None
		} else {
			Some(self.incoming_shares.remove(0))
		}
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > MAX_ID_BYTES
		|| !value
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_text(value: &str, max_bytes: usize, allow_empty: bool) -> Result<(), Error> {
	if value.len() > max_bytes
		|| (!allow_empty && value.trim().is_empty())
		|| value.chars().any(char::is_control)
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_safe_url(url: &str) -> Result<(), Error> {
	if url.is_empty() || url.len() > MAX_URL_BYTES || url.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}
	let lower = url.to_ascii_lowercase();
	for blocked in ["javascript:", "data:", "file:", "blob:", "vbscript:"] {
		if lower.starts_with(blocked) {
			return Err(Error::InvalidArgument);
		}
	}
	if !url.contains("://") {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn app_intents_spotlight_handoff_and_incoming_share_round_trip() {
		let intent = AppIntentDefinition::new("quick_note", "Create Note", "New Note")
			.unwrap()
			.phrases(["Take a note in App", "New quick note"])
			.unwrap()
			.symbol_name("square.and.pencil")
			.unwrap()
			.deep_link("notes://new")
			.unwrap();

		let spotlight = SpotlightItem::new(
			"note_42",
			"notes",
			"Architecture Plan",
			"Cross-platform widget & desktop design",
			"notes://note/42",
		)
		.unwrap()
		.keywords(["architecture", "widgets"])
		.unwrap()
		.expires_at(1_900_000_000);

		let activity = UserActivity::new("com.example.notes.view", "Architecture Plan")
			.unwrap()
			.webpage_url("https://notes.example.com/42")
			.unwrap()
			.search(true)
			.prediction(true);

		let mut host = MemoryMobileIntegrationHost::new();
		host.register_intent(intent).unwrap();
		host.index_spotlight(spotlight).unwrap();
		host.publish_user_activity(activity).unwrap();
		host.push_incoming_share(IncomingShare {
			text: Some("Check this out".into()),
			url: Some("https://example.com/article".into()),
			subject: Some("Reading".into()),
			attachments: vec![],
		})
		.unwrap();

		assert_eq!(host.intents().len(), 1);
		assert_eq!(host.spotlight_items().len(), 1);
		assert_eq!(host.activities().len(), 1);
		assert!(host.take_incoming_share().is_some());
		assert!(host.take_incoming_share().is_none());
	}

	#[test]
	fn mobile_chrome_validators_reject_invalid_values() {
		assert!(SheetDetent::Fraction(0.5).validate().is_ok());
		assert_eq!(
			SheetDetent::Fraction(0.05).validate(),
			Err(Error::InvalidArgument)
		);
		assert!(
			BubbleConfig {
				shortcut_id: "chat_1".into(),
				desired_height_dp: 600,
				auto_expand: true,
				suppress_notification: false,
			}
			.validate()
			.is_ok()
		);
		assert_eq!(
			BubbleConfig {
				shortcut_id: "chat_1".into(),
				desired_height_dp: 50,
				auto_expand: false,
				suppress_notification: false,
			}
			.validate(),
			Err(Error::InvalidArgument)
		);
	}
}
