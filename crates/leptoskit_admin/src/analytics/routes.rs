//! The Analytics area's routes. Pages check the person holds an admin role
//! the way every admin page does; the API under `api/analytics` checks it
//! the way every admin action does. Only then does the admin call the
//! analytics service, as the app.

use std::convert::Infallible;
use std::time::Duration;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query as UrlQuery;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use axum::response::sse::Event as SseEvent;
use axum::response::sse::KeepAlive;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::post;
use futures::StreamExt as _;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::service::tables::Tile;
use leptoskit_auth::SignedIn;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use super::Connection;
use super::Context;
use super::Facts;
use super::Shown;
use super::insights;
use super::insights::Kind;
use super::library;
use super::params;
use super::params::Defaults;
use super::params::Params;
use super::people;
use super::tail;
use super::web;
use crate::Admin;
use crate::BASE;
use crate::learn;
use crate::server;
use crate::shell::Section;

pub(crate) fn router() -> Router {
	let at = |path: &str| format!("{BASE}{path}");

	Router::new()
		.route(&at("/analytics/overview"), get(overview))
		.route(&at("/analytics/live"), get(live_page))
		.route(&at("/analytics/trends"), get(trends))
		.route(&at("/analytics/funnels"), get(funnels))
		.route(&at("/analytics/retention"), get(retention))
		.route(&at("/analytics/insights/new"), get(new_insight))
		.route(&at("/analytics/insights/{id}"), get(saved_insight))
		.route(&at("/analytics/actors"), get(actors))
		.route(&at("/analytics/library"), get(library_page))
		.route(&at("/analytics/dashboards/{id}"), get(dashboard))
		.route(&at("/analytics/people"), get(people_page))
		.route(&at("/analytics/people/{id}"), get(person_page))
		.route(&at("/api/analytics/insights"), post(save_insight))
		.route(
			&at("/api/analytics/dashboards"),
			get(list_dashboards).post(add_to_dashboard),
		)
		.route(&at("/api/analytics/people/{id}/export"), get(export_person))
		.route(
			&at("/api/analytics/people/{id}/delete"),
			post(delete_person),
		)
		.route(&at("/api/analytics/live"), get(live_events))
}

/// A signed-in admin, their projects, and what the service holds.
struct Ready {
	gate: server::Gate,
	states: Vec<crate::infra::ProjectState>,
	connection: Option<Connection>,
	facts: Option<Result<Facts, super::Failure>>,
}

#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request"
)]
async fn ready(
	admin: &Admin,
	me: Option<SignedIn>,
	headers: &HeaderMap,
) -> Result<Ready, Response> {
	let gate = server::gate(admin, me, headers).await?;
	let states = server::states(admin)
		.await
		.map_err(|error| server::failure(&format!("the control service didn't answer: {error}")))?;
	let connection = admin.inner().analytics.clone();
	let facts = match &connection {
		Some(connection) => Some(connection.facts().await),
		None => None,
	};
	Ok(Ready {
		gate,
		states,
		connection,
		facts,
	})
}

/// Renders what a route made. A builder changing in place asks for a
/// page's model as JSON instead; anything else is the page itself.
async fn finish(
	admin: &Admin,
	ready: &Ready,
	headers: &HeaderMap,
	open: Option<String>,
	shown: Shown,
) -> Response {
	let proof = learn::Proof {
		states: &ready.states,
		now: server::now(),
		facts: ready.facts.as_ref().and_then(|facts| facts.as_ref().ok()),
		preferences: &ready.gate.preferences,
		simulated: admin.inner().simulated_infrastructure,
	};

	match shown {
		Shown::Screen(screen) => server::render(admin, &ready.gate, headers, &proof, *screen),
		Shown::Page(page) if headers.contains_key(leptoskit_admin_pages::PAGE_HEADER) => {
			server::no_store(Json(*page).into_response())
		}
		Shown::Page(page) => {
			server::render_page(admin, &ready.gate, headers, &proof, *page, open).await
		}
	}
}

/// Gates, connects, and renders what `$body` makes of a [`Context`] named
/// `$cx`, or the teaching state when the admin isn't connected.
macro_rules! analytics_page {
	(
		$admin:expr,
		$me:expr,
		$headers:expr,
		$params:expr,
		$title:expr,
		$section:expr, |
		$cx:ident |
		$body:expr
	) => {{
		let ready = match ready(&$admin, $me, &$headers).await {
			Ok(ready) => ready,
			Err(response) => return response,
		};
		let shown: Option<Shown> = match (&ready.connection, &ready.facts) {
			(Some(connection), Some(Ok(facts))) => {
				let $cx = Context {
					connection,
					params: &$params,
					preferences: &ready.gate.preferences,
					facts,
				};
				($body).map(Shown::from)
			}
			(Some(_), Some(Err(failure))) => {
				Some(super::unreachable($title, $section, failure).into())
			}
			_ => Some(super::not_configured($title, $section).into()),
		};

		match shown {
			Some(shown) => {
				let open = $params.get("workings").cloned();
				finish(&$admin, &ready, &$headers, open, shown).await
			}
			None => server::not_found(),
		}
	}};
}

