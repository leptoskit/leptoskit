//! The admin's routes. Every page and endpoint under [`BASE`] checks that
//! the person is signed in and holds one of the admin roles; actions go
//! to the control service under their name.

use std::collections::HashMap;
use std::convert::Infallible;

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
use axum::response::sse::Event as SseEvent;
use axum::response::sse::KeepAlive;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::post;
use futures::StreamExt as _;
use leptoskit_auth::SignedIn;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::EnqueueError;
use leptoskit_deploy_console::control::FacadeError;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Principal;
use leptoskit_deploy_console::control::Rejection;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Update;
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
		.route(&at("/analytics/{page}"), get(analytics_page))
		.route(&at("/analytics/insights/new"), get(insights))
		.route(&at("/infra/projects"), get(projects))
		.route(&at("/infra/projects/{id}"), get(project_home))
		.route(&at("/infra/projects/{id}/{tab}"), get(project))
		.route(&at("/infra/operations/{id}"), get(operation))
		.route(&at("/infra/operations/{id}/events"), get(operation_events))
		.route(&at("/api/projects/{id}/actions"), post(action))
		.route(&at("/api/preferences"), post(preferences))
		.route(&at("/learn"), get(learn_hub))
		.route(&at("/learn/glossary"), get(glossary))
		.route(&at("/learn/{track}/{lesson}"), get(lesson))
		.route(&at("/assets/admin.mjs"), get(script))
		.route(&at("/assets/sketch.woff2"), get(|| async { font(SKETCH) }))
		.route(&at("/assets/mono.woff2"), get(|| async { font(MONO) }))
		.merge(auth.router())
		.layer(Extension(admin))
		.layer(auth.layer())
}

fn now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

/// The person's chosen appearance, from the `admin_theme` cookie.
fn appearance(headers: &HeaderMap) -> Option<String> {
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

fn no_store(mut response: Response) -> Response {
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		"no-store".parse().expect("a valid header"),
	);
	response
}

fn html(body: String) -> Response {
	no_store(Html(body).into_response())
}

/// A signed-in admin, or the response that turns everyone else away.
struct Gate {
	who: String,
	preferences: access::Preferences,
}

