//! How a deployment becomes the [`ReleaseSpec`] its cluster applies, and
//! the hostname an environment is served at.
//!
//! It happens in two steps. [`plan`] runs when the deployment is made, from
//! the resolved `[cloud]` table and the cluster's facts: it decides every
//! process's size and scale, refuses what the cloud can't run yet, and
//! writes the fine print. [`spec`] runs once the build says how each
//! process starts, and turns the plan into a spec that has passed
//! [`ReleaseSpec::validate`].
//!
//! # Hostnames
//!
//! An environment is served at `<label>.<apps domain>`, where the label is
//! `<project>-<organization>` for production and
//! `<environment>-<project>-<organization>` for any other environment.
//! A label longer than 63 characters, the DNS limit, keeps its first 54
//! characters and ends with `-` and eight hex digits of the SHA-256 of the
//! whole label ([`hostname_label`]). Slugs have hyphens of their own, so two
//! environments can still want the same label (`notes-app` in `acme` and
//! `notes` in `app-acme`); the control plane keeps hostnames unique, and the
//! second gets [`fallback_label`] instead, from its environment's ID.

use std::collections::BTreeMap;

use leptoskit_cloud_runtime::Egress;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::Image;
use leptoskit_cloud_runtime::Policy;
use leptoskit_cloud_runtime::Probes;
use leptoskit_cloud_runtime::Process;
use leptoskit_cloud_runtime::ProcessKind;
use leptoskit_cloud_runtime::Quota;
use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_cloud_runtime::Resources;
use leptoskit_cloud_runtime::Route;
use leptoskit_cloud_runtime::Scale;
use leptoskit_cloud_runtime::SpecError;
use leptoskit_cloud_runtime::Tls;
use leptoskit_cloud_runtime::Volume;
use leptoskit_config::DbConfig;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ServiceConfig;
use leptoskit_config::ServiceTrust;
use leptoskit_config::cloud::CloudConfig;
use leptoskit_config::cloud::ResolvedDatabase;
use leptoskit_config::cloud::ResolvedEnvironment;
use leptoskit_config::cloud::ResolvedProcess;
use sha2::Digest as _;
use uuid::Uuid;

use crate::deployments::AppFacts;
use crate::deployments::DatabaseDialect;
use crate::deployments::DeployConfig;
use crate::operator::ClusterFacts;
use crate::projects::EnvironmentScale;
use crate::projects::ProcessScale;
use crate::size::Size;
use crate::size::Sleep;
use crate::workers::BuildServerOutput;

/// The longest DNS label.
pub const MAX_LABEL_LEN: usize = 63;

/// The app's process name.
pub const APP: &str = "app";

/// Where a SQLite app keeps its database file.
pub const SQLITE_MOUNT: &str = "/data";

/// The volume a SQLite app's database lives on, in GiB.
pub const SQLITE_VOLUME_GIB: u32 = 1;

/// The `DATABASE_URL` a SQLite app gets.
pub const SQLITE_URL: &str = "sqlite:///data/app.db";

/// The path prefix a public service is reached under on the app's hostname.
#[must_use]
pub fn service_prefix(name: &str) -> String {
	format!("/_leptoskit/services/{name}/")
}

/// The hostname label an environment wants: see the [module docs](self).
#[must_use]
pub fn hostname_label(
	organization: &str,
	project: &str,
	environment: &str,
	production: bool,
) -> String {
	let label = if production {
		format!("{project}-{organization}")
	} else {
		format!("{environment}-{project}-{organization}")
	};
	shorten(&label)
}

/// The label an environment gets when the one it wants is taken: its
/// project and organisation's label, shortened to leave room, and twelve hex
/// digits of the SHA-256 of its ID, which no other environment has.
#[must_use]
pub fn fallback_label(organization: &str, project: &str, environment: Uuid) -> String {
	let digest = hex_digest(environment.as_bytes());
	let suffix = &digest[..12];
	let mut head = format!("{project}-{organization}");
	head.truncate(MAX_LABEL_LEN - suffix.len() - 1);
	let head = head.trim_end_matches('-');
	format!("{head}-{suffix}")
}

