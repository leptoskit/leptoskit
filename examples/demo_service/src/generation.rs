//! One generation: the durable state under `gen/<UTC day>.f<format>`, and
//! everything this process opens on top of it.
//!
//! The rules that make this safe, in the order they happen:
//!
//! 1. **Validate before touching.** A generation is opened only when its
//!    `published` marker names this binary's format and schema fingerprint. The
//!    check runs before any database connection or migration, so incompatible
//!    state is refused while it is still untouched.
//! 2. **Hold the lifetime lock before opening.** A generation's `lifetime.lock`
//!    is taken shared under the build lock, before the first database opens,
//!    and kept until exec or exit. Nothing deletes a generation whose lifetime
//!    lock it cannot take exclusively.
//! 3. **Build at the final path; publish with a marker.** A new generation is
//!    built where it will always live — never renamed after SQLite opened it —
//!    and becomes openable only when its fsynced `published` marker is renamed
//!    into place.
//! 4. **Seed through the real service, and prove it.** The month goes through
//!    the analytics service's own ingestion, whose committed-ack responses mean
//!    the rows are stored; the barrier is the read-back count through the
//!    service's read route, recorded in the marker and re-checked on every
//!    open. No direct SQL writes seed events.
//! 5. **Collect only what nobody holds.** Retention keeps today and yesterday;
//!    anything older — or any unpublished leftover — is removed only if its
//!    lifetime lock can be taken exclusively without blocking. Whatever
//!    survives counts against a small, documented budget, and an over-budget
//!    volume fails honestly instead of growing unbounded or unlinking live
//!    databases.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use axum::Router;
use leptoskit_auth::Keys;
use leptoskit_axum::Readiness;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;
use tokio::sync::Semaphore;
use tower::ServiceExt as _;

use crate::clock;
use crate::config::Config;
use crate::locks;
use crate::marker::GENERATION_FORMAT;
use crate::marker::Published;
use crate::secrets::Secrets;
use crate::site;

/// The demo project's `leptoskit.toml`, as last committed.
const COMMITTED: &str = include_str!("../project/leptoskit.toml");

/// The control service's project: the simulated stand-in every operation
/// rehearses against.
pub const PROJECT: &str = "notes-production";

/// The most generation directories the volume may hold once collection
/// has run. Today and yesterday live here, one directory per format in
/// use, plus at most a couple of survivors a live holder still keeps.
const MAX_GENERATION_DIRS: usize = 4;

/// The most bytes the generation directories may occupy in total. The
/// seed is a few megabytes; the budget catches runaway growth long before
/// the disk fills.
const GENERATION_BUDGET_BYTES: u64 = 1024 * 1024 * 1024;

/// A day, in milliseconds.
const DAY_MS: i64 = 86_400_000;

/// The most concurrent `/demo/generation` streams this process serves.
const GENERATION_STREAMS: usize = 1_000;

/// Everything one process image serves, until exec or exit replaces it.
#[derive(Clone)]
pub struct Generation {
	/// The generation's UTC day.
	pub day: String,
	/// The generation's directory, at its stable final path.
	pub dir: PathBuf,
	/// The credentials this generation runs on.
	pub secrets: Secrets,
	/// The control service, whose worker this process may or may not run.
	pub control: Service,
	/// The site's routes (health included).
	pub site: Router,
	/// The admin's routes (health included, operation streams bounded).
	pub admin: Router,
	/// The private loopback's routes: health and the analytics service.
	pub loopback: Router,
	/// Permits for the generation event stream's concurrency cap.
	pub permits: Arc<Semaphore>,
}

impl Generation {
	/// The generation's event stream concurrency permits.
	#[must_use]
	pub const fn permits(&self) -> &Arc<Semaphore> {
		&self.permits
	}
}

/// The generation directory for `day`.
fn dir_for(volume: &Path, day: &str) -> PathBuf {
	volume
		.join("gen")
		.join(format!("{day}.f{GENERATION_FORMAT}"))
}

