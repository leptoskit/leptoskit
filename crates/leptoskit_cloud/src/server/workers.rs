//! The bodies of [`crate::workers`]'s operations: pairing, and the job
//! protocol.
//!
//! Every operation but `pair` takes a worker's key and nothing else
//! ([`identity::worker`](super::identity::worker)), and a worker touches
//! only the jobs it holds: a job it doesn't hold, or doesn't hold any more,
//! answers `job_not_held`, the same as one that doesn't exist.

use std::time::Duration;

use leptoskit_cloud_runtime::LogLine;
use leptoskit_cloud_runtime::Observed;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::audit;
use super::delta;
use super::identity::WorkerCaller;
use super::identity::worker;
use super::jobs;
use super::jobs::Taker;
use super::logs;
use super::logs::StoredLine;
use super::secret;
use super::services;
use super::storage::Bucket;
use super::storage::log_key;
use crate::operator::JobState;
use crate::tables::Job;
use crate::tables::LogChunk;
use crate::tables::LogKind;
use crate::tables::LogUsage;
use crate::tables::NewLogChunk;
use crate::tables::NewLogUsage;
use crate::tables::NewStatusReport;
use crate::tables::NewWorker;
use crate::tables::Pairing;
use crate::tables::Placement;
use crate::tables::StatusReport;
use crate::tables::Worker;
use crate::workers::Assignment;
use crate::workers::DestroyOutcome;
use crate::workers::Finished;
use crate::workers::JobKind;
use crate::workers::JobOutcome;
use crate::workers::MAX_LOG_BYTES;
use crate::workers::MAX_LOG_LINES;
use crate::workers::MAX_STATUS_LOG_LINES;
use crate::workers::MAX_WAIT_SECONDS;
use crate::workers::PairedWorker;
use crate::workers::Renewed;
use crate::workers::RuntimeValue;
use crate::workers::StatusReceived;
use crate::workers::WORKER_KEY_PREFIX;
use crate::workers::WORKER_LOOKUP_LEN;
use crate::workers::WORKER_SECRET_LEN;
use crate::workers::WorkerRole;
use crate::workers::WorkerSummary;
use crate::workers::check_capabilities;
use crate::workers::is_pairing_code;

/// The longest name a worker may have.
const MAX_NAME_LEN: usize = 64;

/// How often a waiting `claim` looks again when nothing wakes it: another
/// server may have queued a job.
const POLL: Duration = Duration::from_secs(1);

/// How many queued jobs one claim reads at a time. It reads page after page
/// until it finds one this worker may take, so jobs it may not take, however
/// many, never hide one it may.
const CANDIDATES: u64 = 100;

fn not_held(job: Uuid) -> rpc::Error {
	rpc::Error::not_found(format!(
		"job_not_held: this worker doesn't hold job {job}; its lease may have run out"
	))
}

fn pairing_invalid() -> rpc::Error {
	rpc::Error::not_found(
		"pairing_invalid: that pairing code is unknown, used, or expired; ask an operator for a \
		 new one",
	)
}

/// `worker` as operators see it, with its organisation's and cluster's
/// names.
pub(crate) async fn summary(
	db: &impl leptoskit_db::Read,
	worker: &Worker,
) -> rpc::Result<WorkerSummary> {
	let organization = match worker.organization_id {
		Some(id) => {
			crate::tables::Organization::query()
				.filter(|row| row.id.eq(id))
				.first(db)
				.await?
				.map(|row| row.slug)
		}
		None => None,
	};
	let cluster = match worker.cluster_id {
		Some(id) => {
			crate::tables::Cluster::query()
				.filter(|row| row.id.eq(id))
				.first(db)
				.await?
				.map(|row| row.name)
		}
		None => None,
	};
	Ok(WorkerSummary {
		id: worker.id.uuid(),
		name: worker.name.clone(),
		role: worker.role,
		prefix: worker.prefix.clone(),
		organization,
		cluster,
		capabilities: worker.capabilities.0.clone(),
		paired_at: worker.created_at,
		last_seen_at: worker.last_seen_at,
		revoked_at: worker.revoked_at,
	})
}

