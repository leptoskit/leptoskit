//! `[cloud]`: where `LeptosKit` Cloud runs the app, and how big.
//!
//! The rest of `leptoskit.toml` already says what the app is: `[db]` decides
//! whether there is a database and its dialect, and `[services.<name>]`
//! decides the processes. `[cloud]` adds only placement and scale, so nothing
//! is declared twice. Every key is optional; with only `project` set,
//! production runs one `xs` replica that sleeps when idle, on a shared
//! database when the app has `[db]`.
//!
//! ```toml
//! [cloud]
//! project = "notes"
//!
//! [cloud.database]
//! size = "shared"
//! backups = "daily"
//!
//! [cloud.environments.production]
//! size = "s"
//! min = 1
//! max = 3
//! domains = ["notes.example.com"]
//!
//! [cloud.environments.production.services.search]
//! size = "m"
//! ```
//!
//! [`CloudConfig::resolved`] fills the defaults and checks every rule; it is
//! what the cost estimate and the control plane read. Spend limits, secrets,
//! and billing never live here.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::DbDialect;
use crate::DbLiveCapture;
use crate::LeptoskitConfig;

mod domain;
mod size;
mod slug;

pub use domain::DEFAULT_APPS_DOMAIN;
pub use domain::Domain;
pub use domain::DomainError;
pub use domain::RESERVED_DOMAINS;
pub use size::Backups;
pub use size::DatabaseInstance;
pub use size::DatabaseSize;
pub use size::MAX_REPLICAS;
pub use size::REGION_PATTERN;
pub use size::Region;
pub use size::RegionError;
pub use size::Scale;
pub use size::ScaleError;
pub use size::Size;
pub use size::Sleep;
pub use size::SleepError;
pub use size::UnknownChoice;
pub use size::check_sleep;
pub use slug::RESERVED_SLUGS;
pub use slug::SLUG_MAX_LEN;
pub use slug::SLUG_MIN_LEN;
pub use slug::SLUG_PATTERN;
pub use slug::Slug;
pub use slug::SlugError;

/// The cloud `lk cloud` talks to when `[cloud] url` isn't set. It is a
/// configuration default, not a promise that the address serves anything
/// yet.
pub const DEFAULT_CLOUD_URL: &str = "https://cloud.leptoskit.dev";

/// The environment every project has, whether `[cloud.environments]` names
/// it or not.
pub const PRODUCTION: &str = "production";

/// `LeptosKit` Cloud: where the app runs and how big. `lk deploy` deploys to
/// the cloud while `project` is set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudConfig {
	/// The project's slug in the cloud, such as `notes`. `lk cloud launch`
	/// writes it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub project: Option<Slug>,
	/// The organisation that owns the project. Defaults to the signed-in
	/// person's own organisation.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub organization: Option<Slug>,
	/// Where the environments run, such as `eu-central`. Defaults to the
	/// cloud's default region.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub region: Option<Region>,
	/// The cloud's address, for a self-hosted cloud: `https://` with no path,
	/// or `http://` on localhost. Defaults to `https://cloud.leptoskit.dev`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub url: Option<String>,
	/// The database every environment gets when the app has `[db]`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub database: Option<CloudDatabase>,
	/// Application-level S3-compatible object storage buckets provisioned per
	/// environment (`[cloud.storage.buckets.<name>]`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub storage: Option<CloudStorage>,
	/// Each environment's size, scale, and domains, by name, such as
	/// `[cloud.environments.production]`. Production exists even when it
	/// isn't listed.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub environments: BTreeMap<String, CloudEnvironment>,
}

/// `[cloud.storage]`: application-level S3-compatible object storage buckets
/// provisioned per environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudStorage {
	/// Named buckets provisioned for the app (e.g.
	/// `[cloud.storage.buckets.uploads]`). Each bucket injects
	/// `LEPTOSKIT_BUCKET_<NAME>_ENDPOINT`, `LEPTOSKIT_BUCKET_<NAME>_NAME`, and
	/// credentials into the environment.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub buckets: BTreeMap<String, CloudBucket>,
}

