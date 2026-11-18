//! The desk: the sign-in, welcome and device sheets, centred on paper with
//! no console around them.

use leptoskit::prelude::*;

/// Puts the sheet on the desk.
#[component]
pub fn Layout(children: Children) -> Element {
	crate::shell::desk(children)
}
