//! A main site and its admin, running side by side on this machine.
//!
//! The main site (`http://localhost:4800`) runs `leptoskit_auth` and
//! `leptoskit_idp` with two people: Ada, who is an admin, and Grace, who
//! isn't. The admin (`http://127.0.0.1:4801/_leptoskit/admin`) signs people
//! in through it and manages a demo project's `[infra]`.
//!
//! The main site also runs the real analytics service, which the admin's
//! Analytics area reads as the app. `--seed` fills it with 30 days of
//! visits, sign-ups, a checkout funnel, and returning members, sent
//! through the service's own ingestion; the site's pages add more as you
//! click around.
//!
//! ```text
//! cargo run -p admin_demo                  # simulated operations
//! cargo run -p admin_demo -- --seed        # and a month of analytics
//! cargo run -p admin_demo -- --executor lk # the real `lk` commands
//! ```
//!
//! The simulated executor never contacts a cloud provider: it generates
//! the real `main.tf.json`, compares it with what it last "applied", and
//! says it's a simulation in every operation it runs. With `--executor lk`
//! the worker runs `lk infra plan` and friends for real, and without
//! provider tokens those fail, which the admin shows as it is.

mod main_site;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use admin_demo::analytics;
use admin_demo::executor;
use anyhow::Context as _;
use anyhow::Result;
use leptoskit_admin::Admin;
use leptoskit_admin::AnalyticsService;
use leptoskit_admin::MainSite;
use leptoskit_auth::Keys;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;

const MAIN_SITE: &str = "http://localhost:4800";
/// Where the admin reaches the main site's analytics service: its private
/// address, as a deployment's would be.
const ANALYTICS: &str = "http://127.0.0.1:4800";
/// The analytics service's token for the app's own callers: the main
/// site's server and the admin. The demo invented it and nothing outside
/// this process accepts it, so it lives here, with the rest of the demo's
/// invented credentials.
const ANALYTICS_TOKEN: &str = "admin-demo-analytics-app-token-not-a-secret-0123";
const ADMIN: &str = "http://127.0.0.1:4801";
const PROJECT: &str = "notes-production";
/// The demo's own owner credential for the in-process control service.
/// Nothing outside this process accepts it.
const CONTROL_KEY: &str = "admin-demo-owner-key-not-a-secret-0123456789";

/// The demo project's `leptoskit.toml`, as last committed: one host.
const COMMITTED: &str = include_str!("../project/leptoskit.toml");

fn main() -> Result<()> {
	// The real executor runs this same binary as `lk`.
	if std::env::args()
		.nth(1)
		.is_some_and(|argument| !argument.starts_with("--"))
	{
		leptoskit_cli::cli::main();
		return Ok(());
	}

	let mut real = false;
	let mut seed = false;
	let mut arguments = std::env::args().skip(1);

	while let Some(argument) = arguments.next() {
		match argument.as_str() {
			"--seed" => seed = true,
			"--executor" => {
				real = match arguments.next().as_deref() {
					Some("lk") => true,
					Some("demo") => false,
					_ => anyhow::bail!("--executor takes `demo` or `lk`"),
				};
			}
			other => anyhow::bail!("unknown argument `{other}`; try `--seed` or `--executor lk`"),
		}
	}

	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?
		.block_on(run(real, seed))
}

