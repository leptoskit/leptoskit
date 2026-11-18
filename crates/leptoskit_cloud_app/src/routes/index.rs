//! `/`: sends a signed-in person to their organisation's projects, and anyone
//! else to sign in.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// Where `/` goes.
#[component]
pub fn Page(navigation: &Navigation) -> Element {
	crate::screens::home(navigation)
}
