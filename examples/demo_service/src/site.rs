//! The main site: the front door, visitor passes, the bounded demo
//! inbox, real email-code sign-in behind the pass gate, and the
//! identity provider the admin signs in through.
//!
//! The division of labor:
//!
//! - [`crate::visitors`] owns the durable contract: the pass and person tables,
//!   the cookie, the caps, and the ownership checks.
//! - [`crate::inbox`] owns the mailer: bounded memory, durable receipts.
//! - [`crate::return_to`] owns the local-path allowlist.
//! - This module owns the wiring: the routes, the pass gate that sits in front
//!   of exactly `POST /auth/email/send` and `POST /auth/email/verify`, and the
//!   Auth hooks' registration.
//!
//! Private notes are [`crate::notes`], sent through [`crate::activity`]'s
//! one analytics event; this module owns their two routes and the
//! ownership gate in front of them.
//!
//! The pages are [`crate::pages`]: real leptoskit trees, built here from
//! what the visitor's pass and session say, and always `no-store`.
//! Everything a visitor can reach is tested through the real routers, and
//! no response carries a generation key, the analytics token, the control
//! key, or a client secret.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use axum::Router;
use axum::extract::ConnectInfo;
use axum::extract::RawQuery;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::middleware::Next;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::SignedIn;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use tower::ServiceExt as _;

mod operations;

use demo_pages::model;

use crate::activity::Analytics;
use crate::client::Client;
use crate::inbox::DemoInbox;
use crate::notes;
use crate::notes::CreateError;
use crate::notes::NoteText;
use crate::notes::Notebook;
use crate::pages;
use crate::return_to;
use crate::visitors;
use crate::visitors::DemoPerson;
use crate::visitors::MintError;
use crate::visitors::MintOutcome;
use crate::visitors::PassState;
use crate::visitors::Persona;
use crate::visitors::Visitors;
use crate::visitors::carried_cookie;
use crate::visitors::display_tag;
use crate::visitors::seconds_until_next_midnight;

/// The faces the admin draws with, served from this site so a page never
/// reaches another origin for them.
const SKETCH_FONT: &[u8] =
	include_bytes!("../../../crates/leptoskit_admin/assets/fonts/SkribbleNotoSketch-Latin.woff2");
const MONO_FONT: &[u8] =
	include_bytes!("../../../crates/leptoskit_admin/assets/fonts/SkribbleMonoPlayful-Latin.woff2");

/// The most bytes a mint form may carry.
const MAX_FORM_BYTES: usize = 4 * 1024;

/// The most bytes a gated sign-in body may carry; Auth's original bytes
/// are forwarded inside this bound.
const MAX_GATE_BODY: usize = 4 * 1024;

/// The most bytes a note's JSON body may carry. A note is at most 280
/// characters, so even a body of `\uXXXX` escapes fits with room to spare.
const MAX_NOTE_BODY: usize = 4 * 1024;

/// What every build registers for the admin: its first-party client on
/// this site's identity provider. The client secret is shown once by the
/// provider, so the caller persists it in the generation's secrets.
///
/// The scopes include `roles` because the site now has real sign-in and
/// a roles hook: Ada carries the `admin` role, Grace none, and the
/// admin's gate reads the released `roles` claim. The admin requests
/// that scope in its own client registration.
pub async fn register_admin_client(idp: &Idp, admin_origin: &str) -> Result<(String, String)> {
	let output = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Notes admin".into(),
			redirect_uris: vec![format!("{admin_origin}/auth/callback/main")],
			post_logout_redirect_uris: vec![format!(
				"{admin_origin}{}/sign-in",
				leptoskit_admin::BASE
			)],
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
	/// The site's sign-in service, for tests that prove an Auth-level
	/// refusal directly — never a second way in.
	auth: Auth,
}

impl Site {
	/// The site's Auth, for tests that prove an Auth-level refusal
	/// directly.
	#[must_use]
	pub fn auth(&self) -> &Auth {
		&self.auth
	}
}

/// The site's own options for one generation: its UTC day, the optional
/// client IP header, and the visitors' caps.
#[derive(Clone, Debug)]
pub struct SiteOptions {
	day: String,
	client_ip_header: Option<String>,
	caps: visitors::Caps,
	client: Client,
}

impl SiteOptions {
	/// The options a generation runs under: production caps, the given
	/// day, and the optional client IP header.
	#[must_use]
	pub fn new(day: impl Into<String>, client_ip_header: Option<String>) -> Self {
		Self {
			day: day.into(),
			client_ip_header,
			caps: visitors::Caps::default(),
			client: Client::absent(),
		}
	}

	/// Serves `client` as the pages' browser half. Without one, pages
	/// render and stay as the server drew them.
	#[must_use]
	pub fn with_client(mut self, client: Client) -> Self {
		self.client = client;
		self
	}

	/// Narrows the visitors' caps for a test. Debug builds only: a
	/// release binary never sees this constructor, so production always
	/// runs on the constants.
	#[cfg(debug_assertions)]
	#[must_use]
	pub fn with_test_caps(mut self, per_ip_per_hour: u64, per_day: u64) -> Self {
		self.caps = visitors::Caps {
			new_passes_per_ip_per_hour: per_ip_per_hour,
			new_passes_per_day: per_day,
		};
		self
	}
}

/// What the routes share: everything is either immutable for the
/// generation's life or inside an `Arc`.
#[derive(Clone)]
struct Shared(Arc<Parts>);

