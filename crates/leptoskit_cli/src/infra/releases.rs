use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::infra::InfraConfig;
use serde_json::json;

use super::Release;
use super::ReleaseArgs;
use super::ServiceSchema;
use super::applied_bindings;
use super::directory;
use super::generate;
use super::history_fingerprint;
use super::lock;
use super::managed_settings;
use super::output;
use super::remote::Reservation;
use super::remote::{self};
use super::remote_history;
use super::remote_service_history;
use super::runtime;
use super::services::APP_DATABASE;

pub(super) fn history(ip: &str, c: &InfraConfig) -> Result<Vec<Release>> {
	serde_json::from_slice(&remote::read(ip, &c.name, "releases.json")?)
		.context("invalid shared release history; refusing to overwrite")
}

/// Stops the database-backed services' containers on the primary host, so
/// nothing writes to their databases while their schemas change. The next
/// `kamal app boot` starts them again with every other role.
fn stop_services(ip: &str, c: &InfraConfig, project: &crate::deploy::Project) -> Result<()> {
	for service in project
		.services
		.iter()
		.filter(|service| service.is_process() && service.database)
	{
		remote::execute(
			ip,
			&format!(
				"set -eu; ids=$(docker ps -q --filter label=service={} --filter label=role={}); \
				 if [ -n \"$ids\" ]; then docker stop --time 30 $ids; fi",
				c.name,
				remote::quote(&super::services::role(service))
			),
			None,
		)?;
	}
	Ok(())
}

/// Fails when a service's migrations changed since `previous`, the release
/// a new one follows, without `--maintenance`. Services new since `previous`
/// aren't checked.
fn check_service_changes(
	args: &ReleaseArgs,
	previous: Option<&Release>,
	local: &BTreeMap<String, ServiceSchema>,
) -> Result<()> {
	let Some(previous) = previous else {
		return Ok(());
	};
	for (name, schema) in local {
		if let Some(recorded) = previous.services.get(name) {
			ensure!(
				recorded.migrations == schema.migrations || args.maintenance,
				"services.{name}: schema changes require --maintenance"
			);
		}
	}
	Ok(())
}

/// Fails when the app's database, or a service's, no longer holds what
/// `previous` recorded, or, under `--maintenance`, what this checkout
/// migrates it to.
fn check_drift(
	ip: &str,
	c: &InfraConfig,
	args: &ReleaseArgs,
	previous: &Release,
	(local_hash, local): (&str, &BTreeMap<String, ServiceSchema>),
) -> Result<()> {
	let actual = remote_history(ip, &c.name)?;
	ensure!(
		actual == previous.migrations || (args.maintenance && actual == local_hash),
		"database drifted from shared history; investigate before a new release"
	);
	for (name, recorded) in &previous.services {
		let actual = remote_service_history(ip, &c.name, &service_database(name))?;
		let local = local.get(name).map(|schema| &schema.migrations);
		ensure!(
			actual == recorded.migrations || (args.maintenance && Some(&actual) == local),
			"services.{name}: its database drifted from shared history; investigate before a new \
			 release"
		);
	}
	Ok(())
}

/// Takes each service the target release recorded back to its schema when
/// `rollback` is asked, then fails unless every one holds it.
fn reach_service_schemas(
	ip: &str,
	private: &str,
	c: &InfraConfig,
	project: &crate::deploy::Project,
	target: &Release,
	rollback: bool,
) -> Result<()> {
	for (name, schema) in &target.services {
		if rollback {
			rollback_service_schema(ip, private, c, project, name, &schema.schema)?;
		}
		ensure!(
			remote_service_history(ip, &c.name, &service_database(name))? == schema.migrations,
			"services.{name}: its database differs from target; use --rollback-schema --yes with \
			 the newer migration files, or recover explicitly"
		);
	}
	Ok(())
}

/// A service's database on the managed Postgres, which its role owns.
fn service_database(name: &str) -> String {
	crate::services::postgres_database(APP_DATABASE, name)
}

