//! Extended widget surfaces across Apple and Android: Lock Screen accessory
//! widgets, iOS 18+ Control Center controls (`ControlWidget`), Android Quick
//! Settings tiles (`TileService`), multi-entry widget timelines, and
//! interactive widget actions.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

use crate::Snapshot;

/// Maximum number of scheduled timeline entries in one [`TimelineSchedule`].
pub const MAX_TIMELINE_ENTRIES: usize = 32;
const MAX_ID_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 128;
const MAX_URL_BYTES: usize = 2048;

/// Concrete OS surface family a widget or quick control targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SurfaceKind {
	/// Standard Home Screen widget (`WidgetKit` / Android App Widget).
	HomeScreen = 0,
	/// iOS Lock Screen / watchOS circular accessory (`accessoryCircular`).
	LockScreenCircular = 1,
	/// iOS Lock Screen / watchOS rectangular accessory
	/// (`accessoryRectangular`).
	LockScreenRectangular = 2,
	/// iOS Lock Screen inline text above the clock (`accessoryInline`).
	LockScreenInline = 3,
	/// iOS 18+ Control Center interactive control (`ControlWidget`).
	ControlCenter = 4,
	/// Android Quick Settings tile
	/// (`android.service.quicksettings.TileService`).
	QuickSettingsTile = 5,
	/// Android Jetpack Glance resizable widget.
	AndroidGlance = 6,
}

/// When the OS should request the next widget timeline refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReloadPolicy {
	/// Refresh after the last scheduled [`TimelineEntry`] passes.
	AtEnd,
	/// Ask the OS to refresh no sooner than the given number of seconds.
	AfterSeconds(u32),
	/// Only refresh when the app explicitly calls reload.
	Never,
}

/// One time-stamped entry in a widget [`TimelineSchedule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEntry {
	/// Unix timestamp (seconds) when this snapshot becomes active.
	pub deliver_at_epoch_secs: u64,
	/// Snapshot payload displayed at that time.
	pub snapshot: Snapshot,
}

/// Bounded multi-entry timeline for `WidgetKit` or scheduled Android updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineSchedule {
	surface: SurfaceKind,
	entries: Vec<TimelineEntry>,
	policy: ReloadPolicy,
}

impl TimelineSchedule {
	/// Validates that entries are non-empty, bounded to
	/// [`MAX_TIMELINE_ENTRIES`], and ordered by `deliver_at_epoch_secs`.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if empty or out of order, or
	/// [`Error::TooLarge`] if more than [`MAX_TIMELINE_ENTRIES`] are provided.
	pub fn new(
		surface: SurfaceKind,
		entries: Vec<TimelineEntry>,
		policy: ReloadPolicy,
	) -> Result<Self, Error> {
		if entries.is_empty() {
			return Err(Error::InvalidArgument);
		}
		if entries.len() > MAX_TIMELINE_ENTRIES {
			return Err(Error::TooLarge);
		}
		for pair in entries.windows(2) {
			if pair[1].deliver_at_epoch_secs < pair[0].deliver_at_epoch_secs {
				return Err(Error::InvalidArgument);
			}
		}
		if let ReloadPolicy::AfterSeconds(seconds) = policy
			&& seconds == 0
		{
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			surface,
			entries,
			policy,
		})
	}

	/// Scheduled entries in chronological order.
	#[must_use]
	pub fn entries(&self) -> &[TimelineEntry] {
		&self.entries
	}

	/// Reload policy.
	#[must_use]
	pub const fn policy(&self) -> ReloadPolicy {
		self.policy
	}

	/// Target surface kind.
	#[must_use]
	pub const fn surface(&self) -> SurfaceKind {
		self.surface
	}
}

/// Interactive button or toggle action invoked from a Home Screen widget,
/// Control Center control, or Android Quick Settings tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetAction {
	id: String,
	label: String,
	deep_link: Option<String>,
}

impl WidgetAction {
	/// Validates and constructs an interactive widget action.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] for invalid IDs, labels, or unsafe
	/// URL schemes (`javascript:`, `data:`, `file:`, `blob:`, `vbscript:`).
	pub fn new(
		id: impl Into<String>,
		label: impl Into<String>,
		deep_link: Option<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let label = label.into();
		validate_id(&id)?;
		validate_text(&label, MAX_LABEL_BYTES, false)?;
		if let Some(url) = deep_link.as_deref() {
			validate_safe_url(url)?;
		}
		Ok(Self {
			id,
			label,
			deep_link,
		})
	}

	/// Optional deep-link URL to open when triggered.
	#[must_use]
	pub fn deep_link(&self) -> Option<&str> {
		self.deep_link.as_deref()
	}