struct Parts {
	visitors: Visitors,
	notes: Notebook,
	analytics: Analytics,
	inbox: Arc<DemoInbox>,
	/// The configured site origin, which a mint's Origin must equal.
	origin: String,
	/// The configured admin origin, which the pages link to.
	admin_origin: String,
	/// The request header the visitor's canonical IP arrives in, when a
	/// deployment configured one.
	client_ip_header: Option<String>,
	/// The pages' browser half.
	client: Client,
	/// Where the rpc server keeps its mutation receipts: a private
	/// in-memory database, so no operation's bookkeeping shares a
	/// transaction with the site's own stores.
	receipts: leptoskit_db::Db,
	/// How many pages may watch for the next day at once. A permit lives
	/// inside its stream, so a disconnect returns it at once.
	watchers: Arc<tokio::sync::Semaphore>,
}

/// The most pages that may watch for the next day at once.
const WATCHERS: usize = 1_000;

/// Builds the site for a fresh generation: its identity provider client
/// is registered once, and the credentials are returned for persistence.
/// `analytics` is the generation's one analytics service, already started,
/// which notes send their event through.
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
	options: &SiteOptions,
	analytics: Analytics,
) -> Result<Site> {
	let inbox = Arc::new(DemoInbox::new(db.system("demo: inbox")));
	let visitors = Visitors::new(db.clone(), &keys, &options.day, site_origin, options.caps)?;
	let auth = site_auth(db, keys, site_origin, options, &inbox)?;
	let idp = Idp::builder(db, &auth).build()?;
	let (client_id, client_secret) = register_admin_client(&idp, admin_origin).await?;

	let router = routes(
		Shared(Arc::new(Parts {
			visitors,
			notes: Notebook::new(db.clone()),
			analytics,
			inbox,
			origin: site_origin.to_owned(),
			admin_origin: admin_origin.to_owned(),
			client_ip_header: options.client_ip_header.clone(),
			client: options.client.clone(),
			receipts: operations::receipts().await?,
			watchers: Arc::new(tokio::sync::Semaphore::new(WATCHERS)),
		})),
		&auth,
		&idp,
	);

	Ok(Site {
		router,
		client_id,
		client_secret,
		auth,
	})
}

/// Rebuilds the site's routes for a reopened generation: same keys, same
/// database, the identity provider client it already has, and the
/// generation's analytics service.
///
/// # Errors
///
/// When sign-in or the provider can't start.
pub async fn rebuild(
	db: &leptoskit_db::Db,
	keys: Keys,
	site_origin: &str,
	admin_origin: &str,
	options: &SiteOptions,
	analytics: Analytics,
) -> Result<Router> {
	let inbox = Arc::new(DemoInbox::new(db.system("demo: inbox")));
	let visitors = Visitors::new(db.clone(), &keys, &options.day, site_origin, options.caps)?;
	let auth = site_auth(db, keys, site_origin, options, &inbox)?;
	let idp = Idp::builder(db, &auth).build()?;

	Ok(routes(
		Shared(Arc::new(Parts {
			visitors,
			notes: Notebook::new(db.clone()),
			analytics,
			inbox,
			origin: site_origin.to_owned(),
			admin_origin: admin_origin.to_owned(),
			client_ip_header: options.client_ip_header.clone(),
			client: options.client.clone(),
			receipts: operations::receipts().await?,
			watchers: Arc::new(tokio::sync::Semaphore::new(WATCHERS)),
		})),
		&auth,
		&idp,
	))
}

fn site_auth(
	db: &leptoskit_db::Db,
	keys: Keys,
	site_origin: &str,
	options: &SiteOptions,
	inbox: &Arc<DemoInbox>,
) -> Result<Auth> {
	let mut builder = Auth::builder(db, keys)
		.app_name("Notes")
		.origin(site_origin)
		// The demo inbox stands in for an email app: bounded, private to
		// the pass that opened the address, receipts recorded durably.
		.mailer(inbox.as_ref().clone())
		.on_sign_up::<DemoPerson>()
		.roles::<DemoPerson>()
		// Email is the only way in: no wallet, passkey, or password route
		// can introduce another onboarding path, and neither dev sign-in
		// nor the dev mailbox is ever mounted.
		.sign_in_methods(["email"]);

	if let Some(header) = &options.client_ip_header {
		builder = builder.client_ip_header(header.clone());
	}

	Ok(builder.build()?)
}

fn routes(shared: Shared, auth: &Auth, idp: &Idp) -> Router {
	Router::new()
		.merge(operations::router(&shared))
		.route("/pkg/{file}", get(client_file))
		.route("/", get(home))
		.route("/sign-in", get(sign_in))
		.route("/notes", get(notes))
		.route("/demo/pass", post(mint_pass))
		.route("/demo/inbox", get(demo_inbox))
		.route("/demo/notes", get(list_notes).post(create_note))
		.route("/demo/assets/{file}", get(font))
		.with_state(shared.clone())
		.merge(auth.router())
		.merge(idp.router())
		// The pass gate wraps every route and acts on exactly the two
		// email-code posts, after the session layer's same-site check and
		// before Auth writes a challenge, mails, or counts a rate limit.
		.layer(axum::middleware::from_fn_with_state(shared, pass_gate))
		// The IdP answers discovery and tokens from its layer, so the
		// layers go on last, as admin_demo's site does.
		.layer(auth.layer())
		.layer(idp.layer())
}

/// Whether the gate acts on this request: exactly the email-code posts.
fn gated(method: &axum::http::Method, path: &str) -> bool {
	method == axum::http::Method::POST && matches!(path, "/auth/email/send" | "/auth/email/verify")
}

