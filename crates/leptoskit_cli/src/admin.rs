//! `lk admin serve`: the admin web app, with the deployment service and its
//! worker embedded, signed in through the main site.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use leptoskit_admin::Admin;
use leptoskit_admin::Environment;
use leptoskit_admin::MainSite;

use crate::console::CommandRunner;

#[derive(Debug, clap::Subcommand)]
pub enum AdminCommand {
	/// Serves the admin for trusted repositories. People sign in on the main
	/// site; only accounts it gives an admin role get in.
	///
	/// Reads `LEPTOSKIT_ADMIN_CLIENT_SECRET` (the client secret the main site
	/// issued), `LEPTOSKIT_ADMIN_KEYS` (the admin's own session keys), and
	/// `LEPTOSKIT_CONTROL_KEY` (the deployment service's owner credential).
	/// For the Analytics area it reads `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`,
	/// the analytics service's token, the same one the main site's server
	/// has.
	Serve {
		/// A repository whose `[infra]` the admin manages. Repeat for more.
		#[arg(long, required = true)]
		project: Vec<PathBuf>,
		/// Where the admin keeps its database and the deployment journal.
		#[arg(long)]
		data: Option<PathBuf>,
		#[arg(long, default_value = "127.0.0.1:4801")]
		bind: std::net::SocketAddr,
		/// The admin's public origin, such as `https://admin.notes.example`.
		#[arg(long, default_value = "http://127.0.0.1:4801")]
		url: String,
		/// The main site's origin, which is the issuer the admin signs in
		/// through.
		#[arg(long)]
		main_site: String,
		/// The client ID the main site issued to the admin.
		#[arg(long)]
		client_id: String,
		/// The main site's name, for "Continue with …". Defaults to
		/// `--app-name`.
		#[arg(long)]
		main_site_name: Option<String>,
		/// A role that lets an account in. Repeat to accept several.
		#[arg(long = "role", default_value = "admin")]
		roles: Vec<String>,
		/// Which deployment this admin watches.
		#[arg(long, default_value = "production")]
		environment: Environment,
		/// The app's name, shown in the top bar.
		#[arg(long, default_value = "LeptosKit")]
		app_name: String,
		/// Where the main site's analytics service answers, such as
		/// `http://10.0.0.2:4800`: the address the admin can reach, not
		/// necessarily the public one. Defaults to
		/// `LEPTOSKIT_SERVICE_ANALYTICS_URL`; without either, the Analytics
		/// area explains how to connect.
		#[arg(long)]
		analytics_url: Option<String>,
		/// The admin pages' browser half: the directory `leptoskit build
		/// --release --platform web` wrote in `crates/leptoskit_admin_pages`.
		/// Defaults to `LEPTOSKIT_ADMIN_CLIENT_DIR`. Without either, the
		/// pages that moved to Wasm render but don't respond to presses.
		#[arg(long)]
		client_dir: Option<PathBuf>,
	},
}

pub fn run(command: AdminCommand) -> Result<()> {
	match command {
		AdminCommand::Serve {
			project,
			data,
			bind,
			url,
			main_site,
			client_id,
			main_site_name,
			roles,
			environment,
			app_name,
			analytics_url,
			client_dir,
		} => {
			let secret = std::env::var("LEPTOSKIT_ADMIN_CLIENT_SECRET").context(
				"set LEPTOSKIT_ADMIN_CLIENT_SECRET to the client secret the main site issued to \
				 the admin",
			)?;
			let keys = leptoskit_auth::Keys::from_env("LEPTOSKIT_ADMIN_KEYS")?;
			let data = crate::console::data_directory(data)?;
			let lock = crate::console::directory_lock(&data)?;
			let service = crate::console::open_service(&data, project, &url)?;
			let mut site = MainSite::new(main_site, client_id, secret);

			if let Some(name) = main_site_name {
				site = site.name(name);
			}

			let analytics_url = analytics_url
				.or_else(|| std::env::var("LEPTOSKIT_SERVICE_ANALYTICS_URL").ok())
				.filter(|url| !url.trim().is_empty());
			let analytics = match analytics_url {
				Some(url) => {
					Some(leptoskit_admin::AnalyticsService::new(
						url,
						std::env::var("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN").context(
							"set LEPTOSKIT_SERVICE_ANALYTICS_TOKEN to the analytics service's \
							 token, the one the main site's server uses",
						)?,
					))
				}
				None => None,
			};

			let client_dir = client_dir.or_else(|| {
				std::env::var_os("LEPTOSKIT_ADMIN_CLIENT_DIR")
					.filter(|directory| !directory.is_empty())
					.map(PathBuf::from)
			});
			let runtime = tokio::runtime::Builder::new_multi_thread()
				.worker_threads(4)
				.enable_all()
				.build()?;
			runtime.block_on(async {
				let admin = Admin::builder(app_name)
					.origin(&url)
					.main_site(site)
					.roles(roles)
					.environment(environment)
					.keys(keys)
					.data_dir(data.join("admin"))
					.control(service.clone());
				let admin = match client_dir {
					Some(directory) => admin.client_dir(directory),
					None => admin,
				};
				let admin = match analytics {
					Some(analytics) => admin.analytics(analytics),
					None => admin,
				}
				.build()
				.await?;
				let mut worker = service.worker(Arc::new(CommandRunner::new(
					data,
					std::env::current_exe()?,
					service.clone(),
				)))?;
				let listener = tokio::net::TcpListener::bind(bind).await?;
				println!("LeptosKit admin: {url}{}", leptoskit_admin::BASE);
				let result = tokio::select! {
					result = leptoskit_axum::serve_listener(listener, admin.router()) => result,
					_ = &mut worker => anyhow::bail!(
						"deployment worker stopped; restart the admin and reconcile interrupted \
						 operations"
					),
				};
				worker.abort();
				let _ = worker.await;
				result?;
				Ok::<_, anyhow::Error>(())
			})?;
			drop(lock);
			Ok(())
		}
	}
}