/// Whether each service's database holds the migrations `services` records.
fn services_match(
	ip: &str,
	c: &InfraConfig,
	services: &BTreeMap<String, ServiceSchema>,
) -> Result<bool> {
	for (name, schema) in services {
		if remote_service_history(ip, &c.name, &service_database(name))? != schema.migrations {
			return Ok(false);
		}
	}
	Ok(true)
}

pub(super) fn stop(hosts: &[String], c: &InfraConfig) -> Result<()> {
	for host in hosts {
		remote::execute(
			host,
			&format!(
				"set -eu; ids=$(docker ps -q --filter label=service={} --filter label=role=web); \
				 if [ -n \"$ids\" ]; then docker stop --time 30 $ids; fi",
				c.name
			),
			None,
		)?;
	}
	Ok(())
}

fn schema(root: &Path, config: &LeptoskitConfig) -> Result<Vec<(String, String)>> {
	let migrations = leptoskit_db::load_migrations(
		&root.join(&config.db.as_ref().context("missing database")?.migrations),
	)?;
	Ok(migrations
		.iter()
		.map(|m| (m.name.to_string(), m.checksum.to_string()))
		.collect())
}

pub(super) fn hosts(dir: &Path) -> Result<Vec<String>> {
	let result = Command::new("tofu")
		.current_dir(dir)
		.args(["output", "-json", "server_ips"])
		.output()?;
	ensure!(
		result.status.success(),
		"couldn't read application hosts from applied infrastructure"
	);
	let hosts: Vec<String> = serde_json::from_slice(&result.stdout)?;
	ensure!(
		!hosts.is_empty()
			&& hosts.len() <= 16
			&& hosts
				.iter()
				.all(|s| s.parse::<std::net::Ipv4Addr>().is_ok()),
		"invalid applied application hosts"
	);
	Ok(hosts)
}

fn image(c: &InfraConfig) -> String {
	if c.image.starts_with(&format!("{}/", c.registry)) {
		c.image.clone()
	} else {
		format!("{}/{}", c.registry, c.image)
	}
}

fn digest(ip: &str, c: &InfraConfig, version: &str) -> Result<String> {
	let container = remote::quote(&format!("{}-web-{version}", c.name));
	let script = format!(
		"set -eu; id=$(docker inspect --format '{{{{.Image}}}}' {container}); docker image \
		 inspect --format '{{{{json .RepoDigests}}}}' \"$id\""
	);
	let values: Vec<String> = serde_json::from_slice(&remote::execute(ip, &script, None)?)?;
	values
		.into_iter()
		.find(|value| value.starts_with(&format!("{}@sha256:", image(c))))
		.context("running container has no verified registry digest")
}

fn cached_digest(ip: &str, c: &InfraConfig, version: &str) -> Result<String> {
	let tag = remote::quote(&format!("{}:{version}", image(c)));
	let bytes = remote::execute(
		ip,
		&format!("docker image inspect --format '{{{{json .RepoDigests}}}}' {tag}"),
		None,
	)?;
	let values: Vec<String> = serde_json::from_slice(&bytes)?;
	values
		.into_iter()
		.find(|v| v.starts_with(&format!("{}@sha256:", image(c))))
		.context("pulled image lacks a registry digest")
}

fn prepare_recorded(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	release: &Release,
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let expected = release
		.image_digest
		.as_ref()
		.context("release predates digest recording; import a verified image as a new version")?;
	execute(dir, &["registry", "login"], env, reservation)?;
	execute(
		dir,
		&["build", "pull", "--version", &release.version],
		env,
		reservation,
	)?;
	for host in hosts {
		ensure!(
			&cached_digest(host, c, &release.version)? == expected,
			"registry tag differs from the recorded image digest; release blocked before database \
			 or traffic changes"
		);
	}
	verify_schema_labels(
		hosts,
		c,
		&release.version,
		&release.migrations,
		super::services_fingerprint(&release.services)?.as_deref(),
	)?;
	Ok(())
}