pub(crate) async fn pair(
	cx: &rpc::ActionCx,
	code: String,
	name: String,
	capabilities: Vec<String>,
) -> rpc::Result<PairedWorker> {
	cx.mutate(PairWorker {
		code,
		name,
		capabilities,
	})
	.await
}

/// `pair`'s writes, in one transaction. Never registered, so it never has
/// a receipt: its answer holds the key.
#[derive(Clone, Serialize, Deserialize)]
struct PairWorker {
	code: String,
	name: String,
	capabilities: Vec<String>,
}

impl rpc::Operation for PairWorker {
	type Context = rpc::MutationCx;
	type Output = PairedWorker;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "pair";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<PairedWorker>> + Send {
		pair_worker(cx, self)
	}
}

async fn pair_worker(cx: &rpc::MutationCx, args: PairWorker) -> rpc::Result<PairedWorker> {
	let PairWorker {
		code,
		name,
		capabilities,
	} = args;
	let code = code.trim();

	if !is_pairing_code(code) {
		return Err(pairing_invalid());
	}

	let hash = secret::hash(code);
	let now = leptoskit_db::now();
	let Some(mut pairing) = Pairing::query()
		.filter(|pairing| pairing.code_hash.eq(hash.clone()))
		.first(cx.tx())
		.await?
		.filter(|pairing| pairing.used_at.is_none() && pairing.expires_at > now)
	else {
		return Err(pairing_invalid());
	};
	let name = name.trim().to_owned();

	if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
	{
		return Err(rpc::Error::invalid(format!(
			"invalid_name: a worker's name has 1 to {MAX_NAME_LEN} characters and no control \
			 characters"
		)));
	}

	let capabilities = check_capabilities(capabilities)
		.map_err(|error| rpc::Error::invalid(format!("invalid_capabilities: {error}")))?;
	let lookup = secret::random_hex(WORKER_LOOKUP_LEN / 2)
		.map_err(|_| rpc::Error::server("no randomness to make a key with"))?;
	let worker_secret = secret::random_hex(WORKER_SECRET_LEN / 2)
		.map_err(|_| rpc::Error::server("no randomness to make a key with"))?;
	let worker = NewWorker {
		name,
		role: pairing.role,
		organization_id: pairing.organization_id,
		cluster_id: pairing.cluster_id,
		prefix: lookup.clone(),
		secret_hash: secret::hash(&worker_secret),
		capabilities: Json(capabilities),
		last_seen_at: None,
		revoked_at: None,
	}
	.insert(cx.tx())
	.await?;
	pairing.used_at = Some(now);
	pairing.worker_id = Some(worker.id);
	pairing.save(cx.tx()).await?;
	audit::record_system(
		cx.tx(),
		"worker.pair",
		Some(format!("{} {} ({})", worker.id, worker.name, worker.role)),
	)
	.await?;
	let services = services(cx.context())?;
	Ok(PairedWorker {
		worker: summary(cx.tx(), &worker).await?,
		key: format!("{WORKER_KEY_PREFIX}{lookup}_{worker_secret}"),
		lease_seconds: services.shipping().settings.lease_seconds(),
	})
}

pub(crate) async fn claim(
	cx: &rpc::ActionCx,
	wait_seconds: Option<u32>,
) -> rpc::Result<Option<Assignment>> {
	worker(cx.context())?;
	let services = services(cx.context())?.clone();
	let wait = Duration::from_secs(u64::from(
		wait_seconds.unwrap_or_default().min(MAX_WAIT_SECONDS),
	));
	let deadline = tokio::time::Instant::now() + wait;

	loop {
		let notified = services.shipping().jobs.notified();
		tokio::pin!(notified);
		notified.as_mut().enable();

		if let Some(assignment) = Box::pin(cx.mutate(ClaimJob {})).await? {
			return Ok(Some(assignment));
		}

		let now = tokio::time::Instant::now();

		if now >= deadline {
			return Ok(None);
		}

		let pause = (deadline - now).min(POLL);
		tokio::select! {
			() = &mut notified => {}
			() = tokio::time::sleep(pause) => {}
		}
	}
}

/// One attempt to claim, in one transaction. Never registered.
#[derive(Clone, Serialize, Deserialize)]
struct ClaimJob {}

impl rpc::Operation for ClaimJob {
	type Context = rpc::MutationCx;
	type Output = Option<Assignment>;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "claim";

