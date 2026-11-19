//! The sign-in page: one button that goes to the main site, and what it
//! says to someone the main site signed in who isn't an admin.

use serde::Deserialize;
use serde::Serialize;

/// The sign-in page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignIn {
	/// The main site's host: `notes.example`.
	pub site: String,
	/// What the button names: `Continue with {button}`.
	pub button: String,
	/// The roles that let an account in.
	pub roles: Vec<String>,
	/// What the page says.
	pub outcome: Outcome,
}

/// What the sign-in page says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
	/// Nobody's signed in yet.
	Ready,
	/// The main site signed them in, but they don't have an admin role. The
	/// name is theirs when the admin knows it.
	Refused(Option<String>),
	/// Sign-in didn't finish, with the code the callback gave.
	Failed(String),
}