/// `[cloud.storage.buckets.<name>]`: one application object storage bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudBucket {
	/// Whether objects in this bucket may be read publicly over HTTPS without
	/// presigned URLs (defaults to `false`, private).
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	pub public: bool,
	/// Optional soft size cap in gigabytes (must be >= 1 when set).
	#[cfg_attr(feature = "schema", schemars(range(min = 1)))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_size_gb: Option<u32>,
}

/// `[cloud.database]`: the database each environment gets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudDatabase {
	/// `shared` (a database on a shared Postgres cluster), or `s`, `m`, `l`
	/// (a dedicated Postgres instance with 1, 2, or 4 GiB). A SQLite app's
	/// database is a file on its volume, so it stays `shared`.
	pub size: DatabaseSize,
	/// `daily` snapshots, or `continuous` archiving of Postgres's
	/// write-ahead log.
	pub backups: Backups,
}

/// `[cloud.environments.<name>]`: one running copy of the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudEnvironment {
	/// How much CPU and memory each replica of the app's server gets: `xs`,
	/// `s`, `m`, `l`, or `xl`.
	pub size: Size,
	/// The fewest replicas that run. `0` sleeps when idle. Defaults to `0`,
	/// or `1` when the app serves live queries.
	#[cfg_attr(feature = "schema", schemars(range(max = 20)))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub min: Option<u32>,
	/// The most replicas that run under load, up to 20. Defaults to `min`,
	/// or `1`.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 20)))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max: Option<u32>,
	/// `never`, or `when-idle`, which needs `min = 0`. Follows `min` when
	/// unset.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sleep: Option<Sleep>,
	/// Lets an app that serves live queries sleep anyway. Sleeping closes
	/// every live connection, so screens reconnect when it wakes.
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	pub allow_sleep_with_live: bool,
	/// Custom hostnames the environment answers on, such as
	/// `notes.example.com`: lower case, no wildcards.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub domains: Vec<String>,
	/// Size and scale for the app's `[services.<name>]`, by the same name.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub services: BTreeMap<String, CloudService>,
}

/// `[cloud.environments.<env>.services.<name>]`: one service's process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct CloudService {
	/// How much CPU and memory each replica gets.
	pub size: Size,
	/// The fewest replicas that run; `0` sleeps when idle. Defaults to `1`
	/// when the app's server keeps a replica running, and `0` when it
	/// sleeps.
	#[cfg_attr(feature = "schema", schemars(range(max = 20)))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub min: Option<u32>,
	/// The most replicas that run under load, up to 20. Defaults to `min`,
	/// or `1`.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 20)))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max: Option<u32>,
}

/// `[cloud]` with every default filled in and every rule checked: what the
/// cost estimate and the control plane work from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedCloud {
	/// The project's slug, once `lk cloud launch` has written it.
	pub project: Option<Slug>,
	/// The organisation, when the table names one.
	pub organization: Option<Slug>,
	/// The region, when the table names one.
	pub region: Option<Region>,
	/// The cloud's address: `[cloud] url`, or [`DEFAULT_CLOUD_URL`].
	pub url: String,
	/// Application-level object storage buckets declared under
	/// `[cloud.storage]`.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub storage: BTreeMap<Slug, ResolvedBucket>,
	/// Every environment, production included.
	pub environments: BTreeMap<Slug, ResolvedEnvironment>,
}

/// One resolved application object storage bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResolvedBucket {
	/// Whether reads are public over HTTPS without presigned URLs.
	pub public: bool,
	/// Optional soft size cap in gigabytes.
	pub max_size_gb: Option<u32>,
}

/// One environment, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedEnvironment {
	/// The app's server.
	pub app: ResolvedProcess,
	/// Each `[services.<name>]` that runs as a process. Remote services run
	/// elsewhere and aren't here.
	pub services: BTreeMap<String, ResolvedService>,
	/// The environment's database.
	pub database: ResolvedDatabase,
	/// Custom hostnames, in the order the table lists them.
	pub domains: Vec<Domain>,
}