/// Pulls a new release's image onto every host and checks it carries this
/// checkout's schemas: the app's, and the services' when there are any.
fn pull_new(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	version: &str,
	(schema, services): (&str, Option<&str>),
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	execute(dir, &["registry", "login"], env, reservation)?;
	execute(
		dir,
		&["build", "pull", "--version", version],
		env,
		reservation,
	)?;
	verify_schema_labels(hosts, c, version, schema, services)
}

fn verify_schema_labels(
	hosts: &[String],
	c: &InfraConfig,
	version: &str,
	expected: &str,
	services: Option<&str>,
) -> Result<()> {
	let first = cached_digest(&hosts[0], c, version)?;
	for host in hosts {
		ensure!(
			cached_digest(host, c, version)? == first,
			"hosts pulled different image digests; release blocked before traffic changes"
		);
		let script = format!(
			"docker image inspect --format '{{{{index .Config.Labels \"{}\"}}}}' {}",
			super::build::SCHEMA_LABEL,
			remote::quote(&format!("{}:{version}", image(c)))
		);
		let value = remote::execute(host, &script, None)?;
		ensure!(
			String::from_utf8(value)?.trim() == expected,
			"image migration metadata differs from the expected schema; build this checkout with \
			 `lk infra build --version <tag> --push` before releasing"
		);
		if let Some(services) = services {
			let script = format!(
				"docker image inspect --format '{{{{index .Config.Labels \"{}\"}}}}' {}",
				super::build::SERVICES_SCHEMA_LABEL,
				remote::quote(&format!("{}:{version}", image(c)))
			);
			let value = remote::execute(host, &script, None)?;
			ensure!(
				String::from_utf8(value)?.trim() == services,
				"image service migration metadata differs from the services' expected schemas; \
				 build this checkout with `lk infra build --version <tag> --push` before releasing"
			);
		}
	}
	Ok(())
}

fn secrets(config: &LeptoskitConfig, private: &str) -> Result<BTreeMap<String, String>> {
	let c = config.infra.as_ref().context("missing infrastructure")?;
	let required = |name: &str| {
		std::env::var(name)
			.ok()
			.filter(|s| !s.is_empty())
			.with_context(|| format!("set required secret {name}"))
	};
	let admin = required("POSTGRES_PASSWORD")?;
	let password = required("DATABASE_PASSWORD")?;
	ensure!(
		admin != password,
		"administrator and application database passwords must differ"
	);
	let mut db = url::Url::parse(&format!(
		"postgres://leptoskit@{private}:5432/leptoskit?sslmode=disable"
	))?;
	db.set_password(Some(&password))
		.map_err(|()| anyhow::anyhow!("couldn't bind database password"))?;
	let mut values = BTreeMap::new();
	if c.plugins.analytics.is_some() {
		let key = required("LEPTOSKIT_ANALYTICS_READ_KEY")?;
		ensure!(
			(32..=256).contains(&key.len()) && key.bytes().all(|b| (33..127).contains(&b)),
			"LEPTOSKIT_ANALYTICS_READ_KEY needs 32..=256 printable non-space ASCII characters"
		);
		values.insert("LEPTOSKIT_ANALYTICS_READ_KEY".into(), key);
	}
	values.insert(config.db.as_ref().unwrap().url_env.clone(), db.to_string());
	if c.services.cache {
		let mut url = url::Url::parse(&format!("redis://{private}:6379"))?;
		url.set_password(Some(&required("VALKEY_PASSWORD")?))
			.map_err(|()| anyhow::anyhow!("couldn't bind cache password"))?;
		values.insert("VALKEY_URL".into(), url.to_string());
	}
	if c.services.relay {
		let mut url = url::Url::parse(&format!("nats://{private}:4222"))?;
		url.set_username(&required("NATS_TOKEN")?)
			.map_err(|()| anyhow::anyhow!("couldn't bind relay token"))?;
		values.insert(
			config
				.rpc
				.live
				.relay_url_env
				.clone()
				.context("configure rpc.live.relay_url_env")?,
			url.to_string(),
		);
	}
	for name in c
		.secrets
		.iter()
		.chain(config.auth.iter().map(|auth| &auth.keys_env))
	{
		required(name)?;
	}
	required("KAMAL_REGISTRY_PASSWORD")?;
	Ok(values)
}

