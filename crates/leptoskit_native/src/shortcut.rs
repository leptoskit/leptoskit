//! Keyboard shortcuts for the whole app.
//!
//! [`Element::on_key`](crate::Element::on_key) hears keys pressed inside an
//! element, so it suits a field or a list. A shortcut such as ⌘K to search
//! should work wherever focus is, including nowhere at all, so it belongs to
//! the app instead:
//!
//! ```
//! use leptoskit_native::FocusRequest;
//! use leptoskit_native::shortcut::Shortcut;
//! use leptoskit_native::shortcut::use_shortcut;
//!
//! # reactive_graph::owner::Owner::new().with(|| {
//! let search = FocusRequest::new();
//! use_shortcut(Shortcut::primary("k"), move || search.request());
//! use_shortcut(
//! 	Shortcut::key("w").unless_typing(),
//! 	|| { /* toggle the drawer */ },
//! );
//! # });
//! ```
//!
//! A key an element's own `on_key` handles never reaches a shortcut, so a
//! dialog that takes Enter keeps it. While a `Modal` is open, no shortcut
//! runs: the modal's keys are its own, and Escape closes it, not whatever
//! the app's Escape does.
//!
//! On the web a shortcut listens on the window. Native hosts don't deliver
//! keys to shortcuts yet, so there a shortcut does nothing for now.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use reactive_graph::traits::WithUntracked;

use crate::KeyPress;
use crate::remember::remember;

/// A key, and the modifiers that must be held with it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
	clippy::struct_excessive_bools,
	reason = "each modifier is held or not on its own, as in `KeyPress`"
)]
pub struct Shortcut {
	key: String,
	primary: bool,
	shift: bool,
	alt: bool,
	unless_typing: bool,
}

impl Shortcut {
	/// `key` with no modifiers, such as `"w"` or `"Escape"`. A letter
	/// matches either case, so `Shortcut::key("w").shift()` is Shift+W.
	#[must_use]
	pub fn key(key: impl Into<String>) -> Self {
		Self {
			key: key.into(),
			primary: false,
			shift: false,
			alt: false,
			unless_typing: false,
		}
	}

	/// `key` with the platform's command modifier: ⌘ on Apple platforms
	/// and control everywhere else, as [`KeyPress::is_primary`] decides.
	#[must_use]
	pub fn primary(key: impl Into<String>) -> Self {
		Self {
			primary: true,
			..Self::key(key)
		}
	}

	/// The same shortcut with shift held.
	#[must_use]
	pub fn shift(mut self) -> Self {
		self.shift = true;
		self
	}

	/// The same shortcut with alt (option) held.
	#[must_use]
	pub fn alt(mut self) -> Self {
		self.alt = true;
		self
	}

	/// The same shortcut, ignored while the person types in a text field,
	/// so a plain letter stays a letter there.
	#[must_use]
	pub fn unless_typing(mut self) -> Self {
		self.unless_typing = true;
		self
	}

	/// Whether `press` is this shortcut. `typing` says whether focus is in
	/// a text field.
	#[must_use]
	pub fn matches(&self, press: &KeyPress, typing: bool) -> bool {
		if self.unless_typing && typing {
			return false;
		}

		let modifiers = if self.primary {
			press.is_primary()
		} else {
			!press.ctrl && !press.meta
		};
		let key = if self.key.chars().count() == 1 {
			self.key.eq_ignore_ascii_case(&press.key)
		} else {
			self.key == press.key
		};

		key && modifiers && press.shift == self.shift && press.alt == self.alt
	}
}

type Run = Rc<dyn Fn()>;