/// Opens today's generation, or builds it. Called under the build lock.
///
/// # Errors
///
/// When validation, locking, opening, or building fails. The caller keeps
/// the build lock it passed in and re-executes for a bounded retry.
pub async fn open_or_build(
	config: &Config,
	day: &str,
	now_millis: i64,
	terminated: &crate::shutdown::Terminated,
	readiness: &Readiness,
) -> Result<Generation> {
	let dir = dir_for(&config.volume, day);
	let fingerprint = Published::current_fingerprint()?;

	let published = Published::read(&dir).ok();
	if let Some(marker) = &published {
		if marker.is_valid_for(&dir_name(&dir), &fingerprint) {
			return open(config, &dir, marker, terminated, readiness).await;
		}
		// Same format, different schema: the source's schemas drifted
		// without a format bump. Refuse before anything in the
		// generation is opened or migrated — rebuilding would wipe a
		// live day's state instead of surfacing the bug, and migrating
		// would change databases this binary doesn't understand. The
		// fix is a format bump and a new directory, never this.
		if marker.format == GENERATION_FORMAT && marker.day == day {
			anyhow::bail!(
				"the published generation at {} carries schema fingerprint {} but this binary \
				 builds {fingerprint}: the schemas drifted without a GENERATION_FORMAT bump, so \
				 the generation is refused untouched",
				dir.display(),
				marker.fingerprint
			);
		}
	}

	build(config, &dir, day, now_millis, terminated, readiness).await
}

/// A generation directory's file name.
fn dir_name(dir: &Path) -> String {
	dir.file_name()
		.unwrap_or_default()
		.to_string_lossy()
		.into_owned()
}

/// Opens a published generation: credentials, then databases, then the
/// services on top. The seed barrier is re-checked against the marker,
/// so a generation whose durable events are gone is refused, never
/// silently served empty.
async fn open(
	config: &Config,
	dir: &Path,
	marker: &Published,
	terminated: &crate::shutdown::Terminated,
	readiness: &Readiness,
) -> Result<Generation> {
	locks::lifetime_shared(dir, terminated.cancellable()).await?;
	let secrets = Secrets::load(dir, &marker.day)?;

	// The analytics service first: the barrier's read goes through it.
	let analytics = site::start_analytics(dir, &config.site_origin, &secrets.analytics_token)
		.await
		.context("starting the analytics service")?;
	let counted = site::count_events(&analytics, &secrets.analytics_token).await?;
	ensure!(
		counted == marker.seed_events,
		"the generation's seeded events are gone: the marker records {} and the service reports \
		 {counted}",
		marker.seed_events
	);

	let site_db = connect_site_db(dir).await?;
	let site = site::rebuild(&site_db, site_keys(&secrets), &config.site_origin)
		.context("starting the site")?;

	let store = Store::open(&dir.join("console.sqlite3")).context("opening the console store")?;
	let control = Service::new(store, &secrets.control_key, &config.admin_origin)
		.context("starting the control service")?;
	let admin = admin_router(config, dir, &secrets, &control, site.clone()).await?;

	Ok(assemble(GenerationParts {
		day: marker.day.clone(),
		dir: dir.to_owned(),
		secrets,
		control,
		site: public_router(readiness, site),
		admin: public_router(readiness, admin),
		loopback: loopback_router(readiness, analytics),
	}))
}

