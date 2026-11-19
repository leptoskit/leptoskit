//! `QrCode`: a link a phone's camera can open, drawn as a `Vector` from a
//! code made in Rust, on the device. It always comes with the link as text.

use leptoskit::auth::client::qr_code;
use leptoskit::native::Color;
use leptoskit::native::PathData;
use leptoskit::native::Role;
use leptoskit::native::VectorPath;
use leptoskit::native::ViewBox;
use leptoskit::prelude::*;

use crate::style;

/// A QR code for `url`, black on white whatever the appearance, so every
/// scanner reads it, with the link under it as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
	/// The link the code opens.
	pub url: String,
	/// What the code is for, for assistive technology: "QR code to approve
	/// on your phone".
	pub label: String,
}

impl QrCode {
	/// The code and its link. A link too long for a QR code shows as text
	/// alone.
	#[must_use]
	pub fn build(&self) -> Element {
		let link = Text(self.url.clone())
			.class(style::CODE)
			.class("text-[var(--cloud-ink-2)]");

		View()
			.class("flex-col items-start gap-2")
			.children(self.drawing())
			.child(link)
	}

	fn drawing(&self) -> Option<Element> {
		let code = qr_code(&self.url)?;
		let side = f32::from(u16::try_from(code.side()).ok()?);
		let view_box = ViewBox::new(0.0, 0.0, side, side).ok()?;
		let path = PathData::parse(&code.path()).ok()?;

		Some(
			Vector(view_box)
				.class("h-40 w-40 rounded-lg bg-[#ffffff]")
				.role(Role::Img)
				.accessibility_label(self.label.clone())
				.path(VectorPath::new(path).fill(Color::rgba(0, 0, 0, 255))),
		)
	}
}