#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request; boxing it buys nothing"
)]
async fn gate(admin: &Admin, me: Option<SignedIn>, headers: &HeaderMap) -> Result<Gate, Response> {
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
		let page = signin::page(
			inner,
			&signin::Outcome::Refused(Some(who)),
			appearance(headers).as_deref(),
		);
		return Err((StatusCode::FORBIDDEN, html(page)).into_response());
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
async fn api_gate(admin: &Admin, me: Option<SignedIn>) -> Result<(SignedIn, String), Response> {
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

fn problem(status: StatusCode, code: &str, message: &str) -> Response {
	no_store((status, Json(json!({ "error": code, "message": message }))).into_response())
}

fn failure(message: &str) -> Response {
	(
		StatusCode::INTERNAL_SERVER_ERROR,
		format!("The admin couldn't answer: {message}"),
	)
		.into_response()
}

/// Every project and its journal, read in one go.
async fn states(admin: &Admin) -> Result<Vec<ProjectState>, String> {
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

/// Renders a signed-in screen.
fn render(
	admin: &Admin,
	gate: &Gate,
	headers: &HeaderMap,
	states: &[ProjectState],
	screen: Screen,
) -> Response {
	let inner = admin.inner();
	let viewer = Viewer {
		app_name: inner.app_name.clone(),
		environment: inner.environment,
		who: gate.who.clone(),
		workings_mode: gate.preferences.workings_mode,
		appearance: appearance(headers),
		projects: states
			.iter()
			.map(|state| state.project.id.clone())
			.collect(),
		progress: learn::progress(states, now()),
	};
	html(shell::page(&viewer, screen))
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
	let appearance = appearance(&headers);

	if let Some(me) = me {
		let roles = inner.auth.roles(me.account).await.unwrap_or_default();

		if roles.iter().any(|role| inner.roles.contains(role)) {
			return Redirect::to(&format!("{BASE}/")).into_response();
		}

		let who = access::display_name(&inner.db, me.account).await.ok();
		let page = signin::page(inner, &signin::Outcome::Refused(who), appearance.as_deref());
		return (StatusCode::FORBIDDEN, html(page)).into_response();
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
	let status = if matches!(outcome, signin::Outcome::Refused(_)) {
		StatusCode::FORBIDDEN
	} else {
		StatusCode::OK
	};

	(
		status,
		html(signin::page(inner, &outcome, appearance.as_deref())),
	)
		.into_response()
}

async fn analytics_page(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(page): Path<String>,
) -> Response {
	let Some(page) = analytics::Page::parse(&page, None) else {
		return not_found();
	};
	signed_in_screen(&admin, me, &headers, |_| Some(analytics::screen(page))).await
}

#[derive(Deserialize)]
struct InsightQuery {
	kind: Option<String>,
}

async fn insights(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Query(query): Query<InsightQuery>,
) -> Response {
	let page = analytics::Page::parse("insights", query.kind.as_deref())
		.unwrap_or(analytics::Page::Trends);
	signed_in_screen(&admin, me, &headers, |_| Some(analytics::screen(page))).await
}

/// Gates, loads the projects, and renders what `screen` makes of them.
async fn signed_in_screen(
	admin: &Admin,
	me: Option<SignedIn>,
	headers: &HeaderMap,
	screen: impl FnOnce(&[ProjectState]) -> Option<Screen>,
) -> Response {
	let gate = match gate(admin, me, headers).await {
		Ok(gate) => gate,
		Err(response) => return response,
	};
	let states = match states(admin).await {
		Ok(states) => states,
		Err(error) => return failure(&format!("the control service didn't answer: {error}")),
	};

	match screen(&states) {
		Some(screen) => render(admin, &gate, headers, &states, screen),
		None => not_found(),
	}
}

async fn projects(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	signed_in_screen(&admin, me, &headers, |states| {
		Some(infra::projects(states, now()))
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
) -> Response {
	let Some(tab) = infra::Tab::parse(&tab) else {
		return not_found();
	};
	let gate = match gate(&admin, me, &headers).await {
		Ok(gate) => gate,
		Err(response) => return response,
	};
	let states = match states(&admin).await {
		Ok(states) => states,
		Err(error) => return failure(&format!("the control service didn't answer: {error}")),
	};
	let Some(state) = states.iter().find(|state| state.project.id == id) else {
		return not_found();
	};
	let now = now();
	let fleet = (tab == infra::Tab::Health)
		.then(|| admin.inner().control.as_ref())
		.flatten()
		.map(|service| {
			service
				.fleet(&Principal::owner(), &id, now.saturating_sub(3600), now)
				.map_err(|error| error.to_string())
		});
	let context = infra::Context {
		now,
		preferences: &gate.preferences,
		fleet,
	};
	let screen = infra::project(state, tab, &context);
	render(&admin, &gate, &headers, &states, screen)
}

async fn operation(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(id): Path<String>,
) -> Response {
	let Some(service) = admin.inner().control.clone() else {
		return not_found();
	};
	signed_in_screen(&admin, me, &headers, |states| {
		let operation = service.operation(&Principal::owner(), &id).ok()?;
		let state = states
			.iter()
			.find(|state| state.project.id == operation.project)?;
		Some(infra::operation(state, &operation, now()))
	})
	.await
}

/// Server-sent events for one operation: its state now, each progress
/// event as it happens, then `done` with the finished operation.
async fn operation_events(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Path(id): Path<String>,
) -> Response {
	if let Err(response) = api_gate(&admin, me).await {
		return response;
	}

	let Some(service) = admin.inner().control.clone() else {
		return problem(
			StatusCode::NOT_FOUND,
			"not_found",
			"There's no control service.",
		);
	};
	// Subscribe before reading, so nothing that happens in between is lost.
	let subscription = service.subscribe(&id);
	let snapshot = match service.operation(&Principal::owner(), &id) {
		Ok(operation) => operation,
		Err(FacadeError::NotFound | FacadeError::Forbidden) => {
			return problem(
				StatusCode::NOT_FOUND,
				"not_found",
				"There's no such operation.",
			);
		}
		Err(error) => {
			return problem(
				StatusCode::SERVICE_UNAVAILABLE,
				"storage",
				&error.to_string(),
			);
		}
	};
	let finished = !matches!(snapshot.phase, Phase::Queued | Phase::Running);
	let first = SseEvent::default()
		.event(if finished { "done" } else { "operation" })
		.json_data(&snapshot)
		.unwrap_or_default();
	let updates = subscription
		.take_while(move |_| std::future::ready(!finished))
		.map(|update| {
			match update {
				Update::Progress { event, .. } => {
					SseEvent::default()
						.event("progress")
						.json_data(&event)
						.unwrap_or_default()
				}
				Update::Finished { operation } => {
					SseEvent::default()
						.event("done")
						.json_data(&operation)
						.unwrap_or_default()
				}
			}
		});
	let stream = futures::stream::once(std::future::ready(first))
		.chain(updates)
		.map(Ok::<_, Infallible>);

	no_store(
		Sse::new(stream)
			.keep_alive(KeepAlive::default())
			.into_response(),
	)
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
	signed_in_screen(&admin, me, &headers, |states| {
		Some(learn::hub(states, now()))
	})
	.await
}

async fn glossary(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	signed_in_screen(&admin, me, &headers, |_| Some(learn::glossary())).await
}

async fn lesson(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path((track, lesson)): Path<(String, String)>,
) -> Response {
	signed_in_screen(&admin, me, &headers, |states| {
		learn::lesson(&track, &lesson, states, now())
	})
	.await
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

fn not_found() -> Response {
	(StatusCode::NOT_FOUND, "There's nothing here.").into_response()
}
