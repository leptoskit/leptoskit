//! The control plane on a real port, over a database the test chooses, and
//! the ways a test calls it: signing people in with emailed codes from the
//! in-memory mailbox, calling operations with a session or a bearer, and
//! inviting people in.

#![allow(
	dead_code,
	reason = "each test binary uses the helpers it needs from this shared module"
)]

pub mod scenarios;

use std::net::SocketAddr;

use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_cloud::server::Control;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;

/// Signing keys for tests only: 32 zero bytes, unpadded base64url, are as
/// good as any here.
fn keys() -> Keys {
	Keys::parse(&format!("test:{}", "A".repeat(43))).expect("test keys")
}

/// A running control plane.
pub struct App {
	pub address: SocketAddr,
	pub origin: String,
	/// The database, as the server sees it: policies apply. Tests read and
	/// arrange rows through [`App::system`].
	pub db: Db,
	pub mailbox: Mailbox,
	pub control: Control,
	client: reqwest::Client,
}

/// Who a call is from.
#[derive(Clone, Copy, Debug)]
pub enum As<'a> {
	Nobody,
	/// A session cookie, `name=value`.
	Session(&'a str),
	/// An `Authorization: Bearer` value: an API token or an access token.
	Bearer(&'a str),
}

/// An answer: its status and its body.
#[derive(Clone, Debug)]
pub struct Answer {
	pub status: u16,
	pub text: String,
}

impl Answer {
	/// The body as JSON, failing the test with the body when it isn't.
	pub fn json(&self) -> Value {
		serde_json::from_str(&self.text)
			.unwrap_or_else(|error| panic!("{error}: {} {}", self.status, self.text))
	}

	/// Fails the test unless the status is `status`.
	#[track_caller]
	pub fn expect(&self, status: u16) -> &Self {
		assert_eq!(self.status, status, "{}", self.text);
		self
	}

	/// The `Kind|message` error's message, without its kind.
	pub fn message(&self) -> &str {
		self.text
			.split_once('|')
			.map_or(self.text.as_str(), |(_, message)| message)
	}

	/// Fails the test unless this is an error with `status` whose message
	/// starts with `code`.
	#[track_caller]
	pub fn refused(&self, status: u16, code: &str) {
		assert_eq!(self.status, status, "{}", self.text);
		assert!(
			self.message().starts_with(code),
			"expected `{code}`: {}",
			self.text
		);
	}
}

/// A control plane over a migrated in-memory SQLite database.
pub async fn app() -> App {
	app_on(leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await).await
}

/// A control plane over `db`, which must be migrated.
pub async fn app_on(db: Db) -> App {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("a free port");
	let address = listener.local_addr().expect("an address");
	let origin = format!("http://{address}");
	let mailbox = Mailbox::new();
	let control = Control::start(
		&db,
		ControlConfig {
			origin: origin.clone(),
			keys: keys(),
			mailer: SharedMailer::new(mailbox.clone()),
			github: None,
			google: None,
			default_region: DEFAULT_REGION.to_owned(),
			dev: None,
		},
	)
	.await
	.expect("the control plane starts");
	let router = control.router();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router)
			.await
			.expect("serve");
	});

	App {
		address,
		origin,
		db,
		mailbox,
		control,
		client: reqwest::Client::new(),
	}
}

impl App {
	/// The database without policies, to arrange and inspect rows.
	pub fn system(&self) -> Db {
		self.db.system("cloud tests")
	}

	/// `POST /rpc/<operation>` with `args`.
	pub async fn call(&self, operation: &str, args: Value, caller: As<'_>) -> Answer {
		let request = self
			.client
			.post(format!("{}/rpc/{operation}", self.origin))
			.header("origin", &self.origin)
			.json(&args);
		self.send(request, caller).await
	}

	/// `GET path`.
	pub async fn get(&self, path: &str, caller: As<'_>) -> Answer {
		let request = self.client.get(format!("{}{path}", self.origin));
		self.send(request, caller).await
	}

	/// `POST path` with a JSON body.
	pub async fn post_json(&self, path: &str, body: Value, caller: As<'_>) -> Answer {
		let request = self
			.client
			.post(format!("{}{path}", self.origin))
			.header("origin", &self.origin)
			.json(&body);
		self.send(request, caller).await
	}

	/// `POST path` with a form, as an OAuth client sends one.
	pub async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Answer {
		let request = self
			.client
			.post(format!("{}{path}", self.origin))
			.form(form);
		self.send(request, As::Nobody).await
	}