async fn overview(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"Web analytics",
		Section::Overview,
		|cx| Some(web::page(&cx).await)
	)
}

async fn live_page(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
) -> Response {
	let params = Params::new();
	analytics_page!(admin, me, headers, params, "Live", Section::Live, |cx| {
		Some(tail::page(&cx).await)
	})
}

async fn builder(
	admin: Admin,
	me: Option<SignedIn>,
	headers: HeaderMap,
	params: Params,
	kind: Kind,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		kind.title(),
		kind.section(),
		|cx| Some(insights::page(&cx, kind, None).await)
	)
}

async fn trends(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	builder(admin, me, headers, params, Kind::Trends).await
}

async fn funnels(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	builder(admin, me, headers, params, Kind::Funnels).await
}

async fn retention(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	builder(admin, me, headers, params, Kind::Retention).await
}

/// The old address for a new insight: `insights/new?kind=funnels`.
async fn new_insight(UrlQuery(params): UrlQuery<Params>) -> Response {
	let kind = params
		.get("kind")
		.and_then(|kind| Kind::parse(kind))
		.unwrap_or(Kind::Trends);
	Redirect::to(&format!("{BASE}/analytics/{}", kind.slug())).into_response()
}

async fn saved_insight(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(id): Path<Uuid>,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"Insight",
		Section::Library,
		|cx| {
			match cx.connection.insight(id).await {
				Ok(Some(saved)) => {
					match Kind::of(&saved.query) {
						Some(kind) => Some(insights::page(&cx, kind, Some(saved)).await),
						None => Some(web::page(&cx).await),
					}
				}
				Ok(None) => None,
				Err(failure) => {
					Some(super::unreachable("Insight", Section::Library, &failure).into())
				}
			}
		}
	)
}

async fn actors(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"People",
		Section::People,
		|cx| insights::actors(cx.connection, cx.params).await
	)
}

async fn library_page(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"Library",
		Section::Library,
		|cx| Some(library::page(&cx).await)
	)
}

async fn dashboard(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(id): Path<Uuid>,
) -> Response {
	let params = Params::new();
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"Dashboard",
		Section::Library,
		|cx| library::dashboard(&cx, id).await
	)
}

async fn people_page(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	UrlQuery(params): UrlQuery<Params>,
) -> Response {
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"People",
		Section::People,
		|cx| Some(people::list(&cx).await)
	)
}

async fn person_page(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	headers: HeaderMap,
	Path(id): Path<Uuid>,
) -> Response {
	let params = Params::new();
	analytics_page!(
		admin,
		me,
		headers,
		params,
		"Person",
		Section::People,
		|cx| people::person(&cx, id).await
	)
}

// The API ----------------------------------------------------------------------

/// The connection, for an admin the API has let in.
#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request"
)]
fn connection(admin: &Admin) -> Result<Connection, Response> {
	admin.inner().analytics.clone().ok_or_else(|| {
		server::problem(
			StatusCode::NOT_FOUND,
			"no_analytics",
			"This admin isn't connected to an analytics service.",
		)
	})
}

fn unavailable(failure: &super::Failure) -> Response {
	server::problem(StatusCode::BAD_GATEWAY, "analytics", &failure.0)
}

/// What the page asks to save: the builder's address, as parameters.
#[derive(Debug, Deserialize)]
pub(crate) struct SaveRequest {
	/// `trends`, `funnels`, `retention`, or `web`.
	pub kind: String,
	#[serde(default)]
	pub params: Params,
	pub name: String,
	/// The insight being changed, rather than a new one.
	#[serde(default)]
	pub insight: Option<Uuid>,
}

#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request"
)]
fn valid_name(name: &str) -> Result<String, Response> {
	let name = name.trim();

	if name.is_empty() || name.chars().count() > 120 {
		return Err(server::problem(
			StatusCode::BAD_REQUEST,
			"invalid_name",
			"Give it a name of up to 120 characters.",
		));
	}

	Ok(name.to_owned())
}

