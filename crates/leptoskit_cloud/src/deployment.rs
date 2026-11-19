//! Deployments: the states one goes through, the stage and reason it failed,
//! and where its source came from.
//!
//! ```text
//! queued → building → migrating → rolling_out → live → superseded
//!    ↘          ↘        ↘            ↘
//!     failed { stage, blocker }              cancelled (from queued or building)
//! ```
//!
//! A queued deployment fails only for a reason that stops it before any
//! work starts: no build worker can take it ([`Blocker::BuilderUnavailable`]),
//! or the organisation's quota, spend cap, or payment refuses it
//! ([`Blocker::QuotaExceeded`], [`Blocker::SpendCapReached`],
//! [`Blocker::PaymentRequired`]). [`Blocker::fails_queued`] names them.
//!
//! A rollback is a new deployment whose [`Source`] names an earlier one; it
//! goes from `queued` straight to `migrating`, because there is nothing to
//! build. [`DeploymentState::can_become`] is the diagram, and
//! [`DeploymentState::advance`] adds the rule about rollbacks: together they
//! are the one place a transition is allowed or refused.
//!
//! A worker that dies mid-job leaves its deployment `failed` with
//! [`Blocker::Interrupted`] only once an operator or the reconciler has
//! confirmed what the cluster is actually running: an expired lease is not
//! proof that the old process stopped.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use uuid::Uuid;

use crate::archive::Digest;
use crate::slug::Slug;

/// Where a deployment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeploymentState {
	/// Accepted, waiting for a build worker, or, for a rollback, for the
	/// migration check.
	Queued,
	/// A build worker is building the server image.
	Building,
	/// Checking the schema and applying the migrations.
	Migrating,
	/// Replacing the running replicas with the new ones, which must become
	/// ready before the old ones stop.
	RollingOut,
	/// Serving the environment.
	Live,
	/// Was live, and a newer deployment replaced it. It stays available to
	/// roll back to.
	Superseded,
	/// Stopped in `stage`, for the reason `blocker` gives. The previous live
	/// deployment, if any, keeps serving.
	Failed {
		/// The stage it failed in.
		stage: Stage,
		/// Why.
		blocker: Blocker,
	},
	/// Stopped by request before migrating.
	Cancelled,
}

/// A stage a deployment can fail in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
	/// Waiting to start: refused before any work began.
	Queued,
	/// Building the server image.
	Building,
	/// Checking the schema and applying migrations.
	Migrating,
	/// Rolling out the new replicas.
	RollingOut,
}

impl Stage {
	/// Every stage, in the order a deployment passes through them.
	pub const ALL: [Self; 4] = [
		Self::Queued,
		Self::Building,
		Self::Migrating,
		Self::RollingOut,
	];

	/// The stage's name, as the wire and the CLI spell it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Queued => "queued",
			Self::Building => "building",
			Self::Migrating => "migrating",
			Self::RollingOut => "rolling_out",
		}
	}
}

impl fmt::Display for Stage {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

/// A transition the state diagram, or the deployment's source, doesn't
/// allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a deployment can't go from {from} to {to}")]
pub struct IllegalTransition {
	/// The state it was in.
	pub from: DeploymentState,
	/// The state asked for.
	pub to: DeploymentState,
}

impl DeploymentState {
	/// The state's name, as the wire spells it.
	#[must_use]
	pub const fn name(&self) -> &'static str {
		match self {
			Self::Queued => "queued",
			Self::Building => "building",
			Self::Migrating => "migrating",
			Self::RollingOut => "rolling_out",
			Self::Live => "live",
			Self::Superseded => "superseded",
			Self::Failed { .. } => "failed",
			Self::Cancelled => "cancelled",
		}
	}

	/// The stage this state is, for the states a deployment can fail in.
	#[must_use]
	pub const fn stage(&self) -> Option<Stage> {
		match self {
			Self::Queued => Some(Stage::Queued),
			Self::Building => Some(Stage::Building),
			Self::Migrating => Some(Stage::Migrating),
			Self::RollingOut => Some(Stage::RollingOut),
			Self::Live | Self::Superseded | Self::Failed { .. } | Self::Cancelled => None,
		}
	}

	/// Whether nothing follows this state.
	#[must_use]
	pub const fn is_final(&self) -> bool {
		matches!(
			self,
			Self::Superseded | Self::Failed { .. } | Self::Cancelled
		)
	}