	fn run(
		self,
		cx: &rpc::MutationCx,
	) -> impl Future<Output = rpc::Result<Option<Assignment>>> + Send {
		claim_job(cx)
	}
}

/// The worker's row as it is now, or `Unauthorized` once it is revoked.
async fn live_worker(tx: &Tx, caller: &WorkerCaller) -> rpc::Result<Worker> {
	let id = caller.id;
	Worker::query()
		.filter(|worker| worker.id.eq(id) & worker.revoked_at.is_null())
		.first(tx)
		.await?
		.ok_or_else(|| rpc::Error::unauthorized("worker_key_required: this worker was revoked"))
}

async fn claim_job(cx: &rpc::MutationCx) -> rpc::Result<Option<Assignment>> {
	let caller = worker(cx.context())?;
	let services = services(cx.context())?;
	let tx = cx.tx();
	let row = live_worker(tx, caller).await?;
	let worker_id = row.id;

	if let Some(held) = Job::query()
		.filter(|job| job.worker_id.eq(Some(worker_id)) & job.state.eq(JobState::Leased))
		.first(tx)
		.await?
	{
		return Err(rpc::Error::conflict(format!(
			"job_held: this worker holds job {}; renew and finish it before claiming another",
			held.id.uuid()
		)));
	}

	let kinds: Vec<JobKind> = match row.role {
		WorkerRole::Build => vec![JobKind::BuildServer],
		WorkerRole::Cluster => vec![JobKind::Release, JobKind::DestroyEnvironment],
	};
	// A cluster worker takes one job per environment at a time, and none
	// while one is unknown.
	let busy: Vec<_> = if row.role == WorkerRole::Cluster {
		Job::query()
			.filter(|job| {
				job.cluster_id.eq(row.cluster_id)
					& (job.state.eq(JobState::Leased) | job.state.eq(JobState::Unknown))
			})
			.all(tx)
			.await?
			.into_iter()
			.filter_map(|job| job.environment_id)
			.collect()
	} else {
		Vec::new()
	};
	let taker = Taker {
		role: row.role,
		organization: row.organization_id,
		cluster: row.cluster_id,
		capabilities: &row.capabilities.0,
	};
	// Role, organisation, and cluster are decided by the query, so another
	// tenant's or another cluster's queue never fills a page; capabilities
	// and busy environments are decided here, a page at a time.
	let organization = row.organization_id;
	let cluster = row.cluster_id;
	let role = row.role;
	let mut offset = 0;
	let found = loop {
		let page = Job::query()
			.filter(|job| {
				let mut eligible = job.state.eq(JobState::Queued) & job.kind.in_list(kinds.clone());

				if let Some(organization) = organization {
					eligible = eligible & job.organization_id.eq(organization);
				}

				if role == WorkerRole::Cluster {
					eligible = eligible & job.cluster_id.eq(cluster);
				}

				eligible
			})
			.order_by(|job| job.created_at.asc())
			.order_by(|job| job.id.asc())
			.offset(offset)
			.limit(CANDIDATES)
			.all(tx)
			.await?;
		let read = page.len() as u64;
		let found = page.into_iter().find(|job| {
			jobs::takes(&taker, job)
				&& job
					.environment_id
					.is_none_or(|environment| !busy.contains(&environment))
		});

		if found.is_some() || read < CANDIDATES {
			break found;
		}

		offset += CANDIDATES;
	};

	let Some(mut job) = found else {
		return Ok(None);
	};
	let settings = services.shipping().settings;
	let now = leptoskit_db::now();
	job.state = JobState::Leased;
	job.worker_id = Some(worker_id);
	job.attempt += 1;
	job.claimed_at = Some(now);
	job.lease_expires_at = Some(lease_until(&job, now, &settings));
	// A release's secrets: leased to this job, in this transaction, so no
	// value is sent unless the lease and its record commit.
	let secrets = if job.kind == JobKind::Release {
		match super::deployments::release_secrets(tx, services, &job, worker_id).await? {
			Ok(secrets) => secrets,
			Err(why) => {
				// A release whose secrets can't be read never starts: it
				// fails here, so it doesn't sit at the head of its
				// cluster's queue for good, and this worker asks again.
				job.state = JobState::Abandoned;
				job.worker_id = None;
				job.attempt -= 1;
				job.claimed_at = None;
				job.lease_expires_at = None;
				job.save(tx).await?;
				super::deployments::release_unstartable(tx, &job, &why).await?;
				return Ok(None);
			}
		}
	} else {
		crate::workers::LeasedSecrets::default()
	};
	job.save(tx).await?;
	let attempt = u32::try_from(job.attempt).unwrap_or(u32::MAX);

	if job.kind == JobKind::BuildServer {
		super::deployments::build_claimed(tx, &job, attempt).await?;
	} else {
		super::deployments::release_claimed(tx, &job, attempt).await?;
	}

	Ok(Some(Assignment {
		job: job.id.uuid(),
		attempt,
		lease_expires_at: job.lease_expires_at.unwrap_or(now),
		lease_seconds: settings.lease_seconds(),
		input: jobs::assignment_input(services, &job.input.0, settings.download)?,
		secrets,
	}))
}