/// The pass gate. For the two email-code posts it buffers the body,
/// proves the request's pass owns the normalized address, and only then
/// forwards Auth's original bytes; Auth's own limits and same-site
/// checks still run on the forwarded request. A foreign identity is
/// refused before a challenge is written, before mail is sent, and
/// before Auth counts a rate-limit hit.
async fn pass_gate(State(shared): State<Shared>, request: Request, next: Next) -> Response {
	let shared = shared.0;

	if !gated(request.method(), request.uri().path()) {
		return next.run(request).await;
	}

	let is_send = request.uri().path() == "/auth/email/send";
	let (parts, body) = request.into_parts();

	// A configured edge header must arrive usable, or the request is
	// refused outright: it is never treated as an unlimited caller.
	if visitors::client_ip(
		shared.client_ip_header.as_deref(),
		&parts.headers,
		peer(&parts),
	)
	.is_err()
	{
		return json_error(StatusCode::BAD_REQUEST, "bad_client_ip");
	}

	let Ok(bytes) = axum::body::to_bytes(body, MAX_GATE_BODY).await else {
		return json_error(StatusCode::PAYLOAD_TOO_LARGE, "too_large");
	};

	let Some(email) = json_email(&bytes) else {
		return json_error(StatusCode::BAD_REQUEST, "bad_request");
	};

	let pass = match shared.visitors.resolve(&parts.headers).await {
		Ok(pass) => pass,
		Err(error) => return internal_error(&error),
	};

	// A valid pass must own the address; any other answer — no pass, or
	// a pass that never minted this address — is refused before Auth
	// writes a challenge, mails, or counts a rate-limit hit.
	let owned = match &pass {
		PassState::Valid(pass) => {
			match shared.visitors.owns(pass, &email).await {
				Ok(owned) => owned,
				Err(error) => return internal_error(&error),
			}
		}
		PassState::Absent | PassState::EarlierDay => false,
	};

	if !owned {
		let mut response = json_error(StatusCode::FORBIDDEN, "not_your_identity");

		// A cookie that named no pass here goes with the refusal, so a
		// stale browser stops carrying it around.
		if !pass.is_valid()
			&& carried_cookie(&parts.headers, shared.visitors.cookie().name).is_some()
		{
			response
				.headers_mut()
				.append(header::SET_COOKIE, shared.visitors.cookie().clear());
		}

		return response;
	}

	// Only an owned address is opened, and only on a send. The mailer
	// records the receipt before Auth answers, so one inbox read after
	// the send is enough.
	if is_send {
		shared.inbox.open(&email);
	}

	let request = Request::from_parts(parts, axum::body::Body::from(bytes));
	next.run(request).await
}

