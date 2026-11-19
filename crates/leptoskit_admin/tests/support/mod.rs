//! A main site and an admin, in one process: the main site runs
//! `leptoskit_auth` and `leptoskit_idp` with two people, one an admin; the
//! admin signs in through it, and acts on a control service over a
//! temporary project.

#![allow(dead_code, reason = "each test file uses part of the world")]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use http_body_util::BodyExt as _;
use leptoskit_admin::Admin;
use leptoskit_admin::MainSite;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext as _;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::ProviderFuture;
use leptoskit_auth::ProviderHttp;
use leptoskit_auth::ProviderRequest;
use leptoskit_auth::ProviderResponse;
use leptoskit_auth::Result as AuthResult;
use leptoskit_auth::Roles;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;

pub const NOTES: &str = "https://notes.example";
pub const ADMIN: &str = "https://admin.example";
pub const BASE: &str = leptoskit_admin::BASE;
const OWNER: &str = "test-only-management-owner-key-0123456789";

pub const CONFIG: &str = r#"[app]
name = "notes"
render = "ssr"

[db]
dialect = "postgres"

[infra]
name = "notes-production"
domain = "app.example.com"
location = "hel1"
server_type = "cx23"
instances = 2
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates.example.com"
updates_bucket = "notes-releases"
backup_bucket = "notes-backups"
image = "ghcr.io/team/notes"
registry = "ghcr.io"
registry_username = "team"

[infra.services]
relay = true

[infra.backups]
enabled = true
daily = 7

[rpc.live]
relay_url_env = "NATS_URL"
"#;

/// The main site's people, and who among them keeps the place.
#[table(name = "users")]
#[policy(all, |user, cx| cx.signed_in(|me| user.id.eq(me.account)))]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	#[db(max_len = 254)]
	pub name: String,
	pub admin: bool,
}

impl OnSignUp for User {
	async fn on_sign_up(tx: &Tx, account: Id<Account>, profile: &Profile) -> AuthResult<()> {
		let name = profile
			.email
			.as_ref()
			.map(|email| email.to_string().replace("@example.com", ""))
			.unwrap_or_default();
		NewUser {
			id: account,
			name,
			admin: false,
		}
		.insert(tx)
		.await?;
		Ok(())
	}
}

impl Roles for User {
	async fn roles(db: &Db, account: Id<Account>) -> AuthResult<Vec<String>> {
		match User::find(db, account).await? {
			Some(user) if user.admin => Ok(vec!["admin".to_owned()]),
			_ => Ok(Vec::new()),
		}
	}
}

/// The main site's real routes, answering the admin's requests to it.
#[derive(Clone)]
struct InProcess(Router);

impl ProviderHttp for InProcess {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		let router = self.0.clone();
		Box::pin(async move {
			let url = url::Url::parse(&request.url).map_err(|error| error.to_string())?;
			let path = match url.query() {
				Some(query) => format!("{}?{query}", url.path()),
				None => url.path().to_owned(),
			};
			let mut builder = if request.method == "POST" {
				Request::post(path)
			} else {
				Request::get(path)
			};

			for (name, value) in &request.headers {
				builder = builder.header(name.as_str(), value.as_str());
			}

			let response = router
				.oneshot(
					builder
						.body(Body::from(request.body.unwrap_or_default()))
						.unwrap(),
				)
				.await
				.unwrap();
			let status = response.status().as_u16();
			let body = response.into_body().collect().await.unwrap().to_bytes();
			Ok(ProviderResponse {
				status,
				body: body.to_vec(),
			})
		})
	}
}

/// A browser with a cookie jar per site.
#[derive(Default)]
pub struct Browser {
	jars: HashMap<String, HashMap<String, String>>,
}

impl Browser {
	pub async fn send(
		&mut self,
		router: &Router,
		origin: &str,
		request: axum::http::request::Builder,
		body: Body,
	) -> Response {
		let jar = self.jars.entry(origin.to_owned()).or_default();
		let cookies = jar
			.iter()
			.map(|(name, value)| format!("{name}={value}"))
			.collect::<Vec<_>>()
			.join("; ");
		let request = if cookies.is_empty() {
			request
		} else {
			request.header(header::COOKIE, cookies)
		};
		let response = router
			.clone()
			.oneshot(request.body(body).unwrap())
			.await
			.unwrap();

		for value in response.headers().get_all(header::SET_COOKIE) {
			let value = value.to_str().unwrap();
			let (pair, attributes) = value.split_once(';').unwrap_or((value, ""));
			let (name, cookie) = pair.split_once('=').unwrap();

			if cookie.is_empty() || attributes.contains("Max-Age=0") {
				jar.remove(name);
			} else {
				jar.insert(name.to_owned(), cookie.to_owned());
			}
		}

		response
	}

