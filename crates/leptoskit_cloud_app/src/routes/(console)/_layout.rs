//! The console: top bar, rail, and the phone's tab bar around every
//! signed-in page.

use leptoskit::prelude::*;

use crate::routes::Navigation;
use crate::routes::Route;

/// Puts the page in the console's shell.
#[component]
pub fn Layout(children: Children, route: &Route, navigation: &Navigation) -> Element {
	crate::shell::console(children, route, navigation)
}