	/// Stable action identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// User-visible action label.
	#[must_use]
	pub fn label(&self) -> &str {
		&self.label
	}
}

/// Kind of iOS 18+ Control Center `ControlWidget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlKind {
	/// Two-state toggle (`ControlWidgetToggle`).
	Toggle {
		/// Whether the toggle is currently active.
		is_on: bool,
	},
	/// Stateless button (`ControlWidgetButton`).
	Button,
}

/// Descriptor for an iOS 18+ Control Center control (`ControlWidget`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlCenterControl {
	id: String,
	title: String,
	value_label: String,
	symbol_name: String,
	kind: ControlKind,
	action_id: String,
}

impl ControlCenterControl {
	/// Validates and builds a Control Center toggle or button descriptor.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] when identifiers or labels are empty,
	/// oversized, or contain control characters.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		symbol_name: impl Into<String>,
		kind: ControlKind,
		action_id: impl Into<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let title = title.into();
		let symbol_name = symbol_name.into();
		let action_id = action_id.into();
		validate_id(&id)?;
		validate_text(&title, MAX_LABEL_BYTES, false)?;
		validate_id(&symbol_name)?;
		validate_id(&action_id)?;
		Ok(Self {
			id,
			title,
			value_label: String::new(),
			symbol_name,
			kind,
			action_id,
		})
	}

	/// Attaches an optional status value label (such as `"On"` or `"12m"`).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if invalid.
	pub fn value_label(mut self, label: impl Into<String>) -> Result<Self, Error> {
		let label = label.into();
		validate_text(&label, MAX_LABEL_BYTES, true)?;
		self.value_label = label;
		Ok(self)
	}

	/// Action ID invoked when the user taps the control.
	#[must_use]
	pub fn action_id(&self) -> &str {
		&self.action_id
	}

	/// Stable control identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Toggle or button kind.
	#[must_use]
	pub const fn kind(&self) -> ControlKind {
		self.kind
	}

	/// SF Symbol name shown in Control Center.
	#[must_use]
	pub fn symbol_name(&self) -> &str {
		&self.symbol_name
	}

	/// Visible title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Optional value label.
	#[must_use]
	pub fn value(&self) -> &str {
		&self.value_label
	}

	/// Encodes this control for the binary plugin bridge.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut out = Encoder::with_capacity(128);
		out.u32(1);
		out.bytes(self.id.as_bytes())?;
		out.bytes(self.title.as_bytes())?;
		out.bytes(self.value_label.as_bytes())?;
		out.bytes(self.symbol_name.as_bytes())?;
		match self.kind {
			ControlKind::Button => out.u32(0),
			ControlKind::Toggle { is_on: false } => out.u32(1),
			ControlKind::Toggle { is_on: true } => out.u32(2),
		}
		out.bytes(self.action_id.as_bytes())?;
		Ok(out.finish())
	}

	/// Decodes a control descriptor from bridge bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let id = input.string()?;
		let title = input.string()?;
		let value_label = input.string()?;
		let symbol_name = input.string()?;
		let kind = match input.u32()? {
			0 => ControlKind::Button,
			1 => ControlKind::Toggle { is_on: false },
			2 => ControlKind::Toggle { is_on: true },
			_ => return Err(Error::InvalidArgument),
		};
		let action_id = input.string()?;
		input.finish()?;
		Self::new(id, title, symbol_name, kind, action_id)?.value_label(value_label)
	}
}

/// Android Quick Settings tile state (`Tile.STATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TileState {
	/// Tile cannot be toggled right now (`Tile.STATE_UNAVAILABLE`).
	Unavailable = 0,
	/// Tile is off / inactive (`Tile.STATE_INACTIVE`).
	Inactive = 1,
	/// Tile is on / active (`Tile.STATE_ACTIVE`).
	Active = 2,
}

/// Descriptor for an Android Quick Settings tile (`TileService`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSettingsTile {
	id: String,
	label: String,
	subtitle: String,
	icon_name: String,
	state: TileState,
	action_id: String,
}

impl QuickSettingsTile {
	/// Validates and constructs an Android Quick Settings tile state update.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if IDs or text fail validation.
	pub fn new(
		id: impl Into<String>,
		label: impl Into<String>,
		icon_name: impl Into<String>,
		state: TileState,
		action_id: impl Into<String>,
	) -> Result<Self, Error> {
		let id = id.into();
		let label = label.into();
		let icon_name = icon_name.into();
		let action_id = action_id.into();
		validate_id(&id)?;
		validate_text(&label, MAX_LABEL_BYTES, false)?;
		validate_id(&icon_name)?;
		validate_id(&action_id)?;
		Ok(Self {
			id,
			label,
			subtitle: String::new(),
			icon_name,
			state,
			action_id,
		})
	}

