//! The home screen, shared across every platform.

use leptoskit::prelude::Element;

use crate::Notes;

/// Builds the notes screen for every destination.
pub fn screen(notes: &Notes) -> Element {
	notes.screen()
}
