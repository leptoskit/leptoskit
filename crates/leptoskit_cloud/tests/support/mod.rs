//! The control plane on a real port, over a database the test chooses, and
//! the ways a test calls it: signing people in with emailed codes from the
//! in-memory mailbox, calling operations with a session or a bearer, and
//! inviting people in.

#![allow(
	dead_code,
	reason = "each test binary uses the helpers it needs from this shared module"
)]

pub mod backups;
pub mod config;
pub mod databases;
pub mod domains;
pub mod review;
pub mod scenarios;
pub mod shipping;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_cloud::server::Control;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::Lookup;
use leptoskit_cloud::server::ResolveError;
use leptoskit_cloud::server::Resolver;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_cloud::server::ShippingSettings;
use leptoskit_cloud::server::StorageConfig;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;

/// Signing keys for tests only: 32 zero bytes, unpadded base64url, are as
/// good as any here.
fn keys() -> Keys {
	Keys::parse(&format!("test:{}", "A".repeat(43))).expect("test keys")
}

/// DNS as a test arranges it: the TXT records each name holds, whether the
/// resolver answers at all, and every name it was asked for.
#[derive(Clone, Debug, Default)]
pub struct Dns {
	records: Arc<Mutex<HashMap<String, Vec<String>>>>,
	down: Arc<AtomicBool>,
	asked: Arc<Mutex<Vec<String>>>,
}

impl Dns {
	/// Adds a TXT record at `name`.
	pub fn publish(&self, name: &str, value: &str) {
		self.records
			.lock()
			.expect("dns")
			.entry(name.to_owned())
			.or_default()
			.push(value.to_owned());
	}

	/// Removes every record at `name`.
	pub fn clear(&self, name: &str) {
		self.records.lock().expect("dns").remove(name);
	}

	/// Whether lookups fail from now on.
	pub fn down(&self, down: bool) {
		self.down.store(down, Ordering::SeqCst);
	}

	/// Every name asked for so far.
	pub fn asked(&self) -> Vec<String> {
		self.asked.lock().expect("dns").clone()
	}
}

impl Resolver for Dns {
	fn txt<'a>(&'a self, name: &'a str) -> Lookup<'a> {
		Box::pin(async move {
			self.asked.lock().expect("dns").push(name.to_owned());

			if self.down.load(Ordering::SeqCst) {
				return Err(ResolveError);
			}

			Ok(self
				.records
				.lock()
				.expect("dns")
				.get(name)
				.cloned()
				.unwrap_or_default())
		})
	}
}

/// A running control plane.
pub struct App {
	pub address: SocketAddr,
	pub origin: String,
	/// The database, as the server sees it: policies apply. Tests read and
	/// arrange rows through [`App::system`].
	pub db: Db,
	pub mailbox: Mailbox,
	/// What the control plane takes for DNS.
	pub dns: Dns,
	pub control: Control,
	client: reqwest::Client,
	/// How many tokens [`App::token`] has made, to name the next.
	tokens: AtomicUsize,
	/// The local object store's directory, removed with the app.
	pub storage: tempfile::TempDir,
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

/// Text to look for in a database: as text, as bytes, and as the hex
/// Postgres prints `bytea` in.
struct Needle<'a> {
	text: &'a str,
	hex: String,
}

impl<'a> Needle<'a> {
	fn new(text: &'a str) -> Self {
		Self {
			text,
			hex: hex::encode(text),
		}
	}

	fn is_in(&self, value: &leptoskit_db::Value) -> bool {
		match value {
			leptoskit_db::Value::Text(text) => text.contains(self.text) || text.contains(&self.hex),
			leptoskit_db::Value::Bytes(bytes) => {
				bytes
					.windows(self.text.len())
					.any(|window| window == self.text.as_bytes())
			}
			leptoskit_db::Value::Json(json) => json.to_string().contains(self.text),
			_ => false,
		}
	}
}

