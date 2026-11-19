//! The admin's routes. Every page and endpoint under [`BASE`] checks that
//! the person is signed in and holds one of the admin roles; actions go
//! to the control service under their name.

use std::collections::HashMap;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use leptoskit_admin_pages::model::Page;
use leptoskit_auth::SignedIn;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::EnqueueError;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Principal;
use leptoskit_deploy_console::control::Rejection;
use leptoskit_deploy_console::control::Service;
use serde::Deserialize;
use serde_json::json;

use crate::Admin;
use crate::BASE;
use crate::access;
use crate::analytics;
use crate::infra;
use crate::infra::ProjectState;
use crate::learn;
use crate::shell;
use crate::shell::Screen;
use crate::shell::Viewer;
use crate::signin;

const SCRIPT: &str = include_str!("../assets/admin.mjs");
const SKETCH: &[u8] = include_bytes!("../assets/fonts/SkribbleNotoSketch-Latin.woff2");
const MONO: &[u8] = include_bytes!("../assets/fonts/SkribbleMonoPlayful-Latin.woff2");

pub(crate) fn router(admin: Admin) -> Router {
	let auth = admin.inner().auth.clone();
	let at = |path: &str| format!("{BASE}{path}");

	Router::new()
		.route("/", get(home))
		.route(BASE, get(home))
		.route(&at("/"), get(home))
		.route(&at("/sign-in"), get(sign_in))
		.route(&at("/infra/projects"), get(projects))
		.route(&at("/infra/projects/{id}"), get(project_home))
		.route(&at("/infra/projects/{id}/{tab}"), get(project))
		.route(&at("/infra/operations/{id}"), get(operation))
		.route(&at("/api/projects/{id}/actions"), post(action))
		.route(&at("/api/preferences"), post(preferences))
		.route(&at("/learn"), get(learn_hub))
		.route(&at("/learn/glossary"), get(glossary))
		.route(&at("/learn/{track}/{lesson}"), get(lesson))
		.route(&at("/assets/admin.mjs"), get(script))
		.route(&at("/pkg/{file}"), get(client_file))
		.route(&at("/assets/sketch.woff2"), get(|| async { font(SKETCH) }))
		.route(&at("/assets/mono.woff2"), get(|| async { font(MONO) }))
		.merge(analytics::routes::router())
		.merge(crate::live::router(&admin))
		.merge(auth.router())
		.layer(Extension(admin))
		.layer(auth.layer())
}

pub(crate) fn now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

/// The person's chosen appearance, from the `admin_theme` cookie.
pub(crate) fn appearance(headers: &HeaderMap) -> Option<String> {
	headers
		.get_all(header::COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(';'))
		.filter_map(|pair| pair.trim().split_once('='))
		.find(|(name, _)| *name == "admin_theme")
		.map(|(_, value)| value.to_owned())
		.filter(|value| value == "light" || value == "dark")
}

pub(crate) fn no_store(mut response: Response) -> Response {
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		"no-store".parse().expect("a valid header"),
	);
	response
}

pub(crate) fn html(body: String) -> Response {
	no_store(Html(body).into_response())
}

/// A signed-in admin, or the response that turns everyone else away.
pub(crate) struct Gate {
	pub who: String,
	pub preferences: access::Preferences,
}

#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request; boxing it buys nothing"
)]
pub(crate) async fn gate(
	admin: &Admin,
	me: Option<SignedIn>,
	headers: &HeaderMap,
) -> Result<Gate, Response> {
	let inner = admin.inner();
	let Some(me) = me else {
		return Err(Redirect::to(&format!("{BASE}/sign-in")).into_response());
	};
	let roles = inner
		.auth
		.roles(me.account)
		.await
		.map_err(|error| failure(&error.to_string()))?;
	let who = access::display_name(&inner.db, me.account)
		.await
		.map_err(|error| failure(&error.to_string()))?;

	if !roles.iter().any(|role| inner.roles.contains(role)) {
		return Err(signin::render(admin, signin::Outcome::Refused(Some(who)), headers).await);
	}

	access::welcome(&inner.db, me.account)
		.await
		.map_err(|error| failure(&error.to_string()))?;
	let preferences = access::preferences(&inner.db, me.account)
		.await
		.map_err(|error| failure(&error.to_string()))?;

	Ok(Gate { who, preferences })
}

