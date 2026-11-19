//! `/welcome`: the first thirty seconds, once.

use leptoskit::prelude::*;

use crate::routes::Navigation;

/// The welcome sheet.
#[component]
pub fn Page(navigation: &Navigation) -> Element {
	crate::screens::welcome::page(navigation)
}
