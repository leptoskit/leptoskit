//! A project's pages: its tabs, what each shows, and the actions each
//! offers.

use serde::Deserialize;
use serde::Serialize;

use crate::components::Kind;
use crate::components::Line;
use crate::components::Tone;
use crate::workings::Drawer;

/// A project's tabs, in the order the tab row shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
	Overview,
	Changes,
	Releases,
	Operations,
	Health,
	Backups,
	Config,
}

impl Tab {
	/// Every tab, in order.
	pub const ALL: [Self; 7] = [
		Self::Overview,
		Self::Changes,
		Self::Releases,
		Self::Operations,
		Self::Health,
		Self::Backups,
		Self::Config,
	];

	/// The tab its path names: `changes`.
	#[must_use]
	pub fn parse(value: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|tab| tab.slug() == value)
	}

	/// Its part of the path.
	#[must_use]
	pub const fn slug(self) -> &'static str {
		match self {
			Self::Overview => "overview",
			Self::Changes => "changes",
			Self::Releases => "releases",
			Self::Operations => "operations",
			Self::Health => "health",
			Self::Backups => "backups",
			Self::Config => "config",
		}
	}

	/// Its name on the tab row.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Overview => "Overview",
			Self::Changes => "Changes",
			Self::Releases => "Releases",
			Self::Operations => "Operations",
			Self::Health => "Health",
			Self::Backups => "Backups",
			Self::Config => "Config",
		}
	}
}

/// One tab of one project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
	pub id: String,
	/// `Changes · notes-production`.
	pub title: String,
	pub environment: String,
	pub tab: Tab,
	/// A badge for each tab in [`Tab::ALL`]'s order: `3` changes planned,
	/// `1 running`.
	pub badges: Vec<Option<String>>,
	/// Why the admin can't read the project's `[infra]`, when it can't.
	pub unreadable: Option<String>,
	/// The operation running now.
	pub active: Option<Active>,
	/// Whether plans and applies run against a stand-in cloud.
	pub simulated: bool,
	pub body: Body,
	/// The tab's Workings drawers, in the order its cards show them.
	pub drawers: Vec<Drawer>,
}

/// The operation running on a project now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Active {
	/// `Running: Deploy v42 · started 2 minutes ago`.
	pub text: String,
	/// Its page.
	pub href: String,
}

/// An operation as a row in a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
	pub tone: Tone,
	/// Its phase in a word.
	pub word: String,
	/// What it does: `Deploy v42`.
	pub action: String,
	/// Who started it.
	pub actor: String,
	/// `5 minutes ago`.
	pub when: String,
	/// Its page.
	pub href: String,
}

/// What an action asks the control service to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
	Plan,
	Status,
	Backup,
	/// Applies exactly this plan.
	Apply {
		plan_id: String,
	},
	/// Deploys the version typed on the page.
	Deploy,
	/// Deploys the version typed on the page with the database setup.
	FirstRelease,
	/// Deploys the version typed on the page behind the maintenance page.
	Maintenance,
	/// Moves traffic back to `version`.
	Rollback {
		version: String,
	},
}

impl Request {
	/// The action's test id: `action:apply:<plan>`.
	#[must_use]
	pub fn test_id(&self) -> String {
		match self {
			Self::Plan => "action:plan".to_owned(),
			Self::Status => "action:status".to_owned(),
			Self::Backup => "action:backup".to_owned(),
			Self::Apply { plan_id } => format!("action:apply:{plan_id}"),
			Self::Deploy => "action:deploy".to_owned(),
			Self::FirstRelease => "action:first-release".to_owned(),
			Self::Maintenance => "action:maintenance".to_owned(),
			Self::Rollback { version } => format!("action:rollback:{version}"),
		}
	}

	/// Whether it takes the version typed on the page.
	#[must_use]
	pub const fn typed_version(&self) -> bool {
		matches!(self, Self::Deploy | Self::FirstRelease | Self::Maintenance)
	}

	/// What the admin's actions endpoint takes, with `version` for an
	/// action that takes the typed one, as JSON.
	#[must_use]
	pub fn body(&self, version: &str, idempotency_key: &str) -> String {
		let (kind, version, plan_id) = match self {
			Self::Plan => ("plan", None, None),
			Self::Status => ("status", None, None),
			Self::Backup => ("backup", None, None),
			Self::Apply { plan_id } => ("apply", None, Some(plan_id.as_str())),
			Self::Deploy => ("deploy", Some(version), None),
			Self::FirstRelease => ("first_release", Some(version), None),
			Self::Maintenance => ("maintenance", Some(version), None),
			Self::Rollback { version } => ("rollback", Some(version.as_str()), None),
		};
		let mut body = serde_json::json!({
			"kind": kind,
			"idempotency_key": idempotency_key,
		});

		if let Some(version) = version {
			body["version"] = version.into();
		}

		if let Some(plan_id) = plan_id {
			body["plan_id"] = plan_id.into();
		}

		body.to_string()
	}
}

