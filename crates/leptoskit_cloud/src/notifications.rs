//! Hosted notifications: the notifications service, served to every
//! environment on a cluster by that cluster's shared hub.
//!
//! An app that declares `[services.notifications]` with the registry crate
//! [`CRATE`] runs no process of its own for it in the cloud. Its
//! environment becomes a **tenant** of its cluster's hub instead: a
//! database of its own on the cluster's Postgres server, a token the app
//! and the hub share, and the push credentials and settings the service
//! reads, set as the environment's secrets and variables under the names
//! in [`SETTINGS`]. The edge sends `/_leptoskit/services/notifications/` on
//! each of the environment's hostnames to the hub, and the app's server
//! reaches it at [`URL_ENV`].
//!
//! The hub is a worker of the role `notifications`. It asks
//! [`notification_tenants`] what changed among its cluster's tenants, is
//! answered each changed tenant's values (leased, and recorded as such),
//! serves them, and says how each is doing and how many pushes each sent
//! with [`report_notifications`]. The design is in
//! `docs/agents/cloud-notifications.md`.

use std::collections::BTreeMap;

use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::workers::RuntimeValue;

/// The service's name, as `[services]` keys it and its paths name it.
pub const SERVICE: &str = "notifications";

/// The registry crate an app's `[services.notifications]` names for the
/// cloud to serve it.
pub const CRATE: &str = "leptoskit_notifications";

/// Where the app's server finds the hub: its in-cluster address and the
/// tenant's path.
pub const URL_ENV: &str = "LEPTOSKIT_SERVICE_NOTIFICATIONS_URL";

/// The token the app's server signs its calls to the hub with, and that
/// forwarded identities are derived from. A secret.
pub const TOKEN_ENV: &str = "LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN";

/// Tells the app's server to forward the signed-in person to the service.
pub const IDENTITY_ENV: &str = "LEPTOSKIT_SERVICE_NOTIFICATIONS_IDENTITY";

/// What a release leases the tenant's database address to its cluster's
/// worker as, so the worker can make the role with that password. The
/// worker never puts it in the environment's Secret.
pub const DATABASE_URL_LEASE: &str = "LEPTOSKIT_NOTIFICATIONS_DATABASE_URL";

/// The name of the platform service a release routes to and the bridge in
/// its namespace.
pub const PLATFORM_SERVICE: &str = "notifications";

/// The path the hub serves a tenant's private calls under: `/t/<tenant>`,
/// the tenant's ID in simple hex.
#[must_use]
pub fn tenant_path(tenant: Uuid) -> String {
	format!("/t/{}", tenant.simple())
}

/// The variables the notifications service reads, which an environment
/// sets as its secrets or variables for the hub. They are exempt from the
/// cloud's reserved `LEPTOSKIT_` prefix, and no release gives them to the
/// app's processes: only the hub reads them.
///
/// `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` isn't one: it widens which
/// hosts the service sends to, which on a shared hub is the operator's to
/// say, not a tenant's.
pub const SETTINGS: [&str; 14] = [
	"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8",
	"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
	"LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID",
	"LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC",
	"LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT",
	"LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT",
	"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_DELIVERIES_DAYS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS",
];

/// The [`SETTINGS`] that are private keys: an environment keeps them only
/// as secrets, never as variables, which anyone who can view the project
/// reads.
pub const SECRET_SETTINGS: [&str; 3] = [
	"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8",
	"LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY",
];

/// Whether `name` is one of the notifications service's [`SETTINGS`].
#[must_use]
pub fn is_setting(name: &str) -> bool {
	SETTINGS.contains(&name)
}

/// The push providers whose settings `names` has any of, as the service
/// names them: `apns`, `fcm` and `web_push`. Names only: whether a
/// provider is set up whole is the hub's to say.
#[must_use]
pub fn providers<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
	let names: Vec<&str> = names.into_iter().collect();
	[
		("apns", "LEPTOSKIT_NOTIFICATIONS_APNS_"),
		("fcm", "LEPTOSKIT_NOTIFICATIONS_FCM_"),
		("web_push", "LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_"),
	]
	.into_iter()
	.filter(|(_, prefix)| {
		names
			.iter()
			.any(|name| name.starts_with(prefix) && is_setting(name))
	})
	.map(|(provider, _)| provider.to_owned())
	.collect()
}

/// The most changed tenants one answer of [`notification_tenants`] holds.
/// The hub asks again while [`TenantChanges::more`] says so.
pub const MAX_TENANTS_PER_ANSWER: usize = 50;

/// The most tenants a hub may say it holds in one ask.
pub const MAX_KNOWN_TENANTS: usize = 10_000;

/// The most tenant reports, and usage reports, in one
/// [`report_notifications`].
pub const MAX_REPORTS: usize = 500;

/// The longest reason a hub gives for a tenant it can't serve, in
/// characters.
pub const MAX_REASON_LEN: usize = 500;

