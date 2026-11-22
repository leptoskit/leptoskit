//! <!-- {=p_app_shortcutsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_app_shortcuts` as a direct dependency and select the
//! `app-shortcuts` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::AppShortcuts` rather than resolving the plugin on every call. The
//! plugin publishes a bounded static shortcut list to the OS launcher or dock;
//! generated iOS, Android, and macOS hosts call `noteLaunch` when the OS
//! activates one. It never invents icons or deep-link handlers.
//!
//! <!-- {/p_app_shortcutsSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

pub mod integrations;
pub use integrations::AppIntentDefinition;
pub use integrations::BubbleConfig;
pub use integrations::DynamicColorRole;
pub use integrations::IncomingShare;
pub use integrations::MemoryMobileIntegrationHost;
pub use integrations::MobileIntegrationHost;
pub use integrations::MultiSceneConfig;
pub use integrations::PencilPreference;
pub use integrations::SharedAttachment;
pub use integrations::SheetDetent;
pub use integrations::SpotlightItem;
pub use integrations::StatusBarConfig;
pub use integrations::StatusBarStyle;
pub use integrations::UserActivity;
pub use integrations::decode_optional_incoming_share;
pub use integrations::encode_optional_incoming_share;

/// Native registration name.
pub const NAME: &str = "app-shortcuts";
/// Replaces the published static shortcut list.
pub const SET: MethodId = MethodId(0);
/// Clears every shortcut this plugin previously published.
pub const CLEAR: MethodId = MethodId(1);
/// Returns the launch shortcut id, then clears it so the next call is empty.
pub const CONSUME_LAUNCH: MethodId = MethodId(2);
/// Indexes a [`SpotlightItem`] in Apple `CoreSpotlight` (or custom host store).
pub const INDEX_SPOTLIGHT: MethodId = MethodId(3);
/// Removes an indexed [`SpotlightItem`] by its identifier.
pub const DELETE_SPOTLIGHT: MethodId = MethodId(4);
/// Starts/publishes an Apple Handoff [`UserActivity`] (`NSUserActivity`).
pub const START_USER_ACTIVITY: MethodId = MethodId(5);
/// Invalidates the currently active Handoff [`UserActivity`].
pub const END_USER_ACTIVITY: MethodId = MethodId(6);
/// Returns the next pending [`IncomingShare`], then clears it.
pub const CONSUME_INCOMING_SHARE: MethodId = MethodId(7);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[
	SET,
	CLEAR,
	CONSUME_LAUNCH,
	INDEX_SPOTLIGHT,
	DELETE_SPOTLIGHT,
	START_USER_ACTIVITY,
	END_USER_ACTIVITY,
	CONSUME_INCOMING_SHARE,
];

const MAX_SHORTCUTS: usize = 8;
const MAX_ID_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 128;
const MAX_SUBTITLE_BYTES: usize = 128;

/// One launcher, dock, or jump-list entry the OS may show for this app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
	id: String,
	title: String,
	subtitle: String,
}

impl Shortcut {
	/// Validates a portable shortcut. Ids are ASCII letters, digits, `_`, `-`,
	/// and `.` only so every platform can round-trip them.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		subtitle: impl Into<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let title = title.into();
		let subtitle = subtitle.into();
		validate_id(&id)?;
		validate_text(&title, MAX_TITLE_BYTES, false)?;
		validate_text(&subtitle, MAX_SUBTITLE_BYTES, true)?;
		Ok(Self {
			id,
			title,
			subtitle,
		})
	}

	/// Stable identifier the app uses when the OS launches this shortcut.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Short user-visible title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Optional subtitle; empty when unused.
	#[must_use]
	pub fn subtitle(&self) -> &str {
		&self.subtitle
	}
}

/// Bounded list of shortcuts. An empty list clears published entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShortcutList {
	items: Vec<Shortcut>,
}