/// `POST /demo/pass`: mint this browser's pass and persona identity, or
/// reuse what it already owns.
///
/// The answer is `303` to local sign-in, carrying the persona and a
/// validated `return_to`. Refusals are stable codes: not same-origin
/// (`403`), an unusable body (`400`), an unusable client IP header
/// (`400`), the per-IP hourly cap (`429` with `Retry-After`), or the
/// generation's full day (`503`). An existing pass keeps signing in
/// when the caps are full.
async fn mint_pass(State(shared): State<Shared>, request: Request) -> Response {
	let shared = shared.0;
	let (parts, body) = request.into_parts();

	// Same-origin only, before the body is read.
	if !same_origin(&parts.headers, &shared.origin) {
		return json_error(StatusCode::FORBIDDEN, "cross_origin");
	}

	let Ok(bytes) = axum::body::to_bytes(body, MAX_FORM_BYTES).await else {
		return json_error(StatusCode::PAYLOAD_TOO_LARGE, "too_large");
	};

	let Some((persona, return_target)) = form_choice(&bytes) else {
		return json_error(StatusCode::BAD_REQUEST, "bad_request");
	};

	let minted = match mint(
		&shared,
		&parts.headers,
		peer(&parts),
		persona,
		&return_target,
	)
	.await
	{
		Ok(minted) => minted,
		Err(MintRefusal::CrossOrigin) => return json_error(StatusCode::FORBIDDEN, "cross_origin"),
		Err(MintRefusal::BadClientIp) => {
			return json_error(StatusCode::BAD_REQUEST, "bad_client_ip");
		}
		Err(MintRefusal::RateLimited { retry_after }) => {
			let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
			response.headers_mut().insert(
				header::RETRY_AFTER,
				axum::http::HeaderValue::from(retry_after),
			);
			return response;
		}
		Err(MintRefusal::Full) => return json_error(StatusCode::SERVICE_UNAVAILABLE, "full"),
		Err(MintRefusal::Failed(error)) => return internal_error(&error),
	};

	let mut response = (
		StatusCode::SEE_OTHER,
		[(header::LOCATION, minted.location)],
		axum::body::Body::empty(),
	)
		.into_response();

	if let Some(cookie) = minted.cookie {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
}

/// A person minted or reused: where to sign in as them, and the pass
/// cookie to set when the pass is new.
struct Minted {
	/// Local sign-in, carrying the persona and the validated return target.
	location: String,
	/// The new pass, when this browser had none.
	cookie: Option<axum::http::HeaderValue>,
}

/// Why a mint was refused. Each is a stable reason a page can name.
enum MintRefusal {
	/// The request did not come from this site's own pages.
	CrossOrigin,
	/// A configured edge header was missing or unusable.
	BadClientIp,
	/// This network has asked for too many new passes this hour.
	RateLimited {
		/// Whole seconds until it may ask again, at least one.
		retry_after: u64,
	},
	/// The day has handed out all its passes.
	Full,
	/// The database failed.
	Failed(anyhow::Error),
}

/// Mints this request's pass and persona identity, or reuses what it
/// already owns: the one mint behind the form post and the page's own
/// request.
///
/// Same-origin only: a present `Origin` must equal this site's origin, and
/// a contradictory `Origin` is never excused by Fetch Metadata. `return_to`
/// must already be validated.
async fn mint(
	shared: &Parts,
	headers: &HeaderMap,
	peer: Option<SocketAddr>,
	persona: Persona,
	return_to: &str,
) -> Result<Minted, MintRefusal> {
	if !same_origin(headers, &shared.origin) {
		return Err(MintRefusal::CrossOrigin);
	}

	let Ok(ip) = visitors::client_ip(shared.client_ip_header.as_deref(), headers, peer) else {
		return Err(MintRefusal::BadClientIp);
	};

	let pass = match shared.visitors.resolve(headers).await {
		Ok(PassState::Valid(pass)) => Some(pass),
		Ok(PassState::Absent | PassState::EarlierDay) => None,
		Err(error) => return Err(MintRefusal::Failed(error)),
	};

	let outcome = match shared.visitors.mint(persona, ip, pass.as_ref()).await {
		Ok(outcome) => outcome,
		Err(MintError::IpLimited { retry_after }) => {
			return Err(MintRefusal::RateLimited {
				retry_after: u64::try_from(retry_after.num_seconds().max(1)).unwrap_or(1),
			});
		}
		Err(MintError::Full) => return Err(MintRefusal::Full),
		Err(MintError::Database(error)) => return Err(MintRefusal::Failed(error)),
	};

	let cookie = match outcome {
		MintOutcome::Fresh { token } => {
			let value = format!("{}.{token}", shared.visitors.day());
			let max_age = seconds_until_next_midnight(chrono::Utc::now());
			Some(shared.visitors.cookie().set(&value, max_age))
		}
		MintOutcome::Reused => None,
	};

	Ok(Minted {
		location: visitors::sign_in_location(persona, return_to),
		cookie,
	})
}

/// `GET /pkg/{file}`: one file of the pages' browser half.
async fn client_file(
	State(shared): State<Shared>,
	axum::extract::Path(file): axum::extract::Path<String>,
	headers: HeaderMap,
) -> Response {
	shared.0.client.serve(&file, &headers)
}

/// `GET /demo/inbox`: this pass's two people and the mail this process
/// still holds for them. `no-store`, always; a query parameter cannot
/// change ownership because queries are not read.
async fn demo_inbox(
	State(shared): State<Shared>,
	headers: HeaderMap,
	session: Option<SignedIn>,
) -> Response {
	let shared = shared.0;
	let no_store = [(header::CACHE_CONTROL, "no-store")];

	let pass = match shared.visitors.resolve(&headers).await {
		Ok(pass) => pass,
		Err(error) => {
			let mut response = internal_error(&error);
			response.headers_mut().insert(
				header::CACHE_CONTROL,
				axum::http::HeaderValue::from_static("no-store"),
			);
			return response;
		}
	};

	let cookie = shared.visitors.cookie();

	let PassState::Valid(pass) = pass else {
		let state = match &pass {
			PassState::EarlierDay => "new_day",
			PassState::Absent | PassState::Valid(_) => "absent",
		};

		let mut body = serde_json::json!({ "error": "no_pass", "pass": state });
		if matches!(&pass, PassState::EarlierDay) {
			body["day"] = serde_json::Value::String(shared.visitors.day().to_owned());
		}

		let mut response = (StatusCode::UNAUTHORIZED, no_store, axum::Json(body)).into_response();

		if carried_cookie(&headers, cookie.name).is_some() {
			response
				.headers_mut()
				.append(header::SET_COOKIE, cookie.clear());
		}

		return response;
	};
	let people = match shared.visitors.people(&pass).await {
		Ok(people) => people,
		Err(error) => {
			let mut response = internal_error(&error);
			response.headers_mut().insert(
				header::CACHE_CONTROL,
				axum::http::HeaderValue::from_static("no-store"),
			);
			return response;
		}
	};

	let session_account = session.as_ref().map(|signed_in| signed_in.account);
	let mut people_json = Vec::new();
	let mut messages_json = Vec::new();
	let mut lost_json = Vec::new();

	for person in people {
		let slug = person.persona.slug();
		let retained = shared.inbox.messages_for(person.email.as_inner());

		if let Some(lost_persona) = DemoInbox::unavailable(
			person.persona,
			person.last_receipt_id.as_deref(),
			person.last_receipt_at,
			&retained,
		) {
			lost_json.push(serde_json::json!({
				"persona": lost_persona.slug(),
				"sent_at": person
					.last_receipt_at
					.expect("unavailable requires a delivery time"),
			}));
		}

		for stored in &retained {
			messages_json.push(serde_json::json!({
				"persona": slug,
				"to": person.email.as_inner(),
				"subject": stored.subject,
				"text": stored.text,
				"receipt": stored.receipt,
				"sent_at": stored.sent_at,
			}));
		}

		people_json.push(serde_json::json!({
			"persona": slug,
			"name": person.persona.name(&display_tag(&person.email)),
			"tag": display_tag(&person.email),
			"email": person.email.as_inner(),
			"signed_in": match (person.account_id, session_account) {
				(Some(account), Some(session)) => account == session,
				_ => false,
			},
		}));
	}

	let body = serde_json::json!({
		"day": shared.visitors.day(),
		"people": people_json,
		"messages": messages_json,
		"lost": lost_json,
	});

	(StatusCode::OK, no_store, axum::Json(body)).into_response()
}

/// What one request's cookies and session say about the visitor, resolved
/// once for a page.
struct Visit {
	/// This pass's people, Ada first; empty without a valid pass.
	people: Vec<Held>,
	/// The pass cookie named an earlier UTC day: the demo reset since.
	earlier_day: bool,
	/// A cookie the browser carried that names no pass here, so the
	/// response clears it.
	stale_cookie: bool,
}

/// A person the visitor's pass owns, and whether the request is signed in
/// as them.
struct Held {
	person: DemoPerson,
	signed_in: bool,
}

impl Held {
	fn view(&self) -> pages::Person {
		person_view(&self.person, self.signed_in)
	}

	/// The person as the hydrated pages show them.
	fn person(&self) -> model::Person {
		let tag = display_tag(&self.person.email);
		model::Person {
			persona: persona_of(self.person.persona),
			name: self.person.persona.name(&tag),
			tag,
			email: self.person.email.as_inner().to_owned(),
			signed_in: self.signed_in,
		}
	}
}

/// The pages' name for a persona.
const fn persona_of(persona: Persona) -> model::Persona {
	match persona {
		Persona::Ada => model::Persona::Ada,
		Persona::Grace => model::Persona::Grace,
	}
}

/// A person as the pages show them. Nothing here is a secret: the
/// address is this browser's own, which only its pass can sign in to.
fn person_view(person: &DemoPerson, signed_in: bool) -> pages::Person {
	let tag = display_tag(&person.email);
	pages::Person {
		persona: person.persona.slug(),
		name: person.persona.name(&tag),
		tag,
		email: person.email.as_inner().to_owned(),
		signed_in,
	}
}

impl Visit {
	/// The person the request is signed in as, if any.
	fn signed_in(&self) -> Option<&Held> {
		self.people.iter().find(|held| held.signed_in)
	}

	/// This pass's person for a persona, if it owns one.
	fn person(&self, persona: Persona) -> Option<&Held> {
		self.people
			.iter()
			.find(|held| held.person.persona == persona)
	}
}

/// Resolves the request's pass and, with it, which of the pass's people
/// the session is signed in as.
async fn visit(shared: &Parts, headers: &HeaderMap, session: Option<&SignedIn>) -> Result<Visit> {
	let state = shared.visitors.resolve(headers).await?;
	let carried = carried_cookie(headers, shared.visitors.cookie().name).is_some();

	let (people, earlier_day) = match state {
		PassState::Valid(pass) => {
			let mut people = shared.visitors.people(&pass).await?;
			// Ada first, always, so the pages never reorder.
			people.sort_by_key(|person| person.persona != Persona::Ada);
			let held = people
				.into_iter()
				.map(|person| {
					let signed_in = matches!(
						(person.account_id, session),
						(Some(account), Some(session)) if account == session.account
					);
					Held { person, signed_in }
				})
				.collect();
			(held, false)
		}
		PassState::EarlierDay => (Vec::new(), true),
		PassState::Absent => (Vec::new(), false),
	};

	Ok(Visit {
		stale_cookie: carried && people.is_empty(),
		people,
		earlier_day,
	})
}

/// What every page knows: the generation's day and the server's clock,
/// and the admin's address from configuration.
fn page_context(shared: &Parts) -> pages::Context {
	let host_of = |origin: &str| {
		origin
			.split_once("://")
			.map_or(origin, |(_, host)| host)
			.trim_end_matches('/')
			.to_owned()
	};

	pages::Context {
		day: shared.visitors.day().to_owned(),
		now_ms: crate::clock::now_millis(),
		site_host: host_of(&shared.origin),
		admin_host: host_of(&shared.admin_origin),
		admin_url: format!(
			"{}{}/",
			shared.admin_origin.trim_end_matches('/'),
			leptoskit_admin::BASE
		),
	}
}

/// What every hydrated page knows: the generation's day, the server's
/// clock and the reset it counts down to, and the admin's address from
/// configuration.
fn context_of(shared: &Parts) -> model::Context {
	let context = page_context(shared);
	let resets_at_ms = chrono::NaiveDate::parse_from_str(&context.day, "%Y-%m-%d")
		.ok()
		.and_then(|day| day.succ_opt())
		.and_then(|next| next.and_hms_opt(0, 0, 0))
		.map_or(context.now_ms, |midnight| {
			midnight.and_utc().timestamp_millis()
		});

	model::Context {
		day: context.day,
		now_ms: context.now_ms,
		resets_at_ms,
		site_host: context.site_host,
		admin_host: context.admin_host,
		admin_url: context.admin_url,
	}
}

/// The first value of each query parameter a page reads. Unknown
/// parameters are ignored and nothing in the query is trusted: callers
/// validate what they use.
struct Params(Vec<(String, String)>);

impl Params {
	fn of(query: Option<&str>) -> Self {
		let mut seen = Vec::<(String, String)>::new();

		for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
			if !seen.iter().any(|(known, _)| *known == name) {
				seen.push((name.into_owned(), value.into_owned()));
			}
		}

		Self(seen)
	}

	fn get(&self, name: &str) -> Option<&str> {
		self.0
			.iter()
			.find(|(known, _)| known == name)
			.map(|(_, value)| value.as_str())
	}

	/// The return target, validated: anything unusable is the fallback.
	fn return_to(&self) -> String {
		self.get("return_to").map_or_else(
			|| return_to::FALLBACK.to_owned(),
			return_to::local_return_to,
		)
	}
}