/// A process's size and scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResolvedProcess {
	/// CPU and memory per replica.
	pub size: Size,
	/// How many replicas run.
	pub scale: Scale,
	/// Whether it sleeps when idle; always agrees with `scale`.
	pub sleep: Sleep,
}

/// A service's process, and whether it has a database of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResolvedService {
	/// Size, scale, and sleep. A service sleeps exactly when `min = 0`.
	pub process: ResolvedProcess,
	/// Whether `[services.<name>] database = true`: the service gets a
	/// database of its own on the app's engine.
	pub database: bool,
}

/// The database decision for an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "dialect", rename_all = "lowercase")]
pub enum ResolvedDatabase {
	/// The app has no `[db]`.
	None,
	/// Postgres: shared, or a dedicated instance.
	Postgres {
		/// `shared`, or the dedicated instance's size.
		size: DatabaseSize,
		/// How often it is backed up.
		backups: Backups,
	},
	/// SQLite: a file on the app's persistent volume, so the app runs one
	/// replica at most.
	Sqlite {
		/// How often it is backed up.
		backups: Backups,
	},
}

impl ResolvedDatabase {
	/// The dialect, or `None` without a database.
	#[must_use]
	pub const fn dialect(self) -> Option<DbDialect> {
		match self {
			Self::None => None,
			Self::Postgres { .. } => Some(DbDialect::Postgres),
			Self::Sqlite { .. } => Some(DbDialect::Sqlite),
		}
	}

	/// Whether the database lives on the shared Postgres cluster.
	#[must_use]
	pub const fn is_shared(self) -> bool {
		matches!(
			self,
			Self::Postgres {
				size: DatabaseSize::Shared,
				..
			}
		)
	}

	/// The dedicated Postgres instance, if the environment has one.
	#[must_use]
	pub const fn instance(self) -> Option<DatabaseInstance> {
		match self {
			Self::Postgres { size, .. } => size.instance(),
			Self::None | Self::Sqlite { .. } => None,
		}
	}
}