	/// Whether the diagram allows going from this state to `next`:
	///
	/// - `queued` to `building`, to `migrating` (a rollback, which builds
	///   nothing), or to `cancelled`;
	/// - `building` to `migrating` or `cancelled`;
	/// - `migrating` to `rolling_out`, `rolling_out` to `live`, and `live` to
	///   `superseded`;
	/// - `building`, `migrating`, and `rolling_out` to `failed`, with the stage
	///   they are;
	/// - `queued` to `failed` in the `queued` stage, only for a blocker that
	///   stops a deployment before any work starts ([`Blocker::fails_queued`]).
	///
	/// Nothing else, and no state to itself.
	#[must_use]
	pub fn can_become(&self, next: &Self) -> bool {
		match (self, next) {
			(Self::Queued, Self::Building | Self::Migrating | Self::Cancelled)
			| (Self::Building, Self::Migrating | Self::Cancelled)
			| (Self::Migrating, Self::RollingOut)
			| (Self::RollingOut, Self::Live)
			| (Self::Live, Self::Superseded) => true,
			(current, Self::Failed { stage, blocker }) => {
				current.stage() == Some(*stage)
					&& (*stage != Stage::Queued || blocker.fails_queued())
			}
			_ => false,
		}
	}

	/// Moves to `next` if the diagram allows it and the deployment's
	/// `source` agrees: a rollback skips `building`, and anything else must
	/// build.
	///
	/// # Errors
	///
	/// [`IllegalTransition`] when the move isn't allowed.
	pub fn advance(&self, next: Self, source: &Source) -> Result<Self, IllegalTransition> {
		let agrees = match (self, &next) {
			(Self::Queued, Self::Building) => source.builds(),
			(Self::Queued, Self::Migrating) => !source.builds(),
			_ => true,
		};

		if agrees && self.can_become(&next) {
			Ok(next)
		} else {
			Err(IllegalTransition {
				from: *self,
				to: next,
			})
		}
	}
}

impl fmt::Display for DeploymentState {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Failed { stage, blocker } => write!(formatter, "failed ({stage}: {blocker})"),
			state => formatter.write_str(state.name()),
		}
	}
}

/// Why a deployment failed, as a stable code the API, the CLI, and the
/// dashboard share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blocker {
	/// The build itself failed: the compiler, a test, or the image.
	BuildFailed,
	/// No build worker could take the job.
	BuilderUnavailable,
	/// The new code can't run against the database's schema.
	SchemaIncompatible,
	/// A migration failed to apply.
	MigrationFailed,
	/// The new replicas never became ready.
	NotReady,
	/// The deployment would go past a plan or operator quota.
	QuotaExceeded,
	/// The organisation's spend cap is reached.
	SpendCapReached,
	/// The organisation needs a working payment method.
	PaymentRequired,
	/// The environment's cluster couldn't be reached.
	ClusterUnavailable,
	/// The worker stopped mid-job, and the cluster's state was confirmed
	/// afterwards.
	Interrupted,
	/// A secret the release starts with couldn't be read: the environment
	/// no longer has it, or the vault couldn't open it.
	SecretUnavailable,
}

/// A string that isn't a [`Blocker`] code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown blocker `{0}`")]
pub struct UnknownBlocker(String);

impl Blocker {
	/// Every blocker.
	pub const ALL: [Self; 11] = [
		Self::BuildFailed,
		Self::BuilderUnavailable,
		Self::SchemaIncompatible,
		Self::MigrationFailed,
		Self::NotReady,
		Self::QuotaExceeded,
		Self::SpendCapReached,
		Self::PaymentRequired,
		Self::ClusterUnavailable,
		Self::Interrupted,
		Self::SecretUnavailable,
	];

	/// Whether this stops a deployment before any work starts, so a queued
	/// deployment may fail with it: no build worker, a quota, the spend cap,
	/// or a payment.
	#[must_use]
	pub const fn fails_queued(self) -> bool {
		matches!(
			self,
			Self::BuilderUnavailable
				| Self::QuotaExceeded
				| Self::SpendCapReached
				| Self::PaymentRequired
		)
	}

	/// The stable code: `snake_case`, never renamed once released.
	#[must_use]
	pub const fn code(self) -> &'static str {
		match self {
			Self::BuildFailed => "build_failed",
			Self::BuilderUnavailable => "builder_unavailable",
			Self::SchemaIncompatible => "schema_incompatible",
			Self::MigrationFailed => "migration_failed",
			Self::NotReady => "not_ready",
			Self::QuotaExceeded => "quota_exceeded",
			Self::SpendCapReached => "spend_cap_reached",
			Self::PaymentRequired => "payment_required",
			Self::ClusterUnavailable => "cluster_unavailable",
			Self::Interrupted => "interrupted",
			Self::SecretUnavailable => "secret_unavailable",
		}
	}
}

