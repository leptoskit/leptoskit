use std::fmt::Write as _;
use std::future::IntoFuture as _;
use std::sync::Arc;
use std::sync::Mutex;

use axum::response::IntoResponse as _;
use axum::response::Response;
use leptoskit_cloud::archive;

use super::credentials::Session;
use super::credentials::Store;
use super::*;

/// Match the private credential directory the real client creates.
fn private_directory() -> tempfile::TempDir {
	let directory = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	directory
}

type Answer = dyn Fn(&str, &str, &str) -> Response + Send + Sync;

struct Server {
	url: String,
	stop: Option<tokio::sync::oneshot::Sender<()>>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Server {
	fn drop(&mut self) {
		if let Some(stop) = self.stop.take() {
			let _ = stop.send(());
		}
		if let Some(thread) = self.thread.take() {
			thread.join().unwrap();
		}
	}
}

fn server(answer: impl Fn(&str, &str, &str) -> Response + Send + Sync + 'static) -> Server {
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	let discovery =
		json!({ "cli_client_id": "cli", "scope": "openid cloud offline_access", "rpc": "/rpc" });
	let (stop, stopped) = tokio::sync::oneshot::channel();
	let oauth = json!({"device_authorization_endpoint": format!("{url}/identity/device"), "token_endpoint": format!("{url}/identity/token"), "revocation_endpoint": format!("{url}/identity/revoke")});
	let answer: Arc<Answer> = Arc::new(answer);
	let thread = std::thread::spawn(move || {
		tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
			let router = axum::Router::new().fallback(move |uri: axum::http::Uri, headers: axum::http::HeaderMap, body: axum::body::Bytes| {
				let answer = Arc::clone(&answer);
				let discovery = discovery.clone();
				let oauth = oauth.clone();
				async move {
					if uri.path() == "/.well-known/leptoskit-cloud" { return axum::Json(discovery).into_response(); }
					if uri.path() == "/.well-known/openid-configuration" { return axum::Json(oauth).into_response(); }
					answer(uri.path(), headers.get("authorization").and_then(|value| value.to_str().ok()).unwrap_or_default(), &String::from_utf8_lossy(&body))
				}
			});
			tokio::select! {
				result = axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), router).into_future() => { result.unwrap(); },
				_ = stopped => {},
			}
		});
	});
	Server {
		url,
		stop: Some(stop),
		thread: Some(thread),
	}
}

fn json_response(value: Value) -> Response {
	axum::Json(value).into_response()
}

fn session(expires_at: u64) -> Session {
	Session {
		client_id: "cli".into(),
		access: Secret::new("known-access".into()),
		refresh: Secret::new("known-refresh".into()),
		expires_at,
	}
}

#[test]
fn cloud_origin_validation() {
	for allowed in [
		"https://cloud.example",
		"http://localhost:1234",
		"http://127.1.2.3:1234",
		"http://[::1]:1234",
	] {
		assert!(transport::origin(allowed).is_ok(), "{allowed}");
	}
	for refused in [
		"http://cloud.example",
		"https://user:secret@cloud.example",
		"https://cloud.example/path",
		"https://cloud.example?q=1",
		"https://cloud.example/#fragment",
	] {
		assert!(transport::origin(refused).is_err());
	}
}

#[test]
fn cloud_refresh_is_serialized_and_rotation_is_stored() {
	let refreshes = Arc::new(Mutex::new(0));
	let recorded = Arc::clone(&refreshes);
	let server = server(move |path, authorization, body| {
		match path {
			"/identity/token" => {
				assert!(body.contains("refresh_token=known-refresh"));
				*recorded.lock().unwrap() += 1;
				json_response(
					json!({ "access_token": "new-access", "refresh_token": "rotated-refresh", "expires_in": 3600 }),
				)
			}
			"/rpc/organizations" => {
				assert_eq!(authorization, "Bearer new-access");
				json_response(json!([]))
			}
			_ => axum::http::StatusCode::NOT_FOUND.into_response(),
		}
	});
	let directory = private_directory();
	let store = Store::new(directory.path(), &server.url);
	// TempDir starts private, matching the real cloud directory.
	store.save(&session(0)).unwrap();
	std::thread::scope(|scope| {
		for _ in 0..2 {
			let url = &server.url;
			let directory = directory.path();
			scope.spawn(move || {
				Client::new(url, directory, None)
					.unwrap()
					.organizations(account::Organizations {})
					.unwrap();
			});
		}
	});
	assert_eq!(*refreshes.lock().unwrap(), 1);
	let stored = store.load().unwrap().unwrap();
	assert_eq!(stored.refresh.expose(), "rotated-refresh");
	for secret in ["new-access", "rotated-refresh"] {
		assert!(!format!("{stored:?}").contains(secret));
	}
}

#[test]
fn cloud_refreshes_once_after_unauthorized_and_ci_token_never_touches_storage() {
	let calls = Arc::new(Mutex::new(Vec::new()));
	let seen = Arc::clone(&calls);
	let server = server(move |path, authorization, _| {
		seen.lock()
			.unwrap()
			.push((path.to_owned(), authorization.to_owned()));
		match path {
			"/identity/token" => {
				json_response(
					json!({"access_token": "new-access", "refresh_token": "rotated-refresh", "expires_in": 3600}),
				)
			}
			"/rpc/organizations" if authorization == "Bearer known-access" => {
				(
					axum::http::StatusCode::UNAUTHORIZED,
					"ServerError|sign_in_required: denied",
				)
					.into_response()
			}
			"/rpc/organizations" => json_response(json!([])),
			_ => axum::http::StatusCode::NOT_FOUND.into_response(),
		}
	});
	let directory = private_directory();
	let store = Store::new(directory.path(), &server.url);
	store.save(&session(u64::MAX)).unwrap();
	Client::new(&server.url, directory.path(), None)
		.unwrap()
		.organizations(account::Organizations {})
		.unwrap();
	assert_eq!(
		calls
			.lock()
			.unwrap()
			.iter()
			.filter(|(path, _)| path == "/identity/token")
			.count(),
		1
	);
	let before = std::fs::read_dir(directory.path())
		.unwrap()
		.map(|entry| entry.unwrap().path())
		.collect::<Vec<_>>();
	Client::new(
		&server.url,
		directory.path(),
		Some(Secret::new("ci-token".into())),
	)
	.unwrap()
	.organizations(account::Organizations {})
	.unwrap();
	assert_eq!(store.load().unwrap().unwrap().access.expose(), "new-access");
	assert_eq!(
		before.len(),
		std::fs::read_dir(directory.path()).unwrap().count()
	);
}

