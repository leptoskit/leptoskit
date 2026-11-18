//! The first-party CLI against the real development control plane.
#![cfg(feature = "server")]

use leptoskit_cli::cloud::client::Client;
use leptoskit_cli::publish::secrets::Secret;
use leptoskit_cloud::account;
use leptoskit_cloud::projects;
use leptoskit_cloud::tokens;
use serde_json::json;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;

#[tokio::test(flavor = "multi_thread")]
async fn cloud_cli_login_project_and_api_token() {
	let directory = tempfile::tempdir().unwrap();
	let mut server = tokio::process::Command::new(env!("CARGO_BIN_EXE_leptoskit-cloud-control"))
		.env_clear()
		.env(
			"LEPTOSKIT_CLOUD_DATABASE_URL",
			format!("sqlite://{}", directory.path().join("cloud.db").display()),
		)
		.env("LEPTOSKIT_CLOUD_BIND", "127.0.0.1:0")
		.args(["serve", "--dev"])
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::inherit())
		.kill_on_drop(true)
		.spawn()
		.unwrap();
	let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
	let line = tokio::time::timeout(std::time::Duration::from_secs(120), lines.next_line())
		.await
		.unwrap()
		.unwrap()
		.unwrap();
	let address = line
		.strip_prefix("listening on ")
		.unwrap()
		.split(' ')
		.next()
		.unwrap();
	let origin = format!("http://{address}");
	tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
	let http = reqwest::Client::builder().no_proxy().build().unwrap();
	let signed_in = http
		.post(format!("{origin}/auth/dev/sign-in"))
		.header("origin", &origin)
		.json(&json!({ "email": "cli@example.com" }))
		.send()
		.await
		.unwrap();
	assert!(signed_in.status().is_success());
	let cookie = signed_in.headers()["set-cookie"]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned();
	let credentials = directory.path().join("credentials");
	let (show, mut shown) = tokio::sync::mpsc::unbounded_channel();
	let login_origin = origin.clone();
	let login_credentials = credentials.clone();
	let login = tokio::task::spawn_blocking(move || {
		Client::new(&login_origin, &login_credentials, None)
			.unwrap()
			.login(|verification| {
				show.send(verification.code.clone()).unwrap();
				Ok(())
			})
			.unwrap_or_else(|error| {
				panic!(
					"login rejected: {error:#}; {:?}",
					error.downcast_ref::<leptoskit_cli::cloud::client::Rejected>()
				)
			});
	});
	let code = tokio::time::timeout(std::time::Duration::from_secs(30), shown.recv())
		.await
		.unwrap()
		.unwrap();
	let approved = http
		.post(format!("{origin}/oauth/device"))
		.header("origin", &origin)
		.header("cookie", cookie)
		.json(&json!({ "user_code": code, "approve": true }))
		.send()
		.await
		.unwrap();
	assert_eq!(approved.status().as_u16(), 204);
	login.await.unwrap();
	tokio::task::spawn_blocking(move || {
		let client = Client::new(&origin, &credentials, None).unwrap();
		let me = client.whoami(account::Whoami {}).unwrap();
		assert_eq!(me.via, account::Via::Cli);
		let organization = me
			.organizations
			.into_iter()
			.find(|organization| organization.personal)
			.unwrap()
			.slug;
		let created = client
			.create_project(projects::CreateProject {
				organization: organization.clone(),
				slug: "cli-notes".into(),
				name: "CLI notes".into(),
				region: None,
				detected: projects::Detected::default(),
			})
			.unwrap();
		assert_eq!(created.project.slug, "cli-notes");
		let token = client
			.create_token(tokens::CreateToken {
				organization,
				name: "cli-ci".into(),
				scopes: vec![leptoskit_cloud::access::TokenScope::Read],
				project: Some("cli-notes".into()),
				expires_in_days: Some(30),
			})
			.unwrap();
		let ci = Client::new(&origin, &credentials, Some(Secret::new(token.value))).unwrap();
		assert_eq!(
			ci.whoami(account::Whoami {}).unwrap().via,
			account::Via::Token
		);
	})
	.await
	.unwrap();
}
