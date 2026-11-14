//! A main site and its admin, running side by side on this machine.
//!
//! The main site (`http://localhost:4800`) runs `leptoskit_auth` and
//! `leptoskit_idp` with two people: Ada, who is an admin, and Grace, who
//! isn't. The admin (`http://127.0.0.1:4801/_leptoskit/admin`) signs people
//! in through it and manages a demo project's `[infra]`.
//!
//! ```text
//! cargo run -p admin_demo                  # simulated operations
//! cargo run -p admin_demo -- --executor lk # the real `lk` commands
//! ```
//!
//! The simulated executor never contacts a cloud provider: it generates
//! the real `main.tf.json`, compares it with what it last "applied", and
//! says it's a simulation in every operation it runs. With `--executor lk`
//! the worker runs `lk infra plan` and friends for real, and without
//! provider tokens those fail, which the admin shows as it is.

mod executor;
mod main_site;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_admin::Admin;
use leptoskit_admin::MainSite;
use leptoskit_auth::Keys;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;

const MAIN_SITE: &str = "http://localhost:4800";
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

	let real = match std::env::args().nth(1).as_deref() {
		None => false,
		Some("--executor") => {
			match std::env::args().nth(2).as_deref() {
				Some("lk") => true,
				Some("demo") => false,
				_ => anyhow::bail!("--executor takes `demo` or `lk`"),
			}
		}
		Some(other) => anyhow::bail!("unknown argument `{other}`; try `--executor lk`"),
	};

	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?
		.block_on(run(real))
}

async fn run(real: bool) -> Result<()> {
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

	let site = main_site::start(store.clone()).await?;
	let admin = Admin::builder("Notes")
		.origin(ADMIN)
		.main_site(MainSite::new(MAIN_SITE, site.client_id, site.client_secret).name("Notes"))
		.keys(Keys::single("admin-demo", [7; 32]))
		.data_dir(data.join("admin"))
		.control(service.clone())
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
