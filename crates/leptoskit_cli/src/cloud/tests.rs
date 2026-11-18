use std::fmt::Write as _;
use std::future::IntoFuture as _;
use std::sync::Arc;
use std::sync::Mutex;

use axum::response::IntoResponse as _;
use axum::response::Response;

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
	let answer: Arc<Answer> = Arc::new(answer);
	let thread = std::thread::spawn(move || {
		tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
			let router = axum::Router::new().fallback(move |uri: axum::http::Uri, headers: axum::http::HeaderMap, body: axum::body::Bytes| {
				let answer = Arc::clone(&answer);
				let discovery = discovery.clone();
				async move {
					if uri.path() == "/.well-known/leptoskit-cloud" { return axum::Json(discovery).into_response(); }
					answer(uri.path(), headers.get("authorization").and_then(|value| value.to_str().ok()).unwrap_or_default(), std::str::from_utf8(&body).unwrap())
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
			"/oauth/token" => {
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
			"/oauth/token" => {
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
			.filter(|(path, _)| path == "/oauth/token")
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
		assert_eq!(path, "/oauth/revoke");
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
			if path == "/oauth/device_authorization" {
				assert!(body.contains("client_id=cli"));
				return json_response(
					json!({ "device_code": "known-device", "user_code": "ABCD-EFGH", "verification_uri": format!("{}/device", host.lock().unwrap()), "interval": 1, "expires_in": 60 }),
				);
			}
			assert_eq!(path, "/oauth/token");
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

#[test]
fn cloud_commands_send_named_arguments_and_render_results() {
	use clap::Parser as _;
	let id = "00000000-0000-0000-0000-000000000001";
	let time = "2026-11-18T00:00:00Z";
	let organization = json!({"id": id, "slug": "team", "name": "Team", "plan": "hobby", "personal": false, "role": "owner"});
	let project = json!({"id": id, "slug": "notes", "name": "Notes", "region": "eu-central", "created_at": time});
	let scale =
		json!({"app": {"size": "xs", "min": 0, "max": 1, "sleep": "when-idle"}, "services": {}});
	let environment = json!({"id": id, "name": "staging", "kind": "staging", "scale": scale, "revision": 7, "scale_differs": false, "created_at": time, "updated_at": time});
	let token = json!({"id": id, "name": "ci", "prefix": "lkc_0123456789ab", "scopes": ["read"], "project": null, "expires_at": time, "last_used_at": null, "created_by": id, "created_at": time});
	let invitation = json!({"id": id, "email": "invite@example.com", "role": "viewer", "invited_by": id, "expires_at": time, "created_at": time});
	let cases: Vec<(Vec<&str>, &str, Value, Value)> = vec![
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
	];
	for (args, operation, expected, response) in cases {
		let environment = environment.clone();
		let returned = response.clone();
		let seen = Arc::new(Mutex::new(0));
		let count = Arc::clone(&seen);
		let server = server(move |path, authorization, body| {
			assert_eq!(authorization, "Bearer known-access");
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