/// `label` when it fits a DNS label, or its first 54 characters, `-`, and
/// eight hex digits of its SHA-256.
fn shorten(label: &str) -> String {
	if label.len() <= MAX_LABEL_LEN {
		return label.to_owned();
	}

	let digest = hex_digest(label.as_bytes());
	let head = label[..MAX_LABEL_LEN - 9].trim_end_matches('-');
	format!("{head}-{}", &digest[..8])
}

fn hex_digest(bytes: &[u8]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let hash = sha2::Sha256::digest(bytes);
	let mut out = String::with_capacity(64);

	for byte in hash {
		out.push(char::from(HEX[usize::from(byte >> 4)]));
		out.push(char::from(HEX[usize::from(byte & 0x0f)]));
	}

	out
}

/// What a remote service's address is taken to be when `[cloud]` is
/// resolved: resolution reads only whether a service has one.
const REMOTE_SERVICE: &str = "https://remote.service.invalid";

/// The parts of `leptoskit.toml` resolving `[cloud]` reads, rebuilt from
/// what the CLI reported: `[db] dialect`, whether the app serves live
/// queries, and each service's address, publicity, trust, and database.
#[must_use]
pub fn app_config(facts: &AppFacts) -> LeptoskitConfig {
	let mut app = LeptoskitConfig::default();

	if let Some(dialect) = facts.database {
		app.db = Some(DbConfig {
			dialect: match dialect {
				DatabaseDialect::Sqlite => DbDialect::Sqlite,
				DatabaseDialect::Postgres => DbDialect::Postgres,
			},
			..DbConfig::default()
		});
	}

	if facts.live {
		app.rpc.live_path = Some("/_leptoskit/live".to_owned());
	}

	for (name, service) in &facts.services {
		app.services.insert(
			name.clone(),
			ServiceConfig {
				url: service.remote.then(|| REMOTE_SERVICE.to_owned()),
				public: Some(service.public),
				database: Some(service.database),
				trust: service.isolated.then_some(ServiceTrust::Isolated),
				..ServiceConfig::default()
			},
		);
	}

	app
}

/// Why a deployment's configuration doesn't resolve.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid_config: {0}")]
pub struct ResolveError(String);

/// The environment `environment` as `config` resolves it, with
/// [`app_config`]'s facts. An environment the table doesn't list gets the
/// defaults, as production does.
///
/// # Errors
///
/// [`ResolveError`], naming the table and key.
pub fn resolve(
	config: &DeployConfig,
	environment: &str,
) -> Result<ResolvedEnvironment, ResolveError> {
	let app = app_config(&config.app);
	let mut cloud: CloudConfig = config.cloud.clone();
	cloud
		.environments
		.entry(environment.to_owned())
		.or_default();
	let resolved = cloud
		.resolved(&app)
		.map_err(|error| ResolveError(error.to_string()))?;
	resolved
		.environments
		.into_iter()
		.find(|(name, _)| name.as_str() == environment)
		.map(|(_, resolved)| resolved)
		.ok_or_else(|| ResolveError(format!("`{environment}` isn't an environment's name")))
}

/// One process, as the plan runs it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlannedProcess {
	pub name: String,
	pub kind: ProcessKind,
	pub size: Size,
	pub min: u32,
	pub max: u32,
	pub sleep: bool,
	/// Reached from browsers: the app always, a service when `public`.
	pub routed: bool,
	/// Keeps the app's SQLite database on a volume.
	pub sqlite: bool,
}

/// What a deployment will run, before the build says how each process
/// starts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Plan {
	/// The app first, then services by name.
	pub processes: Vec<PlannedProcess>,
	/// Whether the app has a SQLite database.
	pub sqlite: bool,
	/// Whether the app has a database at all.
	pub database: bool,
	/// What the release won't do, as sentences.
	pub fine_print: Vec<String>,
}

