//! Dynamic Island region layouts (`ActivityKit`) and Android 16+ Live Updates
//! (`Notification.ProgressStyle`) so cross-platform mobile apps can describe
//! real-time status surfaces once and map cleanly to both iOS and Android.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

const MAX_SLOT_TEXT: usize = 128;
const MAX_SEGMENTS: usize = 8;

/// Dynamic Island presentation slots rendered by an iOS Widget Extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicIslandLayout {
	/// Compact leading slot (left pill beside the camera cutout).
	pub compact_leading: String,
	/// Compact trailing slot (right pill beside the camera cutout).
	pub compact_trailing: String,
	/// Minimal circular slot shown when two Live Activities share the island.
	pub minimal: String,
	/// Expanded top-leading region.
	pub expanded_leading: String,
	/// Expanded top-trailing region.
	pub expanded_trailing: String,
	/// Expanded center headline.
	pub expanded_center: String,
	/// Expanded bottom detail region.
	pub expanded_bottom: String,
	/// Optional `#RRGGBB` keyline tint color for the Dynamic Island border.
	pub keyline_tint_hex: Option<String>,
}

impl DynamicIslandLayout {
	/// Starts an empty Dynamic Island layout builder.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the compact leading and trailing pill text.
	#[must_use]
	pub fn compact(mut self, leading: impl Into<String>, trailing: impl Into<String>) -> Self {
		self.compact_leading = leading.into();
		self.compact_trailing = trailing.into();
		self
	}

	/// Sets the minimal circular badge text.
	#[must_use]
	pub fn minimal(mut self, text: impl Into<String>) -> Self {
		self.minimal = text.into();
		self
	}

	/// Sets the expanded regions shown when the user long-presses the island.
	#[must_use]
	pub fn expanded(
		mut self,
		leading: impl Into<String>,
		trailing: impl Into<String>,
		center: impl Into<String>,
		bottom: impl Into<String>,
	) -> Self {
		self.expanded_leading = leading.into();
		self.expanded_trailing = trailing.into();
		self.expanded_center = center.into();
		self.expanded_bottom = bottom.into();
		self
	}

	/// Sets an optional `#RRGGBB` keyline border tint.
	#[must_use]
	pub fn keyline_tint(mut self, hex: impl Into<String>) -> Self {
		self.keyline_tint_hex = Some(hex.into());
		self
	}

	/// Validates slot text bounds and optional `#RRGGBB` hex color.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] when any slot exceeds bounds,
	/// contains control characters, or uses a malformed hex color.
	pub fn validate(&self) -> Result<(), Error> {
		for slot in [
			&self.compact_leading,
			&self.compact_trailing,
			&self.minimal,
			&self.expanded_leading,
			&self.expanded_trailing,
			&self.expanded_center,
			&self.expanded_bottom,
		] {
			if slot.len() > MAX_SLOT_TEXT || slot.chars().any(char::is_control) {
				return Err(Error::InvalidArgument);
			}
		}
		if let Some(hex) = self.keyline_tint_hex.as_deref() {
			validate_hex_color(hex)?;
		}
		Ok(())
	}
}

/// Freshness and ranking hints for an active Live Activity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivitySchedule {
	/// Seconds until the OS marks the activity stale (`1..=86_400`).
	pub stale_after_seconds: Option<u32>,
	/// Priority hint (`0.0..=1.0`) when multiple activities compete for the
	/// Dynamic Island.
	pub relevance_score: f32,
	/// Whether the server may start this activity remotely via an `ActivityKit`
	/// push-to-start token (iOS 17.2+).
	pub allow_push_to_start: bool,
}

impl Default for ActivitySchedule {
	fn default() -> Self {
		Self {
			stale_after_seconds: None,
			relevance_score: 0.5,
			allow_push_to_start: false,
		}
	}
}

impl ActivitySchedule {
	/// Validates finite `0.0..=1.0` relevance and bounded stale interval.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] on invalid values.
	pub fn validate(self) -> Result<Self, Error> {
		if !self.relevance_score.is_finite() || !(0.0..=1.0).contains(&self.relevance_score) {
			return Err(Error::InvalidArgument);
		}
		if let Some(seconds) = self.stale_after_seconds
			&& (seconds == 0 || seconds > 86_400)
		{
			return Err(Error::InvalidArgument);
		}
		Ok(self)
	}
}

/// One colored segment in an Android 16+ Live Update progress bar
/// (`Notification.ProgressStyle.Segment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressSegment {
	/// Relative weight/length of this segment (`1..=10_000`).
	pub length: u32,
	/// Optional `#RRGGBB` segment color.
	pub color_hex: Option<String>,
}

/// Android 16+ Live Update payload (`Notification.ProgressStyle`), pairing with
/// iOS Live Activities for real-time delivery, rideshare, or timer progress.
#[derive(Debug, Clone, PartialEq)]
pub struct AndroidLiveUpdate {
	title: String,
	body: String,
	short_critical_text: String,
	progress: f32,
	segments: Vec<ProgressSegment>,
}

