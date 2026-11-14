//! An admin app signs people in through the main site's own sign-in, end
//! to end in one process. The main site is the provider (`leptoskit_auth`
//! plus `leptoskit_idp`, on SQLite and Postgres). The admin app signs in
//! with it as a generic OpenID Connect provider, with `userinfo` on, a
//! gate that requires the `admin` role from the provider's `roles` claim,
//! and provider-only sign-in. The admin app's requests to the provider are
//! answered by the provider's real routes, so discovery, JWKS, ID tokens,
//! and `userinfo` are all the real thing.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::collections::HashMap;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext;
use leptoskit_auth::AuthError;
use leptoskit_auth::Identity;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderFuture;
use leptoskit_auth::ProviderHttp;
use leptoskit_auth::ProviderRequest;
use leptoskit_auth::ProviderResponse;
use leptoskit_auth::Result as AuthResult;
use leptoskit_auth::Roles;
use leptoskit_auth::SessionResponse;
use leptoskit_auth::SignInAttempt;
use leptoskit_auth::SignInGate;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use tower::ServiceExt;

/// The main site, which signs people in for the admin app.
const NOTES: &str = "https://notes.example";
/// The admin app.
const ADMIN: &str = "https://admin.example";
/// The PKCE verifier the tests trade codes with.
const VERIFIER: &str = "a-verifier-of-at-least-forty-three-characters-0123456789";

/// A query value, percent-encoded, for an authorize URL built by hand.
fn encoded(value: &str) -> String {
	url::form_urlencoded::Serializer::new(String::new())
		.append_pair("v", value)
		.finish()
		.trim_start_matches("v=")
		.to_owned()
}

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

/// The admin app's own record of the roles the provider released at
/// sign-in, by the provider's subject. Its `Roles` reads this back, so a
/// role taken away at the provider applies at the next sign-in.
#[table(name = "admin_roles")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct AdminRole {
	pub id: Id<Self>,
	#[db(unique)]
	pub subject: String,
	pub admin: bool,
}

/// The admin app's gate: only people the provider says are admins sign in,
/// and what it said is recorded for `AdminUser` to read back, on the
/// sign-in's own transaction.
struct AdminGate;

impl SignInGate for AdminGate {
	async fn check(&self, tx: &Tx, attempt: &SignInAttempt<'_>) -> AuthResult<()> {
		if !attempt.roles().contains(&"admin") {
			return Err(AuthError::NotAllowed);
		}

		let subject = attempt.subject.ok_or(AuthError::NotAllowed)?;
		let existing = AdminRole::query()
			.filter(|stored| stored.subject.eq(subject.to_owned()))
			.first(tx)
			.await?;

		match existing {
			Some(stored) => {
				AdminRole::update()
					.filter(|row| row.id.eq(stored.id))
					.set(|row| row.admin, true)
					.run(tx)
					.await?;
			}
			None => {
				NewAdminRole {
					subject: subject.to_owned(),
					admin: true,
				}
				.insert(tx)
				.await?;
			}
		}

		Ok(())
	}
}

/// Marker for the admin app's `Roles`, which reads what its gate kept.
struct AdminUser;

impl Roles for AdminUser {
	async fn roles(db: &Db, account: Id<Account>) -> AuthResult<Vec<String>> {
		let identity = Identity::query()
			.filter(|identity| {
				identity.account_id.eq(account) & identity.provider.eq("main".to_owned())
			})
			.first(db)
			.await?;
		let Some(identity) = identity else {
			return Ok(Vec::new());
		};
		let stored = AdminRole::query()
			.filter(|stored| stored.subject.eq(identity.subject.clone()))
			.first(db)
			.await?;

		match stored {
			Some(stored) if stored.admin => Ok(vec!["admin".to_owned()]),
			_ => Ok(Vec::new()),
		}
	}
}

/// The provider's real routes, in memory, answering the admin app's
/// requests to the main site.
struct MainSite {
	router: Router,
}