#[test]
fn cloud_logout_removes_credentials_online_and_offline() {
	let server = server(|path, _, body| {
		assert_eq!(path, "/identity/revoke");
		assert!(body.contains("token=known-refresh"));
		axum::http::StatusCode::OK.into_response()
	});
	let directory = private_directory();
	let store = Store::new(directory.path(), &server.url);
	store.save(&session(0)).unwrap();
	let client = Client::new(&server.url, directory.path(), None).unwrap();
	assert!(client.logout().unwrap());
	assert!(store.load().unwrap().is_none());
	store.save(&session(0)).unwrap();
	drop(server);
	assert!(!client.logout().unwrap());
	assert!(store.load().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn cloud_credentials_are_private_and_refuse_broad_permissions() {
	use std::os::unix::fs::PermissionsExt as _;
	let directory = private_directory();
	let root = directory.path().join("cloud");
	let store = Store::new(&root, DEFAULT_URL);
	store.save(&session(0)).unwrap();
	let file = std::fs::read_dir(&root)
		.unwrap()
		.next()
		.unwrap()
		.unwrap()
		.path();
	assert_eq!(
		std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
		0o700
	);
	assert_eq!(
		std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
		0o600
	);
	std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
	let error = format!("{:#}", store.load().unwrap_err());
	assert!(error.contains("chmod 600"));
	assert!(!error.contains("known-access"));
	assert!(!error.contains("known-refresh"));
}

#[test]
fn cloud_link_preserves_unrelated_comments_and_refuses_overwrite() {
	let directory = private_directory();
	let file = directory.path().join("leptoskit.toml");
	std::fs::write(
		&file,
		"# project comment\n[app]\nname = \"Notes\" # app comment\n\n[cloud]\nregion = \
		 \"eu-central\" # region comment\n",
	)
	.unwrap();
	write_link(&file, "team", "notes", DEFAULT_URL, false, false).unwrap();
	let text = std::fs::read_to_string(&file).unwrap();
	assert!(text.contains("# project comment"));
	assert!(text.contains("# app comment"));
	assert!(text.contains("# region comment"));
	let parsed: toml::Value = toml::from_str(&text).unwrap();
	let config: leptoskit_config::cloud::CloudConfig = parsed["cloud"].clone().try_into().unwrap();
	leptoskit_config::LeptoskitConfig::load_from(directory.path()).unwrap();
	assert_eq!(config.project.unwrap().as_str(), "notes");
	assert!(write_link(&file, "team", "other", DEFAULT_URL, false, false).is_err());
}

#[test]
fn cloud_help_tree_is_pinned() {
	use clap::CommandFactory as _;
	fn visit(mut command: clap::Command, path: &str) {
		let help = command.render_long_help().to_string();
		insta::assert_snapshot!(path.replace(' ', "_"), help);
		for child in command.get_subcommands() {
			visit(child.clone(), &format!("{path} {}", child.get_name()));
		}
	}
	let command = crate::cli::Cli::command();
	visit(command.find_subcommand("cloud").unwrap().clone(), "cloud");
}

#[test]
fn cloud_human_output_is_pinned() {
	insta::assert_snapshot!(human(
		&json!({ "account": "account-id", "email": "person@example.com", "via": "cli", "organizations": [{ "name": "Notes", "slug": "notes", "role": "owner" }] })
	));
}

#[test]
fn cloud_device_login_pending_slow_down_approved_denied_and_expired() {
	for outcome in ["approved", "access_denied", "expired_token"] {
		let calls = Arc::new(Mutex::new(0));
		let seen = Arc::clone(&calls);
		let origin = Arc::new(Mutex::new(String::new()));
		let host = Arc::clone(&origin);
		let server = server(move |path, _, body| {
			if path == "/identity/device" {
				assert!(body.contains("client_id=cli"));
				return json_response(
					json!({ "device_code": "known-device", "user_code": "ABCD-EFGH", "verification_uri": format!("{}/device", host.lock().unwrap()), "interval": 1, "expires_in": 60 }),
				);
			}
			assert_eq!(path, "/identity/token");
			assert!(body.contains("device_code=known-device"));
			let mut count = seen.lock().unwrap();
			*count += 1;
			if outcome != "approved" {
				return (
					axum::http::StatusCode::BAD_REQUEST,
					axum::Json(json!({ "error": outcome })),
				)
					.into_response();
			}
			match *count {
				1 => {
					(
						axum::http::StatusCode::BAD_REQUEST,
						axum::Json(json!({ "error": "authorization_pending" })),
					)
						.into_response()
				}
				2 => {
					(
						axum::http::StatusCode::BAD_REQUEST,
						axum::Json(json!({ "error": "slow_down" })),
					)
						.into_response()
				}
				_ => {
					json_response(
						json!({ "access_token": "known-access", "refresh_token": "known-refresh", "expires_in": 3600 }),
					)
				}
			}
		});
		*origin.lock().unwrap() = server.url.clone();
		let directory = private_directory();
		let client = Client::new(&server.url, directory.path(), None).unwrap();
		let mut printed = String::new();
		let result = client.login(|verification| {
			printed = format!("{verification:?}");
			Ok(())
		});
		if outcome == "approved" {
			result.unwrap();
		} else {
			write!(printed, "{:?}", result.unwrap_err()).unwrap();
		}
		for secret in ["known-device", "known-access", "known-refresh"] {
			assert!(!printed.contains(secret));
		}
	}
}

fn command_cases() -> Vec<(Vec<&'static str>, &'static str, Value, Value)> {
	let id = "00000000-0000-0000-0000-000000000001";
	let time = "2026-11-18T00:00:00Z";
	let organization = json!({"id": id, "slug": "team", "name": "Team", "plan": "hobby", "personal": false, "role": "owner"});
	let project = json!({"id": id, "slug": "notes", "name": "Notes", "region": "eu-central", "created_at": time});
	let scale =
		json!({"app": {"size": "xs", "min": 0, "max": 1, "sleep": "when-idle"}, "services": {}});
	let environment = json!({"id": id, "name": "staging", "kind": "staging", "scale": scale, "revision": 7, "scale_differs": false, "created_at": time, "updated_at": time});
	let token = json!({"id": id, "name": "ci", "prefix": "lkc_0123456789ab", "scopes": ["read"], "project": null, "expires_at": time, "last_used_at": null, "created_by": id, "created_at": time});
	let invitation = json!({"id": id, "email": "invite@example.com", "role": "viewer", "invited_by": id, "expires_at": time, "created_at": time});
	vec![
		(
			vec!["whoami"],
			"whoami",
			json!({}),
			json!({"account": id, "email": "person@example.com", "via": "cli", "operator": false, "organizations": [organization.clone()], "token": null}),
		),
		(
			vec!["org", "list"],
			"organizations",
			json!({}),
			json!([organization.clone()]),
		),
		(
			vec!["org", "create", "Team"],
			"create_organization",
			json!({"slug": "team", "name": "Team"}),
			organization,
		),
		(
			vec!["org", "members", "team"],
			"members",
			json!({"organization": "team"}),
			json!([{"account": id, "email": "person@example.com", "role": "owner", "joined_at": time}]),
		),
		(
			vec![
				"org",
				"invite",
				"team",
				"invite@example.com",
				"--role",
				"viewer",
			],
			"invite_member",
			json!({"organization": "team", "email": "invite@example.com", "role": "viewer"}),
			invitation.clone(),
		),
		(
			vec!["org", "invitations", "team"],
			"invitations",
			json!({"organization": "team"}),
			json!([invitation]),
		),
		(
			vec!["org", "revoke-invitation", "team", id, "--confirm", id],
			"revoke_invitation",
			json!({"organization": "team", "invitation": id}),
			Value::Null,
		),
		(
			vec!["org", "set-role", "team", id, "--role", "viewer"],
			"set_member_role",
			json!({"organization": "team", "account": id, "role": "viewer"}),
			Value::Null,
		),
		(
			vec!["org", "remove-member", "team", id, "--confirm", id],
			"remove_member",
			json!({"organization": "team", "account": id}),
			Value::Null,
		),
		(
			vec!["project", "list"],
			"projects",
			json!({"organization": "team"}),
			json!([project.clone()]),
		),
		(
			vec!["project", "create", "Notes", "--region", "eu-central"],
			"create_project",
			json!({"organization": "team", "slug": "notes", "name": "Notes", "region": "eu-central", "detected": {"targets": [], "variables": []}}),
			json!({"project": project.clone(), "environment": environment.clone(), "detected": {"targets": [], "variables": []}}),
		),
		(
			vec!["project", "show"],
			"project",
			json!({"organization": "team", "project": "notes"}),
			json!({"project": project, "environments": [environment.clone()], "detected": {"targets": [], "variables": []}}),
		),
		(
			vec!["project", "delete", "notes", "--confirm", "notes"],
			"delete_project",
			json!({"organization": "team", "project": "notes", "confirm": "notes"}),
			Value::Null,
		),
		(
			vec!["env", "list"],
			"environments",
			json!({"organization": "team", "project": "notes"}),
			json!([environment.clone()]),
		),
		(
			vec!["env", "create", "staging"],
			"create_environment",
			json!({"organization": "team", "project": "notes", "name": "staging", "kind": "staging"}),
			environment.clone(),
		),
		(
			vec!["env", "update", "staging", "--max", "2"],
			"update_environment",
			json!({"organization": "team", "project": "notes", "environment": "staging", "revision": 7, "scale": {"app": {"size": "xs", "min": 0, "max": 2, "sleep": "when-idle"}, "services": {}}}),
			environment.clone(),
		),
		(
			vec!["env", "delete", "staging", "--confirm", "staging"],
			"delete_environment",
			json!({"organization": "team", "project": "notes", "environment": "staging", "confirm": "staging"}),
			Value::Null,
		),
		(
			vec!["token", "list"],
			"tokens",
			json!({"organization": "team"}),
			json!([token.clone()]),
		),
		(
			vec![
				"token",
				"create",
				"ci",
				"--scope",
				"read",
				"--expires",
				"30",
			],
			"create_token",
			json!({"organization": "team", "name": "ci", "scopes": ["read"], "project": "notes", "expires_in_days": 30}),
			json!({"token": token, "value": "shown-once-api-token"}),
		),
		(
			vec!["token", "revoke", id, "--confirm", id],
			"revoke_token",
			json!({"organization": "team", "token": id}),
			Value::Null,
		),
	]
}

/// CLI 2 and 3: an environment's configuration, history, publishes and
/// usage.
fn environment_cases() -> Vec<(Vec<&'static str>, &'static str, Value, Value)> {
	let id = "00000000-0000-0000-0000-000000000001";
	let time = "2026-11-18T00:00:00Z";
	let variable =
		json!({"name": "API_URL", "value": "https://api.example.com", "updated_at": time});
	let domain = json!({"hostname": "app.example.com", "status": "pending", "records": [{"kind": "TXT", "name": "_leptoskit-challenge.app.example.com", "value": "leptoskit-verify=abc"}], "verified_at": null, "created_at": time});
	let deployment = json!({"id": id, "project": "notes", "environment": "production", "number": 3, "state": {"state": "queued"}, "source": {"kind": "rollback", "deployment": id}, "rollback_of": 1, "signer": {"by": {"kind": "person", "account": id, "email": "person@example.com"}, "via": "cli", "host": null}, "unknown": false, "created_at": time, "updated_at": time});
	let backup = json!({"id": id, "kind": "manual", "state": "pending", "bytes": null, "failure": null, "created_at": time, "completed_at": null, "expires_at": time});
	let publication = publication_fixture("queued", &Value::Null);
	vec![
		(
			vec!["var", "list", "-e", "staging"],
			"variables",
			json!({"organization": "team", "project": "notes", "environment": "staging"}),
			json!([variable.clone()]),
		),
		(
			vec!["var", "set", "API_URL", "https://api.example.com"],
			"set_variable",
			json!({"organization": "team", "project": "notes", "environment": "production", "name": "API_URL", "value": "https://api.example.com"}),
			variable,
		),
		(
			vec!["var", "unset", "API_URL"],
			"unset_variable",
			json!({"organization": "team", "project": "notes", "environment": "production", "name": "API_URL"}),
			Value::Null,
		),
		(
			vec!["secret", "list"],
			"secrets",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			json!([{"name": "STRIPE_KEY", "version": 2, "updated_at": time}]),
		),
		(
			vec!["secret", "unset", "STRIPE_KEY"],
			"unset_secret",
			json!({"organization": "team", "project": "notes", "environment": "production", "name": "STRIPE_KEY"}),
			Value::Null,
		),
		(
			vec!["domain", "list"],
			"domains",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			json!([domain.clone()]),
		),
		(
			vec!["domain", "add", "app.example.com"],
			"attach_domain",
			json!({"organization": "team", "project": "notes", "environment": "production", "hostname": "app.example.com"}),
			domain.clone(),
		),
		(
			vec!["domain", "verify", "app.example.com"],
			"verify_domain",
			json!({"organization": "team", "project": "notes", "environment": "production", "hostname": "app.example.com"}),
			domain,
		),
		(
			vec![
				"domain",
				"remove",
				"app.example.com",
				"--confirm",
				"app.example.com",
			],
			"detach_domain",
			json!({"organization": "team", "project": "notes", "environment": "production", "hostname": "app.example.com"}),
			Value::Null,
		),
		(
			vec!["deployments", "--limit", "5"],
			"deployments",
			json!({"organization": "team", "project": "notes", "environment": "production", "before": null, "limit": 5}),
			json!([deployment.clone()]),
		),
		(
			vec!["rollback", "1"],
			"rollback",
			json!({"organization": "team", "project": "notes", "environment": "production", "to": 1}),
			deployment,
		),
		(
			vec!["db", "show"],
			"database",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			json!({"name": "notes_production", "size": "shared", "backups": "daily", "ready": true, "created_at": time}),
		),
		(
			vec!["db", "backups"],
			"database_backups",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			json!({"backups": [backup.clone()], "restore": null}),
		),
		(
			vec!["db", "backup"],
			"create_backup",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			backup,
		),
		(
			vec!["db", "restore", id, "--confirm", "notes/production"],
			"restore_backup",
			json!({"organization": "team", "project": "notes", "environment": "production", "backup": id, "confirm": "notes/production"}),
			json!({"backup": id, "state": "pending", "failure": null, "requested_at": time, "updated_at": time}),
		),
		(
			vec!["notifications"],
			"notifications",
			json!({"organization": "team", "project": "notes", "environment": "production"}),
			json!({"tenant": id, "database": "ntf_00000000000000000000000000000001", "state": {"state": "serving"}, "reported_at": time, "providers": ["apns"], "settings": ["LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8"], "project_pushes_this_month": 12, "rotating": false, "created_at": time}),
		),
		(
			vec!["publications", "list"],
			"publications",
			json!({"organization": "team", "project": "notes", "environment": "production", "limit": null}),
			json!([publication.clone()]),
		),
		(
			vec!["publications", "show", id],
			"publication",
			json!({"organization": "team", "project": "notes", "environment": "production", "id": id}),
			publication,
		),
		(
			vec!["usage", "--period", "2026-11"],
			"usage",
			json!({"organization": "team", "period": "2026-11"}),
			json!({"period": "2026-11", "plan": "hobby", "quantities": [{"meter": "build_minutes_linux", "quantity": 3}], "cost": {"kind": "unpriced", "reason": "This cloud has no prices."}, "projects": []}),
		),
	]
}

fn publication_fixture(state: &str, outcome: &Value) -> Value {
	json!({
		"id": "00000000-0000-0000-0000-000000000001",
		"environment": "production",
		"stores": ["play"],
		"submit": false,
		"digest": format!("sha256:{}", "a".repeat(64)),
		"credentials": ["LEPTOSKIT_PLAY_SERVICE_ACCOUNT_JSON"],
		"requirements": [],
		"state": state,
		"outcome": outcome,
		"created_at": "2026-11-18T00:00:00Z",
		"updated_at": "2026-11-18T00:00:00Z",
	})
}

#[test]
fn cloud_commands_send_named_arguments_and_render_results() {
	use clap::Parser as _;
	for (args, operation, expected, response) in
		command_cases().into_iter().chain(environment_cases())
	{
		let environment = command_cases()
			.into_iter()
			.find(|(_, name, ..)| *name == "create_environment")
			.unwrap()
			.3;
		let returned = response.clone();
		let seen = Arc::new(Mutex::new(0));
		let count = Arc::clone(&seen);
		let server = server(move |path, authorization, body| {
			assert_eq!(
				authorization,
				if operation == "members" {
					"Bearer known-api"
				} else {
					"Bearer known-access"
				}
			);
			if operation == "update_environment" && path == "/rpc/environments" {
				return json_response(json!([environment]));
			}
			assert_eq!(path, format!("/rpc/{operation}"));
			assert_eq!(serde_json::from_str::<Value>(body).unwrap(), expected);
			*count.lock().unwrap() += 1;
			json_response(returned.clone())
		});
		let directory = private_directory();
		Store::new(directory.path(), &server.url)
			.save(&session(u64::MAX))
			.unwrap();
		let mut arguments = vec!["lk", "cloud", "--org", "team", "--project", "notes"];
		arguments.extend(args);
		let cli = crate::cli::Cli::try_parse_from(arguments).unwrap();
		let crate::cli::Command::Cloud(args) = cli.command else {
			panic!("cloud command")
		};
		let context = Context {
			client: Client::new(
				&server.url,
				directory.path(),
				(operation == "members").then(|| Secret::new("known-api".into())),
			)
			.unwrap(),
			url: server.url.clone(),
			config: leptoskit_config::cloud::CloudConfig::default(),
			file: None,
		};
		let output = execute(&args, &context).unwrap();
		assert_eq!(output, response);
		assert_eq!(*seen.lock().unwrap(), 1);
		insta::assert_snapshot!(format!("cloud_output_{operation}"), human(&output));
		for secret in ["known-access", "known-refresh", "known-device"] {
			assert!(!human(&output).contains(secret));
			assert!(!format!("{output:?}").contains(secret));
		}
	}
}

#[test]
fn cloud_error_codes_are_actionable_and_never_echo_credentials() {
	let codes = [
		"sign_in_required",
		"person_required",
		"organization_not_found",
		"project_not_found",
		"environment_not_found",
		"insufficient_role",
		"insufficient_scope",
		"stale_revision",
		"confirmation_mismatch",
		"production",
		"slug_taken",
		"name_taken",
		"already_member",
		"last_owner",
		"token_too_strong",
		"project_token",
		"invalid_email",
		"invalid_name",
		"invalid_slug",
		"invalid_detected",
		"invalid_environment",
		"invalid_region",
		"invalid_scale",
		"invalid_lifetime",
		"invalid_scopes",
		"invitation_invalid",
		"invitation_not_found",
		"member_not_found",
		"token_not_found",
		"wrong_email",
		"too_many_invitations",
		"too_many_projects",
		"too_many_environments",
		"token_name_taken",
		"unknown_code",
	];
	let secret = Secret::new("known-access".into());
	let output = codes
		.iter()
		.map(|code| {
			let reply = transport::Reply {
				status: 403,
				body: format!("ServerError|{code}: existing token ci; known-access"),
			};
			let error = reply.result::<Value>(&[&secret]).unwrap_err();
			let rejected = error.downcast_ref::<transport::Rejected>().unwrap();
			assert_eq!(rejected.status, 403);
			assert_eq!(rejected.code, *code);
			assert!(!format!("{error:?}").contains(secret.expose()));
			format!("{code}: {error}")
		})
		.collect::<Vec<_>>()
		.join("\n");
	insta::assert_snapshot!(output);
}

#[test]
fn cloud_person_only_commands_and_destructive_confirmations_fail_before_rpc() {
	use clap::Parser as _;
	let directory = private_directory();
	for command in [
		vec!["token", "list"],
		vec!["org", "create", "Team"],
		vec!["org", "invitations", "team"],
	] {
		let mut args = vec!["lk", "cloud"];
		args.extend(command);
		let cli = crate::cli::Cli::try_parse_from(args).unwrap();
		let crate::cli::Command::Cloud(args) = cli.command else {
			panic!("cloud")
		};
		let context = Context {
			client: Client::new(
				"http://127.0.0.1:1",
				directory.path(),
				Some(Secret::new("known-api".into())),
			)
			.unwrap(),
			url: "http://127.0.0.1:1".into(),
			config: leptoskit_config::cloud::CloudConfig::default(),
			file: None,
		};
		let error = execute(&args, &context).unwrap_err().to_string();
		assert!(error.contains("lk cloud login"));
		assert!(!error.contains("known-api"));
	}
	for args in [
		vec!["lk", "cloud", "project", "delete", "notes"],
		vec!["lk", "cloud", "env", "delete", "staging"],
		vec![
			"lk",
			"cloud",
			"token",
			"revoke",
			"00000000-0000-0000-0000-000000000001",
		],
	] {
		assert!(crate::cli::Cli::try_parse_from(args).is_err());
	}
}

fn parsed(arguments: &[&str]) -> CloudArgs {
	use clap::Parser as _;
	let mut all = vec!["lk", "cloud", "--org", "team", "--project", "notes"];
	all.extend_from_slice(arguments);
	let cli = crate::cli::Cli::try_parse_from(all).unwrap();
	let crate::cli::Command::Cloud(args) = cli.command else {
		panic!("cloud command")
	};
	args
}

fn signed_in(server: &Server, directory: &tempfile::TempDir, file: Option<PathBuf>) -> Context {
	Store::new(directory.path(), &server.url)
		.save(&session(u64::MAX))
		.unwrap();
	Context {
		client: Client::new(&server.url, directory.path(), None).unwrap(),
		url: server.url.clone(),
		config: leptoskit_config::cloud::CloudConfig::default(),
		file,
	}
}

#[test]
fn cloud_secret_values_come_from_a_pipe_or_a_variable_and_are_never_echoed() {
	let calls = Arc::new(Mutex::new(Vec::new()));
	let seen = Arc::clone(&calls);
	let server = server(move |path, _, body| {
		seen.lock()
			.unwrap()
			.push((path.to_owned(), body.to_owned()));
		(
			axum::http::StatusCode::BAD_REQUEST,
			"ServerError|value_too_long: refused known-secret-value".to_owned(),
		)
			.into_response()
	});
	let directory = private_directory();
	let context = signed_in(&server, &directory, None);
	let args = parsed(&["secret", "set", "STRIPE_KEY"]);
	let CloudCommand::Secret { command } = &args.command else {
		panic!("secret")
	};

	// A terminal: nothing is sent, and the error says how to pipe it in.
	let error = ship::secret(command, &args, &context, &mut || {
		Ok(ship::SecretInput::Terminal)
	})
	.unwrap_err()
	.to_string();
	assert!(error.contains("--from-env"), "{error}");
	assert!(calls.lock().unwrap().is_empty());

	// An empty pipe: nothing is sent either.
	assert!(
		ship::secret(command, &args, &context, &mut || {
			Ok(ship::SecretInput::Text(String::new()))
		})
		.is_err()
	);
	assert!(calls.lock().unwrap().is_empty());

	// A piped value is sent once, as `value`, and the refusal that repeats
	// it is redacted.
	let error = ship::secret(command, &args, &context, &mut || {
		Ok(ship::SecretInput::Text("known-secret-value".into()))
	})
	.unwrap_err();
	let calls = calls.lock().unwrap();
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0].0, "/rpc/set_secret");
	assert_eq!(
		serde_json::from_str::<Value>(&calls[0].1).unwrap(),
		json!({"organization": "team", "project": "notes", "environment": "production", "name": "STRIPE_KEY", "value": "known-secret-value"})
	);
	assert!(!format!("{error:?}").contains("known-secret-value"));
	assert!(!error.to_string().contains("known-secret-value"));
}