impl fmt::Display for Blocker {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.code())
	}
}

impl FromStr for Blocker {
	type Err = UnknownBlocker;

	fn from_str(code: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.into_iter()
			.find(|blocker| blocker.code() == code)
			.ok_or_else(|| UnknownBlocker(code.to_owned()))
	}
}

impl Serialize for Blocker {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.code())
	}
}

impl<'de> Deserialize<'de> for Blocker {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let code = String::deserialize(deserializer)?;
		code.parse().map_err(serde::de::Error::custom)
	}
}

impl schemars::JsonSchema for Blocker {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Blocker".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		let codes: Vec<&str> = Self::ALL.iter().map(|blocker| blocker.code()).collect();
		schemars::json_schema!({
			"type": "string",
			"description": "Why a deployment failed, as a stable code.",
			"enum": codes
		})
	}
}

/// Where a deployment's source came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
	/// A source archive the CLI packed and uploaded, by its digest.
	Upload {
		/// The archive's digest.
		archive: Digest,
	},
	/// A commit in a Git repository the cloud fetches.
	Git {
		/// The repository's `https://` address.
		repository: GitRepository,
		/// The commit.
		commit: CommitId,
	},
	/// A starter the cloud scaffolds with `lk new`, for a first deploy with
	/// nothing set up locally.
	Template {
		/// The template's name.
		name: Slug,
	},
	/// An earlier deployment of the same environment, run again without
	/// building.
	Rollback {
		/// The deployment to return to.
		deployment: Uuid,
	},
}

impl Source {
	/// Whether a deployment from this source builds: everything but a
	/// rollback, which reuses the earlier deployment's image.
	#[must_use]
	pub const fn builds(&self) -> bool {
		!matches!(self, Self::Rollback { .. })
	}
}

/// A Git repository's address: `https://` with a host and path, and no
/// credentials, query, or fragment. Private repositories are reached with a
/// connection the organisation sets up, never a token in the address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitRepository(String);

/// Why a string isn't a [`GitRepository`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{repository}` {reason}")]
pub struct GitRepositoryError {
	repository: String,
	reason: &'static str,
}

impl GitRepository {
	/// Checks `value`.
	///
	/// # Errors
	///
	/// [`GitRepositoryError`] saying what is wrong.
	pub fn parse(value: &str) -> Result<Self, GitRepositoryError> {
		let problem = |reason| {
			Err(GitRepositoryError {
				repository: value.to_owned(),
				reason,
			})
		};
		let Ok(url) = url::Url::parse(value) else {
			return problem("isn't an address, such as `https://github.com/acme/notes`");
		};

		if url.scheme() != "https" {
			return problem("must be an https:// address");
		}

		if url.host_str().is_none_or(str::is_empty) {
			return problem("has no host");
		}

		if !url.username().is_empty() || url.password().is_some() {
			return problem("must not carry credentials");
		}

		if url.query().is_some() || url.fragment().is_some() {
			return problem("must not have a query or fragment");
		}

		if url.path().trim_matches('/').is_empty() {
			return problem("names no repository");
		}

		Ok(Self(value.to_owned()))
	}

	/// The address.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for GitRepository {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for GitRepository {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for GitRepository {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Self::parse(&value).map_err(serde::de::Error::custom)
	}
}

impl schemars::JsonSchema for GitRepository {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"GitRepository".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"format": "uri",
			"description": "An `https://` repository address with no credentials, query, or fragment."
		})
	}
}

/// A full Git commit ID: 40 lower-case hex digits (SHA-1), or 64 (SHA-256).
/// Branch names and short IDs aren't commits: a deployment names exactly
/// what it built.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitId(String);

/// Why a string isn't a [`CommitId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` isn't a full commit ID: 40 or 64 lower-case hex digits")]
pub struct CommitIdError(String);

impl CommitId {
	/// Checks `value`.
	///
	/// # Errors
	///
	/// [`CommitIdError`] when it isn't 40 or 64 lower-case hex digits.
	pub fn parse(value: &str) -> Result<Self, CommitIdError> {
		let hex = value
			.chars()
			.all(|character| matches!(character, '0'..='9' | 'a'..='f'));

		if hex && matches!(value.len(), 40 | 64) {
			Ok(Self(value.to_owned()))
		} else {
			Err(CommitIdError(value.to_owned()))
		}
	}