/// A page response: HTML, never stored, clearing a stale pass cookie when
/// the browser carried one.
fn html_page(body: String, clear: Option<axum::http::HeaderValue>) -> Response {
	let mut response = (
		StatusCode::OK,
		[
			(header::CONTENT_TYPE, "text/html; charset=utf-8"),
			(header::CACHE_CONTROL, "no-store"),
		],
		body,
	)
		.into_response();

	if let Some(cookie) = clear {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
}

/// A `303` to a local path, never stored.
fn see_other(location: &str, clear: Option<axum::http::HeaderValue>) -> Response {
	let mut response = (
		StatusCode::SEE_OTHER,
		[
			(header::LOCATION, location.to_owned()),
			(header::CACHE_CONTROL, "no-store".to_owned()),
		],
		axum::body::Body::empty(),
	)
		.into_response();

	if let Some(cookie) = clear {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
}

/// The cookie that clears a stale pass, when this visit carried one.
fn clearing(shared: &Parts, visit: &Visit) -> Option<axum::http::HeaderValue> {
	(visit.stale_cookie || visit.earlier_day).then(|| shared.visitors.cookie().clear())
}

/// `GET /`: the front door.
async fn home(
	State(shared): State<Shared>,
	headers: HeaderMap,
	RawQuery(query): RawQuery,
	session: Option<SignedIn>,
) -> Response {
	let shared = shared.0;
	let params = Params::of(query.as_deref());
	let visit = match visit(&shared, &headers, session.as_ref()).await {
		Ok(visit) => visit,
		Err(error) => return no_store(internal_error(&error)),
	};

	let notice = if visit.earlier_day {
		model::Notice::NewDay
	} else if params.get("signed_out") == Some("1")
		&& !visit.people.is_empty()
		&& visit.signed_in().is_none()
	{
		model::Notice::SignedOut
	} else {
		model::Notice::None
	};

	let door = model::Door {
		context: context_of(&shared),
		return_to: params.return_to(),
		notice,
		people: visit.people.iter().map(Held::person).collect(),
	};
	let before = demo_pages::noscript_form(&door.return_to);

	page(
		&shared,
		model::Page::Front(door),
		before,
		clearing(&shared, &visit),
	)
	.await
}

/// A page response: the document, never stored, clearing a stale pass
/// cookie when the browser carried one.
async fn page(
	shared: &Parts,
	page: model::Page,
	before: String,
	clear: Option<axum::http::HeaderValue>,
) -> Response {
	let mut response = no_store(shared.client.render(page, before).await);

	if let Some(cookie) = clear {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
}

/// `GET /sign-in`: choose, or sign in as the person this pass minted.
///
/// A visitor who is already signed in goes straight on to their return
/// target, except for the admin's `/oauth/authorize`: that request is
/// asking for a fresh sign-in (`max_age`), so the page shows, and the
/// identity provider would send the visitor straight back if it
/// redirected.
async fn sign_in(
	State(shared): State<Shared>,
	headers: HeaderMap,
	RawQuery(query): RawQuery,
	session: Option<SignedIn>,
) -> Response {
	let shared = shared.0;
	let params = Params::of(query.as_deref());
	let visit = match visit(&shared, &headers, session.as_ref()).await {
		Ok(visit) => visit,
		Err(error) => return no_store(internal_error(&error)),
	};

	let target = params.return_to();
	let oauth = target.split('?').next() == Some("/oauth/authorize");
	let signed_in = visit.signed_in();

	if signed_in.is_some() && !oauth {
		return see_other(&target, None);
	}

	let requested = params.get("as").and_then(Persona::of_form);

	// The person this page signs in: the one asked for when the pass owns
	// them, the pass's only person when it owns just one, and otherwise
	// the visitor chooses.
	let chosen = match requested {
		Some(persona) => visit.person(persona),
		// Someone is signed in and the admin asked who is signing in: they
		// pick, even when the pass owns one person.
		None if signed_in.is_some() => None,
		None if visit.people.len() == 1 => visit.people.first(),
		None => None,
	};

	let preselect = requested
		.or_else(|| {
			oauth
				.then_some(Persona::Ada)
				.filter(|_| signed_in.is_some())
		})
		.map(Persona::slug);

	let flow = match (oauth, signed_in) {
		(false, _) => pages::Flow::Plain,
		(true, None) => pages::Flow::Admin,
		(true, Some(held)) => pages::Flow::Switch(held.view()),
	};

	let mail = chosen
		.map(|held| mail_for(&shared, &held.person))
		.unwrap_or_default();

	let view = pages::SignIn {
		context: page_context(&shared),
		return_to: target,
		flow,
		identity: chosen.map(Held::view),
		preselect,
		new_day: visit.earlier_day,
		mail,
	};

	html_page(pages::sign_in(&view), clearing(&shared, &visit))
}

/// What this process holds for one person, without the messages: how
/// many it still has, and whether the person's latest code is
/// unavailable here.
fn mail_for(shared: &Parts, person: &DemoPerson) -> pages::Mail {
	let retained = shared.inbox.messages_for(person.email.as_inner());
	let lost = DemoInbox::unavailable(
		person.persona,
		person.last_receipt_id.as_deref(),
		person.last_receipt_at,
		&retained,
	)
	.and(person.last_receipt_at);

	pages::Mail {
		held: retained.len(),
		lost_at_ms: lost.map(|at| at.timestamp_millis()),
	}
}

/// `GET /notes`: the welcome for the account the request is signed in as.
/// Anyone else is sent to sign in, then back here.
async fn notes(
	State(shared): State<Shared>,
	headers: HeaderMap,
	session: Option<SignedIn>,
) -> Response {
	let shared = shared.0;
	let visit = match visit(&shared, &headers, session.as_ref()).await {
		Ok(visit) => visit,
		Err(error) => return no_store(internal_error(&error)),
	};

	let Some(held) = visit.signed_in() else {
		let location = format!(
			"/sign-in?return_to={}",
			return_to::encoded_query_value(return_to::FALLBACK)
		);
		return see_other(&location, clearing(&shared, &visit));
	};

	let welcome = pages::Welcome {
		context: page_context(&shared),
		person: held.view(),
	};

	html_page(pages::notes(&welcome), None)
}

/// The person a notes request is for: the pass's person whose real
/// account the request's session is signed in as.
struct Owner {
	person: leptoskit_db::Id<DemoPerson>,
	persona: Persona,
	who: SignedIn,
}

/// Resolves who a notes request is for, from the pass cookie and the
/// session and nothing the caller sends: the pass must be this
/// generation's, and the session must be the account one of its people
/// claimed. Anything else is a `401` that names only what kind of visit
/// it was, never anyone's data.
async fn owner(
	shared: &Parts,
	headers: &HeaderMap,
	session: Option<&SignedIn>,
) -> Result<Owner, Box<Response>> {
	let visit = visit(shared, headers, session)
		.await
		.map_err(|error| Box::new(no_store(internal_error(&error))))?;

	let held = session.and_then(|session| {
		visit
			.signed_in()
			.map(|held| (held.person.id, held.person.persona, session.clone()))
	});
	let Some((person, persona, who)) = held else {
		let state = if visit.earlier_day {
			"new_day"
		} else if visit.people.is_empty() {
			"absent"
		} else {
			"signed_out"
		};
		let mut body = serde_json::json!({ "error": "no_visit", "pass": state });
		if visit.earlier_day {
			body["day"] = serde_json::Value::String(shared.visitors.day().to_owned());
		}
		return Err(Box::new(notes_response(
			StatusCode::UNAUTHORIZED,
			body,
			clearing(shared, &visit),
		)));
	};

	Ok(Owner {
		person,
		persona,
		who,
	})
}

/// A notes answer: JSON, never stored, clearing a stale pass cookie when
/// the browser carried one.
fn notes_response(
	status: StatusCode,
	body: serde_json::Value,
	clear: Option<axum::http::HeaderValue>,
) -> Response {
	let mut response = no_store((status, axum::Json(body)).into_response());

	if let Some(cookie) = clear {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
}

/// A refusal with a stable code and nothing else.
fn notes_error(status: StatusCode, code: &str) -> Response {
	notes_response(status, serde_json::json!({ "error": code }), None)
}

/// A note as the page shows it.
fn note_json(note: &notes::DemoNote) -> serde_json::Value {
	serde_json::json!({
		"id": note.id.to_string(),
		"text": note.text,
		"created_at": note.created_at.timestamp_millis(),
	})
}

/// `GET /demo/notes`: this signed-in person's notes, newest first, at most
/// the notebook's cap. The account is the session's; nothing in the
/// request can name another.
async fn list_notes(
	State(shared): State<Shared>,
	headers: HeaderMap,
	session: Option<SignedIn>,
) -> Response {
	let shared = shared.0;
	let owner = match owner(&shared, &headers, session.as_ref()).await {
		Ok(owner) => owner,
		Err(refusal) => return *refusal,
	};

	let notes = match shared.notes.list(&owner.who).await {
		Ok(notes) => notes,
		Err(error) => return no_store(internal_error(&error)),
	};

	notes_response(
		StatusCode::OK,
		serde_json::json!({
			"notes": notes.iter().map(note_json).collect::<Vec<_>>(),
			"limit": notes::MAX_NOTES,
			"max_chars": notes::MAX_CHARS,
			"actor": crate::activity::actor(owner.person),
		}),
		None,
	)
}

/// What `POST /demo/notes` accepts: the text, and nothing else.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewNote {
	text: String,
}

/// Whether the request's one `Content-Type` is `application/json`,
/// parameters aside.
fn is_json(headers: &HeaderMap) -> bool {
	let mut types = headers.get_all(header::CONTENT_TYPE).iter();
	let Some(first) = types.next() else {
		return false;
	};

	types.next().is_none()
		&& first.to_str().is_ok_and(|value| {
			value
				.split(';')
				.next()
				.is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
		})
}

/// `POST /demo/notes`: saves one private note for the signed-in person.
///
/// The order is the contract. A cross-origin request is refused first
/// (`403`); then the visit must be a real one (`401`) before the body is
/// read at all; then the media type (`415`), the 4 KiB bound (`413`), the
/// JSON shape with no unknown fields (`400`), and the text's length
/// (`422`). Only a request that passes every one reaches the notebook,
/// where a full notebook is `409 notes_full`. A refusal has no note and no
/// event as a side effect.
///
/// The note commits first, in its own transaction. Its event is then sent
/// once, and the answer says whether the analytics service confirmed it:
/// the note is saved either way, and nothing sends the event again.
async fn create_note(
	State(shared): State<Shared>,
	headers: HeaderMap,
	session: Option<SignedIn>,
	body: axum::body::Body,
) -> Response {
	let shared = shared.0;

	if !same_origin(&headers, &shared.origin) {
		return notes_error(StatusCode::FORBIDDEN, "cross_origin");
	}

	let owner = match owner(&shared, &headers, session.as_ref()).await {
		Ok(owner) => owner,
		Err(refusal) => return *refusal,
	};

	if !is_json(&headers) {
		return notes_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type");
	}

	let Ok(bytes) = axum::body::to_bytes(body, MAX_NOTE_BODY).await else {
		return notes_error(StatusCode::PAYLOAD_TOO_LARGE, "too_large");
	};

	let Ok(submitted) = serde_json::from_slice::<NewNote>(&bytes) else {
		return notes_error(StatusCode::BAD_REQUEST, "bad_request");
	};

	let text = match NoteText::parse(&submitted.text) {
		Ok(text) => text,
		Err(error) => return notes_error(StatusCode::UNPROCESSABLE_ENTITY, error.code()),
	};

	let note = match shared.notes.create(&owner.who, &text).await {
		Ok(note) => note,
		Err(CreateError::Full) => return notes_error(StatusCode::CONFLICT, "notes_full"),
		Err(CreateError::Database(error)) => return no_store(internal_error(&error)),
	};

	let actor = crate::activity::actor(owner.person);
	let words = crate::activity::words(text.as_str());
	let delivery = shared
		.analytics
		.note_created(&actor, owner.persona, words)
		.await;

	notes_response(
		StatusCode::CREATED,
		serde_json::json!({
			"note": note_json(&note),
			"words": words,
			"analytics": delivery.as_str(),
			"actor": actor,
		}),
		None,
	)
}

/// The admin's two faces, served from this site: `GET /demo/assets/…`.
async fn font(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
	let bytes = match file.as_str() {
		"sketch.woff2" => SKETCH_FONT,
		"mono.woff2" => MONO_FONT,
		_ => return StatusCode::NOT_FOUND.into_response(),
	};

	(
		[
			(header::CONTENT_TYPE, "font/woff2"),
			(header::CACHE_CONTROL, "public, max-age=86400"),
		],
		bytes,
	)
		.into_response()
}

/// Marks a response `no-store`: an error a page route produced still
/// belongs to one visitor.
fn no_store(mut response: Response) -> Response {
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		axum::http::HeaderValue::from_static("no-store"),
	);
	response
}

/// Whether a mint may act: a present `Origin` must equal the configured
/// site origin; only when `Origin` is absent does `Sec-Fetch-Site:
/// same-origin` stand in. Neither, or a contradictory pair, refuses.
fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
	let mut origins = headers.get_all(header::ORIGIN).iter();
	if let Some(requested) = origins.next() {
		return origins.next().is_none() && requested.to_str().ok() == Some(origin);
	}

	headers
		.get("sec-fetch-site")
		.and_then(|value| value.to_str().ok())
		== Some("same-origin")
}

/// The persona and validated return target a mint form names, bounded by
/// the body limit this handler already applied.
fn form_choice(bytes: &[u8]) -> Option<(Persona, String)> {
	let mut persona = None;
	let mut return_target = None;

	for (field, value) in url::form_urlencoded::parse(bytes) {
		match field.as_ref() {
			"persona" if persona.is_none() => persona = Persona::of_form(&value),
			"return_to" if return_target.is_none() => {
				return_target = Some(return_to::local_return_to(&value));
			}
			_ => {}
		}
	}

	Some((
		persona?,
		return_target.unwrap_or_else(|| return_to::FALLBACK.to_owned()),
	))
}

/// The normalized address a gated body names, or nothing: the gate only
/// needs the address, never the rest of Auth's request.
fn json_email(bytes: &[u8]) -> Option<String> {
	let parsed: serde_json::Value = serde_json::from_slice(bytes).ok()?;
	parsed
		.get("email")?
		.as_str()
		.map(|email| email.trim().to_ascii_lowercase())
}

/// The peer address the server recorded, when it serves with one.
fn peer(parts: &axum::http::request::Parts) -> Option<SocketAddr> {
	parts
		.extensions
		.get::<ConnectInfo<SocketAddr>>()
		.map(|ConnectInfo(peer)| *peer)
}

/// A stable JSON refusal: no message text, only a code a page can name.
fn json_error(status: StatusCode, code: &str) -> Response {
	(status, axum::Json(serde_json::json!({ "error": code }))).into_response()
}

/// A database failure: the visitor hears only that the server failed,
/// and the reason stays in the logs.
fn internal_error(error: &anyhow::Error) -> Response {
	eprintln!("the demo site's request failed: {error:#}");
	json_error(StatusCode::INTERNAL_SERVER_ERROR, "server")
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
/// observable barrier: the receipt is checked against the service's
/// typed answers (every entry accounted, nothing refused or
/// rate-limited), and the read-back must match it exactly — a fresh
/// generation serves exactly the month it was seeded with, no more and
/// no less.
///
/// # Errors
///
/// When ingestion refuses a batch, when the service's answers do not
/// account for every entry, or when the read-back count differs from the
/// receipt's entries.
pub async fn seed_with_barrier(router: &Router, token: &str, now_millis: i64) -> Result<i64> {
	let receipt = admin_demo::analytics::seed_with(router.clone(), token, now_millis)
		.await
		.context("seeding the analytics service")?;
	anyhow::ensure!(
		receipt.entries > 0 && receipt.events > 0 && receipt.identities > 0,
		"the seed barrier failed: the month sent no traffic: {receipt:?}"
	);
	anyhow::ensure!(
		receipt.stored + receipt.duplicates == receipt.entries,
		"the seed barrier failed: the service accounted for {} of the {} entries sent: {receipt:?}",
		receipt.stored + receipt.duplicates,
		receipt.entries
	);

	// The exact initial read-back: the catalogue counts every accepted
	// entry — events and identity entries both — and a fresh generation
	// holds exactly the month it was sent.
	let counted = count_events(router, token).await?;
	anyhow::ensure!(
		i64::try_from(receipt.entries).unwrap_or(i64::MAX) == counted,
		"the seed barrier failed: {} entries were accepted and the service reads back {counted} \
		 stored",
		receipt.entries
	);
	Ok(counted)
}

/// The service's own count of stored events, read through its read route
/// — the durable, observable side of the seed barrier. A warm open calls
/// this as a healthy read: it proves the generation's analytics answer,
/// and deliberately checks nothing against the marker's historical
/// count, which visitors' events and deletions move.
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
				.header(header::ACCEPT, "application/json")
				.header(header::CONTENT_TYPE, "application/json")
				.header(header::AUTHORIZATION, format!("Bearer {token}"))
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn malformed_or_duplicate_origins_cannot_use_fetch_metadata() {
		let origin = "https://notes.example";
		let mut headers = HeaderMap::new();
		headers.insert("sec-fetch-site", "same-origin".parse().expect("header"));
		assert!(same_origin(&headers, origin));
		headers.insert(
			header::ORIGIN,
			header::HeaderValue::from_bytes(b"\xff").expect("header"),
		);
		assert!(!same_origin(&headers, origin));
		headers.insert(header::ORIGIN, origin.parse().expect("header"));
		assert!(same_origin(&headers, origin));
		headers.append(
			header::ORIGIN,
			"https://foreign.example".parse().expect("header"),
		);
		assert!(!same_origin(&headers, origin));
	}
}
