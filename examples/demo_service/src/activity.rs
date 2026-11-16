//! The one analytics event a visitor causes: `NoteCreated`, sent through
//! the analytics service's own ingestion route.
//!
//! The site holds a private clone of the analytics router, started by the
//! generation before the site is built, so a note's event travels the same
//! way a client's batch would (`wire::encode`, `POST …/v1/batch`, the app's
//! token) and is stored by the same writer, without touching a socket and
//! without any SQL of the site's own. The site is the app here, so it
//! sends as the app; visitors never see the token and can name no event,
//! property, or actor.
//!
//! Delivery is honest rather than promised. The note is committed in its
//! own transaction first; this module then makes exactly one attempt,
//! bounded in time and in bytes, and reports [`Delivery::Confirmed`] only
//! when the service's typed receipt accounts for exactly the one entry
//! sent: stored, not refused, not rate-limited. Any error, timeout, or
//! partial receipt is [`Delivery::Unconfirmed`], and nothing retries: a
//! timed-out request may have committed, so a second send could count the
//! note twice. Notes and events are separate transactions, and a crash
//! between them leaves a saved note with no event.
//!
//! The analytics actor is `v-` plus a domain-separated SHA-256 of the
//! person's random id: stable for that person for the generation, and
//! derived from nothing a visitor typed or a network revealed. The only
//! property beside the event's own fields is the persona, which this
//! server chose.

use std::sync::Arc;
use std::time::Duration;

use admin_demo::analytics::NoteCreated;
use axum::Router;
use axum::http::Method;
use axum::http::Request;
use axum::http::header;
use leptoskit_analytics::Event as _;
use leptoskit_analytics::service::CaptureResult;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Value;
use leptoskit_db::Id;
use sha2::Digest as _;
use sha2::Sha256;
use tower::ServiceExt as _;

use crate::clock;
use crate::visitors::DemoPerson;
use crate::visitors::Persona;

/// The most bytes of the service's answer this module reads.
const ANSWER_BYTES: usize = 16 * 1024;

/// How long one capture may take, answer included.
const CAPTURE_WAIT: Duration = Duration::from_secs(2);

/// The domain separator for an actor's hash.
const ACTOR_DOMAIN: &[u8] = b"leptoskit demo analytics actor";

/// What one capture came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
	/// The service's receipt accounts for exactly the one event: stored.
	Confirmed,
	/// An error, a timeout, a refusal, a rate limit, or a duplicate: the
	/// event may or may not be stored, and nothing will try again.
	Unconfirmed,
}

impl Delivery {
	/// The word the JSON contract uses.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Confirmed => "confirmed",
			Self::Unconfirmed => "unconfirmed",
		}
	}
}

/// The analytics service as this site's app sees it: its own routes, in
/// process, and the token it presents as the app.
#[derive(Clone)]
pub struct Analytics {
	router: Router,
	token: Arc<str>,
}

impl std::fmt::Debug for Analytics {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Analytics")
			.field("token", &"<redacted>")
			.finish_non_exhaustive()
	}
}

impl Analytics {
	/// The service reached through `router`, signing as the app with
	/// `token`. `router` must be a clone of the generation's one analytics
	/// router, so events land in the database the admin reads.
	#[must_use]
	pub fn new(router: Router, token: &str) -> Self {
		Self {
			router,
			token: Arc::from(token),
		}
	}

	/// Sends one `NoteCreated` for `actor`, once. `words` is the note's
	/// whitespace-separated word count.
	pub async fn note_created(&self, actor: &str, persona: Persona, words: u32) -> Delivery {
		let sent = tokio::time::timeout(CAPTURE_WAIT, self.send(actor, persona, words)).await;

		match sent {
			Ok(Ok(true)) => Delivery::Confirmed,
			Ok(Ok(false)) => {
				eprintln!("the demo's analytics service did not confirm a note event");
				Delivery::Unconfirmed
			}
			Ok(Err(error)) => {
				eprintln!("the demo's note event was not delivered: {error:#}");
				Delivery::Unconfirmed
			}
			Err(_) => {
				eprintln!("the demo's analytics service did not answer a note event in time");
				Delivery::Unconfirmed
			}
		}
	}

