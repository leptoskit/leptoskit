//! The console's pages, served beside the control plane on a real port:
//! each route answers with the right status, a signed-out visit is sent to
//! sign in and back, sign-in offers only the methods the server set up,
//! every page carries the hydration loader and no other script, and a
//! terminal signs in by approving its code on `/device`.

use std::net::SocketAddr;

use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_cloud::server::Control;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::DISCOVERY_PATH;
use leptoskit_cloud::server::OAuthApp;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_cloud::server::ShippingSettings;
use leptoskit_cloud::server::StorageConfig;
use leptoskit_cloud_app::routes::Route;
use leptoskit_cloud_app::routes::SignIn;
use leptoskit_cloud_app::server::console;
use leptoskit_router::Routable as _;
use reqwest::redirect::Policy;
use serde_json::Value;
use serde_json::json;

/// Signing keys for tests only.
fn keys() -> Keys {
	Keys::parse(&format!("test:{}", "A".repeat(43))).expect("test keys")
}

/// The control plane and the console on one port, as `serve` runs them.
struct Console {
	origin: String,
	mailbox: Mailbox,
	client: reqwest::Client,
	/// Where the control plane keeps objects; removed when the test ends.
	_storage: tempfile::TempDir,
}

/// An answer: its status, where it redirects, and its body.
#[derive(Debug)]
struct Page {
	status: u16,
	location: Option<String>,
	content_type: Option<String>,
	text: String,
}

impl Page {
	#[track_caller]
	fn expect(&self, status: u16) -> &Self {
		assert_eq!(self.status, status, "{}", self.text);
		self
	}

	#[track_caller]
	fn redirects_to(&self, location: &str) {
		assert!(
			(300..400).contains(&self.status),
			"{} {}",
			self.status,
			self.text
		);
		assert_eq!(self.location.as_deref(), Some(location));
	}

	#[track_caller]
	fn shows(&self, text: &str) -> &Self {
		assert!(
			self.text.contains(text),
			"expected {text:?} in\n{}",
			self.text
		);
		self
	}

	#[track_caller]
	fn hides(&self, text: &str) -> &Self {
		assert!(
			!self.text.contains(text),
			"unexpected {text:?} in\n{}",
			self.text
		);
		self
	}

	fn json(&self) -> Value {
		serde_json::from_str(&self.text)
			.unwrap_or_else(|error| panic!("{error}: {} {}", self.status, self.text))
	}
}

impl Console {
	async fn start(github: bool) -> Self {
		let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
			.await
			.expect("a free port");
		let address: SocketAddr = listener.local_addr().expect("an address");
		let origin = format!("http://{address}");
		let mailbox = Mailbox::new();
		let storage = tempfile::tempdir().expect("a storage directory");
		let control = Control::start(
			&db,
			ControlConfig {
				origin: origin.clone(),
				keys: keys(),
				vault: Keys::single("vault", [9; 32]),
				mailer: SharedMailer::new(mailbox.clone()),
				// No test here verifies a domain: nothing asks it.
				resolver: std::sync::Arc::new(
					leptoskit_cloud::server::DnsOverHttps::new("https://dns.invalid/dns-query")
						.expect("a resolver"),
				),
				github: github.then(|| {
					OAuthApp {
						client_id: "github-client".to_owned(),
						client_secret: "github-secret".to_owned(),
					}
				}),
				google: None,
				default_region: DEFAULT_REGION.to_owned(),
				storage: StorageConfig::Local {
					directory: storage.path().to_owned(),
				},
				apps_domain: "apps.example.test".to_owned(),
				registry: "registry.example.test/cloud".to_owned(),
				catalogue: None,
				payments: None,
				rate_limits: leptoskit_cloud::server::RateLimits::default(),
				client_ip_header: None,
				shipping: ShippingSettings::default(),
				dev: None,
			},
		)
		.await
		.expect("the control plane starts");
		let router = leptoskit_cloud_app::server::router(&control, console::routes(None));
		tokio::spawn(async move {
			leptoskit_axum::axum::serve(listener, router)
				.await
				.expect("serve");
		});

		Self {
			origin,
			mailbox,
			_storage: storage,
			client: reqwest::Client::builder()
				.redirect(Policy::none())
				.build()
				.expect("a client"),
		}
	}

	async fn send(&self, request: reqwest::RequestBuilder) -> Page {
		let response = request.send().await.expect("the server answers");
		let header = |name| {
			response
				.headers()
				.get(name)
				.and_then(|value| value.to_str().ok())
				.map(str::to_owned)
		};
		let location = header(reqwest::header::LOCATION);
		let content_type = header(reqwest::header::CONTENT_TYPE);
		Page {
			status: response.status().as_u16(),
			location,
			content_type,
			text: response.text().await.expect("a body"),
		}
	}