#[test]
fn cloud_destructive_environment_commands_need_their_confirmation() {
	let directory = private_directory();
	let server = server(|path, _, _| panic!("no call expected, got {path}"));
	let context = signed_in(&server, &directory, None);
	for arguments in [
		vec![
			"domain",
			"remove",
			"app.example.com",
			"--confirm",
			"other.example.com",
		],
		vec![
			"db",
			"restore",
			"00000000-0000-0000-0000-000000000001",
			"--confirm",
			"notes/staging",
		],
	] {
		assert!(execute(&parsed(&arguments), &context).is_err());
	}
	for arguments in [
		vec!["lk", "cloud", "domain", "remove", "app.example.com"],
		vec![
			"lk",
			"cloud",
			"db",
			"restore",
			"00000000-0000-0000-0000-000000000001",
		],
		vec!["lk", "cloud", "publish"],
		vec!["lk", "cloud", "publish", "macos"],
		vec!["lk", "cloud", "publish", "steam"],
	] {
		use clap::Parser as _;
		assert!(crate::cli::Cli::try_parse_from(arguments).is_err());
	}
}

#[test]
fn cloud_publish_uploads_the_project_once_follows_the_log_and_prints_the_report() {
	let project = tempfile::tempdir().unwrap();
	std::fs::write(
		project.path().join("leptoskit.toml"),
		"[app]\nname = \"notes\"\n",
	)
	.unwrap();
	std::fs::create_dir(project.path().join("src")).unwrap();
	std::fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();
	let expected = archive::pack(project.path(), &archive::PackOptions::default()).unwrap();

	let calls = Arc::new(Mutex::new(Vec::<String>::new()));
	let seen = Arc::clone(&calls);
	let origin = Arc::new(Mutex::new(String::new()));
	let host = Arc::clone(&origin);
	let digest = expected.digest.as_str().to_owned();
	let polls = Arc::new(Mutex::new(0));
	let server = server(move |path, authorization, body| {
		seen.lock().unwrap().push(path.to_owned());
		if path == "/upload" {
			// The store gets the bytes, and never the cloud's credential.
			assert_eq!(authorization, "");
			return axum::http::StatusCode::OK.into_response();
		}
		assert_eq!(authorization, "Bearer known-access");
		let body: Value = serde_json::from_str(body).unwrap();
		match path {
			"/rpc/begin_upload" => {
				assert_eq!(body["digest"], digest.as_str());
				json_response(json!({"status": "upload", "ticket": {
					"url": format!("{}/upload", host.lock().unwrap()),
					"method": "PUT",
					"headers": {"x-amz-checksum-sha256": "abc"},
					"expires_at": "2026-11-18T00:00:00Z",
				}}))
			}
			"/rpc/complete_upload" => {
				json_response(
					json!({"digest": digest, "size": 1, "verified_at": "2026-11-18T00:00:00Z", "expires_at": "2026-11-18T00:00:00Z"}),
				)
			}
			"/rpc/publish" => {
				assert_eq!(
					body,
					json!({"organization": "team", "project": "notes", "environment": "production", "archive": digest, "stores": ["play"], "submit": true})
				);
				json_response(publication_fixture("queued", &Value::Null))
			}
			"/rpc/publication" => {
				let mut count = polls.lock().unwrap();
				*count += 1;
				if *count == 1 {
					json_response(publication_fixture("leased", &Value::Null))
				} else {
					json_response(publication_fixture(
						"done",
						&json!({"result": "published", "stores": [{"store": "play", "summary": "uploaded to internal"}], "steps": []}),
					))
				}
			}
			"/rpc/publication_log" => {
				if body["after"].is_null() {
					json_response(
						json!({"lines": [{"n": 0, "at": "2026-11-18T00:00:00Z", "attempt": null, "process": null, "pod": null, "text": "uploading\u{1b}]0;title\u{7}"}], "next": 0, "retention_days": 30}),
					)
				} else {
					json_response(json!({"lines": [], "next": 0, "retention_days": 30}))
				}
			}
			other => panic!("unexpected {other}"),
		}
	});
	*origin.lock().unwrap() = server.url.clone();
	let directory = private_directory();
	let context = signed_in(
		&server,
		&directory,
		Some(project.path().join("leptoskit.toml")),
	);
	let args = parsed(&["--json", "publish", "play", "--submit"]);
	let output = execute(&args, &context).unwrap();
	assert_eq!(output["state"], "done");
	let calls = calls.lock().unwrap();
	assert_eq!(
		calls[..4],
		[
			"/rpc/begin_upload",
			"/upload",
			"/rpc/complete_upload",
			"/rpc/publish"
		]
	);
	assert_eq!(
		calls
			.iter()
			.filter(|call| *call == "/rpc/publication")
			.count(),
		2
	);

	// The report reads as `lk publish`'s own, and terminal controls from the
	// organization's program never reach the screen.
	let report = serde_json::from_value::<leptoskit_cloud::workers::PublishReport>(
		json!({"stores": [{"store": "play", "summary": "uploaded\u{1b}[2J"}], "steps": []}),
	)
	.unwrap();
	let text = stores::report_text(&report);
	assert!(text.starts_with("What to do next"));
	assert!(!text.contains('\u{1b}'));
	assert_eq!(printable("a\u{1b}]0;x\u{7}b\nc"), "a ]0;x b\nc");
}