impl ShortcutList {
	/// Builds a list of at most eight shortcuts with unique ids.
	pub fn new(items: impl IntoIterator<Item = Shortcut>) -> Result<Self, Error> {
		let items: Vec<_> = items.into_iter().collect();

		if items.len() > MAX_SHORTCUTS {
			return Err(Error::TooLarge);
		}

		let mut seen = Vec::with_capacity(items.len());

		for item in &items {
			if seen.iter().any(|id| id == &item.id) {
				return Err(Error::InvalidArgument);
			}

			seen.push(item.id.clone());
		}

		Ok(Self { items })
	}

	/// The shortcuts in declaration order.
	#[must_use]
	pub fn items(&self) -> &[Shortcut] {
		&self.items
	}

	/// Wire: count, then id/title/subtitle length-prefixed UTF-8 strings.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut capacity = 4;

		for item in &self.items {
			capacity += 12 + item.id.len() + item.title.len() + item.subtitle.len();
		}

		let mut encoder = Encoder::with_capacity(capacity);
		encoder.u32(u32::try_from(self.items.len()).map_err(|_| Error::TooLarge)?);

		for item in &self.items {
			encoder.bytes(item.id.as_bytes())?;
			encoder.bytes(item.title.as_bytes())?;

			encoder.bytes(item.subtitle.as_bytes())?;
		}