	/// `GET path` as a browser asks for a page, as someone with `cookie`,
	/// or nobody. A redirect is a `302` only for a request that accepts
	/// HTML, as a browser's navigation does.
	async fn get(&self, path: &str, cookie: Option<&str>) -> Page {
		let request = self.client.get(format!("{}{path}", self.origin)).header(
			reqwest::header::ACCEPT,
			"text/html,application/xhtml+xml,*/*;q=0.8",
		);
		let request = match cookie {
			Some(cookie) => request.header("cookie", cookie),
			None => request,
		};
		self.send(request).await
	}

	async fn post_json(&self, path: &str, body: &Value, cookie: Option<&str>) -> Page {
		let request = self
			.client
			.post(format!("{}{path}", self.origin))
			.header("origin", &self.origin)
			.json(body);
		let request = match cookie {
			Some(cookie) => request.header("cookie", cookie),
			None => request,
		};
		self.send(request).await
	}

	async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Page {
		let request = self
			.client
			.post(format!("{}{path}", self.origin))
			.form(form);
		self.send(request).await
	}

	/// Signs in with an emailed code, and returns the session cookie.
	async fn sign_in(&self, email: &str) -> String {
		self.post_json("/auth/email/send", &json!({ "email": email }), None)
			.await
			.expect(204);
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

	/// `POST /rpc/<operation>` with a bearer token.
	async fn call(&self, operation: &str, bearer: &str) -> Page {
		let request = self
			.client
			.post(format!("{}/rpc/{operation}", self.origin))
			.header("origin", &self.origin)
			.bearer_auth(bearer)
			.json(&json!({}));
		self.send(request).await
	}
}

/// Holds the process's one server for a test.
///
/// A page rendered on the server calls its operations in process, through
/// the server `leptoskit_rpc` last installed, and a process has one. Tests
/// that each start a control plane would otherwise render one test's page
/// against another test's database.
async fn one_server() -> tokio::sync::MutexGuard<'static, ()> {
	static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

	ONE.lock().await
}

/// The `<script` elements in `html`, each up to its closing tag.
fn scripts(html: &str) -> Vec<&str> {
	html.match_indices("<script")
		.map(|(start, _)| {
			let rest = &html[start..];
			let end = rest
				.find("</script>")
				.map_or(rest.len(), |end| end + "</script>".len());
			&rest[..end]
		})
		.collect()
}

/// Fails unless every script in `html` is leptos's: the module loader that
/// imports the console's Wasm from its package path and calls `hydrate`,
/// or the data it hands the hydrating page. Nothing loads a script from
/// elsewhere, and no element carries a handler.
#[track_caller]
fn only_the_framework_loader(path: &str, html: &str) {
	let scripts = scripts(html);
	let package = format!("\"{}\", \"{}\"", console::PKG_PATH, console::OUTPUT_NAME);
	let is_loader = |script: &str| script.contains(&package) && script.contains("mod.hydrate()");
	assert_eq!(
		scripts.iter().filter(|script| is_loader(script)).count(),
		1,
		"{path} has one hydration loader:\n{html}"
	);
	assert!(
		html.contains(&format!(
			"<link rel=\"modulepreload\" href=\"/{}/{}.js\"",
			console::PKG_PATH,
			console::OUTPUT_NAME
		)),
		"{path} preloads the console's client:\n{html}"
	);

	for script in &scripts {
		let data = script.contains("__RESOLVED_RESOURCES")
			|| script.contains("__SERIALIZED_ERRORS")
			|| script.contains("__INCOMPLETE_CHUNKS")
			|| script.ends_with("\"></script>");
		assert!(
			is_loader(script) || data,
			"{path} runs a script that isn't leptos's:\n{script}"
		);
		assert!(
			!script.contains(" src="),
			"{path} loads a script:\n{script}"
		);
	}

	for handler in [
		" onclick=",
		" onload=",
		" onerror=",
		" oninput=",
		" onsubmit=",
		" onkeydown=",
	] {
		assert!(
			!html.contains(handler),
			"{path} has an inline `{handler}` handler"
		);
	}
}

/// Fails unless `location` is the sign-in page, coming back to `next`.
#[track_caller]
fn signs_in_and_returns(page: &Page, next: &str) {
	assert!(
		(300..400).contains(&page.status),
		"{} {}",
		page.status,
		page.text
	);
	let location = page.location.as_deref().expect("a location");
	assert_eq!(
		Route::parse(location).expect("a console address"),
		Route::SignIn(SignIn {
			next: Some(next.to_owned()),
			via: None,
			auth: None,
			auth_error: None,
			auth_factors: None,
		}),
		"{location}"
	);
}