impl AndroidLiveUpdate {
	/// Validates and constructs an Android Live Update payload.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if progress is outside `0.0..=1.0`,
	/// text is invalid, or segments exceed [`MAX_SEGMENTS`].
	pub fn new(
		title: impl Into<String>,
		body: impl Into<String>,
		short_critical_text: impl Into<String>,
		progress: f32,
		segments: Vec<ProgressSegment>,
	) -> Result<Self, Error> {
		let title = title.into();
		let body = body.into();
		let short_critical_text = short_critical_text.into();

		if title.trim().is_empty()
			|| title.len() > MAX_SLOT_TEXT
			|| title.chars().any(char::is_control)
			|| body.len() > 256
			|| body.chars().any(char::is_control)
			|| short_critical_text.len() > 32
			|| short_critical_text.chars().any(char::is_control)
			|| !progress.is_finite()
			|| !(0.0..=1.0).contains(&progress)
		{
			return Err(Error::InvalidArgument);
		}
		if segments.len() > MAX_SEGMENTS {
			return Err(Error::TooLarge);
		}
		for segment in &segments {
			if segment.length == 0 || segment.length > 10_000 {
				return Err(Error::InvalidArgument);
			}
			if let Some(color) = segment.color_hex.as_deref() {
				validate_hex_color(color)?;
			}
		}

		Ok(Self {
			title,
			body,
			short_critical_text,
			progress,
			segments,
		})
	}

	/// Detail text shown in the expanded notification.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Normalized progress in `0.0..=1.0`.
	#[must_use]
	pub const fn progress(&self) -> f32 {
		self.progress
	}

	/// Progress bar segments.
	#[must_use]
	pub fn segments(&self) -> &[ProgressSegment] {
		&self.segments
	}

	/// Short status-bar chip label (for example `"4m"`).
	#[must_use]
	pub fn short_critical_text(&self) -> &str {
		&self.short_critical_text
	}

	/// Notification title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Encodes this Android Live Update into bridge bytes.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut out = Encoder::with_capacity(128);
		out.u32(1);
		out.bytes(self.title.as_bytes())?;
		out.bytes(self.body.as_bytes())?;
		out.bytes(self.short_critical_text.as_bytes())?;
		out.u32(self.progress.to_bits());
		out.u32(u32::try_from(self.segments.len()).map_err(|_| Error::TooLarge)?);
		for segment in &self.segments {
			out.u32(segment.length);
			match &segment.color_hex {
				None => out.u32(0),
				Some(color) => {
					out.u32(1);
					out.bytes(color.as_bytes())?;
				}
			}
		}
		Ok(out.finish())
	}

	/// Decodes an Android Live Update from bridge bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let title = input.string()?;
		let body = input.string()?;
		let short_critical_text = input.string()?;
		let progress = f32::from_bits(input.u32()?);
		let count = input.u32()? as usize;
		if count > MAX_SEGMENTS {
			return Err(Error::TooLarge);
		}
		let mut segments = Vec::with_capacity(count);
		for _ in 0..count {
			let length = input.u32()?;
			let color_hex = match input.u32()? {
				0 => None,
				1 => Some(input.string()?.to_owned()),
				_ => return Err(Error::InvalidArgument),
			};
			segments.push(ProgressSegment { length, color_hex });
		}
		input.finish()?;
		Self::new(title, body, short_critical_text, progress, segments)
	}
}

fn validate_hex_color(value: &str) -> Result<(), Error> {
	let bytes = value.as_bytes();
	if bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit) {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn dynamic_island_and_schedule_validate() {
		let island = DynamicIslandLayout::new()
			.compact("Courier", "4m")
			.minimal("4m")
			.expanded("Order #42", "4m", "Arriving soon", "Main St & 3rd")
			.keyline_tint("#4F46E5");
		island.validate().unwrap();

		assert_eq!(
			DynamicIslandLayout::new().keyline_tint("indigo").validate(),
			Err(Error::InvalidArgument)
		);
		assert!(
			ActivitySchedule {
				stale_after_seconds: Some(300),
				relevance_score: 0.9,
				allow_push_to_start: true,
			}
			.validate()
			.is_ok()
		);
		assert_eq!(
			ActivitySchedule {
				stale_after_seconds: Some(0),
				..ActivitySchedule::default()
			}
			.validate(),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn android_live_update_round_trips() {
		let update = AndroidLiveUpdate::new(
			"Order #42",
			"Courier is 4 minutes away",
			"4m",
			0.75,
			vec![
				ProgressSegment {
					length: 50,
					color_hex: Some("#22C55E".into()),
				},
				ProgressSegment {
					length: 50,
					color_hex: None,
				},
			],
		)
		.unwrap();
		let wire = update.encode().unwrap();
		assert_eq!(AndroidLiveUpdate::decode(&wire).unwrap(), update);
	}
}