/// The longest usage cursor, in characters.
pub const MAX_CURSOR_LEN: usize = 128;

/// The most pushes one usage report may count.
pub const MAX_PUSHES_PER_REPORT: u64 = 100_000_000;

/// How the hub last said an environment's tenant is doing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TenantState {
	/// No release that uses it has been applied yet, so its database may
	/// not exist.
	Pending,
	/// Its database exists, and the live release doesn't use it, so the hub
	/// doesn't serve it. Deploying with the service again finds its inbox
	/// as it was.
	Unused,
	/// Its database exists, and the hub hasn't said it serves this version
	/// of it yet: it does within a minute or so.
	Waiting,
	/// The hub serves it.
	Serving,
	/// The hub can't serve it, and says why in the service's own words,
	/// such as a push provider that is half configured.
	Failed { reason: String },
	/// Its environment is being deleted.
	Removing,
}

/// An environment's hosted notifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationsSummary {
	/// The tenant's ID, which names it in the hub's logs and paths.
	pub tenant: Uuid,
	/// Its database's name on the cluster's Postgres server.
	pub database: String,
	pub state: TenantState,
	/// When the hub last said how it is doing.
	pub reported_at: Option<Timestamp>,
	/// The push providers some setting is set for: `apns`, `fcm`,
	/// `web_push`.
	pub providers: Vec<String>,
	/// The [`SETTINGS`] the environment has, as secrets or variables, by
	/// name. Never a value.
	pub settings: Vec<String>,
	/// Pushes the providers accepted this calendar month (UTC) for the
	/// whole project: usage is counted per project.
	pub project_pushes_this_month: u64,
	/// Its pushes are stopped until next month, or until the organisation
	/// is on a paid plan: the plan costs nothing and its push allowance is
	/// used. The inbox works as before.
	#[serde(default)]
	pub pushes_paused: bool,
	/// Its token and its database's password are being replaced, because a
	/// worker that was given them was revoked: the next deployment
	/// finishes it.
	#[serde(default)]
	pub rotating: bool,
	pub created_at: Timestamp,
}

/// An environment's hosted notifications, or `None` when it has none: its
/// app doesn't declare the service, or hasn't been deployed with it.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "notifications")]
#[rpc::query]
pub async fn notifications(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Option<NotificationsSummary>> {
	crate::server::notifications::summary(cx, &organization, &project, &environment).await
}

/// A tenant the hub serves, and the version of it it serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KnownTenant {
	pub tenant: Uuid,
	pub version: i64,
}

/// A tenant's values: what only the hub is given. `Debug` prints names
/// only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TenantSecrets {
	/// The tenant's database's address, with its role's password.
	pub database_url: String,
	/// The token the app signs its calls with; forwarded identities are
	/// derived from it.
	pub token: String,
	/// The environment's [`SETTINGS`], from its variables and secrets, by
	/// name.
	pub settings: BTreeMap<String, String>,
	/// The values replacing these, while they are replaced: the app holds
	/// either until a release with the new ones is applied, so the hub
	/// takes either token and reaches the database with either password.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next: Option<NextSecrets>,
}

impl std::fmt::Debug for TenantSecrets {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("TenantSecrets")
			.field("database_url", &"[redacted]")
			.field("token", &"[redacted]")
			.field("settings", &self.settings.keys().collect::<Vec<_>>())
			.field("next", &self.next)
			.finish()
	}
}

/// A tenant's database address and token that replace its current ones.
/// `Debug` prints neither.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NextSecrets {
	pub database_url: String,
	pub token: String,
}

impl std::fmt::Debug for NextSecrets {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("NextSecrets")
			.field("database_url", &"[redacted]")
			.field("token", &"[redacted]")
			.finish()
	}
}

/// One tenant as the hub serves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TenantConfig {
	pub tenant: Uuid,
	/// Goes up whenever anything here changes.
	pub version: i64,
	pub environment: RuntimeValue<EnvironmentRef>,
	/// The hostnames whose `/_leptoskit/services/notifications/` is this
	/// tenant's: the environment's own, then its verified custom domains.
	pub hostnames: Vec<String>,
	/// The one origin a browser's live connection may come from, or `None`
	/// when the environment has custom domains, so several do.
	pub origin: Option<String>,
	/// Where the hub's usage count for it starts: what it last reported
	/// with [`report_notifications`], or `None` from the beginning.
	pub usage_cursor: Option<String>,
	pub secrets: TenantSecrets,
}

/// What [`notification_tenants`] answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TenantChanges {
	/// Tenants the hub doesn't hold, or holds at another version.
	pub changed: Vec<TenantConfig>,
	/// Tenants the hub holds that it should stop serving.
	pub removed: Vec<Uuid>,
	/// More changed tenants are waiting: ask again.
	pub more: bool,
	/// The tenants among those served on the cluster whose pushes are
	/// stopped now, because their organisation's plan costs nothing and its
	/// push allowance for the month is used. Every answer lists all of
	/// them; the hub stops their pushes and starts the others'.
	#[serde(default)]
	pub paused: Vec<Uuid>,
}