#[tokio::test]
async fn signed_out_visits_are_sent_to_sign_in_and_back() {
	let _one = one_server().await;
	let app = Console::start(false).await;

	for path in [
		"/organizations",
		"/account",
		"/acme",
		"/welcome",
		"/",
		"/device?user_code=WDJB-MJHT",
	] {
		signs_in_and_returns(&app.get(path, None).await, path);
	}

	let sign_in = app.get("/sign-in?next=%2Forganizations", None).await;
	sign_in.expect(200).shows("Sign here.");
	only_the_framework_loader("/sign-in", &sign_in.text);
	app.get("/sign-in/email?to=ana%40example.com", None)
		.await
		.expect(200)
		.shows("Check your email.")
		.shows("ana@example.com");
	app.get("/sign-in/email", None)
		.await
		.redirects_to("/sign-in");
}

#[tokio::test]
async fn signed_in_pages_render_with_their_status() {
	let _one = one_server().await;
	let app = Console::start(false).await;
	let ana = app.sign_in("ana@example.com").await;
	let cookie = Some(ana.as_str());

	let welcome = app.get("/welcome", cookie).await;
	welcome.expect(200).shows("Hello, ana.");
	only_the_framework_loader("/welcome", &welcome.text);
	let personal = welcome
		.text
		.split("Your projects live at ")
		.nth(1)
		.and_then(|rest| rest.split('/').nth(1))
		.and_then(|rest| rest.split(['.', '<']).next())
		.expect("the welcome names the organisation")
		.to_owned();

	app.get("/", cookie)
		.await
		.redirects_to(&format!("/{personal}"));
	app.get("/sign-in", cookie).await.redirects_to("/welcome");

	for (path, heading) in [
		("/organizations", "Organisations"),
		("/account", "Account"),
		(&format!("/{personal}"), "Ship your first app"),
		("/device", "Enter the code your terminal shows."),
	] {
		let page = app.get(path, cookie).await;
		page.expect(200).shows(heading);
		only_the_framework_loader(path, &page.text);
	}

	let missing = app.get("/someone-else", cookie).await;
	missing.expect(404).shows("Not here.");
	only_the_framework_loader("/someone-else", &missing.text);
	app.get("/acme/notes/production/42", cookie)
		.await
		.expect(404)
		.shows("Not here.");
}

/// The console's queries in the order `Console::new` makes them, by the
/// start of each scope's name.
const QUERIES_IN_ORDER: &[&str] = &[
	"leptoskit_auth::client::session",
	"leptoskit_auth::client::providers",
	"leptoskit_auth::client::wallet_settings",
	"rpc:whoami",
	"rpc:organizations",
	"leptoskit_idp::client::device_request",
	"leptoskit_auth::client::identities",
	"leptoskit_auth::client::passkeys",
	"leptoskit_auth::client::devices",
	"rpc:projects",
];

/// The answers a page hands the browser: each one's number and its query's
/// scope name, in number order. A failed query's answer doesn't name its
/// query, so it's left out.
fn serialized_queries(html: &str) -> Vec<(u32, String)> {
	let marker = "__RESOLVED_RESOURCES[\"";
	let mut queries: Vec<(u32, String)> = html
		.split(marker)
		.skip(1)
		.filter_map(|answer| {
			let number = answer
				.split('"')
				.next()
				.and_then(|number| number.parse().ok())
				.expect("a numbered answer");
			let name = answer
				.split("scope_name\\\":\\\"")
				.nth(1)?
				.split('\\')
				.next()?;
			Some((number, name.to_owned()))
		})
		.collect();
	queries.sort_unstable();
	queries
}

#[tokio::test]
async fn pages_number_their_queries_in_the_order_the_console_makes_them() {
	let _one = one_server().await;
	// The browser starts every query as the console makes it, and finds each
	// answer by its number, so the server must number them in that order
	// too, or hydration reads the wrong answers and fails.
	let app = Console::start(false).await;
	let ana = app.sign_in("ana@example.com").await;
	let cookie = Some(ana.as_str());
	let discovery = app.get(DISCOVERY_PATH, None).await.expect(200).json();
	let client_id = discovery["cli_client_id"]
		.as_str()
		.expect("a client")
		.to_owned();
	let scope = discovery["scope"].as_str().expect("scopes").to_owned();
	let started = app
		.post_form(
			"/oauth/device_authorization",
			&[("client_id", &client_id), ("scope", &scope)],
		)
		.await
		.expect(200)
		.json();
	let device = format!(
		"/device?user_code={}",
		started["user_code"].as_str().expect("a user code")
	);

	for path in [
		"/welcome",
		"/ana",
		"/organizations",
		"/account",
		device.as_str(),
		"/someone-else",
	] {
		let page = app.get(path, cookie).await;
		let queries = serialized_queries(&page.text);
		let places: Vec<usize> = queries
			.iter()
			.map(|(_, name)| {
				QUERIES_IN_ORDER
					.iter()
					.position(|query| name.starts_with(query))
					.unwrap_or_else(|| panic!("{path} hands over an unknown query {name}"))
			})
			.collect();

		assert!(
			queries.iter().any(|(_, name)| name == "rpc:whoami"),
			"{path} hands over whoami: {queries:?}"
		);
		assert!(
			places.is_sorted_by(|earlier, later| earlier < later),
			"{path} numbers its queries out of the console's order: {queries:?}"
		);

		// Away from an organisation's pages the projects wait on `whoami`,
		// which the server can't promise to wait for, so only the live page
		// asks for them, and the server's page never has them. (The
		// `/someone-else` answer is a failure, which names no query.)
		assert_eq!(
			queries.iter().any(|(_, name)| name == "rpc:projects"),
			path == "/ana",
			"{path}: {queries:?}"
		);
	}
}