fn execute(
	dir: &Path,
	args: &[&str],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	reservation.check()?;
	let status = Command::new("kamal")
		.current_dir(dir)
		.args(args)
		.args(["-c", "release.yml"])
		.envs(env)
		// Every boot (including failure recovery) gets the actual selected
		// version, rather than the version baked into a reused config file.
		.env(
			"LEPTOSKIT_RELEASE",
			args.windows(2)
				.find(|pair| pair[0] == "--version")
				.map_or("unknown", |pair| pair[1]),
		)
		.status()
		.context("couldn't run Kamal 2")?;
	reservation.check()?;
	ensure!(
		status.success(),
		"Kamal operation failed ({status}); shared operation status records the failure"
	);
	Ok(())
}

fn record(ip: &str, c: &InfraConfig, owner: &str, phase: &str, version: &str) -> Result<()> {
	remote::write(
		ip,
		&format!("{}/operation.json", remote::base(&c.name)),
		&serde_json::to_vec(&json!({
		"owner": owner, "phase": phase, "version": version, "updated_at": chrono::Utc::now().to_rfc3339()}))?,
	)
}

struct Flight<'a> {
	ip: &'a str,
	config: &'a InfraConfig,
	owner: &'a str,
	version: &'a str,
	finished: bool,
}

impl Drop for Flight<'_> {
	fn drop(&mut self) {
		if self.finished {
			return;
		}
		let phase = remote::read(self.ip, &self.config.name, "operation.json")
			.ok()
			.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
		if phase.as_ref().is_some_and(|value| {
			!matches!(
				value["phase"].as_str(),
				Some("previous-restored" | "verification-failed-app-stopped" | "failed")
			)
		}) {
			let _ = record(self.ip, self.config, self.owner, "failed", self.version);
		}
	}
}

fn bootstrap(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let ip = &hosts[0];
	if c.services.relay {
		remote::execute(
			ip,
			&format!(
				"set -eu; install -d -m 700 -o 1000 -g 1000 /var/lib/leptoskit/{}/nats",
				c.name
			),
			None,
		)?;
	}

	for host in hosts {
		remote::execute(host, "cloud-init status --wait >/dev/null", None)?;
	}
	execute(dir, &["server", "bootstrap"], env, reservation)?;
	for accessory in [
		Some("db"),
		Some("tunnel"),
		c.services.cache.then_some("cache"),
		c.services.relay.then_some("relay"),
	]
	.into_iter()
	.flatten()
	{
		let accessory_hosts = if accessory == "tunnel" {
			hosts
		} else {
			&hosts[..1]
		};
		for host in accessory_hosts {
			let inspect = remote::execute(
				host,
				&format!(
					"docker inspect --format '{{{{.State.Status}}}}' {}-{}",
					c.name, accessory
				),
				None,
			);
			if inspect.as_ref().is_ok_and(|v| v == b"running\n") {
				continue;
			}
			let action = if inspect.is_ok() { "start" } else { "boot" };
			execute(
				dir,
				&["accessory", action, accessory, "--hosts", host],
				env,
				reservation,
			)?;
		}
	}
	remote::execute(
		ip,
		&format!(
			"for i in $(seq 1 60); do if docker exec {}-db pg_isready -U leptoskit -d leptoskit \
			 >/dev/null; then exit 0; fi; sleep 1; done; exit 1",
			c.name
		),
		None,
	)?;

	Ok(())
}

