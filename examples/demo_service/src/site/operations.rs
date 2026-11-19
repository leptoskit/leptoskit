//! What a page asks the site for once it is open: the operations
//! `demo_pages` declares, answered from this request's pass, session, and
//! peer.
//!
//! The operations travel over leptoskit's rpc routes and its live
//! connection, and each body hands the call to the [`Backend`] this module
//! builds for the request. The rules behind them are the site's own and
//! live where they always have: [`super::mint`] is the one mint behind both
//! the form post and the page's request, with the same same-origin check,
//! the same caps, and the same pass cookie.
//!
//! Two things are particular to this site:
//!
//! - **Only the demo's operations are served.** The binary links other crates'
//!   operations too; a request for any name not listed in [`SERVED`] is a 404
//!   before the rpc server sees it.
//! - **The rpc server keeps its receipts in memory.** A mutation's receipt is
//!   what makes a repeated call answer once. It is written in a transaction of
//!   its own on a private in-memory database, so the visitors' store keeps
//!   counting and inserting in its own immediate transaction on the site's
//!   database, exactly as it does for the form post.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::middleware::Next;
use axum::response::IntoResponse as _;
use axum::response::Response;
use demo_pages::model;
use demo_pages::ops;
use demo_pages::ops::Backend;
use demo_pages::ops::Port;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use leptoskit_rpc as rpc;

use super::MintRefusal;
use super::Parts;
use super::Shared;
use crate::inbox::DemoInbox;
use crate::return_to;
use crate::visitors::PassState;
use crate::visitors::Persona;
use crate::visitors::carried_cookie;

/// The operations this site answers, by the path each is called at.
const SERVED: [&str; 2] = ["/rpc/mint", "/rpc/inbox"];

/// The cookies an operation asks the response to set. An operation has no
/// response of its own, so it leaves them here and [`set_cookies`] adds
/// them to the answer.
#[derive(Clone, Default)]
struct Cookies(Arc<Mutex<Vec<HeaderValue>>>);

impl Cookies {
	fn push(&self, cookie: HeaderValue) {
		self.0
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.push(cookie);
	}

	fn take(&self) -> Vec<HeaderValue> {
		std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
	}
}

/// One request, as the operations see it.
struct RequestBackend {
	shared: Arc<Parts>,
	headers: HeaderMap,
	peer: Option<SocketAddr>,
	cookies: Cookies,
}

impl Backend for RequestBackend {
	fn mint(
		&self,
		persona: model::Persona,
		return_to: String,
	) -> BoxFuture<'_, rpc::Result<ops::Minted>> {
		Box::pin(async move {
			let persona = match persona {
				model::Persona::Ada => Persona::Ada,
				model::Persona::Grace => Persona::Grace,
			};
			// Never trusted: anything but one of this site's own paths
			// becomes the fallback.
			let return_to = return_to::local_return_to(&return_to);

			match super::mint(&self.shared, &self.headers, self.peer, persona, &return_to).await {
				Ok(minted) => {
					if let Some(cookie) = minted.cookie {
						self.cookies.push(cookie);
					}

					Ok(ops::Minted {
						next: minted.location,
					})
				}
				Err(MintRefusal::CrossOrigin) => Err(rpc::Error::forbidden("cross_origin")),
				Err(MintRefusal::BadClientIp) => Err(rpc::Error::invalid("bad_client_ip")),
				Err(MintRefusal::RateLimited { retry_after }) => {
					Err(rpc::Error::rate_limited(Duration::from_secs(retry_after)))
				}
				Err(MintRefusal::Full) => {
					Err(rpc::Error::refused(
						ops::FULL,
						"today's demo has handed out all its people",
					))
				}
				Err(MintRefusal::Failed(error)) => {
					eprintln!("the demo site's request failed: {error:#}");
					Err(rpc::Error::server("server"))
				}
			}
		})
	}

	fn inbox(&self, persona: model::Persona) -> BoxFuture<'_, rpc::Result<ops::Mailbox>> {
		Box::pin(async move {
			let persona = match persona {
				model::Persona::Ada => Persona::Ada,
				model::Persona::Grace => Persona::Grace,
			};
			let visitors = &self.shared.visitors;
			let failed = |error: anyhow::Error| {
				eprintln!("the demo site's request failed: {error:#}");
				rpc::Error::server("server")
			};
			let pass = match visitors.resolve(&self.headers).await.map_err(failed)? {
				PassState::Valid(pass) => pass,
				gone => {
					// A cookie that names no pass here goes with the refusal,
					// so a stale browser stops carrying it around.
					if carried_cookie(&self.headers, visitors.cookie().name).is_some() {
						self.cookies.push(visitors.cookie().clear());
					}

					return Err(if matches!(gone, PassState::EarlierDay) {
						rpc::Error::refused(
							ops::NEW_DAY,
							"this browser's pass is from an earlier day",
						)
					} else {
						rpc::Error::refused(ops::NO_PASS, "this browser has no pass for today")
					});
				}
			};
			let people = visitors.people(&pass).await.map_err(failed)?;
			let Some(person) = people.into_iter().find(|person| person.persona == persona) else {
				// The pass never minted this person, so there is no mail.
				return Ok(ops::Mailbox::default());
			};
			let retained = self.shared.inbox.messages_for(person.email.as_inner());
			let lost_at_ms = DemoInbox::unavailable(
				person.persona,
				person.last_receipt_id.as_deref(),
				person.last_receipt_at,
				&retained,
			)
			.and(person.last_receipt_at)
			.map(|at| at.timestamp_millis());
			let mut letters = retained
				.iter()
				.map(|stored| {
					ops::Letter {
						to: person.email.as_inner().to_owned(),
						subject: stored.subject.clone(),
						text: stored.text.clone(),
						receipt: stored.receipt.clone(),
						sent_at_ms: stored.sent_at.timestamp_millis(),
					}
				})
				.collect::<Vec<_>>();
			letters.sort_by_key(|letter| letter.sent_at_ms);

			Ok(ops::Mailbox {
				letters,
				lost_at_ms,
			})
		})
	}

	fn generation(&self) -> rpc::Result<BoxStream<'static, ops::Day>> {
		let Ok(permit) = Arc::clone(&self.shared.watchers).try_acquire_owned() else {
			return Err(rpc::Error::rate_limited(Duration::from_secs(5)));
		};
		let day = ops::Day {
			day: self.shared.visitors.day().to_owned(),
		};

		// The day once, then nothing for as long as this generation serves.
		// The permit lives inside the stream, so a page that leaves, or a
		// reset that drops the connection, returns it at once.
		Ok(futures::stream::once(std::future::ready(day))
			.chain(futures::stream::pending().map(move |day: ops::Day| {
				let _held = &permit;
				day
			}))
			.boxed())
	}
}