/// When a lease given or renewed at `now` ends: a lease's length on, but
/// for a build never past its claim and the build deadline.
fn lease_until(
	job: &Job,
	now: leptoskit_db::Timestamp,
	settings: &super::ShippingSettings,
) -> leptoskit_db::Timestamp {
	let lease = now + delta(settings.lease);

	match (job.kind, job.claimed_at) {
		(JobKind::BuildServer, Some(claimed)) => {
			lease.min(claimed + delta(settings.build_deadline))
		}
		_ => lease,
	}
}

/// The job `id` while this worker holds it under a live lease. A lease
/// past its end holds nothing, whether or not the sweeper has seen it yet.
async fn held(tx: &Tx, caller: &WorkerCaller, id: Uuid) -> rpc::Result<Job> {
	let worker_id = caller.id;
	let now = leptoskit_db::now();
	Job::query()
		.filter(|job| {
			job.id.eq(Id::<Job>::from_uuid(id))
				& job.worker_id.eq(Some(worker_id))
				& job.state.eq(JobState::Leased)
				& job.lease_expires_at.gt(now)
		})
		.first(tx)
		.await?
		.ok_or_else(|| not_held(id))
}

pub(crate) async fn renew(cx: &rpc::MutationCx, job: Uuid) -> rpc::Result<Renewed> {
	let caller = worker(cx.context())?;
	let services = services(cx.context())?;
	live_worker(cx.tx(), caller).await?;
	let mut row = held(cx.tx(), caller, job).await?;
	let now = leptoskit_db::now();
	let expires = lease_until(&row, now, &services.shipping().settings);

	if expires <= now {
		return Err(not_held(job));
	}

	row.lease_expires_at = Some(expires);
	row.save(cx.tx()).await?;
	Ok(Renewed {
		lease_expires_at: expires,
		cancelled: row.cancel_requested,
	})
}

pub(crate) async fn append_log(
	cx: &rpc::ActionCx,
	job: Uuid,
	attempt: u32,
	seq: u32,
	lines: Vec<String>,
) -> rpc::Result<()> {
	worker(cx.context())?;
	let services = services(cx.context())?.clone();

	if lines.is_empty() {
		return Err(rpc::Error::invalid(
			"log_empty: a chunk has at least one line",
		));
	}

	let size: usize = lines.iter().map(String::len).sum();

	if lines.len() > MAX_LOG_LINES || size > MAX_LOG_BYTES {
		return Err(rpc::Error::invalid(format!(
			"log_too_large: a chunk is at most {MAX_LOG_LINES} lines and {} KiB",
			MAX_LOG_BYTES / 1024
		)));
	}

	let digest = logs::chunk_digest(lines.iter().map(String::as_str));
	let chunk = cx
		.mutate(AppendChunk {
			job,
			attempt,
			seq,
			lines: u32::try_from(lines.len()).unwrap_or(u32::MAX),
			bytes: u32::try_from(size).unwrap_or(u32::MAX),
			digest,
		})
		.await?;
	let secrets = &services.shipping().redactions;
	let now = leptoskit_db::now();
	let cut: Vec<String> = lines.iter().map(|line| logs::cut(line)).collect();
	let cut: Vec<&str> = cut.iter().map(String::as_str).collect();
	let stored: Vec<StoredLine> = logs::redact_lines(&cut, secrets)
		.into_iter()
		.map(|text| {
			StoredLine {
				at: now,
				process: None,
				pod: None,
				text,
			}
		})
		.collect();
	let bytes = serde_json::to_vec(&stored)
		.map_err(|_| rpc::Error::server("couldn't write a log chunk"))?;
	// Sent again, the chunk's row is there and its object may not be:
	// writing the same lines again is harmless.
	services
		.shipping()
		.store
		.put(Bucket::Logs, &chunk, bytes)
		.await?;
	Ok(())
}