/// Why `[cloud]` can't work as written. Each message names the table and
/// key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
	/// An environment's name isn't a slug.
	#[error("cloud.environments.{name}: an environment's name is a slug, and {reason}")]
	EnvironmentName {
		/// The name as written.
		name: String,
		/// The rule it breaks.
		reason: SlugError,
	},
	/// `min` and `max` can't be a scale.
	#[error("{at}: {error}")]
	Scale {
		/// The table, such as `cloud.environments.production`.
		at: String,
		/// The rule broken.
		error: ScaleError,
	},
	/// `sleep` disagrees with `min`.
	#[error("{at}: {error}")]
	Sleep {
		/// The table.
		at: String,
		/// The disagreement.
		error: SleepError,
	},
	/// The app would sleep while `[rpc]` or `[db]` says it serves live
	/// queries.
	#[error(
		"{at} sleeps when idle, and sleeping closes every live connection, but {signal} says the \
		 app serves live queries; set min = 1 to keep a replica running, or allow_sleep_with_live \
		 = true to let screens reconnect when it wakes"
	)]
	SleepWithLive {
		/// The table.
		at: String,
		/// The key that says the app serves live queries.
		signal: &'static str,
	},
	/// More than one replica of a process whose database is SQLite.
	#[error(
		"{at}.max = {max}: a SQLite database is a file on one volume, so the process runs one \
		 replica; set max = 1, or use Postgres to run more"
	)]
	SqliteReplicas {
		/// The table.
		at: String,
		/// The `max` asked for.
		max: u32,
	},
	/// Cloud settings for a service the app doesn't declare.
	#[error(
		"cloud.environments.{environment}.services.{service} places a service that has no \
		 [services.{service}] table"
	)]
	UnknownService {
		/// The environment.
		environment: String,
		/// The service named.
		service: String,
	},
	/// Cloud settings for a remote service.
	#[error(
		"cloud.environments.{environment}.services.{service}: [services.{service}] sets url, so \
		 it runs elsewhere and takes no cloud settings"
	)]
	RemoteService {
		/// The environment.
		environment: String,
		/// The service named.
		service: String,
	},
	/// `[cloud]` and `infra.control_url` both claim `lk deploy`.
	#[error(
		"[cloud] and infra.control_url both say where `lk deploy` sends the app; keep one of them"
	)]
	ControlUrl,
	/// `url` isn't a cloud's address.
	#[error("cloud.url `{url}` {reason}")]
	Url {
		/// The address as written.
		url: String,
		/// What is wrong with it.
		reason: &'static str,
	},
	/// A domain isn't a usable hostname.
	#[error("cloud.environments.{environment}.domains: {error}")]
	Domain {
		/// The environment.
		environment: String,
		/// What is wrong with the domain.
		error: DomainError,
	},
	/// One environment lists a domain twice.
	#[error("cloud.environments.{environment}.domains lists `{domain}` twice")]
	RepeatedDomain {
		/// The environment.
		environment: String,
		/// The domain.
		domain: String,
	},
	/// Two environments claim one domain.
	#[error(
		"`{domain}` is in both cloud.environments.{first}.domains and \
		 cloud.environments.{second}.domains; a hostname serves one environment"
	)]
	SharedDomain {
		/// The domain.
		domain: String,
		/// The environment that lists it first, by name.
		first: String,
		/// The other environment.
		second: String,
	},
	/// `[cloud.database]` without `[db]`.
	#[error("[cloud.database] places the app's database, but the app has no [db] table")]
	DatabaseWithoutDb,
	/// A dedicated instance for a SQLite database.
	#[error(
		r#"cloud.database.size = "{size}" is a dedicated Postgres instance, but [db] dialect = "sqlite" keeps the database in a file on the app's volume; leave size out, or switch to Postgres"#
	)]
	SqliteDatabaseSize {
		/// The size asked for.
		size: DatabaseSize,
	},
	/// Continuous backups for a SQLite database.
	#[error(
		r#"cloud.database.backups = "continuous" archives Postgres's write-ahead log, but [db] dialect = "sqlite"; use backups = "daily", or switch to Postgres"#
	)]
	SqliteContinuousBackups,
	/// A bucket's name isn't a slug.
	#[error("cloud.storage.buckets.{name}: a bucket name is a slug, and {reason}")]
	BucketName {
		/// The bucket name as written.
		name: String,
		/// The rule it breaks.
		reason: SlugError,
	},
	/// A bucket's `max_size_gb` is zero.
	#[error("cloud.storage.buckets.{name}.max_size_gb must be at least 1")]
	BucketSize {
		/// The bucket name as written.
		name: String,
	},
}

impl CloudConfig {
	/// Fills every default and checks every rule against the rest of the
	/// app's configuration.
	///
	/// # Errors
	///
	/// The first problem, as a [`CloudError`] naming the table and key.
	pub fn resolved(&self, app: &LeptoskitConfig) -> Result<ResolvedCloud, CloudError> {
		if app
			.infra
			.as_ref()
			.and_then(crate::infra::InfraConfig::as_managed)
			.is_some_and(|infra| infra.control_url.is_some())
		{
			return Err(CloudError::ControlUrl);
		}

		if let Some(url) = &self.url {
			check_url(url)?;
		}

		let mut storage = BTreeMap::new();
		if let Some(table) = &self.storage {
			for (name, bucket) in &table.buckets {
				let slug = Slug::parse(name).map_err(|reason| {
					CloudError::BucketName {
						name: name.clone(),
						reason,
					}
				})?;
				if bucket.max_size_gb == Some(0) {
					return Err(CloudError::BucketSize { name: name.clone() });
				}
				storage.insert(
					slug,
					ResolvedBucket {
						public: bucket.public,
						max_size_gb: bucket.max_size_gb,
					},
				);
			}
		}

		let database = self.resolved_database(app)?;
		let facts = Facts {
			app,
			database,
			live: live_signal(app),
		};
		let production = CloudEnvironment::default();
		let mut tables: Vec<(&str, &CloudEnvironment)> = self
			.environments
			.iter()
			.map(|(name, table)| (name.as_str(), table))
			.collect();

		if !self.environments.contains_key(PRODUCTION) {
			tables.push((PRODUCTION, &production));
		}

		let mut environments = BTreeMap::new();
		let mut claimed: BTreeMap<Domain, String> = BTreeMap::new();

		for (name, table) in tables {
			let slug = Slug::parse(name).map_err(|reason| {
				CloudError::EnvironmentName {
					name: name.to_owned(),
					reason,
				}
			})?;
			let environment = facts.environment(name, table)?;

			for domain in &environment.domains {
				if let Some(first) = claimed.insert(domain.clone(), name.to_owned()) {
					return Err(CloudError::SharedDomain {
						domain: domain.to_string(),
						first,
						second: name.to_owned(),
					});
				}
			}

			environments.insert(slug, environment);
		}

		Ok(ResolvedCloud {
			project: self.project.clone(),
			organization: self.organization.clone(),
			region: self.region.clone(),
			url: self
				.url
				.clone()
				.unwrap_or_else(|| DEFAULT_CLOUD_URL.to_owned()),
			storage,
			environments,
		})
	}