/// A button that asks first, then sends its request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
	pub kind: Kind,
	pub label: String,
	pub request: Request,
	/// What the confirmation says it does, with `commands` in backticks.
	pub explain: String,
}

/// What a tab shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
	Overview(Box<Overview>),
	Changes(Box<Changes>),
	Releases(Releases),
	/// Every operation, newest first.
	Operations(Vec<Row>),
	Backups(Backups),
	Config(Config),
}

/// Overview: what's serving, what ran lately, the budget, and the topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overview {
	/// The release serving, shortened.
	pub release: Option<String>,
	/// The commit serving, when the release is one, shortened.
	pub commit: Option<String>,
	pub deployed_by: String,
	pub when: String,
	pub health: (Tone, String),
	/// Run health check.
	pub status: Action,
	/// The five newest operations.
	pub recent: Vec<Row>,
	/// `20.00 EUR a month`, when `[infra.budget]` sets one.
	pub budget: Option<String>,
	/// Drawn from `[infra]`, when the admin can read it.
	pub topology: Option<Topology>,
	/// Whether the margin note that tours the page shows.
	pub tour: bool,
}

/// The hosts `[infra]` declares, left to right.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
	pub domain: String,
	/// Each host: `web[0]`, `cx23 · hel1 · primary, Postgres`.
	pub hosts: Vec<(String, String)>,
	/// `encrypted · daily`, when backups are on.
	pub backups: Option<String>,
	/// `from [infra] · hetzner`.
	pub aside: String,
}

/// Changes: the plan and what to do with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
	/// The newest plan, when it failed after the one shown.
	pub failed: Option<Failed>,
	pub plan: Plan,
	/// Apply, when there's a plan to apply.
	pub apply: Apply,
	/// Plan changes, or Plan again.
	pub replan: Action,
	/// Whether the margin note about reviewing a plan shows.
	pub review_note: bool,
	/// Whether the plan's Workings opened by default, which counts as one of
	/// the first three times.
	pub workings_by_default: bool,
}

/// A plan that failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
	/// `The last plan failed 5 minutes ago`.
	pub when: String,
	pub message: String,
	/// Its operation.
	pub href: String,
}

/// The newest plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plan {
	/// No plan yet.
	None,
	/// It was applied: `The plan from 5 minutes ago was applied…`.
	Applied(String),
	/// It found nothing to change.
	Clean,
	/// What it changes.
	Changes {
		/// `1 to create, 0 to update, 0 to delete`.
		summary: String,
		/// `Planned 5 minutes ago by ada.`
		planned: String,
		/// The changes by resource kind, in the plan's order.
		groups: Vec<Group>,
	},
}

/// The changes to one kind of resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
	/// `server`.
	pub kind: String,
	pub rows: Vec<PlanRow>,
}

/// One resource a plan changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRow {
	/// The `[infra]` key that causes it.
	pub key: String,
	/// `+ create`.
	pub label: String,
	/// `create`.
	pub verb: String,
	pub tone: Tone,
	pub address: String,
	/// Whether it keeps data.
	pub persistent: bool,
	/// Why Apply won't run it, when it won't.
	pub blocker: Option<String>,
}

/// What Apply can do with the plan shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Apply {
	/// There's nothing to apply.
	None,
	/// The plan deletes something that keeps data.
	Blocked,
	/// Apply N changes.
	Ready(Action),
}

/// Releases: deploy, roll back, and every release so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Releases {
	/// Deploy, First release, Deploy in maintenance.
	pub deploy: Vec<Action>,
	pub rollbacks: Vec<Rollback>,
	pub history: Vec<Row>,
}

/// An earlier release to roll back to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rollback {
	/// The version, shortened.
	pub version: String,
	pub when: String,
	pub action: Action,
}

/// Backups: what `[infra.backups]` keeps, and the last verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backups {
	pub schedule: Schedule,
	/// The last verification, in a sentence.
	pub last: (Tone, String),
	pub verify: Action,
}

/// What `[infra.backups]` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Schedule {
	On {
		schedule: String,
		daily: u32,
		weekly: u32,
		monthly: u32,
		max_age_hours: u32,
	},
	Off,
	/// The admin can't read `[infra]`.
	Unknown,
}

/// Config: `[infra]` as written, and what each setting does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
	/// `[infra]`'s lines; empty when there's none to show.
	pub declared: Vec<Line>,
	/// Each key with a line on what it does.
	pub explained: Vec<(String, String)>,
}
