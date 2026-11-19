//! The admin's live connection: the streams a page this crate's pages open,
//! served only to a signed-in admin.
//!
//! It is nested under the admin's base path, so a site that serves its own
//! live connection beside the admin keeps it.

use std::sync::Arc;

use axum::Router;
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use leptoskit_admin_pages::ops::Backend;
use leptoskit_admin_pages::ops::Live;
use leptoskit_admin_pages::ops::Port;
use leptoskit_admin_pages::ops::Progress;
use leptoskit_auth::CurrentSession;
use leptoskit_deploy_console::control::FacadeError;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Principal;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Update;
use leptoskit_rpc as rpc;

use crate::Admin;
use crate::BASE;
use crate::infra::phase_word;

/// The live connection's routes: `{BASE}/live`, and its server-sent events
/// for browsers that can't open a WebSocket.
pub(crate) fn router(admin: &Admin) -> Router {
	let streams = rpc::server::LiveStreams::new().identify(Callers(admin.clone()));

	Router::new().nest(BASE, streams.router())
}

/// Lets a signed-in admin open the admin's streams, and nobody else.
struct Callers(Admin);

impl rpc::server::Identify for Callers {
	fn identify<'a>(
		&'a self,
		request: &'a axum::http::request::Parts,
	) -> BoxFuture<'a, Result<rpc::server::Identity, rpc::Error>> {
		let me = request
			.extensions
			.get::<CurrentSession>()
			.and_then(|current| current.signed_in.clone());

		async move {
			let inner = self.0.inner();
			let Some(me) = me else {
				return Err(rpc::Error::unauthorized("Sign in to the admin first."));
			};
			let roles = inner
				.auth
				.roles(me.account)
				.await
				.map_err(|error| rpc::Error::server(error.to_string()))?;

			if !roles.iter().any(|role| inner.roles.contains(role)) {
				return Err(rpc::Error::forbidden(
					"This account isn't an admin. The main site decides who is.",
				));
			}

			let streams = Streams {
				control: inner.control.clone(),
				analytics: inner.analytics.clone(),
			};
			let context = leptoskit_db::Context::new().with(Port(Arc::new(streams)));

			Ok(rpc::server::Identity::new(context))
		}
		.boxed()
	}
}

/// The streams' bodies, for one signed-in admin.
struct Streams {
	control: Option<Service>,
	analytics: Option<crate::analytics::Connection>,
}

impl Backend for Streams {
	fn progress(&self, id: String) -> rpc::Result<BoxStream<'static, Progress>> {
		let service = self
			.control
			.as_ref()
			.ok_or_else(|| rpc::Error::not_found("There's no control service."))?;
		progress(service, &id)
	}

	fn analytics(&self, events: bool) -> rpc::Result<BoxStream<'static, Live>> {
		let connection = self
			.analytics
			.clone()
			.ok_or_else(|| rpc::Error::not_found("Analytics isn't connected."))?;

		Ok(crate::analytics::live::follow(connection, events))
	}
}

/// Operation `id`'s steps so far, each step as it happens, then `Done`.
/// The page drops the steps it already shows.
pub(crate) fn progress(service: &Service, id: &str) -> rpc::Result<BoxStream<'static, Progress>> {
	// Subscribe before reading, so nothing that happens in between is lost.
	let subscription = service.subscribe(id);
	let snapshot = match service.operation(&Principal::owner(), id) {
		Ok(operation) => operation,
		Err(FacadeError::NotFound | FacadeError::Forbidden) => {
			return Err(rpc::Error::not_found("There's no such operation."));
		}
		Err(error) => return Err(rpc::Error::server(error.to_string())),
	};
	let step = |event: &leptoskit_deploy_console::control::Event| {
		let (tone, word) = phase_word(event.phase);
		Progress::Step {
			sequence: event.sequence,
			message: event.message.clone(),
			tone,
			word: word.to_owned(),
		}
	};
	let so_far: Vec<Progress> = snapshot.events.iter().map(step).collect();

	if !matches!(snapshot.phase, Phase::Queued | Phase::Running) {
		return Ok(futures::stream::iter(so_far)
			.chain(futures::stream::once(std::future::ready(Progress::Done)))
			.boxed());
	}

	let updates = subscription.map(move |update| {
		match update {
			Update::Progress { event, .. } => step(&event),
			Update::Finished { .. } => Progress::Done,
		}
	});

	Ok(futures::stream::iter(so_far).chain(updates).boxed())
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use leptoskit_deploy_console::control::Action;
	use leptoskit_deploy_console::control::Intent;
	use leptoskit_deploy_console::control::Project;
	use leptoskit_deploy_console::control::RunResult;
	use leptoskit_deploy_console::control::Store;

	use super::*;

	fn words(progress: &[Progress]) -> Vec<String> {
		progress
			.iter()
			.map(|item| {
				match item {
					Progress::Step {
						sequence, message, ..
					} => format!("{sequence} {message}"),
					Progress::Done => "done".to_owned(),
				}
			})
			.collect()
	}

	#[tokio::test]
	async fn an_operation_streams_its_steps_so_far_then_each_new_one() {
		let root = tempfile::tempdir().unwrap();
		// A directory the store makes itself, so it's private.
		let store = Store::open(&root.path().join("data/console.sqlite3")).unwrap();
		store
			.register(&Project {
				id: "notes".into(),
				environment: "production".into(),
				root: root.path().canonicalize().unwrap(),
				revision: 0,
			})
			.unwrap();
		let service = Service::new(
			store.clone(),
			"test-only-management-owner-key-0123456789",
			"http://localhost:4700",
		)
		.unwrap();
		let intent = Intent {
			expected_revision: 0,
			idempotency_key: "status".into(),
			action: Action::Status,
		};
		let id = store.enqueue("notes", &intent).unwrap().id;
		store.claim_project(Some("notes")).unwrap().unwrap();
		service.progress(&id, "Reading the provider state").unwrap();

		let mut stream = progress(&service, &id).unwrap();
		let mut until = async |text: &str| {
			loop {
				let item = tokio::time::timeout(Duration::from_secs(5), stream.next())
					.await
					.expect("a step in time")
					.expect("the stream is open");

				if words(std::slice::from_ref(&item))[0].ends_with(text) {
					return;
				}
			}
		};
		// The steps so far, then each new one as it happens.
		until("Reading the provider state").await;
		service.progress(&id, "Checking the hosts").unwrap();
		until("Checking the hosts").await;

		// A finished operation says so at once, after its steps.
		store
			.finish(
				&id,
				&RunResult {
					succeeded: true,
					message: "Healthy".into(),
					details: serde_json::Value::Null,
				},
			)
			.unwrap();
		let finished: Vec<Progress> = progress(&service, &id).unwrap().collect().await;
		assert_eq!(finished.last(), Some(&Progress::Done));
		assert!(finished.len() >= 3, "{:?}", words(&finished));

		assert!(progress(&service, "no-such-operation").is_err());
	}
}