#[test]
fn cloud_publish_refuses_an_upload_address_that_is_not_secure() {
	let ticket = |url: &str| {
		serde_json::from_value::<leptoskit_cloud::deployments::UploadTicket>(json!({
			"url": url, "method": "PUT", "headers": {}, "expires_at": "2026-11-18T00:00:00Z"
		}))
		.unwrap()
	};
	for refused in [
		"http://127.0.0.1:9/upload",
		"http://store.example/upload",
		"https://user:pass@store.example/upload",
		"file:///etc/passwd",
	] {
		let error = transport::put(&ticket(refused), b"bytes", false).unwrap_err();
		assert!(!format!("{error:?}").contains("store.example"), "{error:?}");
	}
}

#[test]
fn cloud_output_from_the_server_cannot_drive_the_terminal() {
	let shown = human(&json!({
		"name\u{1b}[2J": "value\u{1b}]52;c;aGk=\u{7}",
		"url": "https://store.example/\u{202e}gnp.exe",
	}));
	assert!(!shown.contains('\u{1b}'));
	assert!(!shown.contains('\u{7}'));
	assert!(!shown.contains('\u{202e}'));
	let rejected = transport::Reply {
		status: 400,
		body: "ServerError|some_new_code: look\u{1b}[1A here".into(),
	}
	.result::<Value>(&[])
	.unwrap_err()
	.to_string();
	assert!(!rejected.contains('\u{1b}'), "{rejected}");
}

