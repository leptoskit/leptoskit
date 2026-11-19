//! How analytics numbers, times, and names read on screen: the pages
//! crate's, and a person's name from what the service stores.

pub(crate) use leptoskit_admin_pages::analytics::format::*;
use leptoskit_analytics::service::read::PersonSummary;

/// A person's name on screen: their email, their name, or their id.
pub(crate) fn person_name(person: &PersonSummary) -> String {
	["email", "name"]
		.iter()
		.find_map(|key| person.properties.get(*key)?.as_str().map(str::to_owned))
		.unwrap_or_else(|| person.id.to_string()[..8].to_owned())
}

/// Two letters for an avatar.
pub(crate) fn initials(name: &str) -> String {
	crate::components::initials(name)
}