/// `append_log`'s check and index row, in one transaction. Never
/// registered.
#[derive(Clone, Serialize, Deserialize)]
struct AppendChunk {
	job: Uuid,
	attempt: u32,
	seq: u32,
	lines: u32,
	bytes: u32,
	digest: String,
}

impl rpc::Operation for AppendChunk {
	type Context = rpc::MutationCx;
	type Output = String;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "append_log";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<String>> + Send {
		append_chunk(cx, self)
	}
}

/// Checks a chunk and indexes it; answers the key its object goes under.
async fn append_chunk(cx: &rpc::MutationCx, args: AppendChunk) -> rpc::Result<String> {
	let caller = worker(cx.context())?;
	let services = services(cx.context())?;
	let tx = cx.tx();
	let mut job = held(tx, caller, args.job).await?;

	if i64::from(args.attempt) != job.attempt {
		return Err(not_held(args.job));
	}

	let job_id = job.id;
	let attempt = job.attempt;
	let seq = i64::from(args.seq);

	if let Some(existing) = LogChunk::query()
		.filter(|chunk| {
			chunk.job_id.eq(Some(job_id)) & chunk.attempt.eq(attempt) & chunk.seq.eq(seq)
		})
		.first(tx)
		.await?
	{
		return if existing.digest == args.digest {
			Ok(existing.object_key)
		} else {
			Err(rpc::Error::conflict(format!(
				"log_chunk_conflict: chunk {} of attempt {} was sent with other lines",
				args.seq, args.attempt
			)))
		};
	}

	let sent = LogChunk::query()
		.filter(|chunk| chunk.job_id.eq(Some(job_id)) & chunk.attempt.eq(attempt))
		.count(tx)
		.await?;

	if sent != u64::from(args.seq) {
		return Err(rpc::Error::conflict(format!(
			"log_out_of_order: the next chunk of attempt {} is {sent}, not {}",
			args.attempt, args.seq
		)));
	}

	let (Some(deployment), Some(environment)) = (job.deployment_id, job.environment_id) else {
		return Err(rpc::Error::invalid(
			"log_unavailable: only a build keeps a log; a cluster worker reports lines with \
			 `report_status`",
		));
	};
	let settings = services.shipping().settings;
	let bytes = i64::from(args.bytes);
	let build_cap = i64::try_from(settings.build_log_cap).unwrap_or(i64::MAX);

	if job.log_bytes.saturating_add(bytes) > build_cap {
		return Err(rpc::Error::conflict(format!(
			"log_full: this build has sent {} KiB of log, and may send at most {} KiB",
			job.log_bytes / 1024,
			build_cap / 1024
		)));
	}

	charge_log(tx, job.organization_id, bytes, &settings).await?;
	job.log_bytes += bytes;
	job.save(tx).await?;
	let first_line = next_line(tx, |chunk| chunk.deployment_id.eq(Some(deployment))).await?;
	let key = log_key(
		job.organization_id.uuid(),
		environment.uuid(),
		Uuid::now_v7(),
	);
	NewLogChunk {
		kind: LogKind::Build,
		organization_id: job.organization_id,
		environment_id: environment,
		deployment_id: Some(deployment),
		job_id: Some(job_id),
		attempt,
		seq,
		first_line,
		lines: i64::from(args.lines),
		bytes,
		digest: args.digest,
		object_key: key.clone(),
		expires_at: leptoskit_db::now() + delta(settings.build_logs),
	}
	.insert(tx)
	.await?;
	Ok(key)
}