/// What changed among the tenants of the calling hub's cluster, given what
/// it holds now: each tenant it doesn't hold at its current version, with
/// its values, and each it holds that it should drop. A tenant is served
/// once a release that uses its database has been applied, and until its
/// environment is deleted.
///
/// Every value in the answer is recorded as leased to the hub before it is
/// sent, so this is an action: no receipt keeps it.
///
/// # Errors
///
/// `Forbidden` (`notifications_worker_required`) for any other worker;
/// `Invalid` (`too_many_tenants`) for more than [`MAX_KNOWN_TENANTS`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn notification_tenants(
	cx: &rpc::ActionCx,
	#[serde(default)] known: Vec<KnownTenant>,
) -> rpc::Result<TenantChanges> {
	crate::server::notifications::tenants(cx, known).await
}

/// How the hub is doing with one tenant, at one version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HubState {
	Serving,
	Failed { reason: String },
}

/// The hub's word on one tenant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TenantReport {
	pub tenant: Uuid,
	/// The version it serves, or failed to.
	pub version: i64,
	#[serde(flatten)]
	pub state: HubState,
}

/// Pushes the providers accepted for one tenant since the last report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageReport {
	pub tenant: Uuid,
	/// Where this count starts: the cursor the control plane holds, which
	/// [`TenantConfig::usage_cursor`] says. A report from anywhere else is
	/// not counted.
	pub after: Option<String>,
	/// Where it ends, opaque to the control plane: the next report starts
	/// here.
	pub until: String,
	pub pushes: u64,
}

/// What [`report_notifications`] kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationsReported {
	/// Tenant reports kept: ones for a tenant of another cluster, or for a
	/// version that isn't the tenant's now, are dropped.
	pub tenants: u32,
	/// For each usage report, the cursor the control plane holds after it:
	/// its `until` when it was counted, and otherwise what to count from.
	pub cursors: BTreeMap<Uuid, Option<String>>,
}

/// The hub says how each of its tenants is doing, and how many pushes each
/// sent. A usage report is counted only when it starts where the last one
/// counted for that tenant ended, so two hubs counting the same deliveries
/// count them once.
///
/// # Errors
///
/// `Forbidden` (`notifications_worker_required`) for any other worker;
/// `Invalid` for more than [`MAX_REPORTS`] of either, a reason or cursor
/// longer than allowed, or more than [`MAX_PUSHES_PER_REPORT`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn report_notifications(
	cx: &rpc::ActionCx,
	#[serde(default)] tenants: Vec<TenantReport>,
	#[serde(default)] usage: Vec<UsageReport>,
) -> rpc::Result<NotificationsReported> {
	crate::server::notifications::report(cx, tenants, usage).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn settings_name_providers_and_nothing_else() {
		assert_eq!(
			providers([
				"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
				"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS",
				"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT",
				"LEPTOSKIT_NOTIFICATIONS_FCM_SOMETHING_ELSE",
			]),
			["apns", "web_push"]
		);
		assert!(is_setting("LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS"));
		assert!(!is_setting("LEPTOSKIT_NOTIFICATIONS_DATABASE_URL"));
		assert!(!is_setting("LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN"));
		assert!(!is_setting("LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS"));

		for name in SECRET_SETTINGS {
			assert!(is_setting(name), "{name}");
		}
	}

	#[test]
	fn a_tenants_secrets_print_no_value() {
		let secrets = TenantSecrets {
			database_url: "postgres://ntf_a:c0ffee@db/ntf_a".to_owned(),
			token: "hunter2".to_owned(),
			settings: BTreeMap::from([(
				"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8".to_owned(),
				"-----BEGIN PRIVATE KEY-----".to_owned(),
			)]),
			next: Some(NextSecrets {
				database_url: "postgres://ntf_a:d00d1e@db/ntf_a".to_owned(),
				token: "swordfish".to_owned(),
			}),
		};
		let printed = format!("{secrets:?}");
		assert!(!printed.contains("c0ffee"), "{printed}");
		assert!(!printed.contains("hunter2"), "{printed}");
		assert!(!printed.contains("d00d1e"), "{printed}");
		assert!(!printed.contains("swordfish"), "{printed}");
		assert!(!printed.contains("BEGIN"), "{printed}");
		assert!(printed.contains("APNS_KEY_P8"), "{printed}");
	}

	#[test]
	fn a_tenants_path_is_its_id_in_hex() {
		let tenant = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
		assert_eq!(tenant_path(tenant), "/t/0123456789abcdef0123456789abcdef");
	}
}