/// Builds a fresh generation at its stable final path, publishes it with
/// the durable marker, and returns everything opened. Nothing here is
/// ever renamed after the databases open: the directory is created at
/// `gen/<day>.f<format>` and stays there.
async fn build(
	config: &Config,
	dir: &Path,
	day: &str,
	now_millis: i64,
	terminated: &crate::shutdown::Terminated,
	readiness: &Readiness,
) -> Result<Generation> {
	// The path must be free of a live holder before anything is removed:
	// a previous initializer that still runs — same process tree, a
	// paused candidate, a crashed build's supervisor — keeps its
	// databases until it is dead.
	if dir.exists() {
		locks::wait_lifetime_free(dir, terminated.cancellable()).await?;
		if let Some(exclusive) = locks::try_lifetime_exclusive(dir)? {
			std::fs::remove_dir_all(dir)
				.with_context(|| format!("removing the unpublished {}", dir.display()))?;
			drop(exclusive);
		}
	}

	make_generation_dir(dir)?;
	// From here every open of this generation holds the lifetime lock
	// until exec or exit.
	locks::lifetime_shared(dir, terminated.cancellable()).await?;

	let mut secrets = Secrets::generate(day)?;

	// The site's tables are created exactly once per generation, at
	// build time. Reopens connect to what exists.
	let site_db = connect_site_db(dir).await?;
	leptoskit_db::testing::create_tables(&site_db)
		.await
		.context("creating the site's tables")?;
	let built = site::build(
		&site_db,
		site_keys(&secrets),
		&config.site_origin,
		&config.admin_origin,
	)
	.await
	.context("building the site")?;
	secrets = secrets.with_idp_client(built.client_id, built.client_secret);
	secrets
		.create(dir)
		.context("persisting the generation's credentials")?;

	// The analytics service, then the month through its own ingestion,
	// then the observable barrier: a read-back of what is durably
	// stored, recorded in the marker before publication.
	let analytics = site::start_analytics(dir, &config.site_origin, &secrets.analytics_token)
		.await
		.context("starting the analytics service")?;

	// A deterministic initialization failure, requested by a debug test
	// hook, at the point where this process image already runs
	// background tasks. The release binary never reads the hook, so this
	// is compiled out where it matters.
	#[cfg(debug_assertions)]
	ensure!(
		!test_should_fail_init(),
		"DEMO_TEST_FAIL_INIT: failing the initialization as requested, with background tasks \
		 already running"
	);

	#[cfg(debug_assertions)]
	if std::env::var_os("DEMO_TEST_CRASH_AFTER_SEED").is_some() {
		// A deterministic crash: after the writes committed, before the
		// marker exists. The generation stays unpublished, its lifetime
		// lock dies with the process, and the next image recovers.
		eprintln!("DEMO_TEST_CRASH_AFTER_SEED: aborting before publication");
		std::process::abort();
	}

	let seed_events =
		site::seed_with_barrier(&analytics, &secrets.analytics_token, now_millis).await?;

	// The identity provider's keys, warmed once so the admin's first
	// sign-in needs no key fetch it can't do yet.
	warm_jwks(&built.router).await?;

	let project = init_project(dir)?;
	let store = Store::open(&dir.join("console.sqlite3")).context("opening the console store")?;
	store
		.register(&Project {
			id: PROJECT.into(),
			environment: "production".into(),
			root: project.canonicalize()?,
			revision: 0,
		})
		.context("registering the demo project")?;
	let control = Service::new(store, &secrets.control_key, &config.admin_origin)
		.context("starting the control service")?;
	let admin = admin_router(config, dir, &secrets, &control, built.router.clone()).await?;

	// Everything is durable on disk; the marker is the commit point.
	let marker = Published {
		format: GENERATION_FORMAT,
		fingerprint: Published::current_fingerprint()?,
		day: day.to_owned(),
		seed_events,
	};
	sync_dir(dir)?;
	marker.commit(dir)?;

	Ok(assemble(GenerationParts {
		day: day.to_owned(),
		dir: dir.to_owned(),
		secrets,
		control,
		site: public_router(readiness, built.router),
		admin: public_router(readiness, admin),
		loopback: loopback_router(readiness, analytics),
	}))
}

