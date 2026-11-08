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

#[test]
#[allow(clippy::cognitive_complexity)]
fn optional_kind_names_frontend_roles() {
	use std::str::FromStr as _;

	use leptoskit_config::workspace::ApplicationKind;

	let workspace: Workspace = toml::from_str(
		r#"
format_version = 1
[applications.landing]
path = "apps/landing"
kind = "landing"
[applications.app]
path = "apps/app"
kind = "product"
[applications.admin]
path = "apps/admin"
kind = "admin"
[applications.api]
path = "apps/api"
kind = "server"
"#,
	)
	.unwrap();
	workspace.validate().unwrap();
	assert_eq!(
		workspace.applications["landing"].kind,
		Some(ApplicationKind::Landing)
	);
	assert_eq!(
		workspace.applications["app"].kind,
		Some(ApplicationKind::Product)
	);
	assert_eq!(
		workspace.applications["admin"].kind,
		Some(ApplicationKind::Admin)
	);
	assert_eq!(
		workspace.applications["api"].kind,
		Some(ApplicationKind::Server)
	);
	assert!(
		toml::from_str::<Workspace>(
			"format_version=1\n[applications.web]\npath='apps/web'\nkind='website'\n"
		)
		.is_err()
	);

	for kind in [
		ApplicationKind::Landing,
		ApplicationKind::Product,
		ApplicationKind::Admin,
		ApplicationKind::Server,
	] {
		assert_eq!(ApplicationKind::from_str(kind.as_str()).unwrap(), kind);
		assert_eq!(kind.to_string(), kind.as_str());
		assert_ne!(kind.role_label(), "");
	}
	assert_eq!(
		ApplicationKind::from_str("app").unwrap(),
		ApplicationKind::Product
	);
	assert!(
		ApplicationKind::from_str("website")
			.unwrap_err()
			.contains("server")
	);

	assert!(ApplicationKind::Landing.is_web_frontend());
	assert!(ApplicationKind::Product.is_web_frontend());
	assert!(ApplicationKind::Admin.is_web_frontend());
	assert!(!ApplicationKind::Server.is_web_frontend());
	assert!(ApplicationKind::Product.includes_native());
	assert!(!ApplicationKind::Landing.includes_native());
	assert!(!ApplicationKind::Admin.includes_native());
	assert!(!ApplicationKind::Server.includes_native());
	assert_eq!(ApplicationKind::Landing.default_port(), 3000);
	assert_eq!(ApplicationKind::Product.default_port(), 3001);
	assert_eq!(ApplicationKind::Admin.default_port(), 3002);
	assert_eq!(ApplicationKind::Server.default_port(), 3003);
	assert_eq!(ApplicationKind::Server.application_id(), "api");
	assert_eq!(
		ApplicationKind::parse_frontend_list(&[]).unwrap(),
		ApplicationKind::DEFAULT_FRONTENDS
	);
}