	pub async fn get(&mut self, router: &Router, origin: &str, path: &str) -> Response {
		self.send(router, origin, Request::get(path), Body::empty())
			.await
	}

	pub async fn post_json(
		&mut self,
		router: &Router,
		origin: &str,
		path: &str,
		body: Value,
	) -> Response {
		let request = Request::post(path)
			.header(header::ORIGIN, origin)
			.header(header::CONTENT_TYPE, "application/json");
		self.send(router, origin, request, Body::from(body.to_string()))
			.await
	}
}

pub fn location(response: &Response) -> String {
	assert!(response.status().is_redirection(), "{response:?}");
	response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned()
}

pub async fn text(response: Response) -> String {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec()).unwrap()
}

pub async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Both sites, the control service, and the project.
pub struct World {
	pub notes: Router,
	pub notes_db: Db,
	mailbox: Mailbox,
	pub admin: Router,
	pub admin_db: Db,
	pub store: Store,
	pub service: Service,
	pub project: PathBuf,
	_root: tempfile::TempDir,
}

pub async fn world() -> World {
	world_with(None).await
}

/// Both sites, with the admin reading `analytics` when one is given.
pub async fn world_with(analytics: Option<leptoskit_admin::AnalyticsService>) -> World {
	world_with_options(analytics, false).await
}

/// Both sites, and whether the admin says its infrastructure is simulated.
pub async fn world_with_options(
	analytics: Option<leptoskit_admin::AnalyticsService>,
	simulated: bool,
) -> World {
	let notes_db = Db::connect("sqlite::memory:").await.unwrap();
	let admin_db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&notes_db)
		.await
		.unwrap();
	leptoskit_db::testing::create_tables(&admin_db)
		.await
		.unwrap();

	let mailbox = Mailbox::new();
	let notes_auth = Auth::builder(&notes_db, Keys::single("notes", [5; 32]))
		.app_name("Notes")
		.origin(NOTES)
		.mailer(mailbox.clone())
		.on_sign_up::<User>()
		.roles::<User>()
		.build()
		.unwrap();
	let idp = Idp::builder(&notes_db, &notes_auth)
		.sign_in_page("/sign-in")
		.consent_page("/consent")
		.build()
		.unwrap();
	let notes = Router::new()
		.merge(notes_auth.router())
		.merge(idp.router())
		.layer(notes_auth.layer())
		.layer(idp.layer());

	let output = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Notes admin".into(),
			redirect_uris: vec![format!("{ADMIN}/auth/callback/main")],
			post_logout_redirect_uris: vec![format!("{ADMIN}{BASE}/sign-in")],
			scopes: ["openid", "email", "profile", "roles"]
				.map(str::to_owned)
				.to_vec(),
			auth_method: ClientAuth::SecretPost,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
			dpop_bound: false,
		}))
		.await;
	let ClientCommandOutput::Added { client, secret } = output else {
		panic!("the admin should register: {output:?}");
	};

	let root = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let project = root.path().join("notes");
	std::fs::create_dir_all(&project).unwrap();
	std::fs::write(project.join("leptoskit.toml"), CONFIG).unwrap();
	let store = Store::open(&root.path().join("console.sqlite3")).unwrap();
	store
		.register(&Project {
			id: "notes-production".into(),
			environment: "production".into(),
			root: project.canonicalize().unwrap(),
			revision: 0,
		})
		.unwrap();
	let service = Service::new(store.clone(), OWNER, "http://localhost:4700").unwrap();

	let mut admin = Admin::builder("Notes")
		.origin(ADMIN)
		.main_site(MainSite::new(NOTES, client.client_id, secret.unwrap()))
		.keys(Keys::single("admin", [7; 32]))
		.database(admin_db.clone())
		.control(service.clone())
		.simulated_infrastructure(simulated)
		.provider_http(InProcess(notes.clone()));

	if let Some(analytics) = analytics {
		admin = admin.analytics(analytics);
	}

	let admin = admin.build().await.unwrap();

	World {
		notes,
		notes_db,
		mailbox,
		admin: admin.router(),
		admin_db,
		store,
		service,
		project,
		_root: root,
	}
}

impl World {
	/// Signs someone in to the main site with an emailed code.
	pub async fn notes_sign_in(&self, browser: &mut Browser, address: &str) {
		let sent = browser
			.post_json(
				&self.notes,
				NOTES,
				"/auth/email/send",
				json!({ "email": address }),
			)
			.await;
		assert_eq!(sent.status(), StatusCode::NO_CONTENT);
		let mail = self.mailbox.latest_to(address).unwrap();
		let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
		let verified = browser
			.post_json(
				&self.notes,
				NOTES,
				"/auth/email/verify",
				json!({ "email": address, "code": code }),
			)
			.await;
		assert_eq!(verified.status(), StatusCode::OK);
	}