	/// The commit ID.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for CommitId {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for CommitId {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for CommitId {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Self::parse(&value).map_err(serde::de::Error::custom)
	}
}

impl schemars::JsonSchema for CommitId {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"CommitId".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"description": "A full commit ID: 40 or 64 lower-case hex digits.",
			"pattern": "^([0-9a-f]{40}|[0-9a-f]{64})$"
		})
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeSet;

	use super::*;

	/// Every state, with every stage and blocker a failure can carry.
	fn every_state() -> Vec<DeploymentState> {
		let mut states = vec![
			DeploymentState::Queued,
			DeploymentState::Building,
			DeploymentState::Migrating,
			DeploymentState::RollingOut,
			DeploymentState::Live,
			DeploymentState::Superseded,
			DeploymentState::Cancelled,
		];

		for stage in Stage::ALL {
			for blocker in Blocker::ALL {
				states.push(DeploymentState::Failed { stage, blocker });
			}
		}

		states
	}

	/// A short label for a state: its name, and a failure's stage. A queued
	/// failure names its blocker too, because only some may end a queued
	/// deployment.
	fn label(state: DeploymentState) -> String {
		match state {
			DeploymentState::Failed {
				stage: Stage::Queued,
				blocker,
			} => format!("failed@queued:{blocker}"),
			DeploymentState::Failed { stage, .. } => format!("failed@{stage}"),
			other => other.name().to_owned(),
		}
	}

	#[test]
	fn every_pair_of_states_follows_the_diagram() {
		let allowed: BTreeSet<(&str, &str)> = [
			("queued", "building"),
			("queued", "migrating"),
			("queued", "cancelled"),
			("queued", "failed@queued:builder_unavailable"),
			("queued", "failed@queued:quota_exceeded"),
			("queued", "failed@queued:spend_cap_reached"),
			("queued", "failed@queued:payment_required"),
			("building", "migrating"),
			("building", "cancelled"),
			("building", "failed@building"),
			("migrating", "rolling_out"),
			("migrating", "failed@migrating"),
			("rolling_out", "live"),
			("rolling_out", "failed@rolling_out"),
			("live", "superseded"),
		]
		.into_iter()
		.collect();
		let states = every_state();
		assert_eq!(states.len(), 7 + 4 * 11);
		let mut seen = BTreeSet::new();

		for from in &states {
			for to in &states {
				let (from_label, to_label) = (label(*from), label(*to));
				let expected = allowed.contains(&(from_label.as_str(), to_label.as_str()));
				assert_eq!(
					from.can_become(to),
					expected,
					"{from} → {to}: expected {expected}"
				);

				if expected {
					seen.insert((from_label, to_label));
				}
			}
		}

		assert_eq!(seen.len(), allowed.len(), "every allowed pair occurs");
	}

	#[test]
	fn final_states_go_nowhere() {
		for state in every_state().into_iter().filter(DeploymentState::is_final) {
			assert!(
				every_state().iter().all(|next| !state.can_become(next)),
				"{state}"
			);
		}

		assert!(!DeploymentState::Live.is_final());
	}

	#[test]
	fn a_queued_deployment_fails_only_before_work_starts() {
		let queued = DeploymentState::Queued;
		let upload = Source::Upload {
			archive: Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap(),
		};

		for blocker in Blocker::ALL {
			let failed = DeploymentState::Failed {
				stage: Stage::Queued,
				blocker,
			};
			let expected = matches!(
				blocker,
				Blocker::BuilderUnavailable
					| Blocker::QuotaExceeded
					| Blocker::SpendCapReached
					| Blocker::PaymentRequired
			);
			assert_eq!(queued.can_become(&failed), expected, "{blocker}");
			assert_eq!(
				queued.advance(failed, &upload).is_ok(),
				expected,
				"{blocker}"
			);
			// Only a queued deployment fails in the queued stage.
			assert!(!DeploymentState::Building.can_become(&failed), "{blocker}");
		}

		let refused = DeploymentState::Failed {
			stage: Stage::Queued,
			blocker: Blocker::SpendCapReached,
		};
		assert_eq!(
			serde_json::to_string(&refused).unwrap(),
			r#"{"state":"failed","stage":"queued","blocker":"spend_cap_reached"}"#
		);
		assert_eq!(refused.to_string(), "failed (queued: spend_cap_reached)");
	}

	#[test]
	fn a_rollback_skips_building_and_nothing_else_does() {
		let upload = Source::Upload {
			archive: Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap(),
		};
		let rollback = Source::Rollback {
			deployment: Uuid::nil(),
		};
		let queued = DeploymentState::Queued;

		assert_eq!(
			queued.advance(DeploymentState::Building, &upload),
			Ok(DeploymentState::Building)
		);
		assert_eq!(
			queued.advance(DeploymentState::Migrating, &rollback),
			Ok(DeploymentState::Migrating)
		);
		assert_eq!(
			queued.advance(DeploymentState::Migrating, &upload),
			Err(IllegalTransition {
				from: queued,
				to: DeploymentState::Migrating
			})
		);
		assert!(
			queued
				.advance(DeploymentState::Building, &rollback)
				.is_err()
		);
		assert!(
			queued
				.advance(DeploymentState::Cancelled, &rollback)
				.is_ok()
		);
		assert_eq!(
			DeploymentState::Live
				.advance(DeploymentState::Building, &upload)
				.unwrap_err()
				.to_string(),
			"a deployment can't go from live to building"
		);
	}

	#[test]
	fn states_serialize_with_a_tag() {
		let failed = DeploymentState::Failed {
			stage: Stage::RollingOut,
			blocker: Blocker::NotReady,
		};
		assert_eq!(
			serde_json::to_string(&failed).unwrap(),
			r#"{"state":"failed","stage":"rolling_out","blocker":"not_ready"}"#
		);
		assert_eq!(
			serde_json::to_string(&DeploymentState::RollingOut).unwrap(),
			r#"{"state":"rolling_out"}"#
		);

		for state in every_state() {
			let json = serde_json::to_string(&state).unwrap();
			let back: DeploymentState = serde_json::from_str(&json).unwrap();
			assert_eq!(back, state);
			assert!(json.contains(state.name()), "{json}");
		}

		assert_eq!(failed.to_string(), "failed (rolling_out: not_ready)");
	}

	#[test]
	fn blockers_have_stable_codes_that_parse_back() {
		let codes: Vec<&str> = Blocker::ALL.into_iter().map(Blocker::code).collect();
		assert_eq!(
			codes,
			[
				"build_failed",
				"builder_unavailable",
				"schema_incompatible",
				"migration_failed",
				"not_ready",
				"quota_exceeded",
				"spend_cap_reached",
				"payment_required",
				"cluster_unavailable",
				"interrupted",
				"secret_unavailable",
			]
		);

		for blocker in Blocker::ALL {
			assert_eq!(blocker.code().parse::<Blocker>(), Ok(blocker));
			assert_eq!(
				serde_json::to_string(&blocker).unwrap(),
				format!("\"{}\"", blocker.code())
			);
		}

		assert_eq!(
			"BuildFailed".parse::<Blocker>(),
			Err(UnknownBlocker("BuildFailed".into()))
		);
		assert!(serde_json::from_str::<Blocker>("\"exploded\"").is_err());
	}

	#[test]
	fn sources_are_validated_and_tagged() {
		let commit = "0123456789abcdef0123456789abcdef01234567";
		let git = Source::Git {
			repository: GitRepository::parse("https://github.com/acme/notes").unwrap(),
			commit: CommitId::parse(commit).unwrap(),
		};
		let json = serde_json::to_string(&git).unwrap();
		assert_eq!(
			json,
			format!(
				r#"{{"kind":"git","repository":"https://github.com/acme/notes","commit":"{commit}"}}"#
			)
		);
		assert_eq!(serde_json::from_str::<Source>(&json).unwrap(), git);
		assert!(git.builds());

		let template: Source =
			serde_json::from_str(r#"{"kind":"template","name":"starter"}"#).unwrap();
		assert!(template.builds());
		assert!(serde_json::from_str::<Source>(r#"{"kind":"template","name":"app"}"#).is_err());

		let rollback: Source = serde_json::from_str(
			r#"{"kind":"rollback","deployment":"0193a8c4-7d6e-7a00-9c3b-0123456789ab"}"#,
		)
		.unwrap();
		assert!(!rollback.builds());

		for bad in [
			"https://github.com/acme/notes?x=1",
			"https://token@github.com/acme/notes",
			"http://github.com/acme/notes",
			"git@github.com:acme/notes.git",
			"https://github.com/",
		] {
			assert!(GitRepository::parse(bad).is_err(), "{bad}");
		}

		for bad in [
			"main",
			"0123456",
			&commit.to_uppercase(),
			&format!("{commit}0"),
		] {
			assert!(CommitId::parse(bad).is_err(), "{bad}");
		}

		assert!(CommitId::parse(&"f".repeat(64)).is_ok());
	}
}