/// Whether an API caller is a signed-in admin.
#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request; boxing it buys nothing"
)]
pub(crate) async fn api_gate(
	admin: &Admin,
	me: Option<SignedIn>,
) -> Result<(SignedIn, String), Response> {
	let inner = admin.inner();
	let Some(me) = me else {
		return Err(problem(
			StatusCode::UNAUTHORIZED,
			"signed_out",
			"Sign in to the admin first.",
		));
	};
	let roles = inner.auth.roles(me.account).await.map_err(|error| {
		problem(
			StatusCode::INTERNAL_SERVER_ERROR,
			"roles",
			&error.to_string(),
		)
	})?;

	if !roles.iter().any(|role| inner.roles.contains(role)) {
		return Err(problem(
			StatusCode::FORBIDDEN,
			"not_admin",
			"This account isn't an admin. The main site decides who is.",
		));
	}

	let who = access::display_name(&inner.db, me.account)
		.await
		.map_err(|error| {
			problem(
				StatusCode::INTERNAL_SERVER_ERROR,
				"database",
				&error.to_string(),
			)
		})?;
	Ok((me, who))
}

pub(crate) fn problem(status: StatusCode, code: &str, message: &str) -> Response {
	no_store((status, Json(json!({ "error": code, "message": message }))).into_response())
}

pub(crate) fn failure(message: &str) -> Response {
	(
		StatusCode::INTERNAL_SERVER_ERROR,
		format!("The admin couldn't answer: {message}"),
	)
		.into_response()
}

/// Every project and its journal, read in one go.
pub(crate) async fn states(admin: &Admin) -> Result<Vec<ProjectState>, String> {
	let Some(service) = admin.inner().control.clone() else {
		return Ok(Vec::new());
	};
	tokio::task::spawn_blocking(move || {
		let overview = service
			.overview(&Principal::owner())
			.map_err(|error| error.to_string())?;
		Ok(overview
			.projects
			.into_iter()
			.map(|project| ProjectState::load(project, &overview.operations))
			.collect())
	})
	.await
	.map_err(|error| error.to_string())?
}

/// The analytics facts lessons are checked against, when the admin is
/// connected and the service answers.
pub(crate) async fn analytics_facts(admin: &Admin) -> Option<analytics::Facts> {
	let connection = admin.inner().analytics.as_ref()?;
	connection.facts().await.ok()
}

/// Renders a signed-in screen.
pub(crate) fn render(
	admin: &Admin,
	gate: &Gate,
	headers: &HeaderMap,
	proof: &learn::Proof<'_>,
	screen: Screen,
) -> Response {
	html(shell::page(&viewer(admin, gate, headers, proof), screen))
}

/// Who is looking at a signed-in screen, and how they like the admin.
fn viewer(admin: &Admin, gate: &Gate, headers: &HeaderMap, proof: &learn::Proof<'_>) -> Viewer {
	let inner = admin.inner();
	Viewer {
		app_name: inner.app_name.clone(),
		environment: inner.environment,
		simulated: inner.simulated_infrastructure,
		who: gate.who.clone(),
		workings_mode: gate.preferences.workings_mode,
		appearance: appearance(headers),
		projects: proof
			.states
			.iter()
			.map(|state| state.project.id.clone())
			.collect(),
		progress: learn::progress(proof),
		pulse: inner
			.analytics
			.as_ref()
			.map(|connection| connection.live().pulse().shown()),
	}
}

async fn home(Extension(admin): Extension<Admin>, me: Option<SignedIn>) -> Response {
	if me.is_none() {
		return Redirect::to(&format!("{BASE}/sign-in")).into_response();
	}

	let states = states(&admin).await.unwrap_or_default();
	match states.as_slice() {
		[only] => {
			Redirect::to(&format!(
				"{BASE}/infra/projects/{}/overview",
				only.project.id
			))
			.into_response()
		}
		_ => Redirect::to(&format!("{BASE}/infra/projects")).into_response(),
	}
}