fn verify_release(
	hosts: &[String],
	c: &InfraConfig,
	version: &str,
	expected: &str,
	services: &BTreeMap<String, ServiceSchema>,
	target: Option<&Release>,
) -> Result<String> {
	ensure!(
		remote_history(&hosts[0], &c.name)? == expected,
		"database does not match release; shared history was not advanced"
	);
	// Each service applies its own migrations as it starts, so its database
	// gets a moment to catch up with the release.
	for (name, schema) in services {
		let database = service_database(name);
		let mut actual = remote_service_history(&hosts[0], &c.name, &database)?;
		for _ in 0..30 {
			if actual == schema.migrations {
				break;
			}
			std::thread::sleep(std::time::Duration::from_secs(2));
			actual = remote_service_history(&hosts[0], &c.name, &database)?;
		}
		ensure!(
			actual == schema.migrations,
			"services.{name}: its database does not match the release; shared history was not \
			 advanced"
		);
	}
	let image = digest(&hosts[0], c, version)?;
	for host in hosts {
		ensure!(
			digest(host, c, version)? == image,
			"hosts are running different image digests"
		);
	}
	if let Some(target) = target {
		ensure!(
			target.image_digest.as_ref() == Some(&image),
			"running image differs from recorded digest"
		);
	}
	Ok(image)
}

fn runtime_files(
	dir: &Path,
	c: &InfraConfig,
	root: &Path,
	config: &LeptoskitConfig,
	hosts: &[String],
	private: &str,
) -> Result<(BTreeMap<String, String>, crate::deploy::Project)> {
	let project = crate::deploy::Project::read(root, config)?;
	ensure!(
		project.has_probes,
		"managed releases require readiness probes"
	);
	let (runtime_config, references) = runtime::kamal(c, &project, hosts, private)?;
	fs::write(dir.join("release.yml"), runtime_config)?;
	fs::write(dir.join("db-init.sh"), runtime::DATABASE_INIT)?;
	fs::write(dir.join("nats.conf"), runtime::NATS_CONFIG)?;
	fs::create_dir_all(dir.join(".kamal"))?;
	fs::write(dir.join(".kamal/secrets"), references)?;
	let mut env = secrets(config, private)?;
	for name in project.secrets.iter().chain(&project.urls) {
		ensure!(
			env.contains_key(name) || std::env::var(name).is_ok_and(|v| !v.is_empty()),
			"set required application secret {name}"
		);
	}
	env.extend(super::services::values(&project, private)?);
	env.insert("TUNNEL_TOKEN".into(), output(dir, "tunnel_token")?);
	Ok((env, project))
}

fn recover_previous(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	history: &[Release],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let ip = &hosts[0];
	let actual = remote_history(ip, &c.name)?;
	if let Some(previous) = history.last()
		&& actual == previous.migrations
		&& services_match(ip, c, &previous.services)?
	{
		prepare_recorded(dir, c, hosts, previous, env, reservation)?;
		execute(
			dir,
			&["app", "boot", "--version", &previous.version],
			env,
			reservation,
		)?;
		verify_release(
			hosts,
			c,
			&previous.version,
			&previous.migrations,
			&previous.services,
			Some(previous),
		)?;
		record(
			ip,
			c,
			&reservation.owner,
			"previous-restored",
			&previous.version,
		)?;
	} else {
		stop(hosts, c)?;
	}
	Ok(())
}

/// Writes the release history to the primary host, where it's shared, and
/// to the operator's workspace.
fn save_history(ip: &str, c: &InfraConfig, dir: &Path, history: &[Release]) -> Result<()> {
	let bytes = serde_json::to_vec_pretty(history)?;
	remote::write(
		ip,
		&format!("{}/releases.json", remote::base(&c.name)),
		&bytes,
	)?;
	fs::write(dir.join("releases.json"), &bytes)?;
	Ok(())
}

fn release_version(args: &ReleaseArgs) -> Result<&String> {
	let version = args
		.version
		.as_ref()
		.or(args.rollback.as_ref())
		.context("pass --version or --rollback")?;
	super::build::validate_version(version)?;
	Ok(version)
}