async fn run(real: bool, seed: bool) -> Result<()> {
	let data = std::env::temp_dir().join("leptoskit-admin-demo");

	// Each run starts from scratch, in a directory only the demo writes.
	if data.join(".admin-demo").exists() {
		std::fs::remove_dir_all(&data)?;
	}

	std::fs::create_dir_all(&data)?;
	std::fs::write(data.join(".admin-demo"), "")?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700))?;
	}

	let project = demo_project(&data.join("notes"))?;
	let store = Store::open(&data.join("console.sqlite3"))?;
	store.register(&Project {
		id: PROJECT.into(),
		environment: "production".into(),
		root: project.canonicalize()?,
		revision: 0,
	})?;
	let service = Service::new(store.clone(), CONTROL_KEY, ADMIN)?;

	let site = main_site::start(store.clone(), &data).await?;
	let admin = Admin::builder("Notes")
		.origin(ADMIN)
		.main_site(MainSite::new(MAIN_SITE, site.client_id, site.client_secret).name("Notes"))
		.keys(Keys::single("admin-demo", [7; 32]))
		.data_dir(data.join("admin"))
		.control(service.clone())
		.analytics(AnalyticsService::new(ANALYTICS, ANALYTICS_TOKEN))
		// The admin says "Simulated" everywhere unless the real `lk`
		// commands run.
		.simulated_infrastructure(!real);
	// The pages that moved to Wasm need their browser half: build it with
	// `leptoskit build --platform web --out <dir>` in
	// crates/leptoskit_admin_pages and name `<dir>` here.
	let admin = match std::env::var_os("LEPTOSKIT_ADMIN_CLIENT_DIR") {
		Some(directory) if !directory.is_empty() => admin.client_dir(directory),
		_ => admin,
	}
	.build()
	.await?;

	let executor: Arc<dyn leptoskit_deploy_console::control::Executor> = if real {
		Arc::new(leptoskit_cli::console::CommandRunner::new(
			data.join("worker"),
			std::env::current_exe()?,
			service.clone(),
		))
	} else {
		Arc::new(executor::Simulated::new(service.clone(), COMMITTED))
	};
	let worker = service.worker(executor)?;

	let main_listener = tokio::net::TcpListener::bind("127.0.0.1:4800").await?;
	let admin_listener = tokio::net::TcpListener::bind("127.0.0.1:4801").await?;

	println!("Main site: {MAIN_SITE}");
	println!("Admin:     {ADMIN}{}", leptoskit_admin::BASE);
	println!(
		"Operations: {}",
		if real {
			"the real `lk` commands (no provider tokens, so plans fail)"
		} else {
			"simulated; no cloud provider is contacted"
		}
	);
	println!(
		"Sign in as Ada (an admin) or Grace (not one). Data: {}",
		data.display()
	);

	// The month is sent once the site is listening, through its own
	// ingestion, so the admin shows it within seconds of starting.
	tokio::spawn(async move {
		let result = if seed {
			analytics::seed(ANALYTICS, ANALYTICS_TOKEN).await.map(Some)
		} else {
			analytics::register(ANALYTICS, ANALYTICS_TOKEN)
				.await
				.map(|()| None)
		};

		match result {
			Ok(Some(entries)) => {
				println!("Analytics: seeded {entries} events from the last 30 days");
			}
			Ok(None) => println!("Analytics: empty; run with --seed for a month of traffic"),
			Err(error) => eprintln!("Analytics: {error:#}"),
		}
	});

	tokio::select! {
		result = leptoskit_axum::serve_listener(main_listener, site.router) => result?,
		result = leptoskit_axum::serve_listener(admin_listener, admin.router()) => result?,
		_ = worker => anyhow::bail!("the worker stopped"),
	}

	Ok(())
}

/// A git repository with `leptoskit.toml` committed at one host and
/// edited to two, so the admin has a change to plan.
///
/// The demo stages the file rather than committing it, so nothing is
/// signed on its behalf; the admin compares against what's staged when
/// there's no commit yet.
fn demo_project(root: &Path) -> Result<PathBuf> {
	std::fs::create_dir_all(root)?;
	let git = |args: &[&str]| -> Result<()> {
		let status = std::process::Command::new("git")
			.args(args)
			.current_dir(root)
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.context("the demo needs git on PATH")?;
		anyhow::ensure!(status.success(), "git {} failed", args.join(" "));
		Ok(())
	};

	git(&["init", "--quiet"])?;
	std::fs::write(root.join("leptoskit.toml"), COMMITTED)?;
	git(&["add", "leptoskit.toml"])?;
	std::fs::write(
		root.join("leptoskit.toml"),
		COMMITTED.replace("instances = 1", "instances = 2"),
	)?;
	Ok(root.to_owned())
}