/// The query a save asks for.
#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request"
)]
async fn requested_query(
	connection: &Connection,
	request: &SaveRequest,
) -> Result<Query, Response> {
	if let Some(id) = request.insight
		&& !params::has_query(&request.params)
	{
		return match connection.insight(id).await {
			Ok(Some(saved)) => Ok(saved.query),
			Ok(None) => {
				Err(server::problem(
					StatusCode::NOT_FOUND,
					"not_found",
					"There's no such insight.",
				))
			}
			Err(failure) => Err(unavailable(&failure)),
		};
	}

	if request.kind == "web" {
		return Ok(Query::WebOverview(params::web(&request.params)));
	}

	let kind = Kind::parse(&request.kind).ok_or_else(|| {
		server::problem(
			StatusCode::BAD_REQUEST,
			"invalid_kind",
			"Save a trend, a funnel, a retention grid, or the web overview.",
		)
	})?;
	let facts = connection
		.facts()
		.await
		.map_err(|failure| unavailable(&failure))?;
	Ok(kind.query(
		&request.params,
		&Defaults::from(&super::definitions(&facts)),
	))
}

/// Saves the request's insight, new or changed, and gives back its id.
#[expect(
	clippy::result_large_err,
	reason = "the error is the response sent instead, once per request"
)]
async fn save(connection: &Connection, who: &str, request: &SaveRequest) -> Result<Uuid, Response> {
	let name = valid_name(&request.name)?;
	let query = requested_query(connection, request).await?;
	let saved = match request.insight {
		Some(id) => connection.update_insight(id, Some(name), Some(query)).await,
		None => connection.create_insight(name, query, who.to_owned()).await,
	};
	connection.forget_facts();
	saved
		.map(|insight| insight.id)
		.map_err(|failure| unavailable(&failure))
}

async fn save_insight(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Json(request): Json<SaveRequest>,
) -> Response {
	let who = match server::api_gate(&admin, me).await {
		Ok((_, who)) => who,
		Err(response) => return response,
	};
	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};

	match save(&connection, &who, &request).await {
		Ok(id) => {
			server::no_store(
				Json(json!({ "insight": id, "href": library::insight_href(id) })).into_response(),
			)
		}
		Err(response) => response,
	}
}

async fn list_dashboards(Extension(admin): Extension<Admin>, me: Option<SignedIn>) -> Response {
	if let Err(response) = server::api_gate(&admin, me).await {
		return response;
	}

	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};

	match connection.dashboards().await {
		Ok(dashboards) => {
			server::no_store(
				Json(json!({
					"dashboards": dashboards
						.iter()
						.map(|dashboard| json!({
							"id": dashboard.id,
							"name": dashboard.name,
							"cards": dashboard.tiles.len(),
						}))
						.collect::<Vec<_>>(),
				}))
				.into_response(),
			)
		}
		Err(failure) => unavailable(&failure),
	}
}

/// Adding an insight to a dashboard, saving either first when it's new.
#[derive(Debug, Deserialize)]
pub(crate) struct DashboardRequest {
	/// The dashboard to add to; without one, a new dashboard named
	/// `dashboard_name`.
	#[serde(default)]
	pub dashboard: Option<Uuid>,
	#[serde(default)]
	pub dashboard_name: Option<String>,
	#[serde(flatten)]
	pub insight: SaveRequest,
}

async fn add_to_dashboard(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Json(request): Json<DashboardRequest>,
) -> Response {
	let who = match server::api_gate(&admin, me).await {
		Ok((_, who)) => who,
		Err(response) => return response,
	};
	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};
	let dashboard = match request.dashboard {
		Some(id) => {
			match connection.dashboard(id).await {
				Ok(Some(dashboard)) => Some(dashboard),
				Ok(None) => {
					return server::problem(
						StatusCode::NOT_FOUND,
						"not_found",
						"There's no such dashboard.",
					);
				}
				Err(failure) => return unavailable(&failure),
			}
		}
		None => None,
	};
	let dashboard_name = match (&dashboard, request.dashboard_name.as_deref()) {
		(Some(_), _) => None,
		(None, Some(name)) => {
			match valid_name(name) {
				Ok(name) => Some(name),
				Err(response) => return response,
			}
		}
		(None, None) => {
			return server::problem(
				StatusCode::BAD_REQUEST,
				"invalid_dashboard",
				"Choose a dashboard or name a new one.",
			);
		}
	};
	let insight = match request.insight.insight {
		Some(id) if !params::has_query(&request.insight.params) => id,
		_ => {
			match save(&connection, &who, &request.insight).await {
				Ok(id) => id,
				Err(response) => return response,
			}
		}
	};
	let wide = request.insight.kind == "web";
	let mut tiles = dashboard
		.as_ref()
		.map(|dashboard| dashboard.tiles.clone())
		.unwrap_or_default();
	let below = tiles.iter().map(|tile| tile.y + tile.h).max().unwrap_or(0);
	tiles.push(Tile {
		insight,
		x: 0,
		y: below,
		w: if wide { 12 } else { 6 },
		h: 4,
	});
	let saved = match (dashboard, dashboard_name) {
		(Some(dashboard), _) => connection.update_dashboard(dashboard.id, tiles).await,
		(None, Some(name)) => connection.create_dashboard(name, tiles).await,
		(None, None) => unreachable!("a dashboard or its name, checked above"),
	};
	connection.forget_facts();

	match saved {
		Ok(dashboard) => {
			server::no_store(
				Json(json!({
					"dashboard": dashboard.id,
					"insight": insight,
					"href": library::dashboard_href(dashboard.id),
				}))
				.into_response(),
			)
		}
		Err(failure) => unavailable(&failure),
	}
}