/// Who is calling: every request gets a [`Backend`] of its own.
struct Callers(Shared);

impl rpc::server::Identify for Callers {
	fn identify<'a>(
		&'a self,
		request: &'a axum::http::request::Parts,
	) -> BoxFuture<'a, Result<rpc::server::Identity, rpc::Error>> {
		let backend = RequestBackend {
			shared: Arc::clone(&self.0.0),
			headers: request.headers.clone(),
			peer: super::peer(request),
			cookies: request
				.extensions
				.get::<Cookies>()
				.cloned()
				.unwrap_or_default(),
		};
		let context = leptoskit_db::Context::new().with(Port(Arc::new(backend)));

		Box::pin(std::future::ready(Ok(rpc::server::Identity::new(context))))
	}
}

/// A private in-memory database for the rpc server's receipts.
///
/// # Errors
///
/// When SQLite cannot open it or the tables do not create.
pub(super) async fn receipts() -> anyhow::Result<leptoskit_db::Db> {
	use anyhow::Context as _;

	let db = leptoskit_db::Db::connect("sqlite::memory:")
		.await
		.context("opening the operations' receipts")?;
	leptoskit_db::testing::create_tables(&db)
		.await
		.context("creating the operations' receipts")?;

	Ok(db)
}

/// The site's operation routes and its live connection.
///
/// # Panics
///
/// When the private receipts database cannot be made, which is a fault in
/// the build and never a request's.
pub(super) fn router(shared: &Shared) -> Router<Shared> {
	let callers = Arc::new(Callers(shared.clone()));
	let streams = rpc::server::LiveStreams::new()
		.identify_arc(Arc::clone(&callers) as Arc<dyn rpc::server::Identify>)
		.origin(shared.0.origin.clone());
	let receipts = shared.0.receipts.clone();
	let operations = rpc::server::Server::new(receipts)
		.identify(callers)
		.router::<Shared>()
		.layer(axum::middleware::from_fn(set_cookies))
		.layer(axum::middleware::from_fn(only_served));

	Router::new().merge(operations).merge(streams.router())
}

/// Refuses every operation this site does not serve, before the rpc server
/// identifies the caller or reads a body.
async fn only_served(request: Request, next: Next) -> Response {
	if !SERVED.contains(&request.uri().path()) {
		return StatusCode::NOT_FOUND.into_response();
	}

	next.run(request).await
}

/// Gives an operation somewhere to leave cookies, and sets them on its
/// answer. An operation's answer is never stored.
async fn set_cookies(mut request: Request, next: Next) -> Response {
	let cookies = Cookies::default();
	request.extensions_mut().insert(cookies.clone());
	let mut response = next.run(request).await;

	for cookie in cookies.take() {
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	response
		.headers_mut()
		.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
	response
}