/// Counts `bytes` more of build log against `organization`, or refuses
/// with `Conflict` (`log_full`) when that takes it past its cap. The count
/// goes down as the sweeper deletes chunks.
async fn charge_log(
	tx: &Tx,
	organization: Id<crate::tables::Organization>,
	bytes: i64,
	settings: &super::ShippingSettings,
) -> rpc::Result<()> {
	let cap = i64::try_from(settings.organization_log_cap).unwrap_or(i64::MAX);
	let usage = LogUsage::query()
		.filter(|usage| usage.organization_id.eq(organization))
		.first(tx)
		.await?;
	let used = usage.as_ref().map_or(0, |usage| usage.bytes);

	if used.saturating_add(bytes) > cap {
		return Err(rpc::Error::conflict(format!(
			"log_full: the organisation keeps {} MiB of build logs, its most; older logs expire \
			 after a while",
			used / 1024 / 1024
		)));
	}

	match usage {
		Some(mut usage) => {
			usage.bytes += bytes;
			usage.save(tx).await?;
		}
		None => {
			NewLogUsage {
				organization_id: organization,
				bytes,
			}
			.insert(tx)
			.await?;
		}
	}

	Ok(())
}

/// The number the next chunk's first line gets in the log `filter` picks:
/// one past the last line so far, from 1.
async fn next_line(
	tx: &Tx,
	filter: impl FnOnce(&<LogChunk as leptoskit_db::Table>::Columns) -> leptoskit_db::Expr,
) -> rpc::Result<i64> {
	let last = LogChunk::query()
		.filter(filter)
		.order_by(|chunk| chunk.first_line.desc())
		.first(tx)
		.await?;
	Ok(last.map_or(1, |chunk| chunk.first_line + chunk.lines))
}

pub(crate) async fn finish(
	cx: &rpc::MutationCx,
	job: Uuid,
	attempt: u32,
	outcome: JobOutcome,
) -> rpc::Result<Finished> {
	let caller = worker(cx.context())?;
	let services = services(cx.context())?;
	let tx = cx.tx();
	live_worker(tx, caller).await?;
	let worker_id = caller.id;
	let Some(mut row) = Job::query()
		.filter(|row| row.id.eq(Id::<Job>::from_uuid(job)) & row.worker_id.eq(Some(worker_id)))
		.first(tx)
		.await?
		.filter(|row| row.attempt == i64::from(attempt))
	else {
		return Err(not_held(job));
	};

	if outcome.kind() != row.kind {
		return Err(rpc::Error::invalid(format!(
			"outcome_kind: job {job} is a {} job, and this is a {} outcome",
			row.kind,
			outcome.kind()
		)));
	}

	// A removal that failed and then worked, on the worker's own retry,
	// may say so: the failure it recorded is what it replaces.
	let retried_removal = row.state == JobState::Unknown
		&& matches!(
			&row.outcome.0,
			Some(JobOutcome::DestroyEnvironment(
				DestroyOutcome::Failed { .. }
			))
		)
		&& outcome == JobOutcome::DestroyEnvironment(DestroyOutcome::Destroyed);

	if let Some(recorded) = row.outcome.0.as_ref().filter(|_| !retried_removal) {
		return if *recorded == outcome {
			Ok(Finished {
				job,
				repeated: true,
			})
		} else {
			Err(rpc::Error::conflict(format!(
				"outcome_conflict: job {job} finished already, with another outcome"
			)))
		};
	}

	match row.state {
		// A build's lease past its end holds nothing, even before the
		// sweeper sees it: the build is the sweeper's to offer again.
		JobState::Leased
			if row.kind == JobKind::BuildServer
				&& row
					.lease_expires_at
					.is_none_or(|expires| expires <= leptoskit_db::now()) =>
		{
			return Err(not_held(job));
		}
		// A release or removal reports what happened whenever it can, its
		// lease run out or the sweeper having called it unknown: the
		// cluster did it or not either way, and is never asked twice.
		JobState::Leased | JobState::Unknown => {}
		JobState::Done => {
			return Err(rpc::Error::conflict(format!(
				"already_reconciled: an operator settled job {job} from what the cluster runs"
			)));
		}
		JobState::Queued | JobState::Abandoned => return Err(not_held(job)),
	}

	// A removal that failed left workloads, routes, or data behind, and
	// nothing else remembers them: the job stays unknown, in front of
	// operators, until one reconciles it, which can run it again.
	row.state = match &outcome {
		JobOutcome::DestroyEnvironment(DestroyOutcome::Failed { .. }) => JobState::Unknown,
		_ => JobState::Done,
	};
	row.lease_expires_at = None;
	row.outcome = Json(Some(outcome.clone()));
	row.save(tx).await?;

	match &outcome {
		JobOutcome::BuildServer(outcome) => {
			super::deployments::build_finished(tx, &row, outcome).await?;
		}
		JobOutcome::Release(outcome) => {
			super::deployments::release_finished(tx, &row, outcome).await?;
		}
		JobOutcome::DestroyEnvironment(DestroyOutcome::Destroyed) => {
			// Off the cluster: the hostnames it held are free.
			if let Some(environment) = row.environment_id {
				super::domains::environment_destroyed(tx, environment).await?;
			}
		}
		JobOutcome::DestroyEnvironment(DestroyOutcome::Failed { .. }) => {}
	}

	let waker = services.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	Ok(Finished {
		job,
		repeated: false,
	})
}