	fn resolved_database(&self, app: &LeptoskitConfig) -> Result<ResolvedDatabase, CloudError> {
		let table = self.database.clone().unwrap_or_default();

		let Some(db) = &app.db else {
			return match self.database {
				Some(_) => Err(CloudError::DatabaseWithoutDb),
				None => Ok(ResolvedDatabase::None),
			};
		};

		match db.dialect {
			DbDialect::Postgres => {
				Ok(ResolvedDatabase::Postgres {
					size: table.size,
					backups: table.backups,
				})
			}
			DbDialect::Sqlite => {
				if table.size.instance().is_some() {
					return Err(CloudError::SqliteDatabaseSize { size: table.size });
				}

				if table.backups == Backups::Continuous {
					return Err(CloudError::SqliteContinuousBackups);
				}

				Ok(ResolvedDatabase::Sqlite {
					backups: table.backups,
				})
			}
		}
	}
}

/// What resolving one environment needs to know about the whole app.
struct Facts<'a> {
	app: &'a LeptoskitConfig,
	database: ResolvedDatabase,
	/// The key that says the app serves live queries, if any does.
	live: Option<&'static str>,
}

impl Facts<'_> {
	fn environment(
		&self,
		name: &str,
		table: &CloudEnvironment,
	) -> Result<ResolvedEnvironment, CloudError> {
		let at = format!("cloud.environments.{name}");
		let app = self.app_process(&at, table)?;

		if matches!(self.database, ResolvedDatabase::Sqlite { .. }) && app.scale.max() > 1 {
			return Err(CloudError::SqliteReplicas {
				at,
				max: app.scale.max(),
			});
		}

		let services = self.services(name, &at, table, app.scale)?;
		let mut domains: Vec<Domain> = Vec::with_capacity(table.domains.len());

		for written in &table.domains {
			let domain = Domain::parse(written).map_err(|error| {
				CloudError::Domain {
					environment: name.to_owned(),
					error,
				}
			})?;

			if domains.contains(&domain) {
				return Err(CloudError::RepeatedDomain {
					environment: name.to_owned(),
					domain: written.clone(),
				});
			}

			domains.push(domain);
		}

		Ok(ResolvedEnvironment {
			app,
			services,
			database: self.database,
			domains,
		})
	}

	/// The app's server: `min` follows `sleep` when only `sleep` is written,
	/// and otherwise defaults to `0`, or `1` for an app that serves live
	/// queries and hasn't allowed sleeping with them.
	fn app_process(
		&self,
		at: &str,
		table: &CloudEnvironment,
	) -> Result<ResolvedProcess, CloudError> {
		let keeps_live = self.live.is_some() && !table.allow_sleep_with_live;
		let min = table.min.unwrap_or(match table.sleep {
			Some(Sleep::WhenIdle) => 0,
			Some(Sleep::Never) => 1,
			None => u32::from(keeps_live),
		});
		let scale = scale(at, min, table.max)?;
		let sleep = table.sleep.unwrap_or(Sleep::for_min(min));
		check_sleep(sleep, scale).map_err(|error| {
			CloudError::Sleep {
				at: at.to_owned(),
				error,
			}
		})?;

		if let (Sleep::WhenIdle, true, Some(signal)) = (sleep, keeps_live, self.live) {
			return Err(CloudError::SleepWithLive {
				at: at.to_owned(),
				signal,
			});
		}

		Ok(ResolvedProcess {
			size: table.size,
			scale,
			sleep,
		})
	}

	/// Every service that runs as a process. One without a table follows the
	/// app: it sleeps when the app sleeps and keeps one replica when the app
	/// keeps any.
	fn services(
		&self,
		environment: &str,
		at: &str,
		table: &CloudEnvironment,
		app: Scale,
	) -> Result<BTreeMap<String, ResolvedService>, CloudError> {
		for service in table.services.keys() {
			let Some(declared) = self.app.services.get(service) else {
				return Err(CloudError::UnknownService {
					environment: environment.to_owned(),
					service: service.clone(),
				});
			};

			if declared.url.is_some() {
				return Err(CloudError::RemoteService {
					environment: environment.to_owned(),
					service: service.clone(),
				});
			}
		}

		let engine = self
			.app
			.db
			.as_ref()
			.map_or(DbDialect::Sqlite, |db| db.dialect);
		let unplaced = CloudService::default();
		let mut services = BTreeMap::new();

		for (name, declared) in &self.app.services {
			if declared.url.is_some() {
				continue;
			}

			let placed = table.services.get(name).unwrap_or(&unplaced);
			let at = format!("{at}.services.{name}");
			let min = placed.min.unwrap_or(app.min().min(1));
			let scale = scale(&at, min, placed.max)?;
			let database = declared.database == Some(true);

			if database && engine == DbDialect::Sqlite && scale.max() > 1 {
				return Err(CloudError::SqliteReplicas {
					at,
					max: scale.max(),
				});
			}

			services.insert(
				name.clone(),
				ResolvedService {
					process: ResolvedProcess {
						size: placed.size,
						scale,
						sleep: Sleep::for_min(min),
					},
					database,
				},
			);
		}

		Ok(services)
	}
}

