//! The sheet's primitives (`docs/agents/cloud-design.md` §9.2): paper on a
//! hard offset block, ruled lines, and stamps.

use std::time::Duration;

use leptoskit::native::Role;
use leptoskit::prelude::*;

use super::Tone;
use crate::style;

/// A sheet of paper on its offset block, holding `children` top to bottom.
pub fn sheet(children: impl IntoIterator<Item = Element>) -> Element {
	View()
		.class(style::SHEET_FRAME)
		.child(View().class(style::SHEET_BLOCK).aria_hidden(true))
		.child(View().class(style::SHEET).children(children))
}

/// A ruled line across a sheet: a 1px view, never a per-edge border.
pub fn sheet_rule() -> Element {
	View().class(style::SHEET_RULE).aria_hidden(true)
}

/// How big a stamp is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampSize {
	/// On a sheet line: 20px tall.
	Line,
	/// Across a sheet: 44px tall.
	Sheet,
}

/// A stamp: one or two words in capitals, tilted, in a tone. It lands once,
/// with a scale and a turn, and people who ask for reduced motion see it at
/// rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
	/// The words, in capitals: `APPROVED 14:01`.
	pub words: String,
	/// What a screen reader says, in sentence case: "Approved at 14:01".
	pub label: String,
	/// The stamp's tone.
	pub tone: Tone,
	/// What its resting rotation is picked from, such as the release's ID, so
	/// it never jiggles between renders.
	pub seed: String,
	/// How big it is.
	pub size: StampSize,
}

impl Stamp {
	/// The stamp, landing.
	#[must_use]
	pub fn build(&self) -> Element {
		let rotation = style::stamp_rotation(&self.seed);
		let resting = style::stamp_degrees(rotation);
		let (frame, words) = match self.size {
			StampSize::Line => (style::STAMP, style::STAMP_TEXT),
			StampSize::Sheet => (style::STAMP_LARGE, style::STAMP_TEXT_LARGE),
		};

		View()
			.class(frame)
			.class(self.tone.border())
			.class(self.tone.text())
			.class(rotation)
			.role(Role::Img)
			.accessibility_label(self.label.clone())
			.motion(
				Motion::enter(Pose::new().opacity(0.0).scale(1.4).rotate(resting - 6.0))
					.duration(Duration::from_millis(320))
					.spring(Spring::SNAPPY),
			)
			.child(Text(self.words.clone()).class(words).aria_hidden(true))
	}
}
