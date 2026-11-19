//! One generation: the durable state under `gen/<UTC day>.f<format>`, and
//! everything this process opens on top of it.
//!
//! The rules that make this safe, in the order they happen:
//!
//! 1. **Validate before touching.** A generation is opened only when its
//!    `published` marker names this binary's format and schema fingerprint. The
//!    check runs before any database connection or migration, so incompatible
//!    state is refused while it is still untouched — and a marker that exists
//!    but cannot be read, or does not name its directory, is refused the same
//!    way: only a genuinely absent marker may take the unpublished build path.
//! 2. **Hold the lifetime lock before opening.** A generation's `lifetime.lock`
//!    is taken shared under the build lock, before the first database opens,
//!    and kept until exec or exit. Nothing deletes a generation whose lifetime
//!    lock it cannot take exclusively.
//! 3. **Build at the final path; publish with a marker.** A new generation is
//!    built where it will always live — never renamed after SQLite opened it —
//!    becomes durable through an explicit fsync barrier over its databases,
//!    WALs, and directory entries, and is openable only when its fsynced
//!    `published` marker is renamed into place after that barrier.
//! 4. **Seed through the real service, and prove it.** The month goes through
//!    the analytics service's own ingestion, and the receipt is checked entry
//!    by entry against the service's typed answers before an exact read-back is
//!    recorded in the marker. The recorded count is a historical initialization
//!    receipt: a warm open re-reads the catalogue to prove the generation is
//!    healthy, never to require the frozen initial count, which visitors change
//!    and deletions reduce. No direct SQL writes seed events.
//! 5. **Collect only what nobody holds, and only what is genuinely done.**
//!    Retention recognizes a published generation structurally — a marker
//!    naming its directory — regardless of format or fingerprint, and keeps
//!    every recent one as same-day rollback state. Only genuinely expired
//!    recognized generations, or genuinely unpublished abandoned builds, are
//!    removed, and only under the exclusive lifetime lock; a marker that cannot
//!    be read is never a deletion reason. Whatever survives counts against a
//!    small, documented budget, and an over-budget volume fails honestly
//!    instead of growing unbounded or unlinking live databases.

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

