//! The loop: claim a job, run it under a lease, report how it ended.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use leptoskit_cloud::workers::Assignment;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerInput;
use leptoskit_cloud::workers::JobInput;
use leptoskit_cloud::workers::JobKind;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud::workers::MAX_WAIT_SECONDS;
use tokio::sync::watch;
use uuid::Uuid;

use crate::build::BuildJob;
use crate::build::Builder;
use crate::build::SourceError;
use crate::build::Stop;
use crate::client::CallError;
use crate::client::Control;
use crate::logs;
use crate::state::Marker;
use crate::state::Saved;
use crate::state::State;
use crate::state::StateError;

/// How long the log's last lines have to arrive before the outcome is
/// reported without them.
const LOG_FLUSH: Duration = Duration::from_secs(30);

/// How long the worker waits before asking again while the control plane
/// still counts a job as its own.
const HELD_WAIT: Duration = Duration::from_secs(5);

/// The longest wait between two tries of anything.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The job is running under a lease that holds.
const RUNNING: u8 = 0;
/// The deployment was cancelled: stop, and say so.
const CANCELLED: u8 = 1;
/// The lease is gone: stop, and say nothing, for nobody is listening.
const LOST: u8 = 2;

/// How a worker runs.
#[derive(Clone, Debug)]
pub struct Settings {
	/// How long one `claim` waits for a job, 20 seconds at most.
	pub wait_seconds: u32,
	/// Where each build's directory is made, and deleted.
	pub workspace: PathBuf,
	/// How often a lease is renewed. A third of the lease when unset.
	pub renew_every: Option<Duration>,
}

impl Settings {
	/// Builds under `workspace`, waiting as long as `claim` allows.
	#[must_use]
	pub fn new(workspace: impl Into<PathBuf>) -> Self {
		Self {
			wait_seconds: MAX_WAIT_SECONDS,
			workspace: workspace.into(),
			renew_every: None,
		}
	}
}

/// Why the worker stopped, or couldn't take a step.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
	#[error(transparent)]
	State(#[from] StateError),
	#[error(transparent)]
	Call(CallError),
	#[error("the control plane no longer knows this worker's key: it was revoked. Pair again")]
	Revoked,
	#[error(
		"job {job} ({kind:?}) was started and never reported, so whether it took effect is \
		 unknown. It is never run twice: an operator reconciles it from what the cluster runs, \
		 and then `leptoskit-cloud-worker forget` lets this worker start"
	)]
	Unreconciled { job: Uuid, kind: JobKind },
}

/// How a job the worker ran ended, for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
	/// The control plane has the outcome.
	Reported(JobOutcome),
	/// Nothing was reported, and the control plane gives the job out again
	/// when its lease runs out: why.
	Abandoned(String),
}

/// What one turn of the loop did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
	/// There was no job.
	Idle,
	/// The control plane still counts a job as this worker's, until its
	/// lease runs out.
	Held,
	/// A job ran.
	Ran { job: Uuid, ended: Ended },
}

/// A paired worker.
#[derive(Debug)]
pub struct Worker<B> {
	control: Control,
	state: State,
	builder: B,
	/// For presigned addresses: no key, and no redirects.
	downloads: reqwest::Client,
	settings: Settings,
}

fn note(text: &str) {
	eprintln!("leptoskit-cloud-worker: {text}");
}

fn kind_of(input: &JobInput) -> JobKind {
	match input {
		JobInput::BuildServer(_) => JobKind::BuildServer,
		JobInput::Release(_) => JobKind::Release,
		JobInput::DestroyEnvironment(_) => JobKind::DestroyEnvironment,
		JobInput::BackupDatabase(_) => JobKind::BackupDatabase,
		JobInput::RestoreDatabase(_) => JobKind::RestoreDatabase,
	}
}

/// Keeps `job`'s lease, and says in `fate` and through `stop` when the job
/// should end early.
async fn keep_lease(
	control: Control,
	job: Uuid,
	every: Duration,
	lease: Duration,
	fate: Arc<AtomicU8>,
	stop: watch::Sender<bool>,
) {
	let mut held_until = Instant::now() + lease;

	loop {
		tokio::time::sleep(every).await;

		match control.renew(job).await {
			Ok(renewed) => {
				held_until = Instant::now() + lease;

				if renewed.cancelled && fate.load(Ordering::SeqCst) == RUNNING {
					fate.store(CANCELLED, Ordering::SeqCst);
					let _ = stop.send(true);
				}
			}
			// Unanswered renewals cost nothing until the lease they were
			// meant to extend has run out.
			Err(error) if error.is_transient() && Instant::now() < held_until => {}
			Err(_) => {
				fate.store(LOST, Ordering::SeqCst);
				let _ = stop.send(true);
				return;
			}
		}
	}
}

/// Waits until `shutdown` says to stop. With nobody left to say so, it
/// waits for ever.
async fn asked(shutdown: &mut watch::Receiver<bool>) {
	if shutdown.wait_for(|asked| *asked).await.is_err() {
		std::future::pending::<()>().await;
	}
}