impl ProviderHttp for MainSite {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		let router = self.router.clone();
		Box::pin(async move {
			let url = url::Url::parse(&request.url).map_err(|error| error.to_string())?;

			if url.origin().ascii_serialization() != NOTES {
				return Err(format!(
					"{} isn't the main site",
					url.origin().ascii_serialization()
				));
			}

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

/// A browser, with a cookie jar for each site it visits.
#[derive(Default)]
struct Browser {
	jars: HashMap<String, HashMap<String, String>>,
}

impl Browser {
	async fn send(
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

	async fn get(&mut self, router: &Router, origin: &str, path: &str) -> Response {
		let request = Request::get(path);
		self.send(router, origin, request, Body::empty()).await
	}

	async fn post_json(
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

/// Where a response sends the browser.
fn location(response: &Response) -> String {
	assert_eq!(response.status(), StatusCode::SEE_OTHER, "{response:?}");
	response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned()
}

async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// The claims of a JWT, unchecked.
fn claims(token: &str) -> Value {
	let payload = token.split('.').nth(1).unwrap();
	serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

fn challenge(verifier: &str) -> String {
	URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

fn param(location: &str, name: &str) -> Option<String> {
	url::Url::parse(&format!("https://any.example{location}"))
		.unwrap()
		.query_pairs()
		.into_owned()
		.find(|(key, _)| key == name)
		.map(|(_, value)| value)
}

/// Both apps, each over its own database, both served from their real
/// routes.
struct Apps {
	notes: Router,
	notes_db: Db,
	notes_mailbox: Mailbox,
	notes_idp: Idp,
	admin: Router,
	admin_db: Db,
	admin_auth: Auth,
	/// The admin app's registration at the main site.
	admin_client: String,
	admin_secret: String,
}

async fn apps(provider_db: Db, admin_db: Db) -> Apps {
	leptoskit_db::testing::create_tables(&provider_db)
		.await
		.unwrap();
	leptoskit_db::testing::create_tables(&admin_db)
		.await
		.unwrap();

	let mailbox = Mailbox::new();
	let notes_auth = Auth::builder(&provider_db, Keys::single("notes", [5; 32]))
		.app_name("Notes")
		.origin(NOTES)
		.mailer(mailbox.clone())
		.on_sign_up::<User>()
		.roles::<User>()
		.build()
		.unwrap();
	let notes_idp = Idp::builder(&provider_db, &notes_auth)
		.sign_in_page("/sign-in")
		.consent_page("/consent")
		.build()
		.unwrap();
	let notes = Router::new()
		.merge(notes_auth.router())
		.merge(notes_idp.router())
		.layer(notes_auth.layer())
		.layer(notes_idp.layer());

	// The admin app is registered at the main site once, as `leptoskit auth
	// idp client add --secret-post --first-party --scopes
	// openid,profile,roles` does (`Provider::oidc` sends its secret in the
	// form body), and holds the client ID and secret it got.
	let (admin_client, admin_secret) = register(
		&notes_idp,
		true,
		ClientAuth::SecretPost,
		&["openid", "email", "profile", "roles"],
	)
	.await;

	let provider = Provider::oidc(
		"main",
		"Notes",
		NOTES,
		&admin_client,
		Some(admin_secret.clone()),
	)
	.unwrap()
	.userinfo(true)
	.scopes(["openid", "email", "profile", "roles"]);
	let admin_auth = Auth::builder(&admin_db, Keys::single("admin", [7; 32]))
		.app_name("Admin")
		.origin(ADMIN)
		.mailer(Mailbox::new())
		.provider(provider)
		.provider_http(MainSite {
			router: notes.clone(),
		})
		.gate(AdminGate)
		.roles::<AdminUser>()
		.sign_in_methods(["oidc:main"])
		.build()
		.unwrap();
	let admin = Router::new()
		.merge(admin_auth.router())
		.layer(admin_auth.layer());

	Apps {
		notes,
		notes_db: provider_db,
		notes_mailbox: mailbox,
		notes_idp,
		admin,
		admin_db,
		admin_auth,
		admin_client,
		admin_secret,
	}
}

/// Registers an app on the provider, and returns its client ID and secret.
async fn register(
	idp: &Idp,
	first_party: bool,
	auth_method: ClientAuth,
	scopes: &[&str],
) -> (String, String) {
	let output = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: if first_party { "Admin" } else { "Partner" }.into(),
			redirect_uris: vec![format!("{ADMIN}/auth/callback/main")],
			post_logout_redirect_uris: Vec::new(),
			scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
			auth_method,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: SigningAlgorithm::Es256,
			first_party,
			introspection: false,
			dpop_bound: false,
		}))
		.await;
	let ClientCommandOutput::Added { client, secret } = output else {
		panic!("the app should register: {output:?}");
	};
	(client.client_id, secret.unwrap())
}

impl Apps {
	/// Signs someone in to the main site with an emailed code.
	async fn notes_sign_in(&self, browser: &mut Browser, address: &str) {
		let sent = browser
			.post_json(
				&self.notes,
				NOTES,
				"/auth/email/send",
				json!({ "email": address }),
			)
			.await;
		assert_eq!(sent.status(), StatusCode::NO_CONTENT);
		let mail = self.notes_mailbox.latest_to(address).unwrap();
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

	/// Signs someone in to the admin app through the main site: starts the
	/// provider flow, signs in at the main site, and comes back with the
	/// code. The one browser visits both sites. Returns its last stop, with
	/// the outcome in its query.
	async fn through(&self, browser: &mut Browser) -> String {
		let started = browser
			.post_json(
				&self.admin,
				ADMIN,
				"/auth/oauth/start",
				json!({ "provider": "main" }),
			)
			.await;
		assert_eq!(started.status(), StatusCode::OK);
		let url = json_body(started).await["url"].as_str().unwrap().to_owned();

		// The provider's authorization page, in the same browser, which is
		// signed in there. A first-party app gets its code straight away.
		let back = location(&browser.get(&self.notes, NOTES, &url).await);
		assert!(
			back.starts_with(&format!("{ADMIN}/auth/callback/main")),
			"{back}"
		);
		assert_eq!(param(&back, "iss").as_deref(), Some(NOTES));

		let finished = browser.get(&self.admin, ADMIN, &back).await;
		let to = location(&finished);
		assert!(to.starts_with("/?"), "{to}");
		to
	}

	/// The accounts, identities, and recorded roles the admin app holds.
	async fn admin_counts(&self) -> (u64, u64, u64) {
		let system = self.admin_db.system("test: count");
		let accounts = Account::query().count(&system).await.unwrap();
		let identities = Identity::query().count(&system).await.unwrap();
		let roles = AdminRole::query().count(&system).await.unwrap();
		(accounts, identities, roles)
	}
}

/// Both kinds of database pair: SQLite, then Postgres when it's here.
async fn pairs(cluster: Option<&PostgresCluster>) -> Vec<(Db, Db)> {
	let mut pairs = vec![(
		Db::connect("sqlite::memory:").await.unwrap(),
		Db::connect("sqlite::memory:").await.unwrap(),
	)];

	if let Some(cluster) = cluster {
		pairs.push((
			Db::connect(&cluster.database()).await.unwrap(),
			Db::connect(&cluster.database()).await.unwrap(),
		));
	}

	pairs
}

#[tokio::test]
async fn an_admin_app_signs_in_through_the_main_sites_own_sign_in() {
	let cluster = PostgresCluster::start();

	for (provider_db, admin_db) in pairs(cluster.as_ref()).await {
		let app = apps(provider_db, admin_db).await;

		// Two accounts on the main site; one of them keeps the place.
		let mut ada = Browser::default();
		app.notes_sign_in(&mut ada, "ada@example.com").await;
		let system = app.notes_db.system("test: promote ada");
		let user = User::query()
			.filter(|user| user.name.eq("ada".to_owned()))
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
		let mut grace = Browser::default();
		app.notes_sign_in(&mut grace, "grace@example.com").await;

		// The provider offers `roles` only because the main site says who
		// has which role, and names it in discovery.
		let discovery: Value = json_body(
			Browser::default()
				.get(&app.notes, NOTES, "/.well-known/openid-configuration")
				.await,
		)
		.await;
		assert!(
			discovery["scopes_supported"]
				.as_array()
				.unwrap()
				.contains(&json!("roles")),
			"{}",
			discovery["scopes_supported"]
		);

		// Ada, an admin there, signs in to the admin app through Notes.
		let to = app.through(&mut ada).await;
		assert_eq!(param(&to, "auth").as_deref(), Some("signed_in"), "{to}");

		// She has a session, and no other account was made.
		let session: SessionResponse = serde_json::from_value(
			json_body(ada.get(&app.admin, ADMIN, "/auth/session").await).await,
		)
		.unwrap();
		let account = session.session.as_ref().unwrap().account_id;
		assert_eq!(app.admin_counts().await, (1, 1, 1));

		// The gate recorded the provider's roles, and the app's `Roles`
		// reads them back.
		let roles = app.admin_auth.roles(account).await.unwrap();
		assert_eq!(roles, ["admin"]);

		// The identity names who the main site says this is: for one of its
		// own apps, that main site's account ID.
		let system = app.admin_db.system("test: identity");
		let identity = Identity::query().first(&system).await.unwrap().unwrap();
		assert_eq!(identity.provider, "main");
		assert!(
			identity.subject.parse::<Id<Account>>().is_ok(),
			"a first-party app sees the main site's account ID as its subject: {}",
			identity.subject
		);

		// Grace is not an admin there: refused, with no session and no
		// account.
		let to = app.through(&mut grace).await;
		assert_eq!(
			param(&to, "auth_error").as_deref(),
			Some("not_allowed"),
			"{to}"
		);
		let session: SessionResponse = serde_json::from_value(
			json_body(grace.get(&app.admin, ADMIN, "/auth/session").await).await,
		)
		.unwrap();
		assert!(session.session.is_none(), "no session");
		assert_eq!(app.admin_counts().await, (1, 1, 1));

		// Only signing in with the main site is on: every other way is a
		// 404, and the pages see none of it.
		for (path, body) in [
			("/auth/email/send", true),
			("/auth/email/verify", true),
			("/auth/password/sign-in", true),
			("/auth/password/config", false),
			("/auth/passkey/sign-in/options", true),
			("/auth/wallets/config", false),
			("/auth/dev/sign-in", true),
		] {
			let response = if body {
				ada.post_json(&app.admin, ADMIN, path, json!({})).await
			} else {
				ada.get(&app.admin, ADMIN, path).await
			};
			assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
		}

		let providers: Value = json_body(ada.get(&app.admin, ADMIN, "/auth/providers").await).await;
		assert_eq!(providers, json!([{ "id": "main", "name": "Notes" }]));

		// Another site can't have the main site's roles: the scope is
		// refused at authorize, before anyone is even asked to sign in.
		let (partner, _partner_secret) = register(
			&app.notes_idp,
			false,
			ClientAuth::SecretBasic,
			&["openid", "roles"],
		)
		.await;
		let authorize_url = format!(
			"/oauth/authorize?response_type=code&client_id={partner}&redirect_uri={}&scope=openid%\
			 20roles&state=st&nonce=n&code_challenge={}&code_challenge_method=S256",
			encoded(&format!("{ADMIN}/auth/callback/main")),
			challenge(VERIFIER),
		);
		let back = location(
			&Browser::default()
				.get(&app.notes, NOTES, &authorize_url)
				.await,
		);
		assert_eq!(
			param(&back, "error").as_deref(),
			Some("invalid_scope"),
			"{back}"
		);

		// The first-party app gets the roles in its ID token and userinfo.
		let authorize_url = format!(
			"/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=openid%\
			 20roles&state=st&nonce=n&code_challenge={}&code_challenge_method=S256",
			app.admin_client,
			encoded(&format!("{ADMIN}/auth/callback/main")),
			challenge(VERIFIER),
		);
		// Ada's browser is still signed in at the main site.
		let back = location(&ada.get(&app.notes, NOTES, &authorize_url).await);
		let code = param(&back, "code").unwrap();
		// The app sends its secret in the form body, as
		// `client_secret_post` asks.
		let exchanged = post_form(
			&app.notes,
			"/oauth/token",
			&[
				("grant_type", "authorization_code"),
				("code", &code),
				("redirect_uri", &format!("{ADMIN}/auth/callback/main")),
				("code_verifier", VERIFIER),
				("client_id", &app.admin_client),
				("client_secret", &app.admin_secret),
			],
			None,
		)
		.await;
		assert_eq!(exchanged.status(), StatusCode::OK);
		let tokens: Value = json_body(exchanged).await;
		let id = claims(tokens["id_token"].as_str().unwrap());
		assert_eq!(id["aud"], app.admin_client);
		assert_eq!(id["roles"], json!(["admin"]), "{id}");

		let info = get_bearer(
			&app.notes,
			"/oauth/userinfo",
			tokens["access_token"].as_str().unwrap(),
		)
		.await;
		assert_eq!(info.status(), StatusCode::OK);
		let info: Value = json_body(info).await;
		assert_eq!(info["roles"], json!(["admin"]), "{info}");
	}
}

async fn post_form(
	router: &Router,
	path: &str,
	pairs: &[(&str, &str)],
	authorization: Option<&str>,
) -> Response {
	let form = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(pairs)
		.finish();
	let mut builder =
		Request::post(path).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");

	if let Some(authorization) = authorization {
		builder = builder.header(header::AUTHORIZATION, authorization);
	}

	router
		.clone()
		.oneshot(builder.body(Body::from(form)).unwrap())
		.await
		.unwrap()
}

async fn get_bearer(router: &Router, path: &str, token: &str) -> Response {
	let request = Request::get(path).header(header::AUTHORIZATION, format!("Bearer {token}"));
	router
		.clone()
		.oneshot(request.body(Body::empty()).unwrap())
		.await
		.unwrap()
}