async fn export_person(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Path(id): Path<Uuid>,
) -> Response {
	if let Err(response) = server::api_gate(&admin, me).await {
		return response;
	}

	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};

	match connection.export_person(id).await {
		Ok(export) => {
			let mut response = Json(export).into_response();
			if let Ok(value) = format!("attachment; filename=\"person-{id}.json\"").parse() {
				response
					.headers_mut()
					.insert(header::CONTENT_DISPOSITION, value);
			}
			server::no_store(response)
		}
		Err(failure) => unavailable(&failure),
	}
}

/// Deleting a person: the page sends back what the person typed to confirm.
#[derive(Debug, Deserialize)]
pub(crate) struct DeleteRequest {
	pub confirm: String,
}

async fn delete_person(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	Path(id): Path<Uuid>,
	Json(request): Json<DeleteRequest>,
) -> Response {
	if let Err(response) = server::api_gate(&admin, me).await {
		return response;
	}

	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};
	let person = match connection.person(id).await {
		Ok(Some(person)) => person,
		Ok(None) => {
			return server::problem(
				StatusCode::NOT_FOUND,
				"not_found",
				"There's no such person.",
			);
		}
		Err(failure) => return unavailable(&failure),
	};
	let expected = people::confirmation(&person.person);

	if request.confirm.trim() != expected {
		return server::problem(
			StatusCode::BAD_REQUEST,
			"confirmation",
			&format!("Type {expected} exactly to delete them. Nothing was deleted."),
		);
	}

	match connection.delete_person(id).await {
		Ok(deleted) => {
			connection.forget_facts();
			server::no_store(
				Json(json!({
					"events": deleted.events,
					"sessions": deleted.sessions,
					"href": format!("{BASE}/analytics/people"),
				}))
				.into_response(),
			)
		}
		Err(failure) => unavailable(&failure),
	}
}

#[derive(Debug, Deserialize)]
struct LiveRequest {
	/// Whether to send each event, or only the pulse.
	#[serde(default)]
	events: Option<String>,
}

/// Server-sent events: `pulse` once a second, and with `events=1`, each
/// event as it arrives.
async fn live_events(
	Extension(admin): Extension<Admin>,
	me: Option<SignedIn>,
	UrlQuery(request): UrlQuery<LiveRequest>,
) -> Response {
	if let Err(response) = server::api_gate(&admin, me).await {
		return response;
	}

	let connection = match connection(&admin) {
		Ok(connection) => connection,
		Err(response) => return response,
	};
	let typed: Vec<String> = connection
		.facts()
		.await
		.map(|facts| {
			facts
				.definitions
				.into_iter()
				.filter(|definition| definition.fields.is_some())
				.map(|definition| definition.name)
				.collect()
		})
		.unwrap_or_default();
	let hub = connection.live().clone();
	let receiver = hub.subscribe();
	let events = request.events.as_deref() == Some("1");
	let mut tick = tokio::time::interval(Duration::from_secs(1));
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
	let stream = futures::stream::unfold(
		(hub, receiver, tick, typed),
		move |(hub, mut receiver, mut tick, typed)| {
			async move {
				loop {
					tokio::select! {
						_ = tick.tick() => {
							let event = SseEvent::default().event("pulse").json_data(hub.pulse()).unwrap_or_default();
							return Some((event, (hub, receiver, tick, typed)));
						}
						item = receiver.recv(), if events => match item {
							Ok(item) => {
								let event = SseEvent::default()
									.event("event")
									.json_data(json!({
										"seq": item.event.seq,
										"name": item.event.name,
										"distinct_id": item.event.distinct_id,
										"timestamp": item.event.timestamp,
										"properties": item.event.properties,
										"typed": typed.contains(&item.event.name),
									}))
									.unwrap_or_default();
								return Some((event, (hub, receiver, tick, typed)));
							}
							Err(RecvError::Lagged(_)) => {}
							Err(RecvError::Closed) => return None,
						},
					}
				}
			}
		},
	)
	.map(Ok::<_, Infallible>);

	server::no_store(
		Sse::new(stream)
			.keep_alive(KeepAlive::default())
			.into_response(),
	)
}