impl<B: Builder> Worker<B> {
	/// A worker that calls `control` with its key, keeps its state in
	/// `state`, and builds with `builder`.
	///
	/// # Errors
	///
	/// When the workspace directory can't be made.
	pub fn new(
		control: Control,
		state: State,
		builder: B,
		settings: Settings,
	) -> Result<Self, RunError> {
		std::fs::create_dir_all(&settings.workspace).map_err(|source| {
			StateError::Io {
				path: settings.workspace.clone(),
				source,
			}
		})?;
		let downloads = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.connect_timeout(Duration::from_secs(10))
			.build()
			.map_err(|error| {
				StateError::Io {
					path: settings.workspace.clone(),
					source: std::io::Error::other(error),
				}
			})?;
		Ok(Self {
			control,
			state,
			builder,
			downloads,
			settings,
		})
	}

	/// The worker's state directory.
	pub fn state(&self) -> &State {
		&self.state
	}

	/// Settles what a worker that stopped mid-job left behind: sends an
	/// outcome that was saved, and drops the marker of a build that never
	/// got as far as one.
	///
	/// # Errors
	///
	/// [`RunError::Unreconciled`] for a job other than a build that was
	/// started and has no outcome; [`RunError::Call`] when a saved outcome
	/// couldn't be sent yet; [`RunError::Revoked`].
	pub async fn recover(&self) -> Result<(), RunError> {
		if let Some(saved) = self.state.saved()? {
			match self
				.control
				.finish(saved.job, saved.attempt, saved.outcome.clone())
				.await
			{
				Ok(_) => {
					note(&format!(
						"job {}: reported the outcome saved earlier",
						saved.job
					));
				}
				Err(error) if error.is_unauthorized() => return Err(RunError::Revoked),
				Err(error) if error.is_transient() => return Err(RunError::Call(error)),
				// The control plane has settled the job another way: the
				// lease ran out and it was given out again, or an operator
				// reconciled it.
				Err(error) => {
					note(&format!(
						"job {}: the saved outcome was no longer wanted ({error})",
						saved.job
					));
				}
			}

			return Ok(self.state.clear()?);
		}

		let Some(marker) = self.state.marker()? else {
			return Ok(());
		};

		if marker.kind != JobKind::BuildServer {
			return Err(RunError::Unreconciled {
				job: marker.job,
				kind: marker.kind,
			});
		}

		// A build's only effect is an image named by its digest, so one
		// that was interrupted is simply built again.
		note(&format!(
			"job {}: the build was interrupted; it is built again when its lease runs out",
			marker.job
		));
		Ok(self.state.clear()?)
	}

	/// Takes one turn: settles anything left behind, asks for a job, and
	/// runs it to its end.
	///
	/// # Errors
	///
	/// As [`recover`](Self::recover), and [`RunError::Call`] when the
	/// control plane couldn't be asked.
	pub async fn step(&self) -> Result<Step, RunError> {
		self.recover().await?;
		let assignment = match self.control.claim(self.settings.wait_seconds).await {
			Ok(Some(assignment)) => assignment,
			Ok(None) => return Ok(Step::Idle),
			Err(error) if error.is_unauthorized() => return Err(RunError::Revoked),
			Err(error) if error.is("job_held") => return Ok(Step::Held),
			Err(error) => return Err(RunError::Call(error)),
		};
		let job = assignment.job;
		let ended = match &assignment.input {
			JobInput::BuildServer(input) => self.build(&assignment, input).await?,
			// Nothing is started, so nothing needs reconciling: the lease
			// runs out and says the job never began.
			other => {
				let kind = kind_of(other);
				note(&format!("job {job}: this worker doesn't run {kind:?} jobs"));
				Ended::Abandoned(format!("this worker doesn't run {kind:?} jobs"))
			}
		};
		Ok(Step::Ran { job, ended })
	}

	/// Fetches and builds, or says why there is nothing to report.
	async fn build_in(
		&self,
		input: &BuildServerInput,
		log: &logs::LogSink,
		stop: &Stop,
	) -> Result<BuildOutcome, String> {
		let workspace = tempfile::Builder::new()
			.prefix("build-")
			.tempdir_in(&self.settings.workspace)
			.map_err(|error| format!("the workspace couldn't be made: {}", error.kind()))?;
		let source = workspace.path().join("source");
		let scratch = workspace.path().join("scratch");
		tokio::fs::create_dir(&scratch)
			.await
			.map_err(|error| format!("the workspace couldn't be made: {}", error.kind()))?;
		log.line("worker: fetching the source");
		let fetched = {
			let mut stop = stop.clone();
			tokio::select! {
				fetched = crate::build::fetch(&self.downloads, input, &source, &scratch) => Some(fetched),
				() = stop.stopped() => None,
			}
		};
		let outcome = match fetched {
			None => Ok(BuildOutcome::Stopped),
			Some(Err(SourceError::Wrong(reason))) => {
				log.line(format!("worker: {reason}"));
				Ok(BuildOutcome::Failed {
					reason,
					steps: Vec::new(),
				})
			}
			Some(Err(error)) => Err(error.to_string()),
			Some(Ok(unpacked)) => {
				log.line(format!(
					"worker: unpacked {} files, {} bytes",
					unpacked.files.len(),
					unpacked.bytes
				));
				Ok(self
					.builder
					.build(BuildJob {
						input,
						source: &source,
						scratch: &scratch,
						log: log.clone(),
						stop: stop.clone(),
					})
					.await)
			}
		};

		// The source and everything the build wrote go, however it ended.
		if tokio::task::spawn_blocking(move || workspace.close())
			.await
			.map_or(true, |closed| closed.is_err())
		{
			note("a build's directory couldn't be deleted");
		}

		outcome
	}