	/// Sets the optional Android 10+ tile subtitle.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if invalid.
	pub fn subtitle(mut self, subtitle: impl Into<String>) -> Result<Self, Error> {
		let subtitle = subtitle.into();
		validate_text(&subtitle, MAX_LABEL_BYTES, true)?;
		self.subtitle = subtitle;
		Ok(self)
	}

	/// Action ID invoked when the user taps the tile.
	#[must_use]
	pub fn action_id(&self) -> &str {
		&self.action_id
	}

	/// Drawable icon resource name.
	#[must_use]
	pub fn icon_name(&self) -> &str {
		&self.icon_name
	}

	/// Stable tile identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Primary visible label.
	#[must_use]
	pub fn label(&self) -> &str {
		&self.label
	}

	/// Active / inactive / unavailable state.
	#[must_use]
	pub const fn state(&self) -> TileState {
		self.state
	}

	/// Optional secondary subtitle.
	#[must_use]
	pub fn subtitle_text(&self) -> &str {
		&self.subtitle
	}

	/// Encodes this tile update for the binary plugin bridge.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut out = Encoder::with_capacity(128);
		out.u32(1);
		out.bytes(self.id.as_bytes())?;
		out.bytes(self.label.as_bytes())?;
		out.bytes(self.subtitle.as_bytes())?;
		out.bytes(self.icon_name.as_bytes())?;
		out.u32(self.state as u32);
		out.bytes(self.action_id.as_bytes())?;
		Ok(out.finish())
	}

	/// Decodes a tile update from bridge bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let id = input.string()?;
		let label = input.string()?;
		let subtitle = input.string()?;
		let icon_name = input.string()?;
		let state = match input.u32()? {
			0 => TileState::Unavailable,
			1 => TileState::Inactive,
			2 => TileState::Active,
			_ => return Err(Error::InvalidArgument),
		};
		let action_id = input.string()?;
		input.finish()?;
		Self::new(id, label, icon_name, state, action_id)?.subtitle(subtitle)
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
	use crate::Family;
	use crate::Kind;

	#[test]
	fn control_center_and_quick_settings_tile_round_trip() {
		let control = ControlCenterControl::new(
			"focus_mode",
			"Focus Timer",
			"timer",
			ControlKind::Toggle { is_on: true },
			"toggle_focus",
		)
		.unwrap()
		.value_label("25m")
		.unwrap();
		let encoded = control.encode().unwrap();
		assert_eq!(ControlCenterControl::decode(&encoded).unwrap(), control);

		let tile = QuickSettingsTile::new(
			"quick_capture",
			"Capture",
			"ic_capture",
			TileState::Active,
			"open_capture",
		)
		.unwrap()
		.subtitle("Ready")
		.unwrap();
		let encoded_tile = tile.encode().unwrap();
		assert_eq!(QuickSettingsTile::decode(&encoded_tile).unwrap(), tile);
	}

	#[test]
	fn timeline_enforces_monotonic_ordering_and_safe_actions() {
		let kind = Kind::new("steps").unwrap();
		let snap_a = Snapshot::new(kind.clone(), Family::Small, "Steps", "5,000", "").unwrap();
		let snap_b = Snapshot::new(kind, Family::Small, "Steps", "6,200", "").unwrap();

		let schedule = TimelineSchedule::new(
			SurfaceKind::LockScreenRectangular,
			vec![
				TimelineEntry {
					deliver_at_epoch_secs: 1_700_000_000,
					snapshot: snap_a.clone(),
				},
				TimelineEntry {
					deliver_at_epoch_secs: 1_700_000_900,
					snapshot: snap_b.clone(),
				},
			],
			ReloadPolicy::AfterSeconds(900),
		)
		.unwrap();
		assert_eq!(schedule.entries().len(), 2);

		// Out-of-order entries fail validation.
		assert_eq!(
			TimelineSchedule::new(
				SurfaceKind::HomeScreen,
				vec![
					TimelineEntry {
						deliver_at_epoch_secs: 200,
						snapshot: snap_a,
					},
					TimelineEntry {
						deliver_at_epoch_secs: 100,
						snapshot: snap_b,
					},
				],
				ReloadPolicy::AtEnd,
			),
			Err(Error::InvalidArgument)
		);

		assert!(WidgetAction::new("mark_done", "Done", Some("myapp://done".into())).is_ok());
		assert_eq!(
			WidgetAction::new("bad", "Bad", Some("javascript:alert(1)".into())),
			Err(Error::InvalidArgument)
		);
	}
}