/// Why the cloud can't run a configuration yet.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
	/// The app's database is Postgres, and the managed one comes later.
	#[error(
		"postgres_unavailable: managed Postgres is not available yet; the app's `[db]` is \
		 Postgres, so it can't be deployed until the databases slice ships"
	)]
	PostgresUnavailable,
	/// An isolated service is public, and stripping cookies on its route
	/// comes later.
	#[error(
		"isolated_public_service: `{name}` is public and `trust = \"isolated\"`, which the cloud \
		 can't serve yet: its route would have to strip the app's cookies"
	)]
	IsolatedPublicService { name: String },
	/// A service shares the app's SQLite database, which lives on the app's
	/// own volume.
	#[error(
		"sqlite_shared: `{name}` uses the app's SQLite database, which lives on the app's own \
		 volume, so it can't run as a separate process"
	)]
	SqliteShared { name: String },
	/// A service's name can't name a process.
	#[error(
		"invalid_service_name: `{name}` can't name a process: it must be a DNS label other than \
		 `app`"
	)]
	ServiceName { name: String },
}

/// Plans an environment's release from its resolved `[cloud]` table.
///
/// `stored` is the environment's scale as the console last set it, when
/// it differs from `leptoskit.toml`: then it wins, and the fine print says
/// so. `facts` says which services are public; `cluster` whether it can
/// wake a sleeping process.
///
/// # Errors
///
/// [`PlanError`] for what the cloud can't run yet.
pub fn plan(
	resolved: &ResolvedEnvironment,
	stored: Option<&EnvironmentScale>,
	facts: &AppFacts,
	cluster: &ClusterFacts,
) -> Result<Plan, PlanError> {
	let mut fine_print = Vec::new();
	let sqlite = match resolved.database {
		ResolvedDatabase::Postgres { .. } => return Err(PlanError::PostgresUnavailable),
		ResolvedDatabase::Sqlite { .. } => true,
		ResolvedDatabase::None => false,
	};

	if stored.is_some() {
		fine_print.push(
			"Its sizes and scale are the ones set in the console, which differ from \
			 `leptoskit.toml`; this deployment keeps them."
				.to_owned(),
		);
	}

	let app_scale = stored.map_or_else(|| from_resolved(resolved.app), |stored| stored.app);
	let mut processes = vec![planned(
		APP,
		ProcessKind::App,
		app_scale,
		true,
		sqlite,
		cluster,
		&mut fine_print,
	)];

	for (name, service) in &resolved.services {
		if name == APP || !is_dns_label(name) {
			return Err(PlanError::ServiceName { name: name.clone() });
		}

		let declared = facts.services.get(name).cloned().unwrap_or_default();

		if declared.public && declared.isolated {
			return Err(PlanError::IsolatedPublicService { name: name.clone() });
		}

		if sqlite && service.database {
			return Err(PlanError::SqliteShared { name: name.clone() });
		}

		let scale = stored
			.and_then(|stored| stored.services.get(name).copied())
			.unwrap_or_else(|| from_resolved(service.process));
		processes.push(planned(
			name,
			ProcessKind::Service,
			scale,
			declared.public,
			false,
			cluster,
			&mut fine_print,
		));
	}

	if sqlite {
		fine_print.push(format!(
			"The SQLite database lives on a {SQLITE_VOLUME_GIB} GiB volume at `{SQLITE_MOUNT}`, \
			 so the app runs one replica, and replacing it stops it before the new one starts."
		));
	}

	if !resolved.domains.is_empty() {
		let domains: Vec<String> = resolved
			.domains
			.iter()
			.map(|domain| format!("`{domain}`"))
			.collect();
		fine_print.push(format!(
			"It serves {} only once {} attached to the environment and verified.",
			domains.join(", "),
			if domains.len() == 1 {
				"it is"
			} else {
				"each is"
			}
		));
	}

	Ok(Plan {
		processes,
		sqlite,
		database: !matches!(resolved.database, ResolvedDatabase::None),
		fine_print,
	})
}

fn from_resolved(process: ResolvedProcess) -> ProcessScale {
	ProcessScale {
		size: process.size,
		min: process.scale.min(),
		max: process.scale.max(),
		sleep: process.sleep,
	}
}

