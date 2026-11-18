//! `/organizations`: your organisations, and making a new one.

use leptoskit::prelude::*;

use crate::routes::Navigation;

/// The organisations page.
#[component]
pub fn Page(navigation: &Navigation) -> Element {
	crate::screens::organizations::page(navigation)
}
