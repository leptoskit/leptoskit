//! `/device?user_code=WDJB-MJHT`: approve a terminal that ran
//! `lk cloud login`.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// The code the terminal shows, when the link carried it.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Device {
	/// The code, as the terminal printed it or as typed.
	pub user_code: Option<String>,
}

/// The device sheet.
#[component]
pub fn Page(route: &Device, navigation: &Navigation) -> Element {
	crate::screens::device::page(route, navigation)
}
