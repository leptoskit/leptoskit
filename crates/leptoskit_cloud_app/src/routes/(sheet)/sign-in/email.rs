//! `/sign-in/email?to=<address>`: enter the emailed code.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;

/// The address the code went to, and where to go after.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct SignInEmail {
	/// The address the code went to.
	pub to: Option<String>,
	/// The page that sent them to sign in, on this site.
	pub next: Option<String>,
}

/// The code sheet.
#[component]
pub fn Page(route: &SignInEmail, navigation: &Navigation) -> Element {
	crate::screens::sign_in::email_page(route, navigation)
}