/// Runs `sql` as written, returning its rows' raw values.
async fn raw_rows(db: &Db, sql: String) -> Vec<Vec<leptoskit_db::Value>> {
	leptoskit_db::Statement::new(sql, Vec::new())
		.reads::<leptoskit_cloud::tables::AuditEvent>()
		.fetch(db)
		.await
		.expect("a raw read")
}

/// The one text column of a raw row.
fn only_text(row: &[leptoskit_db::Value]) -> String {
	match row {
		[leptoskit_db::Value::Text(text)] => text.clone(),
		other => panic!("expected one text column, got {other:?}"),
	}
}

/// Every table in the database, quoted for a `FROM`: the app's, and the
/// toolkit's, which Postgres keeps in their own schemas (`"rpc"."receipts"`).
async fn every_table(db: &Db) -> Vec<String> {
	let sql = match leptoskit_db::Read::dialect(db) {
		leptoskit_db::Dialect::Sqlite => "SELECT name FROM sqlite_master WHERE type = 'table'",
		leptoskit_db::Dialect::Postgres => {
			"SELECT table_schema || '\".\"' || table_name FROM information_schema.tables WHERE \
			 table_type = 'BASE TABLE' AND table_schema NOT IN ('pg_catalog', 'information_schema')"
		}
	};

	raw_rows(db, sql.to_owned())
		.await
		.iter()
		.map(|row| format!("\"{}\"", only_text(row)))
		.collect()
}

/// Every row of `table`, a name from [`every_table`]. On Postgres each row
/// is one JSON text, so a `bytea` shows as hex.
async fn every_row(db: &Db, table: &str) -> Vec<Vec<leptoskit_db::Value>> {
	let sql = match leptoskit_db::Read::dialect(db) {
		leptoskit_db::Dialect::Sqlite => format!("SELECT * FROM {table}"),
		leptoskit_db::Dialect::Postgres => format!("SELECT row_to_json(t)::text FROM {table} AS t"),
	};
	raw_rows(db, sql).await
}

/// A control plane over a migrated in-memory SQLite database.
pub async fn app() -> App {
	app_on(leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await).await
}

/// A control plane over `db`, which must be migrated.
pub async fn app_on(db: Db) -> App {
	app_with(db, |_| {}).await
}

/// The domain environments are served under in tests.
pub const APPS_DOMAIN: &str = "apps.example.test";
/// The registry images go to in tests.
pub const REGISTRY: &str = "registry.example.test/leptoskit";

/// Settings for tests: the sweeper runs only when a test calls
/// [`Control::sweep`], after its first run at start.
pub fn test_settings() -> ShippingSettings {
	ShippingSettings {
		sweep: std::time::Duration::from_secs(3600),
		..ShippingSettings::default()
	}
}

