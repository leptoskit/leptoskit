//! `/account`: how you sign in, where you're signed in, and how the console
//! looks.

use leptoskit::prelude::*;

use crate::routes::Navigation;

/// The account page.
#[component]
pub fn Page(navigation: &Navigation) -> Element {
	crate::screens::account::page(navigation)
}
