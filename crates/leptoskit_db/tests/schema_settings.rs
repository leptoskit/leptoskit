//! The settings and row-level-security checks `schema_with_row_security` runs
//! before a migration is generated: names Postgres can see, settings declared
//! twice, and rules the database can't enforce.

#![cfg(feature = "server")]

use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::ListSetting;
use leptoskit_db::Setting;
use leptoskit_db::policy;
use leptoskit_db::table;

/// The request's tenant.
struct Tenant(String);

impl Setting for Tenant {
	type Value = String;

	const NAME: &'static str = "tenant";

	fn value(&self) -> String {
		self.0.clone()
	}
}

/// Another setting spelled like `tenant`, which Postgres couldn't tell
/// apart.
struct Workspace(String);

impl Setting for Workspace {
	type Value = String;

	const NAME: &'static str = "tenant";

	fn value(&self) -> String {
		self.0.clone()
	}
}

/// A setting whose name isn't snake case.
struct NotSnakeCase;

impl Setting for NotSnakeCase {
	type Value = bool;

	const NAME: &'static str = "NotSnakeCase";

	fn value(&self) -> bool {
		true
	}
}

/// A list setting, to show the list registration path is checked too.
struct Roles(Vec<String>);

impl ListSetting for Roles {
	type Item = String;

	const NAME: &'static str = "roles";

	fn values(&self) -> Vec<String> {
		self.0.clone()
	}
}

leptoskit_db::register_setting!(Tenant, Workspace, Tenant, NotSnakeCase);
leptoskit_db::register_list_setting!(Roles);

/// A folder whose rules count its own rows, which Postgres can't evaluate
/// without recursing into the same policy.
#[table(name = "folders")]
#[policy(select, |folder, cx| cx.exists(
	Folder::query().filter(|child| child.parent_id.eq(folder.id))
))]
pub struct Folder {
	pub id: Id<Self>,
	pub parent_id: Option<Id<Self>>,
	pub name: String,
}

#[test]
fn row_security_reports_settings_and_rules_postgres_cant_evaluate() {
	let error = leptoskit_db::schema_with_row_security(Dialect::Postgres)
		.unwrap_err()
		.to_string();

	assert!(
		error.contains("must be lowercase snake case"),
		"a setting's name is checked: {error}"
	);
	assert!(
		error.contains("is named `tenant` too"),
		"two settings can't share a name: {error}"
	);
	assert!(
		error.contains("row-level security") && error.contains("count its own rows"),
		"a rule the database can't enforce is named: {error}"
	);
	// Registering the same type twice is fine.
	assert!(
		!error.contains("`Tenant` is named"),
		"one type may be registered twice: {error}"
	);
}
