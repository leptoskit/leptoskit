//! Publishing an app to its stores from the cloud: `lk publish`, run on a
//! build worker with the environment's store credentials.
//!
//! ```text
//! begin_upload / complete_upload          → the source archive, as for a deploy
//! publish { …, archive, stores, submit }  → a publication, queued
//! publication { …, id }                   → where it stands, and the report once it ends
//! publication_log { …, id, after }        → what the run printed
//! ```
//!
//! A build worker takes the job, fetches the archive, and runs its
//! operator's publish program in it. The credentials are the environment's
//! secrets named as `lk publish` names them
//! ([`PUBLISH_CREDENTIALS`](crate::workers::PUBLISH_CREDENTIALS)): each is
//! leased to the one worker that claims the job, when it claims it, and to
//! no other. An Apple store needs a build worker on a Mac (the `macos`
//! capability). A publish that stops part way is never run again, because
//! it may have reached a store.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::archive::Digest;
use crate::deployments::LogPage;
use crate::operator::JobState;
use crate::workers::PublishOutcome;

/// A publish, as its organisation sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PublicationSummary {
	pub id: Uuid,
	pub environment: String,
	/// The stores, in the order they are published to.
	pub stores: Vec<String>,
	/// Whether each upload is also sent for review or release.
	pub submit: bool,
	/// The source archive's SHA-256.
	pub digest: Digest,
	/// The environment's secrets it carries, by name; never their values.
	pub credentials: Vec<String>,
	/// What a worker needs to take it, such as `macos`.
	pub requirements: Vec<String>,
	pub state: JobState,
	/// How it ended, once it has.
	pub outcome: Option<PublishOutcome>,
	pub created_at: Timestamp,
	pub updated_at: Timestamp,
}

/// Builds the archive for `stores` on a build worker and uploads it to
/// each, as `lk publish` does: queued, and answered at once.
///
/// # Arguments
///
/// - `archive`: an archive uploaded with `begin_upload` and `complete_upload`.
/// - `stores`: some of `chrome`, `edge`, `firefox`, `app_store`, `macos`,
///   `play`, and `solana`, each configured in the archive's `leptoskit.toml`.
/// - `submit`: also send each upload for review or release, which reaches
///   users.
///
/// The environment's secrets with the names `lk publish credentials` lists
/// for those stores go with it.
///
/// # Errors
///
/// `Invalid` (`invalid_stores`) for no stores, an unknown one, or one named
/// twice; `NotFound` (`archive_not_found`) for an archive the organisation
/// doesn't have; `Conflict` (`publish_in_flight`) while another publish of
/// the environment is queued or running.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "publishing")]
#[rpc::mutation]
pub async fn publish(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	archive: Digest,
	stores: Vec<String>,
	#[serde(default)] submit: bool,
) -> rpc::Result<PublicationSummary> {
	crate::server::publishing::publish(
		cx,
		crate::server::publishing::PublishArgs {
			organization,
			project,
			environment,
			archive,
			stores,
			submit,
		},
	)
	.await
}

/// One publish: where it stands, and its report once it has ended.
/// Subscribe to it to follow it.
///
/// # Errors
///
/// `NotFound` (`publication_not_found`) for a publish the environment
/// doesn't have.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "publishing")]
#[rpc::query]
pub async fn publication(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	id: Uuid,
) -> rpc::Result<PublicationSummary> {
	crate::server::publishing::publication(cx, &organization, &project, &environment, id).await
}

/// An environment's publishes, newest first: at most `limit`, 20 unless
/// asked.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "publishing")]
#[rpc::query]
pub async fn publications(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	#[serde(default)] limit: Option<u32>,
) -> rpc::Result<Vec<PublicationSummary>> {
	crate::server::publishing::publications(cx, &organization, &project, &environment, limit).await
}

/// What a publish printed, after line `after`. Subscribe to it to follow
/// the run.
///
/// # Errors
///
/// `NotFound` (`publication_not_found`) for a publish the environment
/// doesn't have.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "publishing")]
#[rpc::query]
pub async fn publication_log(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	id: Uuid,
	#[serde(default)] after: Option<u64>,
	#[serde(default)] limit: Option<u32>,
) -> rpc::Result<LogPage> {
	crate::server::publishing::publication_log(
		cx,
		crate::server::publishing::LogArgs {
			organization,
			project,
			environment,
			id,
			after,
			limit,
		},
	)
	.await
}
