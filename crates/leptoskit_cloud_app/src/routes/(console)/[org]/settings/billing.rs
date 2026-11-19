//! `/<org>/settings/billing`: the plan, the spend cap, and this month's
//! usage.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// An organisation's billing, by the organisation's slug.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct OrgBilling {
	/// The organisation's slug.
	pub org: String,
}

/// The billing page.
#[component]
pub fn Page(route: &OrgBilling, navigation: &Navigation) -> Element {
	crate::screens::billing::page(route, navigation)
}
