//! The one place a deployment's state changes, and its lines are stamped.
//!
//! A [`Changing`] deployment collects its changes in memory, each with the
//! event that records it; [`Changing::save`] writes the row and every event
//! in the caller's transaction, so a deployment never changes without its
//! history. A state changes only through [`DeploymentState::advance`], and
//! the in-flight and live markers the database keeps unique follow it.

use leptoskit_db::Json;
use leptoskit_db::Max;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;

use crate::deployment::DeploymentState;
use crate::deployments::BuildStep;
use crate::deployments::EventKind;
use crate::deployments::LineKind;
use crate::deployments::Stamp;
use crate::tables::Deployment;
use crate::tables::EventRecord;
use crate::tables::NewDeployment;
use crate::tables::NewEventRecord;

/// A deployment being changed in one transaction.
pub(crate) struct Changing {
	pub(crate) row: Deployment,
	events: Vec<EventKind>,
}

impl Changing {
	pub(crate) const fn new(row: Deployment) -> Self {
		Self {
			row,
			events: Vec::new(),
		}
	}

	/// The state it is in.
	pub(crate) fn state(&self) -> DeploymentState {
		self.row.state.0
	}

	/// Moves to `next`, as [`DeploymentState::advance`] allows, and keeps
	/// the markers in step: in flight from `queued` to `rolling_out`, live
	/// while `live`.
	///
	/// # Errors
	///
	/// `Server` for a move the diagram refuses: a bug in the caller, which
	/// rolls the transaction back.
	pub(crate) fn advance(&mut self, next: DeploymentState) -> rpc::Result<()> {
		let from = self.row.state.0;
		let to = from
			.advance(next, &self.row.source.0)
			.map_err(|error| rpc::Error::server(format!("deployment lifecycle: {error}")))?;
		self.row.state = Json(to);
		let environment = self.row.environment_id;
		self.row.in_flight_environment = in_flight(to).then_some(environment);
		self.row.live_environment = (to == DeploymentState::Live).then_some(environment);
		self.events.push(EventKind::Transition { from, to });
		Ok(())
	}

	/// Stamps the line `kind`, when the deployment has it, with `stamp`
	/// and, when given, a new detail.
	pub(crate) fn stamp(&mut self, kind: &LineKind, stamp: Stamp, detail: Option<String>) {
		let now = leptoskit_db::now();

		let Some(line) = self.row.lines.0.iter_mut().find(|line| &line.kind == kind) else {
			return;
		};

		if line.stamp == stamp && detail.as_ref().is_none_or(|detail| *detail == line.detail) {
			return;
		}

		line.stamp = stamp;
		line.stamped_at = Some(now);

		if let Some(detail) = detail {
			line.detail = detail;
		}

		self.events.push(EventKind::Stamped {
			line: kind.clone(),
			stamp,
			detail: line.detail.clone(),
		});
	}

	/// Stamps every line still `from` with `to`.
	pub(crate) fn stamp_all(&mut self, from: Stamp, to: Stamp) {
		let kinds: Vec<LineKind> = self
			.row
			.lines
			.0
			.iter()
			.filter(|line| line.stamp == from)
			.map(|line| line.kind.clone())
			.collect();

		for kind in kinds {
			self.stamp(&kind, to, None);
		}
	}

	/// Puts a build's steps on the line `kind`.
	pub(crate) fn steps(&mut self, kind: &LineKind, steps: Vec<BuildStep>) {
		let Some(line) = self.row.lines.0.iter_mut().find(|line| &line.kind == kind) else {
			return;
		};

		for step in &steps {
			self.events.push(EventKind::Step {
				line: kind.clone(),
				step: step.clone(),
			});
		}

		line.steps = steps;
	}

	/// Records `event`, which changes nothing else.
	pub(crate) fn record(&mut self, event: EventKind) {
		self.events.push(event);
	}

	/// Writes the row and its events.
	///
	/// # Errors
	///
	/// When the database fails.
	pub(crate) async fn save(mut self, tx: &Tx) -> rpc::Result<Deployment> {
		self.row.save(tx).await?;
		record(tx, &self.row, self.events).await?;
		Ok(self.row)
	}
}

/// The state a new deployment starts in, for [`create`].
pub(crate) const fn queued() -> Json<DeploymentState> {
	Json(DeploymentState::Queued)
}

/// Inserts a deployment, queued and holding its environment, with
/// `events` as its first history.
///
/// # Errors
///
/// `Conflict` (`deployment_in_flight`) when another deployment holds the
/// environment, which a racing `deploy` can only learn here.
pub(crate) async fn create(
	tx: &Tx,
	mut new: NewDeployment,
	events: Vec<EventKind>,
) -> rpc::Result<Deployment> {
	new.state = queued();
	new.in_flight_environment = Some(new.environment_id);
	new.live_environment = None;
	let row = new.insert(tx).await.map_err(|error| {
		if error.is_conflict() {
			rpc::Error::conflict(
				"deployment_in_flight: another deployment of this environment started at the same \
				 moment; wait for it, or cancel it",
			)
		} else {
			error.into()
		}
	})?;
	record(tx, &row, events).await?;
	Ok(row)
}

/// Whether a deployment in `state` holds its environment.
pub(crate) const fn in_flight(state: DeploymentState) -> bool {
	matches!(
		state,
		DeploymentState::Queued
			| DeploymentState::Building
			| DeploymentState::Migrating
			| DeploymentState::RollingOut
	)
}

/// Appends `events` to a deployment's history, numbered after the last.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn record(
	tx: &Tx,
	deployment: &Deployment,
	events: Vec<EventKind>,
) -> rpc::Result<()> {
	if events.is_empty() {
		return Ok(());
	}

	let id = deployment.id;
	let last = EventRecord::query()
		.filter(|event| event.deployment_id.eq(id))
		.aggregate(|event| Max(event.seq), tx)
		.await?
		.unwrap_or_default();

	for (seq, event) in (last + 1..).zip(events) {
		NewEventRecord {
			deployment_id: id,
			seq,
			event: Json(event),
		}
		.insert(tx)
		.await?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	/// A deployment's state is written in this module only: every other
	/// module goes through [`super::Changing::advance`].
	#[test]
	fn only_this_module_writes_a_deployments_state() {
		let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server");

		for entry in std::fs::read_dir(&src).expect("the server's sources") {
			let path = entry.expect("an entry").path();
			let name = path
				.file_name()
				.map(|name| name.to_string_lossy().into_owned())
				.unwrap_or_default();

			if name == "lifecycle.rs" {
				continue;
			}

			let text = std::fs::read_to_string(&path).expect("a source file");

			for forbidden in [".state = Json(", "state: Json(DeploymentState"] {
				assert!(
					!text.contains(forbidden),
					"server/{name} writes a deployment's state (`{forbidden}`): only \
					 server/lifecycle.rs does, through `Changing::advance`"
				);
			}
		}
	}
}