	/// Makes someone an admin at the main site.
	pub async fn promote(&self, name: &str) {
		let system = self.notes_db.system("test: promote");
		let user = User::query()
			.filter(|user| user.name.eq(name.to_owned()))
			.first(&system)
			.await
			.unwrap()
			.unwrap();
		User::update()
			.filter(|row| row.id.eq(user.id))
			.set(|row| row.admin, true)
			.run(&system)
			.await
			.unwrap();
	}

	/// Presses **Continue with Notes**, signs in on the main site (already
	/// signed in there), and follows the browser back. Returns where the
	/// admin sent it last.
	pub async fn continue_with_notes(&self, browser: &mut Browser) -> String {
		let started = browser
			.post_json(
				&self.admin,
				ADMIN,
				"/auth/oauth/start",
				json!({ "provider": "main", "redirect_to": format!("{BASE}/sign-in") }),
			)
			.await;
		assert_eq!(started.status(), StatusCode::OK);
		let url = json_body(started).await["url"].as_str().unwrap().to_owned();
		let back = location(&browser.get(&self.notes, NOTES, &url).await);
		assert!(
			back.starts_with(&format!("{ADMIN}/auth/callback/main")),
			"{back}"
		);
		location(&browser.get(&self.admin, ADMIN, &back).await)
	}

	/// Ada, an admin at the main site, signed in to the admin.
	pub async fn ada(&self) -> Browser {
		let mut ada = Browser::default();
		self.notes_sign_in(&mut ada, "ada@example.com").await;
		self.promote("ada").await;
		let to = self.continue_with_notes(&mut ada).await;
		assert!(to.contains("auth=signed_in"), "{to}");
		ada
	}
}

/// The page as an outline: each element with a test id, role, or label,
/// and every piece of text, indented by nesting. Styles and scripts are
/// left out, so a snapshot shows what's on the screen and how it's named.
pub fn outline(html: &str) -> String {
	let body = html.split_once("<body>").map_or(html, |(_, body)| body);
	let mut out = String::new();
	let mut stack: Vec<bool> = Vec::new();
	let mut rest = body;
	let depth = |stack: &[bool]| stack.iter().filter(|printed| **printed).count();

	while !rest.is_empty() {
		if let Some(after) = rest.strip_prefix("<!--") {
			rest = after.split_once("-->").map_or("", |(_, after)| after);
			continue;
		}

		if let Some(after) = rest.strip_prefix('<') {
			let (tag, after) = after.split_once('>').unwrap_or((after, ""));
			rest = after;

			if let Some(closing) = tag.strip_prefix('/') {
				if closing.trim() != "body" && closing.trim() != "html" {
					stack.pop();
				}
				continue;
			}

			let name = tag.split_whitespace().next().unwrap_or("");

			if name == "style" || name == "script" {
				rest = rest
					.split_once(&format!("</{name}>"))
					.map_or("", |(_, after)| after);
				continue;
			}

			if name == "svg" {
				rest = rest.split_once("</svg>").map_or("", |(_, after)| after);
				out.push_str(&"  ".repeat(depth(&stack)));
				out.push_str("<vector>\n");
				continue;
			}

			let attribute = |key: &str| {
				let needle = format!(" {key}=\"");
				tag.split_once(&needle)
					.map(|(_, value)| value.split('"').next().unwrap_or("").to_owned())
			};
			let mut parts = Vec::new();

			if let Some(id) = attribute("data-testid") {
				parts.push(format!("[{id}]"));
			}

			if let Some(role) = attribute("role") {
				parts.push(format!("role={role}"));
			}

			if let Some(label) = attribute("aria-label") {
				parts.push(format!("label={label:?}"));
			}

			if let Some(href) = attribute("href") {
				parts.push(format!("href={href}"));
			}

			if let Some(current) = attribute("aria-current") {
				parts.push(format!("current={current}"));
			}

			let void = tag.ends_with('/')
				|| name.starts_with('!')
				|| matches!(name, "input" | "br" | "img" | "meta" | "link" | "hr");

			if name == "input"
				&& let Some(placeholder) = attribute("placeholder")
			{
				parts.push(format!("placeholder={placeholder:?}"));
			}

			let printed = !parts.is_empty();

			if printed {
				out.push_str(&"  ".repeat(depth(&stack)));
				let _ = writeln!(out, "{name} {}", parts.join(" "));
			}

			if !void {
				stack.push(printed);
			}

			continue;
		}

		let (text, after) = rest
			.split_once('<')
			.map_or((rest, ""), |(text, after)| (text, after));
		rest = if after.is_empty() && !rest.contains('<') {
			""
		} else {
			&rest[text.len()..]
		};
		let text = text
			.replace("&amp;", "&")
			.replace("&lt;", "<")
			.replace("&gt;", ">")
			.replace("&quot;", "\"")
			.replace("&#39;", "'")
			.replace("&#x27;", "'");
		let text = text.trim();

		if !text.is_empty() {
			out.push_str(&"  ".repeat(depth(&stack)));
			let _ = writeln!(out, "{text:?}");
		}
	}

	out
}

