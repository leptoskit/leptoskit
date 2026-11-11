//! UIKit text attributes matching the shared Core Text measurement contract.

use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Direction;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSAttributedString;
use objc2_foundation::NSAttributedStringKey;
use objc2_foundation::NSDictionary;
use objc2_foundation::NSNumber;
use objc2_foundation::NSString;
use objc2_ui_kit::NSBaselineOffsetAttributeName;
use objc2_ui_kit::NSFontAttributeName;
use objc2_ui_kit::NSForegroundColorAttributeName;
use objc2_ui_kit::NSLineBreakMode;
use objc2_ui_kit::NSMutableParagraphStyle;
use objc2_ui_kit::NSParagraphStyleAttributeName;
use objc2_ui_kit::NSTextAlignment;
use objc2_ui_kit::NSWritingDirection;
use objc2_ui_kit::UIColor;
use objc2_ui_kit::UIFont;

use crate::measure::font_weight;

pub(crate) fn alignment(style: TextStyle) -> NSTextAlignment {
	match (style.text_align, style.direction) {
		(TextAlign::Center, _) => NSTextAlignment::Center,
		(TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => {
			NSTextAlignment::Left
		}
		(TextAlign::Start, Direction::Rtl) | (TextAlign::End, Direction::Ltr) => {
			NSTextAlignment::Right
		}
	}
}

/// How far to raise each line's glyphs so they sit centered in a line of
/// `line_height`, as CSS centers them: UIKit puts the extra height above the
/// glyphs, and takes a shortfall from above them too.
pub(crate) fn baseline_offset(line_height: f64, natural: f64) -> f64 {
	(line_height - natural) / 2.0
}

pub(crate) fn attributed(
	text: &str,
	style: TextStyle,
	color: &UIColor,
) -> Retained<NSAttributedString> {
	let font =
		UIFont::systemFontOfSize_weight(f64::from(style.font_size), font_weight(style.font_weight));
	let paragraph = NSMutableParagraphStyle::new();
	paragraph.setAlignment(alignment(style));
	paragraph.setBaseWritingDirection(match style.direction {
		Direction::Ltr => NSWritingDirection::LeftToRight,
		Direction::Rtl => NSWritingDirection::RightToLeft,
	});

	paragraph.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
	// SAFETY: UIKit exports immutable attribute keys with the documented font,
	// UIColor, paragraph-style, and number value types supplied in this
	// dictionary.
	unsafe {
		let mut keys: Vec<&NSAttributedStringKey> = vec![
			NSFontAttributeName,
			NSForegroundColorAttributeName,
			NSParagraphStyleAttributeName,
		];
		let offset;
		let mut values: Vec<&AnyObject> = vec![&font, color, &paragraph];

		// Every line is exactly the line height, with its glyphs centered.
		if let Some(line_height) = style.line_height {
			let line_height = f64::from(line_height);
			paragraph.setMinimumLineHeight(line_height);
			paragraph.setMaximumLineHeight(line_height);
			offset = NSNumber::new_f64(baseline_offset(
				line_height,
				font.ascender() - font.descender(),
			));
			keys.push(NSBaselineOffsetAttributeName);
			values.push(&offset);
		}

		let attributes = NSDictionary::from_slices(&keys, &values);
		NSAttributedString::initWithString_attributes(
			NSAttributedString::alloc(),
			&NSString::from_str(text),
			Some(&attributes),
		)
	}
}