thread_local! {
	/// The shortcuts in use, oldest first.
	static SHORTCUTS: RefCell<Vec<(u64, Shortcut, Run)>> = const { RefCell::new(Vec::new()) };
	/// The next registration's number.
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// One call site's registration, which leaves the list when the screen or
/// owner that holds it goes.
struct Registration(u64);

impl Drop for Registration {
	fn drop(&mut self) {
		let id = self.0;
		let _ = SHORTCUTS.try_with(|shortcuts| {
			shortcuts.borrow_mut().retain(|(entry, ..)| *entry != id);
		});
	}
}

/// Runs `run` when the person presses `shortcut`, wherever focus is.
///
/// Call it in a component or a screen's build: each call site holds one
/// shortcut, and the latest `run` replaces the last one when the screen
/// builds again. It stops when the screen or the owner it was made in
/// goes. When two shortcuts match, the one made last runs, so a screen's
/// own shortcut wins over the app's.
#[track_caller]
pub fn use_shortcut(shortcut: Shortcut, run: impl Fn() + 'static) {
	let slot = remember(|| Registration(NEXT.with(|next| next.replace(next.get() + 1))));
	let id = slot.with_untracked(|registration| registration.0);
	let run: Run = Rc::new(run);

	SHORTCUTS.with(|shortcuts| {
		let mut shortcuts = shortcuts.borrow_mut();

		if let Some(entry) = shortcuts.iter_mut().find(|(entry, ..)| *entry == id) {
			entry.1 = shortcut;
			entry.2 = run;
		} else {
			shortcuts.push((id, shortcut, run));
		}
	});

	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	crate::web::shortcuts::listen();
}

/// Runs the latest shortcut that `press` matches, and says whether one did.
/// A host calls this for a key that no element handled.
#[must_use]
pub fn dispatch(press: &KeyPress, typing: bool) -> bool {
	let run = SHORTCUTS.with(|shortcuts| {
		shortcuts
			.borrow()
			.iter()
			.rev()
			.find(|(_, shortcut, _)| shortcut.matches(press, typing))
			.map(|(.., run)| Rc::clone(run))
	});

	run.is_some_and(|run| {
		run();
		true
	})
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use reactive_graph::owner::Owner;

	use super::*;

	#[test]
	fn keys_and_modifiers_match_exactly() {
		let w = Shortcut::key("w");
		assert!(w.matches(&KeyPress::new("w"), false));
		assert!(
			w.matches(&KeyPress::new("W"), false),
			"a letter in either case"
		);
		assert!(
			!w.matches(&KeyPress::new("w").shift(), false),
			"shift is a different shortcut"
		);
		assert!(!w.matches(&KeyPress::new("w").ctrl(), false));
		assert!(!w.matches(&KeyPress::new("w").meta(), false));
		assert!(
			Shortcut::key("w")
				.shift()
				.matches(&KeyPress::new("W").shift(), false)
		);
		assert!(!Shortcut::key("Escape").matches(&KeyPress::new("escape"), false));

		let typing = Shortcut::key("w").unless_typing();
		assert!(typing.matches(&KeyPress::new("w"), false));
		assert!(!typing.matches(&KeyPress::new("w"), true));
		assert!(w.matches(&KeyPress::new("w"), true), "only when asked");

		// Off the web and off Apple platforms, the primary modifier is ctrl.
		let search = Shortcut::primary("k");
		assert!(search.matches(&KeyPress::new("k").ctrl(), false));
		assert!(!search.matches(&KeyPress::new("k"), false));
		assert!(!search.matches(&KeyPress::new("k").ctrl().meta(), false));
	}

	#[test]
	fn the_latest_shortcut_runs_and_leaves_with_its_owner() {
		let (first, second) = (
			std::sync::Arc::new(AtomicUsize::new(0)),
			std::sync::Arc::new(AtomicUsize::new(0)),
		);
		let app = Owner::new();
		let screen = app.with(|| {
			let first = std::sync::Arc::clone(&first);
			use_shortcut(Shortcut::key("w"), move || {
				first.fetch_add(1, Ordering::Relaxed);
			});
			let screen = Owner::new();
			screen.with(|| {
				let second = std::sync::Arc::clone(&second);
				use_shortcut(Shortcut::key("w"), move || {
					second.fetch_add(1, Ordering::Relaxed);
				});
			});
			screen
		});

		assert!(dispatch(&KeyPress::new("w"), false));
		assert_eq!(
			(
				first.load(Ordering::Relaxed),
				second.load(Ordering::Relaxed)
			),
			(0, 1)
		);

		screen.cleanup();
		assert!(dispatch(&KeyPress::new("w"), false));
		assert_eq!(
			first.load(Ordering::Relaxed),
			1,
			"the app's runs once the screen goes"
		);

		app.cleanup();
		assert!(!dispatch(&KeyPress::new("w"), false), "nothing is left");
	}
}
