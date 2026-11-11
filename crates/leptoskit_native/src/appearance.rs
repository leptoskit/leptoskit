//! Which appearance, light or dark, the app shows: the device's, or one the
//! person picked.
//!
//! `dark:` utilities follow the device's appearance until the app calls
//! [`set`]. An app that offers "Light", "Dark", and "Match device" calls
//! [`set`] with [`Appearance::Light`], [`Appearance::Dark`], or
//! [`Appearance::System`], and every `dark:` utility follows at once, without
//! a reload:
//!
//! - In a browser, [`set`] marks the document's root element with [`ATTRIBUTE`]
//!   (`data-lk-appearance="dark"`), which the utility stylesheet reads, and
//!   sets the root's `color-scheme`, so the browser's own form controls and
//!   scrollbars follow too.
//! - In a native app, the root of the next render carries the choice, and the
//!   host's [`NativeSurface`](crate::NativeSurface) resolves `dark:` against it
//!   and tells the platform backend, so native controls follow.
//!
//! [`get`] reads the choice and is reactive: a view or effect that reads it
//! runs again when it changes.
//!
//! Remembering the choice is the app's job. Store
//! [`Appearance::as_str`] wherever the app keeps settings (the `store` or
//! `secure-store` plugin, or a cookie on the web), and call [`set`] with
//! [`Appearance::from_str`](std::str::FromStr) of it when the app starts,
//! before its first render. A server render cannot know the choice; to avoid
//! a flash of the device's appearance, put the attribute on the page's
//! `<html>` element yourself from the stored value.

use std::cell::Cell;
use std::cell::RefCell;
use std::fmt;
use std::str::FromStr;

use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Set;

/// The attribute on the document's root element that holds a web app's
/// choice: `light` or `dark`, or absent to follow the device.
pub const ATTRIBUTE: &str = "data-lk-appearance";

/// Light or dark, or whatever the device uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum Appearance {
	/// Follow the device's light or dark setting, as apps do by default.
	#[default]
	System,
	/// Always light: `dark:` utilities never apply.
	Light,
	/// Always dark: `dark:` utilities always apply.
	Dark,
}

impl Appearance {
	/// Every appearance, in the order a settings screen lists them.
	pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

	/// Whether this shows dark, when the device's own setting is
	/// `system_dark`.
	#[must_use]
	pub const fn is_dark(self, system_dark: bool) -> bool {
		match self {
			Self::System => system_dark,
			Self::Light => false,
			Self::Dark => true,
		}
	}

	/// The name to store: `"system"`, `"light"`, or `"dark"`. Parse it back
	/// with [`FromStr`].
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::System => "system",
			Self::Light => "light",
			Self::Dark => "dark",
		}
	}
}

impl fmt::Display for Appearance {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// A stored name that is not `system`, `light`, or `dark`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAppearance(pub String);

impl fmt::Display for UnknownAppearance {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"unknown appearance `{}`; expected system, light, or dark",
			self.0
		)
	}
}

impl std::error::Error for UnknownAppearance {}

impl FromStr for Appearance {
	type Err = UnknownAppearance;

	fn from_str(name: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.into_iter()
			.find(|appearance| appearance.as_str() == name)
			.ok_or_else(|| UnknownAppearance(name.to_owned()))
	}
}

thread_local! {
	/// The app's choice, made on first use. The interface runs on one
	/// thread, so it is that thread's.
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static CHOICE: RefCell<Option<ArcRwSignal<Appearance>>> = const { RefCell::new(None) };
	/// Whether the app has chosen at all. Until it does, a native root
	/// carries nothing, and the host keeps whatever its surface was given.
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static CHOSEN: Cell<bool> = const { Cell::new(false) };
}

/// The signal that holds the app's choice.
fn choice() -> ArcRwSignal<Appearance> {
	CHOICE.with_borrow_mut(|choice| {
		choice
			.get_or_insert_with(|| ArcRwSignal::new(Appearance::System))
			.clone()
	})
}

/// Shows `appearance` from now on: `dark:` utilities follow it instead of the
/// device's setting, or follow the device again for [`Appearance::System`].
///
/// In a browser this takes effect at once. In a native app it takes effect
/// with the next render, which the guest runtime starts when the choice
/// changes.
pub fn set(appearance: Appearance) {
	CHOSEN.set(true);
	let choice = choice();

	if choice.get_untracked() != appearance {
		choice.set(appearance);
	}

	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	mark_document(appearance);
}

/// The appearance the app chose with [`set`], or [`Appearance::System`]
/// before it chose one. Reading it in a view or effect subscribes to it.
#[must_use]
pub fn get() -> Appearance {
	choice().get()
}

/// [`get`], without subscribing.
#[must_use]
pub fn get_untracked() -> Appearance {
	choice().get_untracked()
}

/// The choice a native root carries: `None` until the app has called
/// [`set`].
#[cfg_attr(
	not(any(all(feature = "guest", target_arch = "wasm32"), test)),
	allow(dead_code, reason = "only the native guest runtime sends the choice")
)]
pub(crate) fn chosen() -> Option<Appearance> {
	CHOSEN.get().then(get_untracked)
}

/// Puts the choice on the document's root element, where the utility
/// stylesheet reads it.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn mark_document(appearance: Appearance) {
	use wasm_bindgen::JsCast;

	let Some(root) = web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.document_element())
	else {
		return;
	};

	let _ = match appearance {
		Appearance::System => root.remove_attribute(ATTRIBUTE),
		chosen => root.set_attribute(ATTRIBUTE, chosen.as_str()),
	};

	// The browser's own controls and scrollbars follow `color-scheme`.
	if let Some(root) = root.dyn_ref::<web_sys::HtmlElement>() {
		let style = root.style();
		let _ = match appearance {
			Appearance::System => style.remove_property("color-scheme").map(drop),
			chosen => style.set_property("color-scheme", chosen.as_str()),
		};
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_appearance_resolves_against_the_device() {
		assert!(!Appearance::System.is_dark(false));
		assert!(Appearance::System.is_dark(true));
		assert!(!Appearance::Light.is_dark(true));
		assert!(Appearance::Dark.is_dark(false));
	}

	#[test]
	fn names_round_trip_and_unknown_names_are_errors() {
		for appearance in Appearance::ALL {
			assert_eq!(appearance.as_str().parse(), Ok(appearance));
			assert_eq!(appearance.to_string(), appearance.as_str());
		}

		let error = "sepia".parse::<Appearance>().unwrap_err();
		assert_eq!(error, UnknownAppearance("sepia".to_owned()));
		assert!(error.to_string().contains("sepia"));
	}

	#[test]
	fn the_choice_is_none_until_set_and_then_reactive() {
		// Each test runs on its own thread, so the choice starts unset.
		assert_eq!(chosen(), None);
		assert_eq!(get(), Appearance::System);

		set(Appearance::Dark);
		assert_eq!(chosen(), Some(Appearance::Dark));
		assert_eq!(get(), Appearance::Dark);
		assert_eq!(get_untracked(), Appearance::Dark);

		set(Appearance::System);
		assert_eq!(chosen(), Some(Appearance::System));
	}
}