/// One process of the plan, with what the cluster and the database allow.
fn planned(
	name: &str,
	kind: ProcessKind,
	scale: ProcessScale,
	routed: bool,
	sqlite: bool,
	cluster: &ClusterFacts,
	fine_print: &mut Vec<String>,
) -> PlannedProcess {
	let mut min = scale.min;
	let mut max = scale.max.max(1);
	let mut sleep = scale.sleep == Sleep::WhenIdle && min == 0;

	if sleep && !cluster.wakes_sleeping {
		fine_print.push(format!(
			"`{name}` won't sleep: its cluster can't wake a process on its first request, so it \
			 keeps one replica."
		));
		sleep = false;
	} else if sleep && !routed {
		fine_print.push(format!(
			"`{name}` won't sleep: only a process browsers reach can be woken by a request, so it \
			 keeps one replica."
		));
		sleep = false;
	}

	if !sleep && min == 0 {
		min = 1;
	}

	if sqlite && max > 1 {
		fine_print.push(format!(
			"`{name}` runs one replica, not {max}: its SQLite database can't be shared."
		));
		max = 1;
	}

	min = min.min(max);

	PlannedProcess {
		name: name.to_owned(),
		kind,
		size: scale.size,
		min,
		max,
		sleep,
		routed,
		sqlite,
	}
}

/// Why a plan and a build don't make a spec.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpecBuildError {
	/// The build didn't say how a process starts.
	#[error("the build didn't say how to run `{name}`")]
	MissingProcess { name: String },
	/// The spec failed [`ReleaseSpec::validate`].
	#[error("the release spec is invalid: {0}")]
	Invalid(#[from] SpecError),
}

/// What [`spec`] needs besides the plan and the build.
#[derive(Clone, Debug)]
pub struct SpecTarget<'a> {
	pub environment: EnvironmentRef,
	pub deployment: u64,
	pub hostname: &'a str,
	pub cluster: &'a ClusterFacts,
	/// The environment's variables, which every process starts with.
	pub variables: &'a BTreeMap<String, String>,
	/// The names of the environment's secrets, which every process starts
	/// with. The values never come here: a cluster worker is given them
	/// with the release it claims.
	pub secrets: &'a [String],
	/// The environment's verified custom domains. Each serves what the
	/// hostname does, with a certificate issued for it.
	pub domains: &'a [String],
}

/// The spec for `plan`, built as `output` says, checked with
/// [`ReleaseSpec::validate`]. Its quota is exactly what its processes need
/// at full scale, with a surge replica each for rolling updates.
///
/// # Errors
///
/// [`SpecBuildError`] when the build leaves a process out, or the spec is
/// invalid.
pub fn spec(
	plan: &Plan,
	output: &BuildServerOutput,
	target: &SpecTarget<'_>,
) -> Result<ReleaseSpec, SpecBuildError> {
	let mut processes = Vec::with_capacity(plan.processes.len());
	let mut routes = Vec::new();

	for planned in &plan.processes {
		let facts = output
			.processes
			.iter()
			.find(|facts| facts.name == planned.name)
			.ok_or_else(|| {
				SpecBuildError::MissingProcess {
					name: planned.name.clone(),
				}
			})?;
		let image = facts.image.as_ref().unwrap_or(&output.image);
		// What the cloud sets comes last, so nothing the environment holds
		// stands in for it; `config::check_name` refuses those names too.
		let mut env = target.variables.clone();
		env.insert(
			"LEPTOSKIT_ENVIRONMENT".to_owned(),
			target.environment.environment.slug.clone(),
		);
		env.insert(
			"LEPTOSKIT_DEPLOYMENT".to_owned(),
			target.deployment.to_string(),
		);
		let mut volumes = Vec::new();

		if planned.sqlite {
			env.insert("DATABASE_URL".to_owned(), SQLITE_URL.to_owned());
			volumes.push(Volume {
				name: "data".to_owned(),
				mount_path: SQLITE_MOUNT.to_owned(),
				size_gib: SQLITE_VOLUME_GIB,
			});
		}

		processes.push(Process {
			name: planned.name.clone(),
			kind: planned.kind,
			image: Image {
				repository: image.repository.clone(),
				digest: image.digest.as_str().to_owned(),
			},
			command: facts.command.clone(),
			args: facts.args.clone(),
			port: facts.port,
			probes: Probes {
				ready: facts.ready_path.clone(),
				live: facts.live_path.clone(),
			},
			secret_env: target
				.secrets
				.iter()
				.filter(|name| !env.contains_key(*name))
				.map(|name| (name.clone(), name.clone()))
				.collect(),
			env,
			resources: Resources {
				cpu_request_millis: planned.size.cpu_request_millicores(),
				cpu_limit_millis: planned.size.cpu_limit_millicores(),
				memory_mib: planned.size.memory_mib(),
			},
			scale: Scale {
				min: planned.min,
				max: planned.max,
				sleep: planned.sleep,
			},
			volumes,
		});

		if planned.routed {
			routes.push(Route {
				hostname: target.hostname.to_owned(),
				path_prefix: match planned.kind {
					ProcessKind::App => "/".to_owned(),
					ProcessKind::Service => service_prefix(&planned.name),
				},
				process: planned.name.clone(),
				strip_cookies: false,
				tls: Tls::Wildcard,
			});

			for domain in target.domains {
				routes.push(Route {
					hostname: domain.clone(),
					path_prefix: match planned.kind {
						ProcessKind::App => "/".to_owned(),
						ProcessKind::Service => service_prefix(&planned.name),
					},
					process: planned.name.clone(),
					strip_cookies: false,
					tls: Tls::Issued,
				});
			}
		}
	}

	let quota = Quota::for_processes(&processes);
	let spec = ReleaseSpec {
		environment: target.environment.clone(),
		deployment: target.deployment,
		processes,
		routes,
		policy: Policy {
			runtime_class: target.cluster.runtime_class.clone(),
			egress: Egress::Internet,
			quota,
			database: None,
		},
	};
	spec.validate()?;
	Ok(spec)
}

