//! `/<org>`: the organisation's projects.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// An organisation, by slug.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Org {
	/// Its slug.
	pub org: String,
}

/// The projects page.
#[component]
pub fn Page(route: &Org, navigation: &Navigation) -> Element {
	crate::screens::projects::page(route, navigation)
}