fn target_release(
	args: &ReleaseArgs,
	history: &[Release],
	version: &str,
	local_hash: &str,
) -> Result<Option<Release>> {
	let target = if args.rollback.is_some() || args.recover {
		Some(
			history
				.iter()
				.find(|r| r.version == version)
				.cloned()
				.context("version is absent from shared release history")?,
		)
	} else {
		None
	};
	if target.is_none() {
		ensure!(
			!history.iter().any(|r| r.version == version),
			"tag already recorded; use --recover or a new tag"
		);
		ensure!(
			!history.is_empty() || args.setup,
			"first release requires --setup"
		);
		if let Some(previous) = history.last() {
			ensure!(
				previous.migrations == local_hash || args.maintenance,
				"schema changes require --maintenance"
			);
		}
	}
	Ok(target)
}

pub(super) fn run(args: &ReleaseArgs) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let c = managed_settings(&config)?;
	let version = release_version(args)?;
	let dir = directory(&root, c);
	generate(&dir, c, false)?;
	let _local = lock(&dir)?;
	let hosts = hosts(&dir)?;
	ensure!(
		hosts.len() == usize::from(c.instances),
		"apply the configured host count before release"
	);
	let ip = &hosts[0];
	applied_bindings(&dir, c)?;
	let private = output(&dir, "private_ip")?;
	ensure!(
		private.parse::<std::net::Ipv4Addr>().is_ok(),
		"invalid primary private address"
	);
	let reservation = Reservation::acquire(ip, &c.name)?;
	let reset: serde_json::Value =
		serde_json::from_slice(&remote::read(ip, &c.name, "derived-reset-required.json")?)?;
	ensure!(
		reset.is_array(),
		"database restore has unfinished relay/cache recovery; retry `lk infra restore --snapshot \
		 <id> --yes` before starting applications"
	);
	let mut history = history(ip, c)?;
	let local_schema = schema(&root, &config)?;
	let local_hash = history_fingerprint(local_schema.clone())?;
	let target = target_release(args, &history, version, &local_hash)?;
	let (env, project) = runtime_files(&dir, c, &root, &config, &hosts, &private)?;
	let local_services = super::service_schemas(&project.services)?;
	check_service_changes(
		args,
		target.as_ref().map_or(history.last(), |_| None),
		&local_services,
	)?;
	if c.backups.enabled && args.setup {
		super::recovery::credentials()?;
	}
	record(ip, c, &reservation.owner, "preparing", version)?;
	let mut flight = Flight {
		ip,
		config: c,
		owner: &reservation.owner,
		version,
		finished: false,
	};
	if args.setup {
		bootstrap(&dir, c, &hosts, &env, &reservation)?;
	}
	if let Some(target) = &target {
		prepare_recorded(&dir, c, &hosts, target, &env, &reservation)?;
	} else {
		let services = super::services_fingerprint(&local_services)?;
		pull_new(
			&dir,
			c,
			&hosts,
			version,
			(&local_hash, services.as_deref()),
			&env,
			&reservation,
		)?;
	}
	execute(&dir, &["proxy", "boot"], &env, &reservation)?;
	if !args.setup && target.is_none() {
		let previous = history.last().context("no previous release")?;
		check_drift(ip, c, args, previous, (&local_hash, &local_services))?;
	}
	if args.maintenance || args.rollback_schema {
		stop(&hosts, c)?;
		stop_services(ip, c, &project)?;
		if !history.is_empty() && c.backups.enabled {
			super::recovery::invoke(ip, c, "backup", "latest", false, Some(&reservation.owner))?;
		}
	}
	if let Some(target) = &target {
		if args.rollback_schema {
			rollback_schema(ip, &private, &root, &config, &target.schema)?;
		}
		ensure!(
			remote_history(ip, &c.name)? == target.migrations,
			"database differs from target; use --rollback-schema --yes with the newer migration \
			 files, or recover explicitly"
		);
		reach_service_schemas(ip, &private, c, &project, target, args.rollback_schema)?;
	}
	super::services::prepare(ip, c, &project)?;
	let result = execute(
		&dir,
		&["app", "boot", "--version", version],
		&env,
		&reservation,
	);
	if let Err(error) = result {
		record(ip, c, &reservation.owner, "failed", version)?;
		if recover_previous(&dir, c, &hosts, &history, &env, &reservation).is_err() {
			eprintln!(
				"Automatic recovery did not complete; inspect shared status and recover \
				 explicitly."
			);
		}
		return Err(error);
	}
	let expected = target
		.as_ref()
		.map_or(&local_hash, |target| &target.migrations);
	let services = target
		.as_ref()
		.map_or(local_services, |target| target.services.clone());
	let verified = verify_release(&hosts, c, version, expected, &services, target.as_ref());
	let image = match verified {
		Ok(image) => image,
		Err(error) => {
			stop(&hosts, c)?;
			record(
				ip,
				c,
				&reservation.owner,
				"verification-failed-app-stopped",
				version,
			)?;
			return Err(error);
		}
	};
	history.push(Release {
		version: version.clone(),
		migrations: expected.clone(),
		schema: target.as_ref().map_or(local_schema, |t| t.schema.clone()),
		image_digest: Some(image),
		completed_at: Some(chrono::Utc::now().to_rfc3339()),
		services,
	});
	save_history(ip, c, &dir, &history)?;
	record(ip, c, &reservation.owner, "complete", version)?;
	if args.setup && c.backups.enabled {
		super::recovery::install(ip, c, &output(&dir, "backup_endpoint")?, &reservation)?;
		super::recovery::invoke(ip, c, "backup", "latest", false, Some(&reservation.owner))?;
	}
	execute(&dir, &["prune", "all"], &env, &reservation)?;
	println!(
		"Released {version} on {} host(s) at https://{}",
		hosts.len(),
		c.domain
	);
	flight.finished = true;
	Ok(())
}