#[derive(Deserialize)]
struct SignInQuery {
	auth_error: Option<String>,
}

async fn sign_in(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Query(query): Query<SignInQuery>,
) -> Response {
	let inner = admin.inner();

	if let Some(me) = me {
		let roles = inner.auth.roles(me.account).await.unwrap_or_default();

		if roles.iter().any(|role| inner.roles.contains(role)) {
			return Redirect::to(&format!("{BASE}/")).into_response();
		}

		let who = access::display_name(&inner.db, me.account).await.ok();
		return signin::render(&admin, signin::Outcome::Refused(who), &headers).await;
	}

	let outcome = match query.auth_error.as_deref() {
		Some("not_allowed") => signin::Outcome::Refused(None),
		Some(code) => {
			signin::Outcome::Failed(
				code.chars()
					.filter(|character| character.is_ascii_alphanumeric() || *character == '_')
					.take(64)
					.collect(),
			)
		}
		None => signin::Outcome::Ready,
	};
	signin::render(&admin, outcome, &headers).await
}

/// Gates, loads the projects, and renders the page `page` makes of them:
/// a signed-in page `leptoskit_admin_pages` builds, which the browser picks
/// up where the server left it. `open` names the drawer open on arrival.
async fn hydrated_screen(
	admin: &Admin,
	me: Option<SignedIn>,
	headers: &HeaderMap,
	open: Option<String>,
	page: impl FnOnce(&learn::Proof<'_>) -> Option<Page>,
) -> Response {
	let gate = match gate(admin, me, headers).await {
		Ok(gate) => gate,
		Err(response) => return response,
	};
	let states = match states(admin).await {
		Ok(states) => states,
		Err(error) => return failure(&format!("the control service didn't answer: {error}")),
	};
	let facts = analytics_facts(admin).await;
	let proof = learn::Proof {
		states: &states,
		now: now(),
		facts: facts.as_ref(),
		preferences: &gate.preferences,
		simulated: admin.inner().simulated_infrastructure,
	};
	let Some(page) = page(&proof) else {
		return not_found();
	};

	render_page(admin, &gate, headers, &proof, page, open).await
}

/// Renders `page` from the pages crate, with the drawer `open` names open
/// when the page has it.
pub(crate) async fn render_page(
	admin: &Admin,
	gate: &Gate,
	headers: &HeaderMap,
	proof: &learn::Proof<'_>,
	page: Page,
	open: Option<String>,
) -> Response {
	// Only a drawer the page has opens: the id goes back into the page.
	let open = open
		.or_else(|| opens_by_default(&page))
		.filter(|open| page.drawers().iter().any(|drawer| drawer.id == *open));
	let model = leptoskit_admin_pages::Model {
		viewer: viewer(admin, gate, headers, proof),
		page,
		open,
	};

	admin.inner().client.render(model).await
}

/// The drawer a page opens when no link names one: the plan's, the first
/// three times.
fn opens_by_default(page: &Page) -> Option<String> {
	let Page::Project(project) = page else {
		return None;
	};
	let leptoskit_admin_pages::model::project::Body::Changes(changes) = &project.body else {
		return None;
	};

	changes.workings_by_default.then(|| "plan".to_owned())
}

async fn projects(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	hydrated_screen(&admin, me, &headers, None, |proof| {
		Some(Page::Projects(infra::projects(proof.states, proof.now)))
	})
	.await
}

async fn project_home(Path(id): Path<String>) -> Response {
	Redirect::to(&format!("{BASE}/infra/projects/{id}/overview")).into_response()
}

async fn project(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path((id, tab)): Path<(String, String)>,
	Query(query): Query<WorkingsQuery>,
) -> Response {
	let Some(tab) = infra::Tab::parse(&tab) else {
		return not_found();
	};

	let linked = query.workings.is_some();

	hydrated_screen(&admin, me, &headers, query.workings, |proof| {
		let state = proof.states.iter().find(|state| state.project.id == id)?;
		// Only Health draws the fleet's last hour of measurements.
		let fleet = (tab == infra::Tab::Health)
			.then(|| admin.inner().control.as_ref())
			.flatten()
			.map(|service| {
				service
					.fleet(
						&Principal::owner(),
						&id,
						proof.now.saturating_sub(3600),
						proof.now,
					)
					.map_err(|error| error.to_string())
			});
		let context = infra::Context {
			now: proof.now,
			preferences: proof.preferences,
			fleet,
			simulated: proof.simulated,
			linked,
		};
		Some(Page::Project(Box::new(infra::project(
			state, tab, &context,
		))))
	})
	.await
}

async fn operation(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(id): Path<String>,
	Query(query): Query<WorkingsQuery>,
) -> Response {
	let Some(service) = admin.inner().control.clone() else {
		return not_found();
	};
	hydrated_screen(&admin, me, &headers, query.workings, |proof| {
		let operation = service.operation(&Principal::owner(), &id).ok()?;
		let state = proof
			.states
			.iter()
			.find(|state| state.project.id == operation.project)?;
		Some(Page::Operation(Box::new(infra::operation(
			state, &operation, proof.now,
		))))
	})
	.await
}

/// `?workings=plan`: a link that opens a card with its workings showing.
#[derive(Deserialize)]
struct WorkingsQuery {
	workings: Option<String>,
}

/// What the page asks the control service to do.
#[derive(Debug, Deserialize)]
pub(crate) struct ActionRequest {
	pub kind: String,
	#[serde(default)]
	pub version: Option<String>,
	#[serde(default)]
	pub plan_id: Option<String>,
	pub idempotency_key: String,
}

impl ActionRequest {
	fn action(&self) -> Result<Action, String> {
		let version = || {
			self.version
				.clone()
				.map(|version| version.trim().to_owned())
				.filter(|version| !version.is_empty())
				.ok_or_else(|| "Type an image tag or a git commit first.".to_owned())
		};

		Ok(match self.kind.as_str() {
			"plan" => Action::Plan,
			"apply" => {
				Action::Apply {
					plan_id: self
						.plan_id
						.clone()
						.ok_or("Apply needs the plan to apply.")?,
				}
			}
			"deploy" => {
				Action::Deploy {
					version: version()?,
					setup: false,
					maintenance: false,
				}
			}
			"first_release" => {
				Action::Deploy {
					version: version()?,
					setup: true,
					maintenance: false,
				}
			}
			"maintenance" => {
				Action::Deploy {
					version: version()?,
					setup: false,
					maintenance: true,
				}
			}
			"rollback" => {
				Action::Rollback {
					version: version()?,
				}
			}
			"status" => Action::Status,
			"backup" => Action::Backup,
			other => return Err(format!("`{other}` isn't an action the admin knows.")),
		})
	}
}

/// What a refusal means, and what to do about it.
fn explain(rejection: Rejection) -> &'static str {
	match rejection {
		Rejection::InvalidIntent => {
			"That isn't a version the deploy engine accepts: use letters, digits, dots, dashes, \
			 and underscores, or a full git commit."
		}
		Rejection::InvalidActor => "The admin couldn't record who you are, so it didn't send this.",
		Rejection::RevisionConflict => {
			"The project changed since this page loaded. Reload and try again."
		}
		Rejection::IdempotencyConflict => {
			"This request was already sent with different details. Reload and try again."
		}
		Rejection::OperationInProgress => {
			"Another operation is running on this project. It runs one at a time; wait for it \
			 under Operations."
		}
		Rejection::PlanIsNotCurrent => {
			"This plan is out of date: the configuration or state changed after it was made. Plan \
			 again before applying."
		}
		Rejection::PlanAlreadyConsumed => "This plan was already used. Plan again before retrying.",
		Rejection::UnknownProject => "There's no such project.",
	}
}

