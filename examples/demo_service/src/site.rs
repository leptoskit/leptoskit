//! The main site: sign-in, the identity provider the admin signs in
//! through, and its minimal pages.
//!
//! This slice has no visitor onboarding. The pages are plain responses,
//! the mailer is a sink — an emailed code goes nowhere, and there is no
//! development sign-in — so nobody can actually get in yet. That is the
//! deliberate state of a slice that exists to prove the lifecycle, not
//! the front door.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::get;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use tower::ServiceExt as _;

/// What the site's pages say. Plain text in a plain page: visual design
/// is a later task.
const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Notes</title><h1>Notes</h1><p>The \
                    public demo is preparing its first day.</p>";

/// What every build registers for the admin: its first-party client on
/// this site's identity provider. The client secret is shown once by the
/// provider, so the caller persists it in the generation's secrets.
///
/// The scopes are the ones this slice offers: no `roles`, because the
/// site has no sign-in yet — the mailer is a sink — and a roles hook
/// would describe access nobody can reach. Visitor onboarding, the task
/// that adds the inbox, revisits this list with the roles hook.
pub async fn register_admin_client(idp: &Idp, admin_origin: &str) -> Result<(String, String)> {
	let output = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Notes admin".into(),
			redirect_uris: vec![format!("{admin_origin}/auth/callback/main")],
			post_logout_redirect_uris: vec![format!(
				"{admin_origin}{}/sign-in",
				leptoskit_admin::BASE
			)],
			scopes: ["openid", "email", "profile"].map(str::to_owned).to_vec(),
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
		anyhow::bail!("the site refused to register the admin's client: {output:?}");
	};
	let secret = secret.context("the site issued no client secret")?;
	Ok((client.client_id, secret))
}

/// The site's routes, with the credentials the admin needs to sign in
/// through it. Kept so the admin can reach the site in process
/// (`provider_http`) and the client credentials can be persisted.
pub struct Site {
	pub router: Router,
	pub client_id: String,
	pub client_secret: String,
}

/// Builds the site for a fresh generation: its identity provider client
/// is registered once, and the credentials are returned for persistence.
/// The site database's tables were created by the build before this runs;
/// a reopened generation calls [`rebuild`] instead.
///
/// # Errors
///
/// When sign-in or the provider can't start, or the client registration
/// is refused.
pub async fn build(
	db: &leptoskit_db::Db,
	keys: Keys,
	site_origin: &str,
	admin_origin: &str,
) -> Result<Site> {
	let auth = site_auth(db, keys, site_origin)?;
	let idp = Idp::builder(db, &auth).build()?;
	let (client_id, client_secret) = register_admin_client(&idp, admin_origin).await?;

	let router = routes(&auth, &idp);

	Ok(Site {
		router,
		client_id,
		client_secret,
	})
}

/// Rebuilds the site's routes for a reopened generation: same keys, same
/// database, and the identity provider client it already has.
///
/// # Errors
///
/// When sign-in or the provider can't start.
pub fn rebuild(db: &leptoskit_db::Db, keys: Keys, site_origin: &str) -> Result<Router> {
	let auth = site_auth(db, keys, site_origin)?;
	let idp = Idp::builder(db, &auth).build()?;
	Ok(routes(&auth, &idp))
}

fn site_auth(db: &leptoskit_db::Db, keys: Keys, site_origin: &str) -> Result<Auth> {
	Ok(Auth::builder(db, keys)
		.app_name("Notes")
		.origin(site_origin)
		// Codes go nowhere in this slice: there is no inbox yet, and no
		// development sign-in to bypass that.
		.mailer(Mailbox::new())
		.build()?)
}

fn routes(auth: &Auth, idp: &Idp) -> Router {
	Router::new()
		.route("/", get(home))
		.route("/sign-in", get(sign_in))
		.merge(auth.router())
		.merge(idp.router())
		// The IdP answers discovery and tokens from its layer, so the
		// layers go on last, as admin_demo's site does.
		.layer(auth.layer())
		.layer(idp.layer())
}

async fn home() -> Response {
	page()
}

async fn sign_in() -> Response {
	page()
}

fn page() -> Response {
	(
		StatusCode::OK,
		[(axum::http::header::CACHE_CONTROL, "no-store")],
		PAGE,
	)
		.into_response()
}

/// The analytics service's routes, started against the generation's
/// database. Same-day reopening runs the migrations as a no-op and starts
/// this process image's own live stream.
///
/// # Errors
///
/// When the database can't open or migrate.
pub async fn start_analytics(dir: &Path, site_origin: &str, token: &str) -> Result<Router> {
	admin_demo::analytics::start(dir, site_origin, token).await
}

/// A month of traffic through the service's own routes, then the
/// observable barrier: a read of what the service now reports stored.
///
/// # Errors
///
/// When ingestion refuses a batch, or the read-back count is zero.
pub async fn seed_with_barrier(router: &Router, token: &str, now_millis: i64) -> Result<i64> {
	let sent = admin_demo::analytics::seed_with(router.clone(), token, now_millis)
		.await
		.context("seeding the analytics service")?;
	let counted = count_events(router, token).await?;
	anyhow::ensure!(
		sent > 0 && counted > 0 && counted <= i64::try_from(sent).unwrap_or(i64::MAX),
		"the seed barrier failed: {sent} entries sent, the service reports {counted} stored"
	);
	Ok(counted)
}

/// The service's own count of stored events, read through its read route
/// — the durable, observable side of the seed barrier. A warm open calls
/// this too, and refuses a generation whose count no longer matches its
/// marker.
///
/// # Errors
///
/// When the read route refuses or its answer can't be parsed.
pub async fn count_events(router: &Router, token: &str) -> Result<i64> {
	let path = format!(
		"{}/rpc/event_definitions",
		leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
	);
	let response = router
		.clone()
		.oneshot(
			axum::http::Request::builder()
				.method(axum::http::Method::POST)
				.uri(path)
				.header(axum::http::header::ACCEPT, "application/json")
				.header(axum::http::header::CONTENT_TYPE, "application/json")
				.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
				.body(axum::body::Body::from("{}"))?,
		)
		.await
		.context("reading the event definitions")?;
	let status = response.status();
	let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
		.await
		.context("reading the event definitions' body")?;
	anyhow::ensure!(
		status.is_success(),
		"the service refused the event definitions read ({status}): {}",
		String::from_utf8_lossy(&bytes)
	);

	let definitions: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
	let count: i64 = definitions
		.iter()
		.filter_map(|definition| definition["volume"].as_i64())
		.sum();
	Ok(count)
}
