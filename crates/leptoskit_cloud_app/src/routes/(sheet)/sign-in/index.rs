//! `/sign-in?next=<path>`: sign in or sign up, on one sheet.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// Where to go after signing in, and what a provider said when it sent the
/// person back.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct SignIn {
	/// The page that sent them here, on this site.
	pub next: Option<String>,
	/// The provider they last tried, such as `github`, so its answer can be
	/// named.
	pub via: Option<String>,
	/// How the provider's sign-in ended: `signed_in`, `error`, …
	pub auth: Option<String>,
	/// The provider's error code, when it ended in one.
	pub auth_error: Option<String>,
	/// The second factors the account accepts, when it asked for one.
	pub auth_factors: Option<String>,
}

/// The sign-in sheet.
#[component]
pub fn Page(route: &SignIn, navigation: &Navigation) -> Element {
	crate::screens::sign_in::page(route, navigation)
}