pub(super) fn prepare() -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let c = managed_settings(&config)?;
	let dir = directory(&root, c);
	generate(&dir, c, false)?;
	let _local = lock(&dir)?;
	applied_bindings(&dir, c)?;
	let hosts = hosts(&dir)?;
	ensure!(
		hosts.len() == usize::from(c.instances),
		"apply the configured hosts before preparing services"
	);
	let private = output(&dir, "private_ip")?;
	ensure!(
		private.parse::<std::net::Ipv4Addr>().is_ok(),
		"invalid private IP"
	);
	let ip = &hosts[0];
	let reservation = Reservation::acquire(ip, &c.name)?;
	let (env, project) = runtime_files(&dir, c, &root, &config, &hosts, &private)?;
	record(ip, c, &reservation.owner, "preparing-services", "services")?;
	let mut flight = Flight {
		ip,
		config: c,
		owner: &reservation.owner,
		version: "services",
		finished: false,
	};
	bootstrap(&dir, c, &hosts, &env, &reservation)?;
	super::services::prepare(ip, c, &project)?;
	execute(&dir, &["proxy", "boot"], &env, &reservation)?;
	if c.backups.enabled {
		super::recovery::install(ip, c, &output(&dir, "backup_endpoint")?, &reservation)?;
	}
	record(ip, c, &reservation.owner, "services-ready", "services")?;
	flight.finished = true;
	println!(
		"Private services are prepared. No application or initial backup was started. Restore the \
		 chosen snapshot before `lk deploy --recover --version <recorded-tag>`."
	);
	Ok(())
}

fn rollback_schema(
	ip: &str,
	private: &str,
	root: &Path,
	config: &LeptoskitConfig,
	target: &[(String, String)],
) -> Result<()> {
	let name = &config
		.infra
		.as_ref()
		.context("missing infrastructure")?
		.name;
	let db_config = config.db.as_ref().context("missing database")?;
	let migrations = leptoskit_db::load_migrations(&root.join(&db_config.migrations))?;
	let password = std::env::var("DATABASE_PASSWORD").context("set DATABASE_PASSWORD")?;
	rollback_database(
		ip,
		private,
		name,
		APP_DATABASE,
		&password,
		&migrations,
		target,
	)
}