/// A deterministic initialization failure, requested by a debug test
/// hook: fail while the hook's file exists, so a test can hold the
/// failure in place and then let a fresh image succeed. Debug builds
/// only; the release binary never reads the hook.
#[cfg(debug_assertions)]
fn test_should_fail_init() -> bool {
	std::env::var_os("DEMO_TEST_FAIL_INIT").is_some_and(|path| Path::new(&path).exists())
}

struct GenerationParts {
	day: String,
	dir: PathBuf,
	secrets: Secrets,
	control: Service,
	site: Router,
	admin: Router,
	loopback: Router,
}

fn assemble(parts: GenerationParts) -> Generation {
	Generation {
		day: parts.day,
		dir: parts.dir,
		secrets: parts.secrets,
		control: parts.control,
		site: parts.site,
		admin: parts.admin,
		loopback: parts.loopback,
		permits: Arc::new(Semaphore::new(GENERATION_STREAMS)),
	}
}

/// The admin's router, with the analytics URL on this process's private
/// loopback.
async fn admin_router(
	config: &Config,
	dir: &Path,
	secrets: &Secrets,
	control: &Service,
	site_router: Router,
) -> Result<Router> {
	crate::admin::build(
		&config.admin_origin,
		&config.site_origin,
		Keys::single(&format!("demo-{}-admin", secrets.day), secrets.admin_key),
		&secrets.idp_client_id,
		&secrets.idp_client_secret,
		dir.join("admin"),
		control.clone(),
		format!("http://{}", config.bind),
		&secrets.analytics_token,
		site_router,
	)
	.await
}

fn site_keys(secrets: &Secrets) -> Keys {
	Keys::single(&format!("demo-{}-site", secrets.day), secrets.site_key)
}

async fn connect_site_db(dir: &Path) -> Result<leptoskit_db::Db> {
	leptoskit_db::Db::connect(&format!("sqlite://{}", dir.join("site.sqlite3").display()))
		.await
		.context("connecting the site database")
}

fn loopback_router(readiness: &Readiness, analytics: Router) -> Router {
	readiness
		.routes()
		.merge(analytics)
		.fallback(|| async { axum::http::StatusCode::NOT_FOUND })
}

/// A public host's routes: health checks first — both aliases must answer
/// `/healthz` and `/readyz` — then the host's own router.
fn public_router(readiness: &Readiness, router: Router) -> Router {
	readiness.routes().merge(router)
}

/// Warms the identity provider's public keys through the site's own
/// routes.
async fn warm_jwks(site: &Router) -> Result<()> {
	let response = site
		.clone()
		.oneshot(
			axum::http::Request::builder()
				.method(axum::http::Method::GET)
				.uri("/oauth/jwks")
				.body(axum::body::Body::empty())?,
		)
		.await
		.context("warming the identity provider's keys")?;
	ensure!(
		response.status().is_success(),
		"the identity provider's keys didn't warm up ({})",
		response.status()
	);
	Ok(())
}

/// Creates the generation directory, mode 0700, like its root.
fn make_generation_dir(dir: &Path) -> Result<()> {
	std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
			.with_context(|| format!("restricting {}", dir.display()))?;
	}
	Ok(())
}

/// fsyncs a generation directory's entries before the marker commits.
fn sync_dir(dir: &Path) -> Result<()> {
	let handle =
		std::fs::File::open(dir).with_context(|| format!("opening {} to sync", dir.display()))?;
	handle
		.sync_all()
		.with_context(|| format!("syncing {}", dir.display()))
}

/// The demo project: a git repository with the fixture committed as it
/// stands and the two-instance change staged, so the admin has a change
/// to plan. Nothing here commits — git only ever initializes and stages.
///
/// # Errors
///
/// When git isn't on PATH (the host bundle must supply it) or a git step
/// fails.
fn init_project(dir: &Path) -> Result<PathBuf> {
	let root = dir.join("project");
	std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
	let git = |args: &[&str]| -> Result<()> {
		let status = std::process::Command::new("git")
			.args(args)
			.current_dir(&root)
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.context("the demo needs git on PATH; the host bundle must supply it")?;
		ensure!(status.success(), "git {} failed", args.join(" "));
		Ok(())
	};

	git(&["init", "--quiet"])?;
	std::fs::write(root.join("leptoskit.toml"), COMMITTED)?;
	git(&["add", "leptoskit.toml"])?;
	std::fs::write(
		root.join("leptoskit.toml"),
		COMMITTED.replace("instances = 1", "instances = 2"),
	)?;
	Ok(root)
}