/// Only the top-level blocks of an outline whose first line names one of
/// `test_ids`, such as `main` and `drawer-aside`: the shell around a
/// screen is snapshotted once, not on every page.
pub fn regions(outline: &str, test_ids: &[&str]) -> String {
	let mut out = String::new();
	let mut keeping = false;

	for line in outline.lines() {
		if !line.starts_with(' ') {
			keeping = test_ids.iter().any(|id| line.contains(&format!("[{id}]")));
		}

		if keeping {
			out.push_str(line);
			out.push('\n');
		}
	}

	out
}

/// The token the analytics service knows the app by.
pub const ANALYTICS_TOKEN: &str = "test-only-analytics-token-0123456789";

/// Each request the service received: its path and `Authorization` header.
pub type Authorizations = std::sync::Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

/// The real analytics service on a local port, with every request's
/// `Authorization` header written down.
pub struct AnalyticsFixture {
	pub url: String,
	pub authorizations: Authorizations,
	_data: tempfile::TempDir,
}

/// Starts the service, migrated and listening, its live stream running.
pub async fn analytics_service() -> AnalyticsFixture {
	use leptoskit_analytics::service;
	use leptoskit_rpc::server::APP_CALLER;
	use leptoskit_rpc::server::Anonymous;
	use leptoskit_rpc::server::ServiceCallers;

	let data = tempfile::tempdir().unwrap();
	let db = Db::connect(&format!(
		"sqlite://{}",
		data.path().join("analytics.sqlite3").display()
	))
	.await
	.unwrap();
	db.migrate(service::MIGRATIONS).await.unwrap();
	let (router, live) = service::routes_with(
		db,
		Some(NOTES),
		service::IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller(ANALYTICS_TOKEN, APP_CALLER),
	);
	live.start().await.unwrap();
	let authorizations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
	let seen = authorizations.clone();
	let router = router.layer(axum::middleware::from_fn(
		move |request: axum::extract::Request, next: axum::middleware::Next| {
			let seen = seen.clone();
			async move {
				let authorization = request
					.headers()
					.get(header::AUTHORIZATION)
					.and_then(|value| value.to_str().ok())
					.map(str::to_owned);
				seen.lock()
					.unwrap()
					.push((request.uri().path().to_owned(), authorization));
				next.run(request).await
			}
		},
	));
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

	AnalyticsFixture {
		url,
		authorizations,
		_data: data,
	}
}

impl AnalyticsFixture {
	/// The admin's connection to it, signed with `token`.
	pub fn connection(&self, token: &str) -> leptoskit_admin::AnalyticsService {
		leptoskit_admin::AnalyticsService::new(self.url.clone(), token)
	}

	fn app(&self) -> leptoskit_rpc::client::AppCaller {
		leptoskit_rpc::client::AppCaller::new(
			leptoskit_analytics::SERVICE,
			self.url.clone(),
			ANALYTICS_TOKEN,
		)
	}

	/// Registers typed events' fields, as the app's server does.
	pub async fn register(&self, schemas: &[(&str, &[(&str, &str)])]) {
		let schemas = schemas
			.iter()
			.map(|(name, fields)| {
				leptoskit_analytics::service::catalogue::SchemaInput {
					schema_id: leptoskit_analytics::schema_id(name, fields),
					name: (*name).to_owned(),
					fields: fields
						.iter()
						.map(|(field, kind)| ((*field).to_owned(), (*kind).to_owned()))
						.collect(),
				}
			})
			.collect();
		self.app()
			.call(leptoskit_analytics::service::RegisterEvents { schemas })
			.await
			.unwrap();
	}

	/// Captures events through the service's own capture action, as the
	/// app, and waits for them to be stored.
	pub async fn capture(&self, batch: Vec<leptoskit_analytics::service::JsonEvent>) {
		let result = self
			.app()
			.call(leptoskit_analytics::service::Capture {
				sent_at: None,
				batch,
			})
			.await
			.unwrap();
		assert!(result.refused.is_empty(), "{:?}", result.refused);
	}

	/// The `Authorization` headers the service saw on its rpc routes.
	pub fn rpc_authorizations(&self) -> Vec<Option<String>> {
		self.authorizations
			.lock()
			.unwrap()
			.iter()
			.filter(|(path, _)| path.contains("/rpc/"))
			.map(|(_, authorization)| authorization.clone())
			.collect()
	}
}