/// Rolls a service's database back to `target` with the down migrations in
/// this checkout, logged in as the service's role, which owns its tables.
fn rollback_service_schema(
	ip: &str,
	private: &str,
	c: &InfraConfig,
	project: &crate::deploy::Project,
	name: &str,
	target: &[(String, String)],
) -> Result<()> {
	let service = project
		.services
		.iter()
		.find(|service| service.name == name && service.database)
		.with_context(|| {
			format!(
				"services.{name} is in the target release but has no database in this checkout's \
				 leptoskit.toml, so its down migrations aren't here; restore its \
				 [services.{name}] table, or recover explicitly"
			)
		})?;
	let directory = service
		.migrations
		.as_ref()
		.with_context(|| format!("services.{name} has no migrations directory in this checkout"))?;
	let migrations = if directory.is_dir() {
		leptoskit_db::load_migrations(directory)?
	} else {
		Vec::new()
	};
	let variable = service.database_password_env();
	let password = std::env::var(&variable).with_context(|| format!("set {variable}"))?;
	rollback_database(
		ip,
		private,
		&c.name,
		&service.postgres_name(APP_DATABASE),
		&password,
		&migrations,
		target,
	)
}

/// Runs the down migrations that take `database`, owned by the role of the
/// same name, back to `target`, through an SSH tunnel to the private
/// Postgres, once nothing is connected to it.
fn rollback_database(
	ip: &str,
	private: &str,
	name: &str,
	database: &str,
	password: &str,
	migrations: &[leptoskit_db::Migration],
	target: &[(String, String)],
) -> Result<()> {
	ensure!(
		database
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
		"unexpected database name"
	);
	let clients = remote::execute(
		ip,
		&format!(
			"docker exec {name}-db psql -U leptoskit_admin -d postgres -At -c \"SELECT count(*) \
			 FROM pg_stat_activity WHERE datname = '{database}'\""
		),
		None,
	)?;
	ensure!(
		String::from_utf8(clients)?.trim() == "0",
		"`{database}` still has clients; stop external writers before down migrations"
	);
	ensure!(
		target.len() <= migrations.len()
			&& migrations
				.iter()
				.take(target.len())
				.zip(target)
				.all(|(m, (name, sum))| m.name == *name && m.checksum == *sum),
		"the target migration prefix of `{database}` is not present in this checkout"
	);
	let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
	let port = listener.local_addr()?.port();
	drop(listener);
	let child = Command::new("ssh")
		.args([
			"-o",
			"BatchMode=yes",
			"-o",
			"ExitOnForwardFailure=yes",
			"-N",
			"-L",
			&format!("127.0.0.1:{port}:{private}:5432"),
			&format!("root@{ip}"),
		])
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()?;
	struct Tunnel(std::process::Child);
	impl Drop for Tunnel {
		fn drop(&mut self) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
	let _tunnel = Tunnel(child);
	let mut url = url::Url::parse(&format!(
		"postgres://{database}@127.0.0.1:{port}/{database}?sslmode=disable"
	))?;
	url.set_password(Some(password))
		.map_err(|()| anyhow::anyhow!("invalid database password"))?;
	tokio::runtime::Runtime::new()?.block_on(async {
		let mut connection = None;
		for _ in 0..50 {
			match leptoskit_db::Db::connect(url.as_str()).await {
				Ok(db) => {
					connection = Some(db);
					break;
				}
				Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
			}
		}
		let db = connection.context("database tunnel did not become ready")?;
		let applied = db.applied_migrations().await?;
		ensure!(
			applied.len() >= target.len(),
			"`{database}` is older than the target schema"
		);
		if applied.len() > target.len() {
			db.rollback(migrations, applied.len() - target.len())
				.await?;
		}
		anyhow::Ok(())
	})
}
