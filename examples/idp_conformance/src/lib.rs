//! A test server for the OpenID Foundation's conformance suite.
//!
//! The suite signs in through a scripted browser that fills pages by
//! selector, and emailed codes can't be scripted. So this server is
//! `leptoskit_idp` as an app would set it up, plus two plain HTML pages the
//! browser can drive: a sign-in page with one button that signs in a seeded
//! person, and a consent page with an Allow button. The sign-in page uses
//! `leptoskit_auth`'s development sign-in, which exists only in debug
//! builds, so a release build of this server can't sign anyone in.
//!
//! [`server`] registers the apps the suite signs in as, each allowed to sign
//! in without PKCE (the suite's Basic OP plan sends none), and returns the
//! suite's configuration for them, browser script included.

use std::fmt::Write as _;

use anyhow::Context as _;
use axum::Extension;
use axum::Form;
use axum::Router;
use axum::body::Body;
use axum::extract::Query;
use axum::http::HeaderValue;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use axum::routing::get;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext as _;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_idp::ClaimSet;
use leptoskit_idp::Claims;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::GrantedScopes;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;

/// The one person the sign-in page signs in.
pub const SEEDED: &str = "tester@example.com";

/// The seeded person's name, as the `profile` scope releases it.
const NAME: [&str; 2] = ["Conformance", "Tester"];

/// The seeded person's other `profile` claims.
const PROFILE: [(&str, &str); 8] = [
	("middle_name", "Q"),
	("nickname", "Tess"),
	("profile", "https://example.com/tester"),
	("website", "https://example.com"),
	("gender", "unspecified"),
	("birthdate", "2000-01-01"),
	("zoneinfo", "Europe/London"),
	("locale", "en-GB"),
];

/// Where the server runs and who tests it.
#[derive(Clone, Debug)]
pub struct Settings {
	/// The issuer, an `https` origin the suite's containers reach, such as
	/// `https://idp.test:9443`.
	pub issuer: String,
	/// The suite's own address, such as
	/// `https://localhost.emobix.co.uk:8443`: apps are sent back to its
	/// callback.
	pub suite: String,
	/// The suite's alias for this server, part of its callback address.
	pub alias: String,
	/// The RSA private key, as PEM, that signs RS256 ID tokens, which the
	/// OpenID Connect plans expect.
	pub rsa_key: String,
}

/// The server, and the suite's configuration for it.
pub struct Server {
	/// Every route: the provider's, sign-in's, and the two HTML pages.
	pub router: Router,
	/// The suite's configuration, for `run-test-plan.py`.
	pub suite_config: Value,
}

/// The seeded person's table.
#[table(name = "users")]
#[policy(all, |user, cx| cx.signed_in(|me| user.id.eq(me.account)))]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	#[db(max_len = 100)]
	pub given_name: String,
	#[db(max_len = 100)]
	pub family_name: String,
}

impl OnSignUp for User {
	async fn on_sign_up(tx: &Tx, account: Id<Account>, _: &Profile) -> leptoskit_auth::Result<()> {
		NewUser {
			id: account,
			given_name: NAME[0].to_owned(),
			family_name: NAME[1].to_owned(),
		}
		.insert(tx)
		.await?;
		Ok(())
	}
}

impl Claims for User {
	async fn claims(
		db: &Db,
		account: Id<Account>,
		scopes: &GrantedScopes,
		out: &mut ClaimSet,
	) -> leptoskit_idp::Result<()> {
		if scopes.has("profile")
			&& let Some(user) = User::find(db, account).await?
		{
			out.name(&format!("{} {}", user.given_name, user.family_name))
				.insert("given_name", user.given_name.as_str())
				.insert("family_name", user.family_name.as_str())
				.preferred_username("tester")
				.picture("https://example.com/tester.png");
			// The rest of the `profile` claims, which the suite looks for.
			for (name, value) in PROFILE {
				out.insert(name, value);
			}
			out.insert("updated_at", 1_700_000_000);
		}
		Ok(())
	}
}

/// The server over a fresh in-memory database, with the suite's apps
/// registered.
///
/// # Errors
///
/// When the issuer, the suite's address or the RSA key is unusable, or
/// the database fails.
pub async fn server(settings: &Settings) -> anyhow::Result<Server> {
	let db = Db::connect("sqlite::memory:").await?;
	leptoskit_db::testing::create_tables(&db).await?;
	let mut key = [0; 32];
	getrandom::fill(&mut key).context("no randomness for the server key")?;
	let auth = auth(&db, &settings.issuer, key)?;
	let idp = Idp::builder(&db, &auth)
		.issuer(&settings.issuer)
		.claims::<User>()
		.rsa_key(settings.rsa_key.as_bytes())
		.sign_in_page("/sign-in")
		.consent_page("/consent")
		.build()?;
	let suite_config = register(&idp, settings).await?;

	let router = Router::new()
		.route("/sign-in", get(sign_in_page).post(sign_in))
		.route("/consent", get(consent_page).post(consent))
		.layer(Extension(idp.clone()))
		.layer(Extension(auth.clone()))
		.merge(auth.router())
		.merge(idp.router())
		.layer(auth.layer())
		.layer(idp.layer());

	Ok(Server {
		router,
		suite_config,
	})
}