		Ok(encoder.finish())
	}

	/// Validates an untrusted guest payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(payload);
		let count = decoder.u32()? as usize;

		if count > MAX_SHORTCUTS {
			return Err(Error::TooLarge);
		}

		let mut items = Vec::with_capacity(count);

		for _ in 0..count {
			let id = decoder.string()?;
			let title = decoder.string()?;

			let subtitle = decoder.string()?;
			items.push(Shortcut::new(id, title, subtitle)?);
		}

		decoder.finish()?;
		Self::new(items)
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty() || value.len() > MAX_ID_BYTES {
		return Err(Error::InvalidArgument);
	}

	if !value
		.bytes()
		.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
	{
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

fn validate_text(value: &str, max: usize, allow_empty: bool) -> Result<(), Error> {
	if (!allow_empty && value.is_empty()) || value.len() > max {
		return Err(Error::InvalidArgument);
	}

	if value.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Empty successful replies only.
pub fn require_empty(reply: &[u8]) -> Result<(), Error> {
	if reply.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Launch id replies are one length-prefixed UTF-8 string, possibly empty.
pub fn decode_launch_id(bytes: &[u8]) -> Result<Option<String>, Error> {
	let mut decoder = Decoder::new(bytes);
	let id = decoder.string()?;
	decoder.finish()?;

	if id.is_empty() {
		Ok(None)
	} else {
		validate_id(id)?;
		Ok(Some(id.to_owned()))
	}
}

/// Encodes an optional launch id for native replies.
pub fn encode_launch_id(id: Option<&str>) -> Result<Bytes, Error> {
	let value = id.unwrap_or("");

	if !value.is_empty() {
		validate_id(value)?;
	}

	let mut encoder = Encoder::with_capacity(4 + value.len());
	encoder.bytes(value.as_bytes())?;
	Ok(encoder.finish())
}

/// Validates a method and payload without contacting the OS.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		SET => {
			ShortcutList::decode(payload)?;
			Ok(())
		}
		CLEAR | CONSUME_LAUNCH | END_USER_ACTIVITY | CONSUME_INCOMING_SHARE => {
			if payload.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
		INDEX_SPOTLIGHT => {
			SpotlightItem::decode(payload)?;
			Ok(())
		}
		DELETE_SPOTLIGHT => {
			let mut decoder = Decoder::new(payload);
			let id = decoder.string()?;
			decoder.finish()?;
			validate_id(id)
		}
		START_USER_ACTIVITY => {
			UserActivity::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// In-memory host for desktop runners and tests. Windows jump-list and Linux
/// launcher action publishing remain host responsibilities on top of this
/// registry until generated desktop hosts register runtime plugins.
#[derive(Debug, Default)]
pub struct Registry {
	items: Vec<Shortcut>,
	launch: Option<String>,
	spotlight: Vec<SpotlightItem>,
	active_user_activity: Option<UserActivity>,
	incoming_share: Option<IncomingShare>,
}

impl Registry {
	/// Publishes a new list, replacing any previous one.
	pub fn set(&mut self, list: ShortcutList) {
		self.items = list.items;
	}

	/// Removes every published shortcut.
	pub fn clear(&mut self) {
		self.items.clear();
	}

	/// Records that the OS launched the app through this shortcut id.
	pub fn note_launch(&mut self, id: impl Into<String>) -> Result<(), Error> {
		let id = id.into();
		validate_id(&id)?;
		self.launch = Some(id);
		Ok(())
	}

	/// Returns and clears the pending launch id.
	pub fn consume_launch(&mut self) -> Option<String> {
		self.launch.take()
	}

	/// Records a pending incoming share from another app.
	pub fn note_incoming_share(&mut self, share: IncomingShare) -> Result<(), Error> {
		share.validate()?;
		self.incoming_share = Some(share);
		Ok(())
	}

	/// Returns and clears the pending incoming share.
	pub fn consume_incoming_share(&mut self) -> Option<IncomingShare> {
		self.incoming_share.take()
	}

	/// Currently published shortcuts.
	#[must_use]
	pub fn items(&self) -> &[Shortcut] {
		&self.items
	}

	/// Indexed Spotlight items.
	#[must_use]
	pub fn spotlight_items(&self) -> &[SpotlightItem] {
		&self.spotlight
	}

	/// Currently active Handoff user activity, if any.
	#[must_use]
	pub fn active_user_activity(&self) -> Option<&UserActivity> {
		self.active_user_activity.as_ref()
	}
}

impl leptoskit_bridge::Plugin for Registry {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		validate(method, &payload)?;

		if method == SET {
			self.set(ShortcutList::decode(&payload)?);
			Ok(Bytes::new())
		} else if method == CLEAR {
			self.clear();
			Ok(Bytes::new())
		} else if method == CONSUME_LAUNCH {
			encode_launch_id(self.consume_launch().as_deref())
		} else if method == INDEX_SPOTLIGHT {
			let item = SpotlightItem::decode(&payload)?;
			self.spotlight.retain(|existing| existing.id() != item.id());
			self.spotlight.push(item);
			Ok(Bytes::new())
		} else if method == DELETE_SPOTLIGHT {
			let mut decoder = Decoder::new(&payload);
			let id = decoder.string()?.to_owned();
			decoder.finish()?;
			self.spotlight.retain(|existing| existing.id() != id);
			Ok(Bytes::new())
		} else if method == START_USER_ACTIVITY {
			self.active_user_activity = Some(UserActivity::decode(&payload)?);
			Ok(Bytes::new())
		} else if method == END_USER_ACTIVITY {
			self.active_user_activity = None;
			Ok(Bytes::new())
		} else {
			// CONSUME_INCOMING_SHARE — `validate` already checked all others.
			encode_optional_incoming_share(self.consume_incoming_share().as_ref())
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached numeric native client.
	#[derive(Debug, Clone, Copy)]
	pub struct AppShortcuts(leptoskit_bridge::guest::Client);

	impl AppShortcuts {
		/// Resolves the installed host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Replaces the OS shortcut list for this app.
		pub async fn set(self, list: &ShortcutList) -> Result<(), Error> {
			require_empty(&self.0.call(SET, &list.encode()?)?.await?)
		}

		/// Clears every shortcut this plugin published.
		pub async fn clear(self) -> Result<(), Error> {
			require_empty(&self.0.call(CLEAR, &[])?.await?)
		}

		/// Returns the launch shortcut once, then clears it.
		pub async fn consume_launch(self) -> Result<Option<String>, Error> {
			decode_launch_id(&self.0.call(CONSUME_LAUNCH, &[])?.await?)
		}

		/// Indexes an item in Apple `CoreSpotlight`.
		pub async fn index_spotlight(self, item: &SpotlightItem) -> Result<(), Error> {
			require_empty(&self.0.call(INDEX_SPOTLIGHT, &item.encode()?)?.await?)
		}

		/// Removes an item previously indexed in Apple `CoreSpotlight`.
		pub async fn delete_spotlight(self, id: &str) -> Result<(), Error> {
			validate_id(id)?;
			let mut encoder = Encoder::with_capacity(4 + id.len());
			encoder.bytes(id.as_bytes())?;
			require_empty(&self.0.call(DELETE_SPOTLIGHT, &encoder.finish())?.await?)
		}

		/// Starts an Apple Handoff / Continuity `NSUserActivity`.
		pub async fn start_user_activity(self, activity: &UserActivity) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(START_USER_ACTIVITY, &activity.encode()?)?
					.await?,
			)
		}

		/// Invalidates the currently active Handoff `NSUserActivity`.
		pub async fn end_user_activity(self) -> Result<(), Error> {
			require_empty(&self.0.call(END_USER_ACTIVITY, &[])?.await?)
		}

		/// Returns and clears the pending [`IncomingShare`], if another app
		/// shared text, a URL, or attachments into this app.
		pub async fn consume_incoming_share(self) -> Result<Option<IncomingShare>, Error> {
			decode_optional_incoming_share(&self.0.call(CONSUME_INCOMING_SHARE, &[])?.await?)
		}
	}
}

/// Browsers have no home-screen shortcut publisher for installed PWAs through
/// this API. Progressive Web App shortcuts belong in the web app manifest.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unavailable in the browser.
	pub fn set(_list: &ShortcutList) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn clear() -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn consume_launch() -> Result<Option<String>, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn index_spotlight(_item: &SpotlightItem) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn delete_spotlight(_id: &str) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn start_user_activity(_activity: &UserActivity) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn end_user_activity() -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable in the browser.
	pub fn consume_incoming_share() -> Result<Option<IncomingShare>, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin as _;

	use super::*;

	#[test]
	fn shortcut_list_round_trips_and_rejects_bounds() {
		let shortcut = Shortcut::new("compose", "Compose", "New note").unwrap();
		assert_eq!(shortcut.id(), "compose");
		assert_eq!(shortcut.title(), "Compose");
		assert_eq!(shortcut.subtitle(), "New note");
		let list =
			ShortcutList::new([shortcut, Shortcut::new("search", "Search", "").unwrap()]).unwrap();
		assert_eq!(list.items().len(), 2);
		let bytes = list.encode().unwrap();

		assert_eq!(ShortcutList::decode(&bytes).unwrap(), list);

		assert!(Shortcut::new("", "x", "").is_err());
		assert!(Shortcut::new("bad id", "x", "").is_err());
		assert!(Shortcut::new("ok", "", "").is_err());
		assert!(Shortcut::new("ok", "bad\u{7}title", "").is_err());
		assert!(
			ShortcutList::new(
				(0..9).map(|i| { Shortcut::new(format!("id{i}"), "t", "").unwrap() })
			)
			.is_err()
		);
		assert!(
			ShortcutList::new([
				Shortcut::new("a", "A", "").unwrap(),
				Shortcut::new("a", "B", "").unwrap(),
			])
			.is_err()
		);

		let mut oversized = Encoder::with_capacity(4);
		oversized.u32((MAX_SHORTCUTS + 1) as u32);
		assert_eq!(
			ShortcutList::decode(&oversized.finish()),
			Err(Error::TooLarge)
		);
		assert_eq!(require_empty(&[]), Ok(()));
		assert_eq!(require_empty(&[1]), Err(Error::InvalidArgument));
		assert_eq!(validate(CLEAR, &[0]), Err(Error::InvalidArgument));
		assert_eq!(validate(CONSUME_LAUNCH, &[0]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(99), &[]), Err(Error::Unavailable));
	}

	#[test]
	fn registry_publishes_and_consumes_launch_ids() {
		let mut plugin = Registry::default();
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		let list = ShortcutList::new([Shortcut::new("inbox", "Inbox", "").unwrap()]).unwrap();
		assert!(
			plugin
				.invoke(SET, list.encode().unwrap())
				.unwrap()
				.is_empty()
		);
		assert_eq!(plugin.items().len(), 1);
		plugin.note_launch("inbox").unwrap();
		let reply = plugin.invoke(CONSUME_LAUNCH, Bytes::new()).unwrap();

		assert_eq!(decode_launch_id(&reply).unwrap().as_deref(), Some("inbox"));

		let reply = plugin.invoke(CONSUME_LAUNCH, Bytes::new()).unwrap();
		assert_eq!(decode_launch_id(&reply).unwrap(), None);
		assert!(plugin.invoke(CLEAR, Bytes::new()).unwrap().is_empty());
		assert_eq!(plugin.items(), []);
		assert_eq!(
			plugin.invoke(MethodId(99), Bytes::new()),
			Err(Error::Unavailable)
		);
	}

	#[test]
	fn spotlight_handoff_and_incoming_share_round_trip_through_registry() {
		let mut plugin = Registry::default();

		let item = SpotlightItem::new(
			"note_42",
			"notes",
			"Architecture Plan",
			"Cross-platform design",
			"myapp://notes/42",
		)
		.unwrap()
		.keywords(["rust", "mobile"])
		.unwrap()
		.expires_at(1_900_000_000);
		let item_wire = item.encode().unwrap();
		assert_eq!(SpotlightItem::decode(&item_wire).unwrap(), item);
		assert!(
			plugin
				.invoke(INDEX_SPOTLIGHT, item_wire)
				.unwrap()
				.is_empty()
		);
		assert_eq!(plugin.spotlight_items(), &[item]);

		let mut delete_req = Encoder::with_capacity(16);
		delete_req.bytes(b"note_42").unwrap();
		assert!(
			plugin
				.invoke(DELETE_SPOTLIGHT, delete_req.finish())
				.unwrap()
				.is_empty()
		);
		assert_eq!(plugin.spotlight_items(), []);

		let activity = UserActivity::new("com.example.notes.view", "Architecture Plan")
			.unwrap()
			.webpage_url("https://example.com/notes/42")
			.unwrap()
			.search(true)
			.prediction(true);
		let activity_wire = activity.encode().unwrap();
		assert_eq!(UserActivity::decode(&activity_wire).unwrap(), activity);
		assert!(
			plugin
				.invoke(START_USER_ACTIVITY, activity_wire)
				.unwrap()
				.is_empty()
		);
		assert_eq!(plugin.active_user_activity(), Some(&activity));
		assert!(
			plugin
				.invoke(END_USER_ACTIVITY, Bytes::new())
				.unwrap()
				.is_empty()
		);
		assert!(plugin.active_user_activity().is_none());

		let share = IncomingShare {
			text: Some("Read this".into()),
			url: Some("https://leptoskit.dev".into()),
			subject: Some("Link".into()),
			attachments: vec![SharedAttachment {
				mime_type: "image/png".into(),
				file_name: Some("diagram.png".into()),
				byte_len: 4096,
			}],
		};
		assert_eq!(
			IncomingShare::decode(&share.encode().unwrap()).unwrap(),
			share
		);
		plugin.note_incoming_share(share.clone()).unwrap();
		let first = plugin.invoke(CONSUME_INCOMING_SHARE, Bytes::new()).unwrap();
		assert_eq!(decode_optional_incoming_share(&first).unwrap(), Some(share));
		let second = plugin.invoke(CONSUME_INCOMING_SHARE, Bytes::new()).unwrap();
		assert_eq!(decode_optional_incoming_share(&second).unwrap(), None);
	}
}