#[tokio::test]
async fn sign_in_offers_only_the_methods_the_server_set_up() {
	let _one = one_server().await;
	let without = Console::start(false).await;
	without
		.get("/sign-in", None)
		.await
		.expect(200)
		.shows("Email me a code")
		.hides("Continue with GitHub")
		.hides("Google");

	let with = Console::start(true).await;
	let page = with.get("/sign-in", None).await;
	page.expect(200)
		.shows("Continue with GitHub")
		.shows("Email me a code")
		.hides("Google")
		.hides("github-secret");
	let github = page.text.find("Continue with GitHub").expect("GitHub");
	let email = page.text.find("Email me a code").expect("email");
	assert!(github < email, "GitHub comes first");
}

#[tokio::test]
async fn the_faces_are_served_and_kept() {
	let _one = one_server().await;
	let app = Console::start(false).await;
	let font = app.get("/_leptoskit/fonts/skribble-mono.woff2", None).await;
	font.expect(200);
	assert_eq!(font.content_type.as_deref(), Some("font/woff2"));
	app.get("/_leptoskit/fonts/skribble-sketch.woff2", None)
		.await
		.expect(200);
}

#[tokio::test]
async fn a_terminal_signs_in_by_approving_its_code_on_the_device_page() {
	let _one = one_server().await;
	let app = Console::start(false).await;
	let discovery = app.get(DISCOVERY_PATH, None).await;
	let discovery = discovery.expect(200).json();
	let client_id = discovery["cli_client_id"]
		.as_str()
		.expect("a client")
		.to_owned();
	let scope = discovery["scope"].as_str().expect("scopes").to_owned();

	// The CLI starts the flow and shows its code.
	let started = app
		.post_form(
			"/oauth/device_authorization",
			&[("client_id", &client_id), ("scope", &scope)],
		)
		.await;
	let started = started.expect(200).json();
	let device_code = started["device_code"]
		.as_str()
		.expect("a device code")
		.to_owned();
	let user_code = started["user_code"]
		.as_str()
		.expect("a user code")
		.to_owned();
	let link = started["verification_uri_complete"]
		.as_str()
		.expect("a link with the code");
	let path = link
		.strip_prefix(&app.origin)
		.expect("a link to the console");

	// Signed out, the link goes to sign in and comes back.
	signs_in_and_returns(&app.get(path, None).await, path);

	// Signed in, the page asks about this code, and names the terminal.
	let ana = app.sign_in("ana@example.com").await;
	let page = app.get(path, Some(&ana)).await;
	page.expect(200)
		.shows("Does your terminal show this code?")
		.shows(&user_code)
		.shows("Approve this terminal")
		.shows("Not me, deny")
		.shows("LeptosKit CLI");
	only_the_framework_loader(path, &page.text);

	// The page's Approve sends this, from the same session.
	app.post_json(
		"/oauth/device",
		&json!({ "user_code": user_code, "approve": true }),
		Some(&ana),
	)
	.await
	.expect(204);

	// The CLI exchanges its device code, and is Ana, from the CLI.
	let tokens = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
				("device_code", &device_code),
				("client_id", &client_id),
			],
		)
		.await;
	let tokens = tokens.expect(200).json();
	let access = tokens["access_token"].as_str().expect("an access token");
	let whoami = app.call("whoami", access).await;
	let whoami = whoami.expect(200).json();
	assert_eq!(whoami["via"], "cli");
	assert_eq!(whoami["email"], "ana@example.com");

	// The code is spent: the page says so instead of asking again.
	app.get(path, Some(&ana))
		.await
		.expect(200)
		.shows("This code has expired or was already used.")
		.hides("Approve this terminal");
}