use crate::activity::Analytics;
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

	// Only a genuinely absent marker may take the unpublished path. A
	// marker that exists but cannot be read — corrupt, truncated,
	// unreadable — is a fail-closed fault on a published generation:
	// treating it as absence would delete and rebuild a day that may
	// already be serving, so it is an error while everything stays
	// untouched.
	let published = match Published::read_if_present(&dir) {
		Ok(marker) => marker,
		Err(error) => {
			return Err(error.context(format!(
				"the published marker at {} is unreadable, so the generation is refused untouched \
				 rather than treated as an unpublished build",
				dir.display()
			)));
		}
	};

	if let Some(marker) = &published {
		// Recognition: the marker must structurally name this directory.
		// A marker that names another day or format inside this
		// directory is malformed, and malformed is refused, not rebuilt.
		let name = dir_name(&dir);
		ensure!(
			marker.names(&name),
			"the published marker at {} does not name this directory ({name}): it claims day {} \
			 format {}, and the generation is refused untouched",
			dir.display(),
			marker.day,
			marker.format
		);

		if marker.is_valid_for(&name, &fingerprint) {
			return open(config, &dir, marker, terminated, readiness).await;
		}

		// Same format and day, different schema: the source's schemas
		// drifted without a format bump. Refuse before anything in the
		// generation is opened or migrated — rebuilding would wipe a
		// live day's state instead of surfacing the bug, and migrating
		// would change databases this binary doesn't understand. The
		// fix is a format bump and a new directory, never this.
		anyhow::bail!(
			"the published generation at {} carries schema fingerprint {} but this binary builds \
			 {fingerprint}: the schemas drifted without a GENERATION_FORMAT bump, so the \
			 generation is refused untouched",
			dir.display(),
			marker.fingerprint
		);
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
/// services on top. The marker is a historical initialization receipt —
/// it proves the build finished — and the reopen proves the generation
/// is *healthy now*: the artifacts it names are present and the services
/// open and answer reads. Today's event count is today's business:
/// visitors add events and deletions remove them, so a warm open never
/// requires the frozen initial count.
async fn open(
	config: &Config,
	dir: &Path,
	marker: &Published,
	terminated: &crate::shutdown::Terminated,
	readiness: &Readiness,
) -> Result<Generation> {
	locks::lifetime_shared(dir, terminated.cancellable()).await?;

	// The receipt stands behind files; the files must still be there.
	ensure!(
		crate::marker::artifacts_present(dir),
		"the published generation at {} is missing required artifacts",
		dir.display()
	);
	let secrets = Secrets::load(dir, &marker.day)?;

	// The analytics service first: its catalogue read is the healthy read
	// that proves the database opens, migrates as a no-op, and answers.
	let analytics = site::start_analytics(dir, &config.site_origin, &secrets.analytics_token)
		.await
		.context("starting the analytics service")?;
	let _ = site::count_events(&analytics, &secrets.analytics_token).await?;

	let site_db = connect_site_db(dir).await?;
	let site = site::rebuild(
		&site_db,
		site_keys(&secrets),
		&config.site_origin,
		&config.admin_origin,
		&site_options(config, marker.day.as_str()),
		Analytics::new(analytics.clone(), &secrets.analytics_token),
	)
	.await
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

	// The analytics service starts once, before the site, so the site's
	// private clone of its router and the loopback's are the same service:
	// a visitor's event lands in the database the admin reads. It is not
	// started again below.
	let analytics = site::start_analytics(dir, &config.site_origin, &secrets.analytics_token)
		.await
		.context("starting the analytics service")?;

	let built = site::build(
		&site_db,
		site_keys(&secrets),
		&config.site_origin,
		&config.admin_origin,
		&site_options(config, day),
		Analytics::new(analytics.clone(), &secrets.analytics_token),
	)
	.await
	.context("building the site")?;
	secrets = secrets.with_idp_client(built.client_id, built.client_secret);
	secrets
		.create(dir)
		.context("persisting the generation's credentials")?;

	// Then the month through the service's own ingestion, then the
	// observable barrier: a read-back of what is durably stored, recorded
	// in the marker before publication.

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

	let seed_events =
		site::seed_with_barrier(&analytics, &secrets.analytics_token, now_millis).await?;

	#[cfg(debug_assertions)]
	if std::env::var_os("DEMO_TEST_CRASH_AFTER_SEED").is_some() {
		// A deterministic crash: after the seed committed and read back,
		// before anything is published. The rows are on disk, the marker
		// does not exist, the generation stays unpublished, its lifetime
		// lock dies with the process, and the next image recovers.
		eprintln!(
			"DEMO_TEST_CRASH_AFTER_SEED: aborting after the committed seed, before publication"
		);
		std::process::abort();
	}

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

	// Everything this marker vouches for is made durable *before* the
	// marker exists: the databases and their WALs are written under
	// WAL + synchronous=NORMAL, where a committed, readable transaction
	// is not yet a power-loss barrier. A failure here propagates without
	// publishing; the generation stays unpublished and is rebuilt by the
	// next image.
	crate::barrier::establish(dir)
		.context("establishing the generation's durability barrier before publication")?;

	// The marker is the commit point: written last, over durable state.
	let marker = Published {
		format: GENERATION_FORMAT,
		fingerprint: Published::current_fingerprint()?,
		day: day.to_owned(),
		seed_events,
	};
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

/// The daily site signing key, named by the day so the visitor store can
/// derive its IP-hash salt from the same key.
fn site_keys(secrets: &Secrets) -> Keys {
	Keys::single(&site_key_id(&secrets.day), secrets.site_key)
}

/// The site key's id for one UTC day. Tests build their site keys with
/// the same id, so the visitors' IP-hash salt derives from the key they
/// handed the site.
#[must_use]
pub fn site_key_id(day: &str) -> String {
	format!("demo-{day}-site")
}

/// The site's own options for a generation: its day and the configured
/// client IP header.
fn site_options(config: &Config, day: &str) -> site::SiteOptions {
	site::SiteOptions::new(day, config.client_ip_header.clone()).with_client(config.client.clone())
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

/// What collection found in a generation directory. Recognition is
/// *structural* — does a marker that names this directory exist? — and
/// deliberately knows nothing about this binary's format or fingerprint:
/// a valid recent generation of any format is a same-day rollback
/// target, not an abandoned build.
enum Recognition {
	/// A marker structurally names the directory: this was published by
	/// *some* build.
	Published,
	/// No marker at all: an abandoned build, or a stray directory.
	Unpublished,
	/// A marker exists but cannot be read, parsed, or matched to the
	/// directory. Never a deletion reason: unreadable is kept, counted
	/// against the budget, and reported.
	Unreadable,
}

/// Recognizes what a generation directory is. Called by collection only;
/// opening a generation validates far more strictly.
fn recognize(dir: &Path, name: &str) -> Recognition {
	match Published::read_if_present(dir) {
		Ok(Some(marker)) if marker.names(name) => Recognition::Published,
		// A marker that exists but cannot be read, parsed, or matched to
		// its directory is never evidence of *no* publication: the
		// directory is kept, counted, and reported instead.
		Ok(Some(_)) | Err(_) => Recognition::Unreadable,
		Ok(None) => Recognition::Unpublished,
	}
}

/// Retention and budget. Called under the build lock, after the
/// generation opened or built. A published generation is kept while it
/// is recent (today and yesterday), whatever format or fingerprint built
/// it, because a recent generation of a foreign format is exactly the
/// rollback state a same-day release must leave behind; only a
/// genuinely expired published generation — or a genuinely unpublished
/// leftover — is removed, and only when its lifetime lock can be taken
/// exclusively without blocking. What survives is counted; an
/// over-budget volume refuses to serve, with the evidence in the error,
/// instead of unlinking a live generation or growing without bound.
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
		let expired = name
			.split_once(".f")
			.is_some_and(|(day, _)| day < oldest_kept.as_str());

		// Removable when a recognized publication has genuinely expired,
		// or when nothing was ever published — but never because a
		// marker couldn't be read, and never while a live holder keeps
		// the lifetime lock. A live holder — a paused candidate, a
		// process the clock jumped past, today's serving generation —
		// keeps its database.
		let removable = match recognize(&path, &name) {
			Recognition::Published => expired,
			Recognition::Unpublished => true,
			Recognition::Unreadable => false,
		};

		if removable && let Some(exclusive) = locks::try_lifetime_exclusive(&path)? {
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

#[cfg(test)]
mod tests {
	use super::*;

	/// Recognition separates three facts a collector must not confuse: a
	/// published generation (whatever format), a genuinely unpublished
	/// directory, and a directory whose marker cannot be read, parsed, or
	/// matched to it — the last never being a deletion reason.
	#[test]
	fn recognition_separates_published_unpublished_and_unreadable() -> Result<()> {
		let temp = tempfile::TempDir::new()?;
		let root = temp.path().join("gen");
		std::fs::create_dir_all(&root)?;

		// No marker: unpublished.
		let unpublished = root.join("2026-11-15.building-leftover");
		std::fs::create_dir_all(&unpublished)?;
		assert!(matches!(
			recognize(&unpublished, "2026-11-15.building-leftover"),
			Recognition::Unpublished
		));

		// A marker naming its directory, in a foreign format: published.
		let foreign = root.join("2026-11-14.f2");
		std::fs::create_dir_all(&foreign)?;
		let marker = Published {
			format: 2,
			fingerprint: "0".repeat(64),
			day: "2026-11-14".to_owned(),
			seed_events: 7,
		};
		std::fs::write(foreign.join("published"), serde_json::to_vec(&marker)?)?;
		assert!(matches!(
			recognize(&foreign, "2026-11-14.f2"),
			Recognition::Published
		));

		// A truncated marker: unreadable.
		let truncated = root.join("2026-11-14.f1");
		std::fs::create_dir_all(&truncated)?;
		std::fs::write(truncated.join("published"), b"{\"format\": 1")?;
		assert!(matches!(
			recognize(&truncated, "2026-11-14.f1"),
			Recognition::Unreadable
		));

		// A marker that parses but names another day: still unreadable —
		// malformed is not unpublished.
		let mismatched = root.join("2026-11-13.f1");
		std::fs::create_dir_all(&mismatched)?;
		let mut elsewhere = marker.clone();
		elsewhere.day = "2026-11-12".to_owned();
		std::fs::write(
			mismatched.join("published"),
			serde_json::to_vec(&elsewhere)?,
		)?;
		assert!(matches!(
			recognize(&mismatched, "2026-11-13.f1"),
			Recognition::Unreadable
		));
		Ok(())
	}
}