pub(crate) async fn report_status(
	cx: &rpc::ActionCx,
	environment: Uuid,
	observed: RuntimeValue<Observed>,
	logs: Vec<RuntimeValue<LogLine>>,
) -> rpc::Result<StatusReceived> {
	let caller = worker(cx.context())?;

	if caller.role != WorkerRole::Cluster {
		return Err(rpc::Error::forbidden(
			"cluster_worker_required: only a cluster's worker reports what it runs",
		));
	}

	if logs.len() > MAX_STATUS_LOG_LINES {
		return Err(rpc::Error::invalid(format!(
			"log_too_large: a report carries at most {MAX_STATUS_LOG_LINES} lines"
		)));
	}

	if observed.0.environment.environment.id != environment {
		return Err(rpc::Error::invalid(
			"environment_mismatch: the report observes another environment",
		));
	}

	let services = services(cx.context())?.clone();
	let messages: Vec<String> = logs
		.iter()
		.map(|RuntimeValue(line)| logs::cut(&line.message))
		.collect();
	let messages: Vec<&str> = messages.iter().map(String::as_str).collect();
	let texts = logs::redact_lines(&messages, &services.shipping().redactions);
	let lines: Vec<StoredLine> = logs
		.into_iter()
		.zip(texts)
		.map(|(RuntimeValue(line), text)| {
			let at = chrono::DateTime::from_timestamp_millis(line.at_millis)
				.unwrap_or_else(leptoskit_db::now);
			StoredLine {
				at,
				process: Some(logs::plain(&logs::cut(&line.process))),
				pod: line.pod.as_deref().map(|pod| logs::plain(&logs::cut(pod))),
				text,
			}
		})
		.collect();
	let groups = status_chunks(&lines)?;
	let received = cx
		.mutate(ReportStatus {
			environment,
			observed: observed.0,
			chunks: groups
				.iter()
				.map(|(group, bytes)| {
					ReportedChunk {
						lines: u32::try_from(group.len()).unwrap_or(u32::MAX),
						bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
						digest: logs::chunk_digest(group.iter().map(|line| line.text.as_str())),
					}
				})
				.collect(),
		})
		.await?;

	for (key, (_, bytes)) in received.chunks.iter().zip(groups) {
		services
			.shipping()
			.store
			.put(Bucket::Logs, key, bytes)
			.await?;
	}

	Ok(received.status)
}

/// `lines` in runs whose stored object stays under
/// [`MAX_SMALL_OBJECT`](super::storage::MAX_SMALL_OBJECT), which is all the
/// log readers read, each with its bytes as stored.
fn status_chunks(lines: &[StoredLine]) -> rpc::Result<Vec<(&[StoredLine], Vec<u8>)>> {
	let limit = super::storage::MAX_SMALL_OBJECT;
	let mut chunks = Vec::new();
	let mut start = 0;

	while start < lines.len() {
		// `[` and `]`, then each line and the comma before the next.
		let mut size = 2;
		let mut end = start;

		while end < lines.len() {
			let line = serde_json::to_vec(&lines[end])
				.map_err(|_| rpc::Error::server("couldn't write a log chunk"))?;
			let next = size + line.len() + usize::from(end > start);

			if next > limit && end > start {
				break;
			}

			size = next;
			end += 1;
		}

		let group = &lines[start..end];
		let bytes = serde_json::to_vec(group)
			.map_err(|_| rpc::Error::server("couldn't write a log chunk"))?;
		chunks.push((group, bytes));
		start = end;
	}

	Ok(chunks)
}

