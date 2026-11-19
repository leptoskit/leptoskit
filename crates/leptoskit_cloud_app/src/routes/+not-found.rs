//! Any URL no other route matches: one answer for a page that doesn't exist
//! and one you can't see, so a URL doesn't reveal what exists.

use leptoskit::prelude::*;

/// Says there's nothing here.
#[component]
pub fn Page() -> Element {
	crate::screens::not_found()
}