#[test]
fn cloud_refusals_hide_a_secret_even_when_json_escaped() {
	let secret = Secret::new("pa\"ss\\word".into());
	let error = transport::Reply {
		status: 400,
		body: r#"ServerError|value_not_text: got "pa\"ss\\word""#.into(),
	}
	.result::<Value>(&[&secret])
	.unwrap_err();
	let rejected = error.downcast_ref::<transport::Rejected>().unwrap();
	assert!(
		!format!("{rejected:?}").contains(r#"pa\"ss"#),
		"{rejected:?}"
	);
}

#[test]
fn cloud_env_token_only_goes_to_an_origin_named_outside_the_project() {
	// No token: any origin.
	assert!(check_token_origin(false, None, "https://elsewhere.example").is_ok());
	// A token goes to the hosted cloud, or to an origin the caller named.
	assert!(check_token_origin(true, None, DEFAULT_URL).is_ok());
	assert!(
		check_token_origin(
			true,
			Some("https://self-hosted.example/"),
			"https://self-hosted.example"
		)
		.is_ok()
	);
	// Never to one only leptoskit.toml names.
	let error = check_token_origin(true, None, "https://attacker.example")
		.unwrap_err()
		.to_string();
	assert!(error.contains("LEPTOSKIT_CLOUD_URL"), "{error}");
	assert!(
		check_token_origin(
			true,
			Some("https://self-hosted.example"),
			"https://attacker.example"
		)
		.is_err()
	);
}

/// The tool call, in the child process the tests below start. It lives in
/// `mcp::cloud`'s tests because `call` is that module's.
const CLOUD_TOOL_CALL: &str = "mcp::cloud::tests::subprocess_helper";

/// Runs one cloud tool call, as `leptoskit mcp` would, in a child process
/// with exactly `environment`, in a directory holding `config`. The answer is
/// the tool's. `LEPTOSKIT_CLOUD_TOKEN` is read from the environment of the
/// whole process, which no test can set for itself.
fn tool_call(environment: &[(&str, &str)], config: &str, params: &Value) -> Value {
	let scratch = tempfile::tempdir().unwrap();
	let project = scratch.path().join("project");
	std::fs::create_dir(&project).unwrap();
	std::fs::write(project.join("leptoskit.toml"), config).unwrap();
	let written = scratch.path().join("answer.json");
	let mut command = std::process::Command::new(std::env::current_exe().unwrap());
	command
		.args([
			"--exact",
			CLOUD_TOOL_CALL,
			"--nocapture",
			"--test-threads=1",
		])
		.env_clear()
		.env("LEPTOSKIT_TEST_TOOL_CALL", params.to_string())
		.env("LEPTOSKIT_TEST_TOOL_ANSWER", &written)
		// No stored session, whatever the person running the tests has.
		.env("XDG_CONFIG_HOME", scratch.path())
		.env("PATH", std::env::var_os("PATH").unwrap_or_default())
		.current_dir(&project);

	// Winsock loads its providers relative to SystemRoot on Windows.
	if let Some(system_root) = std::env::var_os("SystemRoot") {
		command.env("SystemRoot", system_root);
	}
	if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
		command.env("LLVM_PROFILE_FILE", profile);
	}
	for (name, value) in environment {
		command.env(name, value);
	}

	let output = command.output().unwrap();
	assert!(
		output.status.success(),
		"the tool call's process failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	serde_json::from_str(&std::fs::read_to_string(written).unwrap()).unwrap()
}

#[test]
fn cloud_tools_do_not_send_the_environment_token_to_a_url_argument() {
	// `LEPTOSKIT_CLOUD_TOKEN` is set, `leptoskit.toml` names an origin, and
	// the call's `url` repeats it. No person named that cloud, so nothing
	// reaches it, and the refusal tells the model the one way to.
	let received = Arc::new(Mutex::new(Vec::new()));
	let seen = Arc::clone(&received);
	let server = server(move |path, authorization, _| {
		seen.lock()
			.unwrap()
			.push((path.to_owned(), authorization.to_owned()));
		axum::http::StatusCode::UNAUTHORIZED.into_response()
	});
	let answer = tool_call(
		&[("LEPTOSKIT_CLOUD_TOKEN", "known-api")],
		&format!("[cloud]\nurl = \"{}\"\n", server.url),
		&json!({"name": "cloud_status", "arguments": {"url": server.url, "org": "team", "project": "notes"}}),
	);
	assert_eq!(*received.lock().unwrap(), []);
	assert_eq!(answer["isError"], true);
	let text = answer["content"][0]["text"].as_str().unwrap();
	assert!(text.contains("LEPTOSKIT_CLOUD_URL"), "{text}");
	assert!(!text.contains("--url"), "{text}");
}

#[test]
fn cloud_tools_send_the_environment_token_where_the_environment_names_a_cloud() {
	// `LEPTOSKIT_CLOUD_URL` names one cloud and the call's `url` another. The
	// token follows the variable, the call succeeds there, and the other
	// origin is never asked for anything.
	let status = json!({"environment": "production", "cluster": null, "hostname": null, "reported_at": null, "processes": []});
	let named = Arc::new(Mutex::new(Vec::new()));
	let called = Arc::new(Mutex::new(Vec::new()));
	let answered = status.clone();
	let chosen = {
		let seen = Arc::clone(&named);
		server(move |path, authorization, _| {
			seen.lock()
				.unwrap()
				.push((path.to_owned(), authorization.to_owned()));
			json_response(answered.clone())
		})
	};
	let argument = {
		let seen = Arc::clone(&called);
		server(move |path, authorization, _| {
			seen.lock()
				.unwrap()
				.push((path.to_owned(), authorization.to_owned()));
			axum::http::StatusCode::UNAUTHORIZED.into_response()
		})
	};
	let answer = tool_call(
		&[
			("LEPTOSKIT_CLOUD_TOKEN", "known-api"),
			("LEPTOSKIT_CLOUD_URL", chosen.url.as_str()),
		],
		"[app]\nname = \"notes\"\n",
		&json!({"name": "cloud_status", "arguments": {"url": argument.url, "org": "team", "project": "notes"}}),
	);
	assert_eq!(
		*named.lock().unwrap(),
		[(
			"/rpc/environment_status".to_owned(),
			"Bearer known-api".to_owned()
		)]
	);
	assert_eq!(*called.lock().unwrap(), []);
	assert_eq!(answer["isError"], false);
	assert_eq!(
		serde_json::from_str::<Value>(answer["content"][0]["text"].as_str().unwrap()).unwrap(),
		status
	);
}

#[test]
fn cloud_publish_refuses_to_pack_a_home_directory() {
	let home = tempfile::tempdir().unwrap();
	std::fs::write(home.path().join("leptoskit.toml"), "[app]\nname = \"x\"\n").unwrap();
	std::fs::create_dir(home.path().join(".ssh")).unwrap();
	let server = server(|path, _, _| panic!("no call expected, got {path}"));
	let directory = private_directory();
	let context = signed_in(
		&server,
		&directory,
		Some(home.path().join("leptoskit.toml")),
	);
	let error = execute(&parsed(&["publish", "play"]), &context)
		.unwrap_err()
		.to_string();
	assert!(error.contains("home or system directory"), "{error}");
}

#[test]
fn notifications_say_what_their_state_means() {
	let none = ship::notifications_text(&Value::Null);
	assert!(none.contains("[services.notifications]"), "{none}");

	let failed = ship::notifications_text(&json!({
		"tenant": "00000000-0000-0000-0000-000000000001",
		"database": "ntf_00000000000000000000000000000001",
		"state": {"state": "failed", "reason": "APNs needs a team ID\u{1b}[2J"},
		"reported_at": "2026-11-25T12:00:00Z",
		"providers": [],
		"settings": [],
		"project_pushes_this_month": 0,
		"rotating": true,
		"created_at": "2026-11-25T12:00:00Z"
	}));
	assert!(
		failed.starts_with("Notifications: failed: APNs needs a team ID"),
		"{failed}"
	);
	assert!(!failed.contains('\u{1b}'), "{failed}");
	assert!(failed.contains("Push providers: none"), "{failed}");
	assert!(failed.contains("Deploy to finish"), "{failed}");

	let serving = ship::notifications_text(&json!({
		"tenant": "00000000-0000-0000-0000-000000000001",
		"database": "ntf_00000000000000000000000000000001",
		"state": {"state": "serving"},
		"reported_at": null,
		"providers": ["apns", "web_push"],
		"settings": [],
		"project_pushes_this_month": 1234,
		"created_at": "2026-11-25T12:00:00Z"
	}));
	assert_eq!(
		serving,
		"Notifications: serving\nPush providers: apns, web_push\nPushes this month (whole \
		 project): 1234\nDatabase: ntf_00000000000000000000000000000001"
	);
}