#[cfg(debug_assertions)]
fn auth(db: &Db, issuer: &str, key: [u8; 32]) -> anyhow::Result<Auth> {
	Ok(Auth::builder(db, Keys::single("conformance", key))
		.app_name("leptoskit conformance")
		.origin(issuer)
		.mailer(Mailbox::new())
		.on_sign_up::<User>()
		.dev_sign_in()
		.build()?)
}

#[cfg(not(debug_assertions))]
fn auth(_: &Db, _: &str, _: [u8; 32]) -> anyhow::Result<Auth> {
	anyhow::bail!(
		"the conformance server signs people in without a code, so it only builds for debug"
	)
}

/// Registers the suite's three apps, and returns its configuration for
/// them: `client` and `client2` send their secret with HTTP Basic,
/// `client_secret_post` in the form, as the plan's modules expect.
async fn register(idp: &Idp, settings: &Settings) -> anyhow::Result<Value> {
	let suite = settings.suite.trim_end_matches('/');
	let callback = format!("{suite}/test/a/{}/callback", settings.alias);
	let mut clients = serde_json::Map::new();

	for (name, auth_method) in [
		("client", ClientAuth::SecretBasic),
		("client2", ClientAuth::SecretBasic),
		("client_secret_post", ClientAuth::SecretPost),
	] {
		let (client, secret) = idp
			.register_client(NewClientSpec {
				name: format!("conformance {name}"),
				redirect_uris: vec![callback.clone()],
				post_logout_redirect_uris: Vec::new(),
				scopes: vec!["openid".into(), "profile".into(), "email".into()],
				auth_method,
				grant_types: leptoskit_idp::default_grant_types(),
				jwks: None,
				id_token_alg: SigningAlgorithm::Rs256,
				first_party: false,
				introspection: false,
				dpop_bound: false,
				// The Basic OP plan's requests carry no PKCE.
				require_pkce: false,
			})
			.await?;
		let secret = secret.context("a secret app has a secret")?;
		clients.insert(
			name.to_owned(),
			json!({
				"client_id": client.client_id,
				"client_secret": secret.expose(),
				"scope": "openid profile email",
			}),
		);
	}

	let mut config = json!({
		"alias": settings.alias,
		"description": "leptoskit_idp",
		"server": {
			"discoveryUrl": format!("{}/.well-known/openid-configuration", settings.issuer),
			"login_hint": SEEDED,
		},
		"browser": browser(&settings.issuer),
		"override": overrides(&settings.issuer),
	});
	if let Some(object) = config.as_object_mut() {
		object.extend(clients);
	}
	Ok(config)
}

/// The browser script: sign in when asked, allow when asked, and wait for
/// the suite's callback.
fn browser(issuer: &str) -> Value {
	json!([{
		"match": format!("{issuer}/oauth/authorize*"),
		"tasks": [
			{
				"task": "Sign in",
				"optional": true,
				"match": format!("{issuer}/sign-in*"),
				"commands": [["click", "id", "sign-in"]],
			},
			{
				"task": "Consent",
				"optional": true,
				"match": format!("{issuer}/consent*"),
				"commands": [["click", "id", "allow"]],
			},
			{
				"task": "Verify Complete",
				"match": "*/test/*/callback*",
				"commands": [["wait", "id", "submission_complete", 10]],
			},
		],
	}])
}

/// Modules that expect something other than a callback.
fn overrides(issuer: &str) -> Value {
	// Signing in again, as `prompt=login` and `max_age=1` ask: the suite
	// keeps a screenshot of the sign-in page as evidence.
	let sign_in_again = json!({
		"browser": [{
			"match": format!("{issuer}/oauth/authorize*"),
			"tasks": [
				{
					"task": "Sign in",
					"optional": true,
					"match": format!("{issuer}/sign-in*"),
					"commands": [
						["wait", "id", "sign-in", 10, "Sign in", "update-image-placeholder-optional"],
						["click", "id", "sign-in"],
					],
				},
				{
					"task": "Consent",
					"optional": true,
					"match": format!("{issuer}/consent*"),
					"commands": [["click", "id", "allow"]],
				},
				{
					"task": "Verify Complete",
					"match": "*/test/*/callback*",
					"commands": [["wait", "id", "submission_complete", 10]],
				},
			],
		}],
	});
	// An unregistered redirect URI gets an error page, never a redirect.
	let error_page = json!({
		"browser": [{
			"match": format!("{issuer}/oauth/authorize*"),
			"tasks": [{
				"task": "Expect the error page",
				"match": format!("{issuer}/oauth/authorize*"),
				"commands": [[
					"wait", "xpath", "//*", 10, "This sign-in link doesn't work", "update-image-placeholder",
				]],
			}],
		}],
	});

	json!({
		"oidcc-prompt-login": sign_in_again,
		"oidcc-max-age-1": sign_in_again,
		"oidcc-ensure-registered-redirect-uri": error_page,
		"oidcc-ensure-request-object-with-redirect-uri": error_page,
	})
}