/// A scale from `min` and an optional `max`, which defaults to `min`, or
/// `1`.
fn scale(at: &str, min: u32, max: Option<u32>) -> Result<Scale, CloudError> {
	Scale::new(min, max.unwrap_or(min.max(1))).map_err(|error| {
		CloudError::Scale {
			at: at.to_owned(),
			error,
		}
	})
}

/// The key that says the app serves live queries, if any does.
///
/// `leptoskit.toml` has no switch for live queries: a table is live when
/// its `#[table]` says so, which only the code knows. These keys only make
/// sense for an app that serves them, so they are what configuration can
/// tell.
fn live_signal(app: &LeptoskitConfig) -> Option<&'static str> {
	if app.rpc.live.relay_url_env.is_some() {
		return Some("rpc.live.relay_url_env");
	}

	if app.rpc.live_path.is_some() {
		return Some("rpc.live_path");
	}

	if app
		.db
		.as_ref()
		.is_some_and(|db| db.live_capture == DbLiveCapture::Replication)
	{
		return Some("db.live_capture");
	}

	None
}

/// A cloud's address: `https://`, or `http://` on localhost, with a host and
/// nothing after it.
fn check_url(written: &str) -> Result<(), CloudError> {
	let problem = |reason| {
		Err(CloudError::Url {
			url: written.to_owned(),
			reason,
		})
	};
	let Ok(url) = url::Url::parse(written) else {
		return problem("isn't an address, such as `https://cloud.example.com`");
	};

	let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));

	match url.scheme() {
		"https" => {}
		"http" if local => {}
		"http" => return problem("must use https://; only localhost may use http://"),
		_ => return problem("must be an https:// address"),
	}

	if url.host_str().is_none_or(str::is_empty) {
		return problem("has no host");
	}

	if !url.username().is_empty() || url.password().is_some() {
		return problem("must not carry credentials; `lk cloud login` signs in");
	}

	if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
		return problem("must be just the origin, with no path, query, or fragment");
	}

	Ok(())
}