	/// One attempt: whether the receipt accounts for exactly the one entry.
	async fn send(&self, actor: &str, persona: Persona, words: u32) -> anyhow::Result<bool> {
		use anyhow::Context as _;

		let now = clock::now_millis();
		let mut random = [0_u8; 10];
		getrandom::fill(&mut random).context("the OS random source failed")?;

		let batch = wire::Batch {
			sent_at: now,
			contexts: vec![vec![(
				"$lib".to_owned(),
				Value::Text("leptoskit-demo-service".to_owned()),
			)]],
			entries: vec![wire::Entry {
				uuid: wire::uuid_v7(now, random),
				distinct_id: actor.to_owned(),
				session_id: None,
				context: 0,
				body: Body::TypedEvent {
					name: NoteCreated::NAME.to_owned(),
					schema_id: NoteCreated::SCHEMA_ID,
					values: NoteCreated {
						words,
						shared: false,
					}
					.into_values(),
					properties: vec![(
						"persona".to_owned(),
						Value::Text(persona.slug().to_owned()),
					)],
				},
			}],
		};

		let response = self
			.router
			.clone()
			.oneshot(
				Request::builder()
					.method(Method::POST)
					.uri(format!(
						"{}/v1/batch",
						leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
					))
					.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
					.header(header::CONTENT_TYPE, "application/octet-stream")
					.body(axum::body::Body::from(wire::encode(&batch)))?,
			)
			.await
			.context("sending the note event")?;
		let status = response.status();
		let answer = axum::body::to_bytes(response.into_body(), ANSWER_BYTES)
			.await
			.context("reading the service's answer")?;

		if !status.is_success() {
			anyhow::bail!("the analytics service answered {status}");
		}

		let receipt: CaptureResult =
			serde_json::from_slice(&answer).context("reading the service's receipt")?;

		Ok(receipt.stored == 1
			&& receipt.duplicates == 0
			&& receipt.rate_limited == 0
			&& receipt.refused.is_empty())
	}
}

/// The analytics actor for a person: `v-` and the hex SHA-256 of the
/// person's random id over [`ACTOR_DOMAIN`]. Stable while the person
/// exists, and a different person never shares one.
#[must_use]
pub fn actor(person: Id<DemoPerson>) -> String {
	let mut hasher = Sha256::new();
	hasher.update(ACTOR_DOMAIN);
	hasher.update([0]);
	hasher.update(person.uuid().as_bytes());
	format!("v-{}", hex::encode(hasher.finalize()))
}

/// The number of whitespace-separated words in `text`.
#[must_use]
pub fn words(text: &str) -> u32 {
	u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn words_are_whitespace_separated() {
		assert_eq!(words("one"), 1);
		assert_eq!(words("  one \t two\nthree  "), 3);
		assert_eq!(words("日本語 は 一語"), 3);
		assert_eq!(words(""), 0);
	}

	#[test]
	fn an_actor_is_prefixed_hashed_and_per_person() {
		let first = Id::<DemoPerson>::new();
		let second = Id::<DemoPerson>::new();
		let actor_of_first = actor(first);

		assert!(actor_of_first.starts_with("v-"));
		assert_eq!(actor_of_first.len(), 2 + 64);
		assert!(
			actor_of_first[2..]
				.bytes()
				.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
		);
		assert_eq!(actor_of_first, actor(first), "stable for one person");
		assert_ne!(actor_of_first, actor(second), "distinct for two");
		assert!(
			!actor_of_first.contains(&first.to_string()),
			"the id itself is never the actor"
		);
	}

	#[test]
	fn the_token_never_shows_in_debug_output() {
		let analytics = Analytics::new(Router::new(), "secret-token-value");
		assert!(!format!("{analytics:?}").contains("secret-token-value"));
	}
}
