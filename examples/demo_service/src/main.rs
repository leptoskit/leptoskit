//! The public demo's service process: one binary, one process image per
//! UTC day, replaced by itself at reset.
//!
//! ```text
//! DEMO_BIND=127.0.0.1:8080
//! LEPTOSKIT_SERVICE_VOLUME_STATE=/var/lib/demo
//! DEMO_SITE_ORIGIN=https://demo.example
//! DEMO_ADMIN_ORIGIN=https://admin.demo.example
//! ```
//!
//! Startup is one path for every arrival — a first boot, a systemd
//! restart, a crash's restart, or this process's own re-exec at midnight:
//! bind the private listener, serve the fallback (alive, not ready), take
//! the build lock, open or build today's generation at its stable path,
//! publish it with the durable marker, collect what nobody holds, go
//! ready, and poll for the worker lock. The day watch checks the UTC wall
//! clock at most every 30 seconds; when the date changes, the image
//! drains, hands its locks to the kernel, and execs the same binary with
//! the same arguments in the same PID. A failed initialization never
//! retries inside this image — background tasks may already run — it
//! keeps the fallback briefly, then re-executes for a bounded retry.

use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use demo_service::Exit;
use demo_service::clock;
use demo_service::config;
use demo_service::config::Config;
use demo_service::dispatch;
use demo_service::generation;
use demo_service::locks;
use demo_service::shutdown;
use demo_service::worker;
use leptoskit_axum::GracefulShutdown;
use leptoskit_axum::Readiness;

fn main() -> std::process::ExitCode {
	// Configuration, the state root, and — on Linux, before the runtime
	// and the listener exist — the inherited-descriptor guard. A
	// misconfigured service refuses to start: that is a nonzero exit, and
	// the supervisor's restart policy shows it to the operator instead of
	// sweeping it under a fallback.
	let config = match Config::from_env() {
		Ok(config) => config,
		Err(ref error) => return refuse(error),
	};
	if let Err(ref error) = config.check_state_root() {
		return refuse(error);
	}
	if let Err(ref error) = config::guard_inherited_descriptors() {
		return refuse(error);
	}

	let runtime = match tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
	{
		Ok(runtime) => runtime,
		Err(error) => {
			let error = anyhow::Error::new(error).context("building the runtime");
			return refuse(&error);
		}
	};

	let outcome = runtime.block_on(run(config));
	// Bounded: pending tasks — a paused executor write among them — get
	// at most this long, while every retained lock file stays open until
	// the exec or exit below replaces or ends the process image.
	runtime.shutdown_timeout(shutdown::runtime_shutdown_timeout());

	match outcome {
		Ok(Exit::Terminate) => std::process::ExitCode::SUCCESS,
		// `re_exec` never returns on success; if exec fails it exits
		// nonzero for the supervisor to restart.
		Ok(Exit::Reset) => re_exec(),
		Err(ref error) => refuse(error),
	}
}

fn refuse(error: &anyhow::Error) -> std::process::ExitCode {
	eprintln!("the demo service refuses to start: {error:#}");
	std::process::ExitCode::FAILURE
}

/// Replaces this process image with the same binary, same arguments, same
/// environment, same PID. Every descriptor closes: the listener, the
/// databases, and the locks — released by the kernel only once this
/// execution has entirely stopped.
fn re_exec() -> ! {
	let executable = match std::env::current_exe() {
		Ok(executable) => executable,
		Err(error) => {
			eprintln!("couldn't find this process's executable to re-exec: {error}");
			std::process::exit(1);
		}
	};
	use std::os::unix::process::CommandExt as _;
	let error = std::process::Command::new(executable)
		.args(std::env::args_os().skip(1))
		.exec();
	eprintln!("re-exec failed: {error}; exiting nonzero for the supervisor to restart");
	std::process::exit(1);
}

/// The process's whole async life: bind, serve, initialize, watch the
/// day, and stop on the signal.
async fn run(config: Config) -> Result<Exit> {
	let readiness = Readiness::new();
	let generation_check = readiness.check("generation");
	let terminated = shutdown::init();

	eprintln!(
		"demo: site {site} admin {admin} bind {bind} volume {volume}",
		site = config.site_origin,
		admin = config.admin_origin,
		bind = config.bind,
		volume = config.volume.display(),
	);
	let _ = demo_service::AUTHORITIES.set(config.authorities.clone());
	let listener = tokio::net::TcpListener::bind(config.bind)
		.await
		.with_context(|| format!("binding {}", config.bind))?;

	// The listener serves from the first moment: the fallback answers
	// every authority it will ever serve, health checks included.
	let graceful = GracefulShutdown::new()
		.readiness(&readiness)
		.drain_delay(Duration::ZERO)
		.grace_period(Duration::from_secs(3))
		.signal(shutdown::serve_signal());
	let server = tokio::spawn(leptoskit_axum::serve_listener_with(
		listener,
		dispatch::router(),
		graceful,
	));

	// One initialization attempt per process image, under the build lock.
	match startup(&config, &readiness, &terminated).await {
		Ok(generation) => {
			generation_check.pass();
			let _ = demo_service::GENERATION.set(generation.clone());

			tokio::spawn(worker::run(generation.clone(), terminated.clone()));
			tokio::spawn(shutdown::watch_day(generation.day, terminated.clone()));
		}
		Err(error) => {
			eprintln!("the demo's generation didn't start: {error:#}");
			// The build lock stays retained inside `startup` on failure:
			// no second initialization happens in this image, the
			// fallback keeps serving, and a bounded retry re-executes.
			tokio::spawn(shutdown::retry_later(
				clock::retry_delay(),
				terminated.clone(),
			));
		}
	}

	// Runs until the drain finishes; a grace-period overrun on stubborn
	// SSE streams is the expected way to stop, not a failure to report.
	let _ = server.await;

	shutdown::reason().map_or_else(|| bail!("the server stopped without a shutdown reason"), Ok)
}

/// The one initialization attempt: build lock, open or build, collect.
/// On failure the build lock is retained until exec or exit — this image
/// has already started background tasks by then, so nothing else may
/// touch the volume until it is gone.
async fn startup(
	config: &Config,
	readiness: &Readiness,
	terminated: &shutdown::Terminated,
) -> Result<generation::Generation> {
	let day = clock::day_of(clock::now_millis());
	let build = locks::build_lock(&config.volume, terminated.cancellable()).await?;

	let attempt = async {
		let opened =
			generation::open_or_build(config, &day, clock::now_millis(), terminated, readiness)
				.await?;
		generation::collect(&config.volume, clock::now_millis())?;
		Ok(opened)
	};

	match attempt.await {
		Ok(generation) => {
			// A successful hand: the next process takes the lock from
			// here, this one holds only its generation's lifetime lock.
			drop(build);
			Ok(generation)
		}
		Err(error) => {
			locks::retain_build(build);
			Err(error)
		}
	}
}
