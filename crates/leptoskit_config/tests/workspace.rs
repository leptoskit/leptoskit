//! Workspace startup ordering is independent of call direction and Cargo.

use leptoskit_config::workspace::Workspace;

fn manifest() -> Workspace {
	toml::from_str(
		r#"
format_version = 1
[applications.web]
path = "apps/web"
depends-on = ["accounts"]
[applications.web.bindings.accounts]
application = "accounts"
url-env = "ACCOUNTS_URL"
token-env = "ACCOUNTS_TOKEN"
[applications.accounts]
path = "apps/accounts"
[applications.accounts.bindings.web]
application = "web"
url-env = "WEB_URL"
token-env = "WEB_TOKEN"
"#,
	)
	.unwrap()
}

#[test]
fn bidirectional_calls_do_not_create_a_startup_cycle() {
	let workspace = manifest();
	assert_eq!(workspace.startup_order().unwrap(), ["accounts", "web"]);
}

#[test]
fn readiness_cycles_missing_targets_and_duplicates_are_actionable() {
	let mut workspace = manifest();
	workspace
		.applications
		.get_mut("accounts")
		.unwrap()
		.depends_on
		.push("web".into());
	assert!(workspace.validate().unwrap_err().contains("cycle"));
	workspace
		.applications
		.get_mut("accounts")
		.unwrap()
		.depends_on = vec!["unknown".into()];
	assert!(
		workspace
			.validate()
			.unwrap_err()
			.contains("unknown application `unknown`")
	);
	workspace
		.applications
		.get_mut("accounts")
		.unwrap()
		.depends_on
		.clear();
	workspace
		.applications
		.get_mut("web")
		.unwrap()
		.depends_on
		.push("accounts".into());
	assert!(workspace.validate().unwrap_err().contains("repeats"));
}

#[test]
fn bindings_require_distinct_environment_names_and_known_peers() {
	let mut workspace = manifest();
	let binding = workspace
		.applications
		.get_mut("web")
		.unwrap()
		.bindings
		.get_mut("accounts")
		.unwrap();
	binding.token_env = binding.url_env.clone();
	assert!(workspace.validate().unwrap_err().contains("already used"));
	let binding = workspace
		.applications
		.get_mut("web")
		.unwrap()
		.bindings
		.get_mut("accounts")
		.unwrap();
	binding.token_env = "literal credential is not a variable".into();
	assert!(
		workspace
			.validate()
			.unwrap_err()
			.contains("uppercase environment variable")
	);
	let binding = workspace
		.applications
		.get_mut("web")
		.unwrap()
		.bindings
		.get_mut("accounts")
		.unwrap();
	binding.token_env = "ACCOUNTS_TOKEN".into();
	binding.application = "web".into();
	assert!(
		workspace
			.validate()
			.unwrap_err()
			.contains("own application")
	);
	let binding = workspace
		.applications
		.get_mut("web")
		.unwrap()
		.bindings
		.get_mut("accounts")
		.unwrap();
	binding.application = "absent".into();
	assert!(
		workspace
			.validate()
			.unwrap_err()
			.contains("unknown application")
	);
}

#[test]
fn versions_paths_and_unknown_fields_fail_before_building() {
	let mut workspace = manifest();
	workspace.format_version = 2;
	assert!(workspace.validate().unwrap_err().contains("format_version"));
	workspace.format_version = 1;
	for path in ["", "../outside", "/outside"] {
		workspace.applications.get_mut("web").unwrap().path = path.into();
		assert!(
			workspace
				.validate()
				.unwrap_err()
				.contains("applications.web.path")
		);
	}
	assert!(toml::from_str::<Workspace>("format_version=1\nunknown=true\n[applications]").is_err());
	workspace.applications.clear();
	assert!(
		workspace
			.validate()
			.unwrap_err()
			.contains("between 1 and 256")
	);
}

#[test]
fn insertion_order_cannot_change_startup_order() {
	let mut workspace = manifest();
	workspace
		.applications
		.get_mut("web")
		.unwrap()
		.depends_on
		.clear();
	assert_eq!(workspace.startup_order().unwrap(), ["accounts", "web"]);
}
