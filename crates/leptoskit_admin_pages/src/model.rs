//! What a page is rendered from: small values the admin builds from the
//! signed-in admin and the page's data.
//!
//! The server renders a page from a [`Model`] and hands the same value to
//! the browser, which builds the same tree before it attaches to the markup.
//! A model carries what the page shows and nothing else: no secret, and no
//! other person's data than the page displays.

use serde::Deserialize;
use serde::Serialize;

/// Which environment the admin acts on, shown as the pill in the top bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
	/// Real people use it: the pill fills with ember.
	#[default]
	Production,
	/// A rehearsal of production.
	Staging,
	/// A developer's own machine.
	Development,
}

/// A name that isn't an [`Environment`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownEnvironment(pub String);

impl std::fmt::Display for UnknownEnvironment {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"`{}` isn't an environment: use production, staging, or development",
			self.0
		)
	}
}

impl std::error::Error for UnknownEnvironment {}

impl std::str::FromStr for Environment {
	type Err = UnknownEnvironment;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"production" => Ok(Self::Production),
			"staging" => Ok(Self::Staging),
			"development" => Ok(Self::Development),
			other => Err(UnknownEnvironment(other.to_owned())),
		}
	}
}

/// How fast analytics events arrive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pulse {
	/// Events a second, over the last ten seconds.
	pub per_second: f64,
	/// Events in each of the last sixty seconds, oldest first.
	pub seconds: Vec<u64>,
	/// Whether the admin is hearing the stream now.
	pub live: bool,
}

/// Who is looking, and how they like the admin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewer {
	/// The app's name: `Notes`.
	pub app_name: String,
	/// Which environment this admin acts on.
	pub environment: Environment,
	/// Whether infrastructure actions run against a stand-in cloud.
	pub simulated: bool,
	/// The signed-in admin's email, or their name at the main site.
	pub who: String,
	/// Whether mint tabs show their sources.
	pub workings_mode: bool,
	/// `light` or `dark` when the person chose one.
	pub appearance: Option<String>,
	/// Registered projects, for the rail and the switcher.
	pub projects: Vec<String>,
	/// Lessons finished and lessons there are.
	pub progress: (usize, usize),
	/// How fast analytics events arrive, when the admin is connected.
	pub pulse: Option<Pulse>,
}

impl Viewer {
	/// Up to two letters for the avatar: `AL` for `ada.lovelace@…`.
	#[must_use]
	pub fn initials(&self) -> String {
		let name = self.who.split('@').next().unwrap_or(&self.who);
		let mut initials: String = name
			.split(['.', '_', '-', ' '])
			.filter_map(|part| part.chars().next())
			.take(2)
			.collect();

		if initials.is_empty() {
			initials.push('?');
		}

		initials.to_uppercase()
	}
}

/// Where a screen sits in the rail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Section {
	Overview,
	Live,
	Trends,
	Funnels,
	Retention,
	Library,
	People,
	Projects,
	Project(String),
	ProjectHealth(String),
	ProjectOperations(String),
	Learn,
	Glossary,
}

/// The pages this crate builds. Every other page is still rendered by
/// `leptoskit_admin` alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
	/// The glossary of words the admin uses.
	Glossary,
}

impl Page {
	/// The page's title, before the app's name.
	#[must_use]
	pub const fn title(&self) -> &'static str {
		match self {
			Self::Glossary => "Glossary",
		}
	}

	/// Where the page sits in the rail.
	#[must_use]
	pub const fn section(&self) -> Section {
		match self {
			Self::Glossary => Section::Glossary,
		}
	}
}

/// Everything a page is built from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
	/// Who is looking.
	pub viewer: Viewer,
	/// Which page, with its data.
	pub page: Page,
}

impl Model {
	/// The document's title: `Glossary · Notes admin`.
	#[must_use]
	pub fn document_title(&self) -> String {
		format!("{} · {} admin", self.page.title(), self.viewer.app_name)
	}
}