/// Whether `name` is a DNS label: 1 to 63 lower-case letters, digits, and
/// hyphens, starting with a letter and not ending with a hyphen.
#[must_use]
pub fn is_dns_label(name: &str) -> bool {
	(1..=MAX_LABEL_LEN).contains(&name.len())
		&& name.starts_with(|first: char| first.is_ascii_lowercase())
		&& !name.ends_with('-')
		&& name.chars().all(|character| {
			character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
		})
}

#[cfg(test)]
mod tests {
	use leptoskit_cloud_runtime::NamedRef;
	use leptoskit_config::LeptoskitConfig;
	use leptoskit_config::cloud::ResolvedCloud;

	use super::*;
	use crate::archive::Digest;
	use crate::deployments::ServiceFacts;
	use crate::workers::BuiltImage;
	use crate::workers::ProcessFacts;

	fn digest(byte: char) -> Digest {
		Digest::parse(&format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
	}

	fn environment() -> EnvironmentRef {
		let named = |slug: &str| {
			NamedRef {
				slug: slug.to_owned(),
				id: Uuid::now_v7(),
			}
		};
		EnvironmentRef {
			organization: named("acme"),
			project: named("notes"),
			environment: named("production"),
		}
	}

	fn resolve(toml: &str) -> ResolvedCloud {
		let app: LeptoskitConfig = toml::from_str(toml).expect("leptoskit.toml");
		app.cloud
			.clone()
			.unwrap_or_default()
			.resolved(&app)
			.expect("[cloud] resolves")
	}

	fn production(cloud: &ResolvedCloud) -> &ResolvedEnvironment {
		cloud
			.environments
			.iter()
			.find(|(name, _)| name.as_str() == "production")
			.map(|(_, environment)| environment)
			.expect("production")
	}

	fn output(names: &[&str]) -> BuildServerOutput {
		BuildServerOutput {
			image: BuiltImage {
				repository: "registry.example.com/acme/notes".to_owned(),
				digest: digest('a'),
			},
			processes: names
				.iter()
				.map(|name| {
					ProcessFacts {
						name: (*name).to_owned(),
						image: None,
						command: vec![format!("/bin/{name}")],
						args: Vec::new(),
						port: 8080,
						ready_path: "/readyz".to_owned(),
						live_path: "/healthz".to_owned(),
					}
				})
				.collect(),
			web_assets: None,
			migrations: Vec::new(),
			steps: Vec::new(),
		}
	}

	fn built(plan: &Plan, names: &[&str]) -> ReleaseSpec {
		let cluster = ClusterFacts {
			runtime_class: Some("gvisor".to_owned()),
			wakes_sleeping: true,
		};
		let spec = spec(
			plan,
			&output(names),
			&SpecTarget {
				environment: environment(),
				deployment: 1,
				hostname: "notes-acme.leptoskit.app",
				cluster: &cluster,
				variables: &BTreeMap::new(),
				secrets: &[],
				domains: &[],
			},
		)
		.expect("a spec");
		// Every spec validates, and its quota fits its processes at full
		// scale with their surge.
		assert_eq!(spec.validate(), Ok(()));
		assert_eq!(spec.policy.quota, Quota::for_processes(&spec.processes));
		spec
	}

	const WAKES: ClusterFacts = ClusterFacts {
		runtime_class: None,
		wakes_sleeping: true,
	};

	#[test]
	fn an_app_alone() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[cloud]\nproject = \
			 \"notes\"\n[cloud.environments.production]\nsize = \"s\"\nmin = 1\nmax = 3\nsleep = \
			 \"never\"\n",
		);
		let plan = plan(production(&cloud), None, &AppFacts::default(), &WAKES).unwrap();
		let spec = built(&plan, &["app"]);
		assert_eq!(spec.processes.len(), 1);
		let app = &spec.processes[0];
		assert_eq!(
			(app.scale.min, app.scale.max, app.scale.sleep),
			(1, 3, false)
		);
		assert_eq!(app.resources.memory_mib, 256);
		assert_eq!(spec.routes.len(), 1);
		assert_eq!(spec.routes[0].path_prefix, "/");
		assert_eq!(spec.policy.runtime_class.as_deref(), Some("gvisor"));
		// Three replicas and one surge.
		assert_eq!(spec.policy.quota.pods, 4);
	}

	#[test]
	fn an_app_and_two_services_one_public() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[services.search]\npackage = \"search\"\npublic = \
			 true\n[services.mailer]\npackage = \"mailer\"\n[cloud]\nproject = \
			 \"notes\"\n[cloud.environments.production]\nmin = 1\n",
		);
		let mut facts = AppFacts::default();
		facts.services.insert(
			"search".to_owned(),
			ServiceFacts {
				public: true,
				..ServiceFacts::default()
			},
		);
		facts
			.services
			.insert("mailer".to_owned(), ServiceFacts::default());
		let plan = plan(production(&cloud), None, &facts, &WAKES).unwrap();
		let spec = built(&plan, &["app", "search", "mailer"]);
		let names: Vec<&str> = spec.processes.iter().map(|p| p.name.as_str()).collect();
		assert_eq!(names, ["app", "mailer", "search"]);
		let routed: Vec<(&str, &str)> = spec
			.routes
			.iter()
			.map(|route| (route.process.as_str(), route.path_prefix.as_str()))
			.collect();
		assert_eq!(
			routed,
			[("app", "/"), ("search", "/_leptoskit/services/search/")]
		);
	}

	#[test]
	fn sqlite_gets_a_volume_and_one_replica() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[db]\ndialect = \"sqlite\"\n[cloud]\nproject = \
			 \"notes\"\n[cloud.environments.production]\nmin = 1\n",
		);
		let plan = plan(production(&cloud), None, &AppFacts::default(), &WAKES).unwrap();
		assert!(plan.sqlite);
		let spec = built(&plan, &["app"]);
		let app = &spec.processes[0];
		assert_eq!(app.scale.max, 1);
		assert_eq!(app.volumes.len(), 1);
		assert_eq!(
			app.env.get("DATABASE_URL").map(String::as_str),
			Some(SQLITE_URL)
		);
		// A process with a volume gets no surge replica.
		assert_eq!(spec.policy.quota.pods, 1);
		assert_eq!(spec.policy.quota.storage_gib, SQLITE_VOLUME_GIB);

		// The console's scale can't give it a second replica either.
		let mut stored = EnvironmentScale::sleeping();
		stored.app = ProcessScale {
			size: Size::M,
			min: 1,
			max: 4,
			sleep: Sleep::Never,
		};
		let plan = super::plan(
			production(&cloud),
			Some(&stored),
			&AppFacts::default(),
			&WAKES,
		)
		.unwrap();
		assert_eq!(plan.processes[0].max, 1);
		assert!(
			plan.fine_print
				.iter()
				.any(|line| line.contains("one replica, not 4"))
		);
	}

	#[test]
	fn postgres_is_refused_until_the_databases_slice() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[db]\ndialect = \"postgres\"\n[cloud]\nproject = \"notes\"\n",
		);
		assert_eq!(
			plan(production(&cloud), None, &AppFacts::default(), &WAKES),
			Err(PlanError::PostgresUnavailable)
		);
	}

	#[test]
	fn every_size_fits_its_quota() {
		for size in [Size::Xs, Size::S, Size::M, Size::L, Size::Xl] {
			let cloud = resolve(&format!(
				"[app]\nname = \"notes\"\n[cloud]\nproject = \
				 \"notes\"\n[cloud.environments.production]\nsize = \"{}\"\nmin = 2\nmax = 20\n",
				size.name()
			));
			let plan = plan(production(&cloud), None, &AppFacts::default(), &WAKES).unwrap();
			let spec = built(&plan, &["app"]);
			let app = &spec.processes[0];
			assert_eq!(app.resources.cpu_limit_millis, size.cpu_limit_millicores());
			assert_eq!(
				spec.policy.quota.memory_mib,
				21 * size.memory_mib(),
				"{size:?}: twenty replicas and a surge"
			);
		}
	}

	#[test]
	fn sleeping_needs_a_cluster_that_wakes_and_a_route() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[services.worker]\npackage = \"worker\"\n[cloud]\nproject = \
			 \"notes\"\n[cloud.environments.production]\nsleep = \"when-idle\"\n",
		);
		let plan = plan(production(&cloud), None, &AppFacts::default(), &WAKES).unwrap();
		let app = &plan.processes[0];
		assert!(app.sleep && app.min == 0);
		// The service isn't public, so nothing could wake it.
		let worker = &plan.processes[1];
		assert!(!worker.sleep && worker.min == 1);

		let asleep = ClusterFacts::default();
		let plan = super::plan(production(&cloud), None, &AppFacts::default(), &asleep).unwrap();
		assert!(!plan.processes[0].sleep);
		assert_eq!(plan.processes[0].min, 1);
		built(&plan, &["app", "worker"]);
	}

	#[test]
	fn isolated_public_services_wait_for_cookie_stripping() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[services.widget]\npackage = \"widget\"\npublic = \
			 true\n[cloud]\nproject = \"notes\"\n",
		);
		let mut facts = AppFacts::default();
		facts.services.insert(
			"widget".to_owned(),
			ServiceFacts {
				public: true,
				isolated: true,
				..ServiceFacts::default()
			},
		);
		assert_eq!(
			plan(production(&cloud), None, &facts, &WAKES),
			Err(PlanError::IsolatedPublicService {
				name: "widget".to_owned()
			})
		);
	}

	#[test]
	fn a_build_must_say_how_every_process_starts() {
		let cloud = resolve(
			"[app]\nname = \"notes\"\n[services.search]\npackage = \"search\"\n[cloud]\nproject = \
			 \"notes\"\n",
		);
		let plan = plan(production(&cloud), None, &AppFacts::default(), &WAKES).unwrap();
		let error = spec(
			&plan,
			&output(&["app"]),
			&SpecTarget {
				environment: environment(),
				deployment: 1,
				hostname: "notes-acme.leptoskit.app",
				cluster: &WAKES,
				variables: &BTreeMap::new(),
				secrets: &[],
				domains: &[],
			},
		)
		.unwrap_err();
		assert_eq!(
			error,
			SpecBuildError::MissingProcess {
				name: "search".to_owned()
			}
		);
	}

	#[test]
	fn hostnames_fit_dns_and_collapse_production() {
		assert_eq!(
			hostname_label("acme", "notes", "production", true),
			"notes-acme"
		);
		assert_eq!(
			hostname_label("acme", "notes", "staging", false),
			"staging-notes-acme"
		);

		let organization = "o".repeat(40);
		let project = "p".repeat(40);
		let long = hostname_label(&organization, &project, "staging", false);
		assert_eq!(long.len(), MAX_LABEL_LEN);
		assert!(is_dns_label(&long), "{long}");
		// Deterministic, and different for different environments.
		assert_eq!(
			long,
			hostname_label(&organization, &project, "staging", false)
		);
		assert_ne!(
			long,
			hostname_label(&organization, &project, "preview", false)
		);
		// A cut that lands on a hyphen doesn't leave two.
		let hyphens = hostname_label(
			"ab-cd-ef-gh-ij-kl-mn-op-qr-st",
			"x-y-z-a-b-c-d-e-f-g-h-i-j-k-l",
			"preview-env",
			false,
		);
		assert!(
			is_dns_label(&hyphens) && !hyphens.contains("--"),
			"{hyphens}"
		);

		let id = Uuid::now_v7();
		let fallback = fallback_label(&organization, &project, id);
		assert!(fallback.len() <= MAX_LABEL_LEN);
		assert!(is_dns_label(&fallback), "{fallback}");
		assert_ne!(
			fallback,
			fallback_label(&organization, &project, Uuid::now_v7())
		);
		assert_eq!(fallback, fallback_label(&organization, &project, id));
	}

	/// `toml` as `leptoskit.toml`, and the facts the CLI would report of it.
	fn reported(toml: &str, facts: AppFacts) -> (ResolvedCloud, DeployConfig) {
		let app: LeptoskitConfig = toml::from_str(toml).expect("leptoskit.toml");
		let config = DeployConfig {
			cloud: app.cloud.clone().unwrap_or_default(),
			app: facts,
		};
		(resolve(toml), config)
	}

	#[test]
	fn the_facts_rebuild_what_resolution_reads() {
		let mut facts = AppFacts {
			database: Some(DatabaseDialect::Sqlite),
			live: true,
			..AppFacts::default()
		};
		facts.services.insert(
			"search".to_owned(),
			ServiceFacts {
				public: true,
				database: true,
				..ServiceFacts::default()
			},
		);
		facts.services.insert(
			"mail".to_owned(),
			ServiceFacts {
				remote: true,
				isolated: true,
				..ServiceFacts::default()
			},
		);
		let app = app_config(&facts);

		assert_eq!(
			app.db.as_ref().map(|db| db.dialect),
			Some(DbDialect::Sqlite)
		);
		assert!(app.rpc.live_path.is_some());
		let search = &app.services["search"];
		assert_eq!(
			(search.url.as_deref(), search.public, search.database),
			(None, Some(true), Some(true))
		);
		let mail = &app.services["mail"];
		assert_eq!(mail.url.as_deref(), Some(REMOTE_SERVICE));
		assert_eq!(mail.trust, Some(ServiceTrust::Isolated));

		let bare = app_config(&AppFacts::default());
		assert!(bare.db.is_none() && bare.rpc.live_path.is_none() && bare.services.is_empty());
	}

	#[test]
	fn resolving_from_the_facts_agrees_with_the_file() {
		let mut facts = AppFacts {
			database: Some(DatabaseDialect::Sqlite),
			..AppFacts::default()
		};
		facts.services.insert(
			"search".to_owned(),
			ServiceFacts {
				public: true,
				..ServiceFacts::default()
			},
		);
		let (cloud, config) = reported(
			"[app]\nname = \"notes\"\n[db]\ndialect = \"sqlite\"\n[services.search]\npackage = \
			 \"search\"\npublic = true\n[cloud]\nproject = \
			 \"notes\"\n[cloud.environments.production]\nmin = 1\nsize = \"m\"\n",
			facts,
		);

		assert_eq!(
			super::resolve(&config, "production").as_ref(),
			Ok(production(&cloud))
		);
	}

	#[test]
	fn an_environment_the_table_does_not_list_gets_the_defaults() {
		let (cloud, config) = reported(
			"[app]\nname = \"notes\"\n[cloud]\nproject = \"notes\"\n",
			AppFacts::default(),
		);
		let staging = super::resolve(&config, "staging").expect("staging resolves");

		assert_eq!(&staging, production(&cloud));
		assert!(super::resolve(&config, "Not A Slug").is_err());
	}

	#[test]
	fn a_table_that_does_not_resolve_says_why() {
		let (_, config) = reported(
			"[app]\nname = \"notes\"\n[cloud]\nproject = \"notes\"\n",
			AppFacts::default(),
		);
		let mut config = config;
		config.app.database = Some(DatabaseDialect::Sqlite);
		config.cloud =
			toml::from_str("project = \"notes\"\n[database]\nbackups = \"continuous\"\n")
				.expect("[cloud]");
		let error = super::resolve(&config, "production")
			.unwrap_err()
			.to_string();

		assert!(
			error.starts_with("invalid_config: cloud.database.backups"),
			"{error}"
		);
	}
}
