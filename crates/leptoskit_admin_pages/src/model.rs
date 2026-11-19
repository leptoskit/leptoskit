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

/// A lesson as a row in its track on the Learn hub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonRow {
	/// The lesson's slug, which names its test id.
	pub slug: String,
	pub title: String,
	pub minutes: u8,
	/// The lesson's own page.
	pub href: String,
	/// What shows the outcome holds, once it does.
	pub evidence: Option<String>,
	/// The outcome it checks: `Check: you applied a plan`.
	pub check: String,
}

/// The first lesson whose outcome doesn't hold yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpNext {
	pub title: String,
	pub minutes: u8,
	/// The track's title.
	pub track: String,
	/// What it teaches.
	pub body: String,
	/// The outcome it checks: `Check: you applied a plan`.
	pub check: String,
	/// Where it's done, and the button that goes there.
	pub target: String,
	pub label: String,
	/// The lesson's own page.
	pub href: String,
}

/// The Learn hub: every lesson, checked against the project's real state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Learn {
	pub analytics: Vec<LessonRow>,
	pub infrastructure: Vec<LessonRow>,
	/// Whether the admin could read analytics to check its lessons.
	pub analytics_readable: bool,
	/// `None` once every lesson holds.
	pub up_next: Option<UpNext>,
}

/// One lesson's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lesson {
	pub title: String,
	/// The track's title.
	pub track: String,
	pub minutes: u8,
	/// What it teaches.
	pub body: String,
	/// The outcome it checks: `You applied a plan`.
	pub outcome: String,
	/// Where it's done.
	pub target: String,
	/// What shows the outcome holds, once it does.
	pub evidence: Option<String>,
}

/// The pages this crate builds. Every other page is still rendered by
/// `leptoskit_admin` alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
	/// The glossary of words the admin uses.
	Glossary,
	/// The Learn hub: progress, what's next, and every lesson.
	Learn(Learn),
	/// One lesson.
	Lesson(Lesson),
}

impl Page {
	/// The page's title, before the app's name.
	#[must_use]
	pub fn title(&self) -> &str {
		match self {
			Self::Glossary => "Glossary",
			Self::Learn(_) => "Your progress",
			Self::Lesson(lesson) => &lesson.title,
		}
	}

	/// Where the page sits in the rail.
	#[must_use]
	pub const fn section(&self) -> Section {
		match self {
			Self::Glossary => Section::Glossary,
			Self::Learn(_) | Self::Lesson(_) => Section::Learn,
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