/// Sends an action to the control service as the signed-in admin.
#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request; boxing it buys nothing"
)]
pub(crate) fn enqueue(
	service: &Service,
	actor: &str,
	project: &str,
	request: &ActionRequest,
) -> Result<String, Response> {
	let action = request
		.action()
		.map_err(|message| problem(StatusCode::BAD_REQUEST, "invalid_action", &message))?;
	let owner = Principal::owner().with_actor(actor);
	let revision = service
		.overview(&owner)
		.map_err(|error| {
			problem(
				StatusCode::SERVICE_UNAVAILABLE,
				"storage",
				&error.to_string(),
			)
		})?
		.projects
		.into_iter()
		.find(|known| known.id == project)
		.map(|known| known.revision)
		.ok_or_else(|| {
			problem(
				StatusCode::NOT_FOUND,
				"not_found",
				"There's no such project.",
			)
		})?;
	let intent = Intent {
		expected_revision: revision,
		idempotency_key: request.idempotency_key.clone(),
		action,
	};

	match service.enqueue(&owner, project, &intent) {
		Ok(operation) => Ok(operation.id),
		Err(EnqueueError::Forbidden) => {
			Err(problem(
				StatusCode::FORBIDDEN,
				"forbidden",
				"The control service refused this action.",
			))
		}
		Err(EnqueueError::Rejected(rejection)) => {
			Err(problem(
				StatusCode::CONFLICT,
				rejection.code(),
				explain(rejection),
			))
		}
		Err(EnqueueError::Storage(error)) => {
			Err(problem(
				StatusCode::SERVICE_UNAVAILABLE,
				"storage",
				&error.to_string(),
			))
		}
	}
}

