//! The signed-in home screen, shared across every platform.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::Shell;

/// Shows the signed-in person's notes, or a loading message while the screen
/// for a newly signed-in account is being built.
pub fn screen(shell: &Shell) -> Element {
	match shell.notes() {
		Some(notes) => notes.screen(),
		None => {
			View()
				.class("w-full max-w-[640px] p-6 bg-slate-950 text-white")
				.child(Text("Loading your notes…").class("text-slate-300"))
		}
	}
}