/// Retention and budget. Called under the build lock, after the
/// generation opened or built: today and yesterday stay, everything older
/// — and any unpublished leftover in any directory — is removed only when
/// its lifetime lock can be taken exclusively without blocking. What
/// survives is counted; an over-budget volume refuses to serve, with the
/// evidence in the error, instead of unlinking a live generation or
/// growing without bound.
///
/// # Errors
///
/// When the generation directory can't be read, a removable directory
/// can't be removed, or the survivors exceed the documented budget.
pub fn collect(volume: &Path, now_millis: i64) -> Result<()> {
	let generations = volume.join("gen");
	if !generations.is_dir() {
		return Ok(());
	}

	let oldest_kept = clock::day_of(now_millis - DAY_MS);
	let fingerprint = Published::current_fingerprint()?;
	let mut survivors: Vec<(String, PathBuf)> = Vec::new();

	for entry in std::fs::read_dir(&generations).context("reading the generations")? {
		let entry = entry.context("reading the generations")?;
		let path = entry.path();
		if !path.is_dir() {
			// A stray file in gen/ is not a generation; leave it for an
			// operator rather than guess.
			survivors.push((dir_name(&path), path));
			continue;
		}

		let name = dir_name(&path);
		let older = name
			.split_once(".f")
			.is_some_and(|(day, _)| day < oldest_kept.as_str());
		let published = Published::read(&path).ok();
		let valid = published
			.as_ref()
			.is_some_and(|marker| marker.is_valid_for(&name, &fingerprint));

		// Removable when older than yesterday, or never published — but
		// only while nobody holds it. A live holder — a paused
		// candidate, a process the clock jumped past, today's serving
		// generation — keeps its database.
		if (!valid || older)
			&& let Some(exclusive) = locks::try_lifetime_exclusive(&path)?
		{
			std::fs::remove_dir_all(&path).with_context(|| {
				format!("collecting the finished generation {}", path.display())
			})?;
			drop(exclusive);
			continue;
		}

		survivors.push((name, path));
	}

	let bytes: u64 = survivors
		.iter()
		.map(|(_, path)| dir_size(path))
		.sum::<Result<u64>>()?;
	ensure!(
		survivors.len() <= MAX_GENERATION_DIRS,
		"{} holds {} generations that cannot be collected (budget {MAX_GENERATION_DIRS}): {:?}. \
		 Their holders keep their lifetime locks, so nothing was deleted; the volume stays over \
		 budget and the demo stays unready rather than unlink live databases.",
		generations.display(),
		survivors.len(),
		survivors
			.iter()
			.map(|(name, _)| name.as_str())
			.collect::<Vec<_>>(),
	);
	ensure!(
		bytes <= GENERATION_BUDGET_BYTES,
		"the generations on {} hold {} bytes (budget {GENERATION_BUDGET_BYTES}). Their holders \
		 keep their lifetime locks, so nothing was deleted; the demo stays unready rather than \
		 grow without bound.",
		generations.display(),
		bytes
	);

	Ok(())
}

/// A directory's total file size, for the budget.
fn dir_size(dir: &Path) -> Result<u64> {
	let mut total = 0_u64;
	for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
		let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
		let metadata = entry
			.metadata()
			.with_context(|| format!("measuring {}", entry.path().display()))?;
		if metadata.is_dir() {
			total += dir_size(&entry.path())?;
		} else {
			total += metadata.len();
		}
	}
	Ok(total)
}