async fn action(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Path(project): Path<String>,
	Json(request): Json<ActionRequest>,
) -> Response {
	let who = match api_gate(&admin, me).await {
		Ok((_, who)) => who,
		Err(response) => return response,
	};
	let Some(service) = admin.inner().control.clone() else {
		return problem(
			StatusCode::NOT_FOUND,
			"no_control",
			"This admin has no control service.",
		);
	};

	match enqueue(&service, &who, &project, &request) {
		Ok(id) => {
			no_store(
				(
					StatusCode::ACCEPTED,
					Json(
						json!({ "operation": id, "href": format!("{BASE}/infra/operations/{id}") }),
					),
				)
					.into_response(),
			)
		}
		Err(response) => response,
	}
}

async fn preferences(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Json(change): Json<access::PreferenceChange>,
) -> Response {
	let me = match api_gate(&admin, me).await {
		Ok((me, _)) => me,
		Err(response) => return response,
	};

	match access::change(&admin.inner().db, me.account, change).await {
		Ok(preferences) => {
			no_store(
				Json(json!({
					"workings_mode": preferences.workings_mode,
					"notes": preferences
						.notes
						.iter()
						.map(|(id, state)| (id.clone(), state.showing()))
						.collect::<HashMap<_, _>>(),
				}))
				.into_response(),
			)
		}
		Err(crate::AdminError::Invalid(message)) => {
			problem(StatusCode::BAD_REQUEST, "invalid", &message)
		}
		Err(error) => {
			problem(
				StatusCode::INTERNAL_SERVER_ERROR,
				"database",
				&error.to_string(),
			)
		}
	}
}

async fn learn_hub(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	hydrated_screen(&admin, me, &headers, None, |proof| {
		Some(Page::Learn(learn::hub(proof)))
	})
	.await
}

async fn glossary(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	hydrated_screen(&admin, me, &headers, None, |_| Some(Page::Glossary)).await
}

async fn lesson(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path((track, lesson)): Path<(String, String)>,
) -> Response {
	hydrated_screen(&admin, me, &headers, None, |proof| {
		learn::lesson(&track, &lesson, proof).map(Page::Lesson)
	})
	.await
}

/// `GET {BASE}/pkg/{file}`: one file of the pages' browser half.
async fn client_file(
	Extension(admin): Extension<Admin>,
	Path(file): Path<String>,
	headers: HeaderMap,
) -> Response {
	admin.inner().client.serve(&file, &headers)
}

async fn script() -> Response {
	(
		[
			(header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
			(header::CACHE_CONTROL, "no-cache"),
		],
		SCRIPT,
	)
		.into_response()
}

fn font(bytes: &'static [u8]) -> Response {
	(
		[
			(header::CONTENT_TYPE, "font/woff2"),
			(header::CACHE_CONTROL, "public, max-age=604800"),
		],
		bytes,
	)
		.into_response()
}

pub(crate) fn not_found() -> Response {
	(StatusCode::NOT_FOUND, "There's nothing here.").into_response()
}