	async fn send(&self, request: reqwest::RequestBuilder, caller: As<'_>) -> Answer {
		let request = match caller {
			As::Nobody => request,
			As::Session(cookie) => request.header("cookie", cookie),
			As::Bearer(token) => request.bearer_auth(token),
		};
		let response = request.send().await.expect("the server answers");
		Answer {
			status: response.status().as_u16(),
			text: response.text().await.expect("a body"),
		}
	}

	/// Signs in as `email` with an emailed code, and returns the session
	/// cookie. The address is verified by it.
	pub async fn sign_in(&self, email: &str) -> String {
		let response = self
			.client
			.post(format!("{}/auth/email/send", self.origin))
			.header("origin", &self.origin)
			.json(&json!({ "email": email }))
			.send()
			.await
			.expect("send a code");
		assert_eq!(
			response.status().as_u16(),
			204,
			"{:?}",
			response.text().await
		);
		let mail = self.mailbox.latest_to(email).expect("a code was mailed");
		let code = mail
			.subject
			.rsplit(' ')
			.next()
			.expect("the subject ends with the code")
			.to_owned();
		let response = self
			.client
			.post(format!("{}/auth/email/verify", self.origin))
			.header("origin", &self.origin)
			.json(&json!({ "email": email, "code": code }))
			.send()
			.await
			.expect("verify the code");
		assert_eq!(response.status().as_u16(), 200);
		response
			.headers()
			.get("set-cookie")
			.expect("a session cookie")
			.to_str()
			.expect("ASCII")
			.split(';')
			.next()
			.expect("name=value")
			.to_owned()
	}

	/// The slug of the personal organisation of whoever `caller` is.
	pub async fn personal(&self, caller: As<'_>) -> String {
		let whoami = self.call("whoami", json!({}), caller).await;
		whoami.expect(200);
		whoami.json()["organizations"]
			.as_array()
			.expect("organisations")
			.iter()
			.find(|organization| organization["personal"] == true)
			.expect("a personal organisation")["slug"]
			.as_str()
			.expect("a slug")
			.to_owned()
	}

	/// Invites `email` to `organization` as `role`, and returns the token
	/// from the link the mail carries.
	pub async fn invite(
		&self,
		inviter: As<'_>,
		organization: &str,
		email: &str,
		role: &str,
	) -> String {
		self.call(
			"invite_member",
			json!({ "organization": organization, "email": email, "role": role }),
			inviter,
		)
		.await
		.expect(200);
		self.invitation_token(email)
	}

	/// The token in the newest invitation mailed to `email`.
	pub fn invitation_token(&self, email: &str) -> String {
		let mail = self.mailbox.latest_to(email).expect("an invitation");
		let start = mail
			.text
			.find("/invitations/")
			.expect("the mail has a link")
			+ "/invitations/".len();
		mail.text[start..]
			.chars()
			.take_while(char::is_ascii_hexdigit)
			.collect()
	}

	/// Brings `email` into `organization` as `role`: invited by `owner`,
	/// signed in, and accepted. Returns their session cookie.
	pub async fn join(&self, owner: As<'_>, organization: &str, email: &str, role: &str) -> String {
		let token = self.invite(owner, organization, email, role).await;
		let cookie = self.sign_in(email).await;
		self.call(
			"accept_invitation",
			json!({ "token": token }),
			As::Session(&cookie),
		)
		.await
		.expect(200);
		cookie
	}

	/// The account ID of whoever `caller` is.
	pub async fn account(&self, caller: As<'_>) -> String {
		let whoami = self.call("whoami", json!({}), caller).await;
		whoami.expect(200);
		whoami.json()["account"]
			.as_str()
			.expect("an account")
			.to_owned()
	}

	/// Makes a token in `organization` and returns its value and ID.
	pub async fn token(
		&self,
		caller: As<'_>,
		organization: &str,
		scopes: &[&str],
		project: Option<&str>,
	) -> (String, String) {
		let created = self
			.call(
				"create_token",
				json!({
					"organization": organization,
					"name": "test",
					"scopes": scopes,
					"project": project,
				}),
				caller,
			)
			.await;
		created.expect(200);
		let json = created.json();
		(
			json["value"].as_str().expect("a value").to_owned(),
			json["token"]["id"].as_str().expect("an id").to_owned(),
		)
	}

	/// Makes a project.
	pub async fn project(&self, caller: As<'_>, organization: &str, slug: &str) -> Value {
		let created = self
			.call(
				"create_project",
				json!({ "organization": organization, "slug": slug, "name": slug }),
				caller,
			)
			.await;
		created.expect(200);
		created.json()
	}
}