#[derive(Deserialize)]
struct ReturnTo {
	#[serde(default)]
	return_to: String,
}

/// The sign-in page: one button, which signs in the seeded person and goes
/// back to `return_to`.
async fn sign_in_page(Query(query): Query<ReturnTo>) -> Html<String> {
	Html(page(
		"Sign in",
		&format!(
			"<form method=post action=/sign-in><input type=hidden name=return_to \
			 value=\"{}\"><p>You are signing in as {SEEDED}.</p><button id=sign-in \
			 type=submit>Sign in</button></form>",
			escape(&query.return_to)
		),
	))
}

/// Signs the browser in as the seeded person, through the development
/// sign-in route, and sends it back to `return_to`.
async fn sign_in(Extension(auth): Extension<Auth>, Form(form): Form<ReturnTo>) -> Response {
	let Some(origin) = auth.origin().map(str::to_owned) else {
		return StatusCode::INTERNAL_SERVER_ERROR.into_response();
	};
	let request = Request::post("/auth/dev/sign-in")
		.header(header::ORIGIN, origin)
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(json!({ "email": SEEDED }).to_string()));
	let Ok(request) = request else {
		return StatusCode::INTERNAL_SERVER_ERROR.into_response();
	};
	let signed_in = auth
		.router::<()>()
		.layer(auth.layer())
		.oneshot(request)
		.await;
	let cookie = match &signed_in {
		Ok(response) if response.status().is_success() => {
			response.headers().get(header::SET_COOKIE).cloned()
		}
		_ => None,
	};
	let Some(cookie) = cookie else {
		return (StatusCode::INTERNAL_SERVER_ERROR, "couldn't sign in").into_response();
	};

	// Only back to this site.
	let to = if form.return_to.starts_with('/') && !form.return_to.starts_with("//") {
		form.return_to.as_str()
	} else {
		"/"
	};
	let mut response = Redirect::to(to).into_response();
	response.headers_mut().insert(header::SET_COOKIE, cookie);
	no_store(response)
}

#[derive(Deserialize)]
struct ConsentQuery {
	request: String,
}

/// The consent page: what the app asks for, and Allow and Cancel.
async fn consent_page(
	me: SignedIn,
	Extension(idp): Extension<Idp>,
	Query(query): Query<ConsentQuery>,
) -> Response {
	let Ok(request) = idp.consent_request(&me, &query.request).await else {
		return (StatusCode::NOT_FOUND, "that request is unknown or answered").into_response();
	};
	let mut scopes = String::new();
	for scope in &request.scopes {
		let _ = write!(scopes, "<li>{}</li>", escape(&scope.description));
	}
	let body = format!(
		"<p>{} asks to:</p><ul>{scopes}</ul><form method=post action=/consent><input type=hidden \
		 name=request value=\"{}\"><button id=allow type=submit name=approve \
		 value=yes>Allow</button><button id=cancel type=submit name=approve \
		 value=no>Cancel</button></form>",
		escape(&request.app.name),
		escape(&query.request),
	);
	no_store(Html(page("Allow access", &body)).into_response())
}

#[derive(Deserialize)]
struct ConsentForm {
	request: String,
	approve: String,
}

/// Answers the consent page, and sends the browser back to the app.
async fn consent(
	me: SignedIn,
	info: RequestInfo,
	Extension(idp): Extension<Idp>,
	Form(form): Form<ConsentForm>,
) -> Response {
	match idp
		.decide(&me, &form.request, form.approve == "yes", &info)
		.await
	{
		Ok(to) => no_store(Redirect::to(&to).into_response()),
		Err(_) => (StatusCode::NOT_FOUND, "that request is unknown or answered").into_response(),
	}
}

fn page(title: &str, body: &str) -> String {
	format!("<!doctype html><meta charset=utf-8><title>{title}</title><h1>{title}</h1>{body}")
}

fn no_store(mut response: Response) -> Response {
	response
		.headers_mut()
		.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
	response
}

fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}