/// `report_status`'s writes. Never registered.
#[derive(Clone, Serialize, Deserialize)]
struct ReportStatus {
	environment: Uuid,
	observed: Observed,
	/// The report's lines, in chunks small enough to read back.
	chunks: Vec<ReportedChunk>,
}

/// One chunk of a report's lines.
#[derive(Clone, Serialize, Deserialize)]
struct ReportedChunk {
	lines: u32,
	bytes: u32,
	digest: String,
}

/// What [`ReportStatus`] wrote: the answer, and where each chunk's lines
/// go.
#[derive(Clone, Serialize, Deserialize)]
struct Reported {
	status: StatusReceived,
	chunks: Vec<String>,
}

impl rpc::Operation for ReportStatus {
	type Context = rpc::MutationCx;
	type Output = Reported;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "report_status";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Reported>> + Send {
		report(cx, self)
	}
}

async fn report(cx: &rpc::MutationCx, args: ReportStatus) -> rpc::Result<Reported> {
	let caller = worker(cx.context())?;
	let services = services(cx.context())?;
	let tx = cx.tx();
	live_worker(tx, caller).await?;
	let environment_id = Id::from_uuid(args.environment);
	let not_found = || {
		rpc::Error::not_found(format!(
			"environment_not_found: environment {} isn't placed on this worker's cluster",
			args.environment
		))
	};
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(tx)
		.await?
		.filter(|placement| Some(placement.cluster_id) == caller.cluster)
		.ok_or_else(not_found)?;
	let environment = crate::tables::Environment::query()
		.filter(|row| row.id.eq(environment_id))
		.one(tx)
		.await?;
	let project_id = environment.project_id;
	let project = crate::tables::Project::query()
		.filter(|row| row.id.eq(project_id))
		.one(tx)
		.await?;
	let now = leptoskit_db::now();

	if let Some(mut existing) = StatusReport::query()
		.filter(|report| report.environment_id.eq(environment_id))
		.first(tx)
		.await?
	{
		existing.observed = Json(args.observed);
		existing.cluster_id = placement.cluster_id;
		existing.reported_at = now;
		existing.save(tx).await?;
	} else {
		NewStatusReport {
			environment_id,
			cluster_id: placement.cluster_id,
			observed: Json(args.observed),
			reported_at: now,
		}
		.insert(tx)
		.await?;
	}

	let mut chunks = Vec::with_capacity(args.chunks.len());
	let mut first_line = next_line(tx, |chunk| {
		chunk.environment_id.eq(environment_id) & chunk.kind.eq(LogKind::Runtime)
	})
	.await?;
	let mut lines = 0;

	for reported in args.chunks {
		let key = log_key(
			project.organization_id.uuid(),
			args.environment,
			Uuid::now_v7(),
		);
		NewLogChunk {
			kind: LogKind::Runtime,
			organization_id: project.organization_id,
			environment_id,
			deployment_id: None,
			job_id: None,
			attempt: 0,
			seq: 0,
			first_line,
			lines: i64::from(reported.lines),
			bytes: i64::from(reported.bytes),
			digest: reported.digest,
			object_key: key.clone(),
			expires_at: now + delta(services.shipping().settings.runtime_logs),
		}
		.insert(tx)
		.await?;
		first_line += i64::from(reported.lines);
		lines += reported.lines;
		chunks.push(key);
	}

	Ok(Reported {
		status: StatusReceived {
			environment: args.environment,
			reported_at: now,
			log_lines: lines,
		},
		chunks,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::workers::PAIRING_CODE_LEN;

	#[test]
	fn a_pairing_code_has_the_length_the_constant_says() {
		let code = format!("lkp_{}", "a".repeat(PAIRING_CODE_LEN));
		assert!(is_pairing_code(&code));
	}
}