	async fn build(
		&self,
		assignment: &Assignment,
		input: &BuildServerInput,
	) -> Result<Ended, RunError> {
		let job = assignment.job;
		let attempt = assignment.attempt;
		// Before anything runs: a worker that dies from here on knows on
		// its return which job it had.
		self.state.start(&Marker {
			job,
			attempt,
			kind: JobKind::BuildServer,
		})?;
		note(&format!(
			"job {job}: building {}/{} #{} (attempt {attempt})",
			input.deployment.organization, input.deployment.project, input.deployment.number
		));
		let lease = Duration::from_secs(u64::from(assignment.lease_seconds.max(3)));
		let every = self.settings.renew_every.unwrap_or(lease / 3);
		let fate = Arc::new(AtomicU8::new(RUNNING));
		let (stopper, stop) = Stop::new();
		let lease_task = tokio::spawn(keep_lease(
			self.control.clone(),
			job,
			every,
			lease,
			Arc::clone(&fate),
			stopper,
		));
		let (log, shipping) = logs::ship(self.control.clone(), job, attempt);
		let built = self.build_in(input, &log, &stop).await;
		let outcome = match (fate.load(Ordering::SeqCst), built) {
			(LOST, _) => Err("its lease ran out".to_owned()),
			(CANCELLED, _) => Ok(BuildOutcome::Stopped),
			(_, built) => built,
		};
		// Every sink is dropped, so the log's task ends once it has sent
		// what it holds.
		drop(log);

		let outcome = match outcome {
			Ok(outcome) => {
				if tokio::time::timeout(LOG_FLUSH, shipping).await.is_err() {
					note(&format!("job {job}: reporting without the end of its log"));
				}

				JobOutcome::BuildServer(outcome)
			}
			Err(reason) => {
				shipping.abort();
				lease_task.abort();
				self.state.clear()?;
				note(&format!("job {job}: left for another attempt: {reason}"));
				return Ok(Ended::Abandoned(reason));
			}
		};

		// Before the control plane is told: a worker that dies between the
		// two sends this on its return.
		self.state.save(&Saved {
			job,
			attempt,
			outcome: outcome.clone(),
		})?;
		let mut wait = Duration::from_secs(1);
		let ended = loop {
			match self.control.finish(job, attempt, outcome.clone()).await {
				Ok(_) => break Ended::Reported(outcome),
				Err(error) if error.is_transient() && fate.load(Ordering::SeqCst) != LOST => {
					tokio::time::sleep(wait).await;
					wait = (wait * 2).min(MAX_BACKOFF);
				}
				Err(error) => break Ended::Abandoned(format!("its outcome wasn't taken: {error}")),
			}
		};
		lease_task.abort();
		self.state.clear()?;

		match &ended {
			Ended::Reported(JobOutcome::BuildServer(BuildOutcome::Succeeded(_))) => {
				note(&format!("job {job}: built"));
			}
			Ended::Reported(JobOutcome::BuildServer(BuildOutcome::Failed { reason, .. })) => {
				note(&format!("job {job}: failed: {reason}"));
			}
			Ended::Reported(_) => note(&format!("job {job}: stopped")),
			Ended::Abandoned(reason) => note(&format!("job {job}: {reason}")),
		}

		Ok(ended)
	}

	/// Runs until `shutdown` says so, finishing the job in hand first.
	///
	/// # Errors
	///
	/// [`RunError::Revoked`], [`RunError::Unreconciled`], or a state
	/// directory that can't be used. A control plane that can't be reached
	/// is waited for.
	pub async fn run(&self, mut shutdown: watch::Receiver<bool>) -> Result<(), RunError> {
		let mut wait = Duration::from_secs(1);

		loop {
			if *shutdown.borrow() {
				return Ok(());
			}

			let pause = match self.step().await {
				// A `claim` that doesn't wait would otherwise ask without rest.
				Ok(Step::Idle) if self.settings.wait_seconds == 0 => Duration::from_secs(1),
				Ok(Step::Idle | Step::Ran { .. }) => {
					wait = Duration::from_secs(1);
					continue;
				}
				Ok(Step::Held) => HELD_WAIT,
				Err(RunError::Call(error)) => {
					note(&format!("waiting for the control plane: {error}"));
					let pause = wait;
					wait = (wait * 2).min(MAX_BACKOFF);
					pause
				}
				Err(error) => return Err(error),
			};

			tokio::select! {
				() = tokio::time::sleep(pause) => {}
				() = asked(&mut shutdown) => {}
			}
		}
	}
}