/// A control plane over `db`, which must be migrated, with `configure`
/// applied to its configuration: a local store in a fresh directory,
/// [`APPS_DOMAIN`], [`REGISTRY`], no price catalogue, and
/// [`test_settings`].
pub async fn app_with(db: Db, configure: impl FnOnce(&mut ControlConfig)) -> App {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("a free port");
	let address = listener.local_addr().expect("an address");
	let origin = format!("http://{address}");
	let mailbox = Mailbox::new();
	let dns = Dns::default();
	let storage = tempfile::tempdir().expect("a storage directory");
	let mut config = ControlConfig {
		origin: origin.clone(),
		keys: keys(),
		vault: Keys::single("vault-test", [42; 32]),
		mailer: SharedMailer::new(mailbox.clone()),
		resolver: Arc::new(dns.clone()),
		github: None,
		google: None,
		default_region: DEFAULT_REGION.to_owned(),
		storage: StorageConfig::Local {
			directory: storage.path().to_owned(),
		},
		apps_domain: APPS_DOMAIN.to_owned(),
		registry: REGISTRY.to_owned(),
		catalogue: None,
		shipping: test_settings(),
		dev: None,
	};
	configure(&mut config);
	let control = Control::start(&db, config)
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
		dns,
		control,
		client: reqwest::Client::new(),
		tokens: AtomicUsize::new(1),
		storage,
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

	/// Where `needle` appears in the database: every row of every table,
	/// the RPC's receipts, sign-in's and the OAuth provider's tables
	/// included, as a backup or anyone reading the database would see them.
	/// It looks for the text, its bytes, and their hex, as Postgres prints
	/// `bytea`. Returns `table: row` for each hit.
	pub async fn find_in_every_table(&self, needle: &str) -> Vec<String> {
		let db = self.system();
		let needle = Needle::new(needle);
		let mut found = Vec::new();

		for table in every_table(&db).await {
			for row in every_row(&db, &table).await {
				if row.iter().any(|value| needle.is_in(value)) {
					found.push(format!("{table}: {row:?}"));
				}
			}
		}

		found
	}

	/// `POST /rpc/<operation>` with `args` and an `Idempotency-Key`, as a
	/// client retrying a call sends it.
	pub async fn call_with_key(
		&self,
		operation: &str,
		args: Value,
		caller: As<'_>,
		key: &str,
	) -> Answer {
		let request = self
			.client
			.post(format!("{}/rpc/{operation}", self.origin))
			.header("origin", &self.origin)
			.header("idempotency-key", key)
			.json(&args);
		self.send(request, caller).await
	}

	/// `method url` with `headers` and `body`, as a client uses a presigned
	/// address: no cookie, no bearer.
	pub async fn presigned(
		&self,
		method: &str,
		url: &str,
		headers: &Value,
		body: Vec<u8>,
	) -> (u16, Vec<u8>) {
		let method = reqwest::Method::from_bytes(method.as_bytes()).expect("a method");
		let mut request = self.client.request(method, url).body(body);

		// The client sets `content-length` from the body, as a real one does,
		// so a body of the wrong size reaches the store as one.
		if let Some(headers) = headers.as_object() {
			for (name, value) in headers
				.iter()
				.filter(|(name, _)| !name.eq_ignore_ascii_case("content-length"))
			{
				request = request.header(name, value.as_str().expect("a header value"));
			}
		}

		let response = request.send().await.expect("the store answers");
		let status = response.status().as_u16();
		(status, response.bytes().await.expect("a body").to_vec())
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

	/// Makes a token in `organization`, named `test-<n>` so no two share a
	/// name, and returns its value and ID.
	pub async fn token(
		&self,
		caller: As<'_>,
		organization: &str,
		scopes: &[&str],
		project: Option<&str>,
	) -> (String, String) {
		let number = self.tokens.fetch_add(1, Ordering::Relaxed);
		let created = self
			.call(
				"create_token",
				json!({
					"organization": organization,
					"name": format!("test-{number}"),
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

	/// Runs the device flow for `client_id`, approved by the session
	/// `cookie`, and returns the token response.
	pub async fn device_flow(&self, client_id: &str, scope: &str, cookie: &str) -> Value {
		let started = self
			.post_form(
				"/oauth/device_authorization",
				&[("client_id", client_id), ("scope", scope)],
			)
			.await;
		let started = started.expect(200).json();
		let device_code = started["device_code"].as_str().expect("a device code");
		let user_code = started["user_code"].as_str().expect("a user code");
		self.post_json(
			"/oauth/device",
			json!({ "user_code": user_code, "approve": true }),
			As::Session(cookie),
		)
		.await
		.expect(204);
		let tokens = self
			.post_form(
				"/oauth/token",
				&[
					("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
					("device_code", device_code),
					("client_id", client_id),
				],
			)
			.await;
		tokens.expect(200).json()
	}

	/// An access token for the first-party CLI, approved by the session
	/// `cookie`: that person, calling through the CLI.
	pub async fn cli_access(&self, cookie: &str) -> String {
		let discovery = self
			.get(leptoskit_cloud::server::DISCOVERY_PATH, As::Nobody)
			.await;
		let discovery = discovery.expect(200).json();
		let scope = discovery["scope"].as_str().expect("scopes");
		let tokens = self
			.device_flow(self.control.cli_client_id(), scope, cookie)
			.await;
		tokens["access_token"]
			.as_str()
			.expect("an access token")
			.to_owned()
	}
}
