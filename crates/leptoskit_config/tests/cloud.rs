//! `[cloud]`: defaults, resolution, and every validation rule.

use std::path::Path;

use leptoskit_config::ConfigError;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::cloud::Backups;
use leptoskit_config::cloud::CloudError;
use leptoskit_config::cloud::DEFAULT_CLOUD_URL;
use leptoskit_config::cloud::DatabaseInstance;
use leptoskit_config::cloud::DatabaseSize;
use leptoskit_config::cloud::Domain;
use leptoskit_config::cloud::Region;
use leptoskit_config::cloud::ResolvedCloud;
use leptoskit_config::cloud::ResolvedDatabase;
use leptoskit_config::cloud::ResolvedEnvironment;
use leptoskit_config::cloud::Size;
use leptoskit_config::cloud::Sleep;
use leptoskit_config::cloud::Slug;

fn load(raw: &str) -> Result<LeptoskitConfig, ConfigError> {
	LeptoskitConfig::from_toml(raw, Path::new("notes"))
}

fn resolved(raw: &str) -> ResolvedCloud {
	let config = load(raw).unwrap_or_else(|error| panic!("{raw}\n{error}"));
	let cloud = config.cloud.as_ref().expect("the file has [cloud]");
	cloud
		.resolved(&config)
		.expect("validation already resolved it")
}

fn environment<'a>(cloud: &'a ResolvedCloud, name: &str) -> &'a ResolvedEnvironment {
	cloud
		.environments
		.get(name)
		.unwrap_or_else(|| panic!("no environment {name}"))
}

/// The `[cloud]` problem `raw` has, with its message.
fn problem(raw: &str) -> (CloudError, String) {
	match load(raw) {
		Err(ConfigError::InvalidCloud(error)) => {
			let message = ConfigError::InvalidCloud(error.clone()).to_string();
			(error, message)
		}
		Err(other) => panic!("{raw}\nfailed with something else: {other}"),
		Ok(_) => panic!("{raw}\nloaded"),
	}
}

fn assert_message(raw: &str, expected: &str) {
	let (_, message) = problem(raw);
	assert!(
		message.contains(expected),
		"{raw}\nexpected `{expected}` in:\n{message}"
	);
}

const MANAGED_INFRA: &str = r#"
[db]

[infra]
name = "notes-production"
domain = "app.example.com"
compute = "hetzner"
location = "hel1"
server_type = "cx23"
instances = 1
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates.example.com"
updates_bucket = "notes-releases"
backup_bucket = "notes-backups"
image = "ghcr.io/team/notes"
registry = "ghcr.io"
registry_username = "team"
secrets = []
"#;

#[test]
fn only_a_project_runs_one_sleeping_xs_replica_in_production() {
	let cloud = resolved("[cloud]\nproject = \"notes\"\n");
	assert_eq!(cloud.project.as_ref().map(Slug::as_str), Some("notes"));
	assert_eq!(cloud.organization, None);
	assert_eq!(cloud.region, None);
	assert_eq!(cloud.url, DEFAULT_CLOUD_URL);
	assert_eq!(cloud.environments.len(), 1);

	let production = environment(&cloud, "production");
	assert_eq!(production.app.size, Size::Xs);
	assert_eq!(
		(production.app.scale.min(), production.app.scale.max()),
		(0, 1)
	);
	assert_eq!(production.app.sleep, Sleep::WhenIdle);
	assert_eq!(production.database, ResolvedDatabase::None);
	assert!(production.services.is_empty());
	assert_eq!(production.domains, Vec::<Domain>::new());
}

#[test]
fn an_app_with_db_gets_a_shared_postgres_database_by_default() {
	let cloud = resolved("[db]\n\n[cloud]\nproject = \"notes\"\n");
	let database = environment(&cloud, "production").database;
	assert_eq!(
		database,
		ResolvedDatabase::Postgres {
			size: DatabaseSize::Shared,
			backups: Backups::Daily,
		}
	);
	assert_eq!(database.dialect(), Some(DbDialect::Postgres));
	assert!(database.is_shared());
	assert_eq!(database.instance(), None);
}

#[test]
fn the_contract_example_resolves() {
	let cloud = resolved(
		r#"
[db]

[services.search]
package = "search"

[cloud]
project = "notes"
organization = "acme"
region = "eu-central"
url = "https://cloud.example.com"

[cloud.database]
size = "m"
backups = "continuous"

[cloud.environments.production]
size = "s"
min = 1
max = 3
sleep = "never"
domains = ["notes.example.com"]

[cloud.environments.production.services.search]
size = "m"
min = 1
max = 2

[cloud.environments.staging]
"#,
	);
	assert_eq!(cloud.organization.as_ref().map(Slug::as_str), Some("acme"));
	assert_eq!(
		cloud.region.as_ref().map(Region::as_str),
		Some("eu-central")
	);
	assert_eq!(cloud.url, "https://cloud.example.com");
	assert_eq!(
		cloud
			.environments
			.keys()
			.map(Slug::as_str)
			.collect::<Vec<_>>(),
		["production", "staging"]
	);

	let production = environment(&cloud, "production");
	assert_eq!(production.app.size, Size::S);
	assert_eq!(
		(production.app.scale.min(), production.app.scale.max()),
		(1, 3)
	);
	assert_eq!(production.app.sleep, Sleep::Never);
	assert_eq!(production.database.instance(), Some(DatabaseInstance::M));
	assert_eq!(
		production
			.domains
			.iter()
			.map(Domain::as_str)
			.collect::<Vec<_>>(),
		["notes.example.com"]
	);
	let search = production.services["search"];
	assert_eq!(search.process.size, Size::M);
	assert_eq!(
		(search.process.scale.min(), search.process.scale.max()),
		(1, 2)
	);
	assert_eq!(search.process.sleep, Sleep::Never);
	assert!(!search.database);

	let staging = environment(&cloud, "staging");
	assert_eq!(staging.app.sleep, Sleep::WhenIdle);
	assert_eq!(staging.services["search"].process.sleep, Sleep::WhenIdle);
}

#[test]
fn production_exists_when_only_other_environments_are_listed() {
	let cloud = resolved("[cloud.environments.staging]\nsize = \"s\"\n");
	assert_eq!(cloud.project, None);
	assert_eq!(environment(&cloud, "staging").app.size, Size::S);
	assert_eq!(environment(&cloud, "production").app.size, Size::Xs);
}

#[test]
fn sleep_and_min_fill_each_other_in() {
	let cloud = resolved(
		r#"
[cloud.environments.production]
sleep = "never"

[cloud.environments.staging]
min = 0
max = 2

[cloud.environments.preview]
min = 2
"#,
	);
	let production = environment(&cloud, "production");
	assert_eq!(production.app.scale.min(), 1);
	assert_eq!(production.app.scale.max(), 1);
	let staging = environment(&cloud, "staging");
	assert_eq!(staging.app.sleep, Sleep::WhenIdle);
	assert_eq!(staging.app.scale.max(), 2);
	let preview = environment(&cloud, "preview");
	assert_eq!(preview.app.sleep, Sleep::Never);
	assert_eq!(preview.app.scale.max(), 2, "max defaults to min");
}

#[test]
fn scales_out_of_bounds_name_the_table() {
	assert_message(
		"[cloud.environments.production]\nmin = 3\nmax = 2\n",
		"invalid [cloud]: cloud.environments.production: min = 3 is more than max = 2",
	);
	assert_message(
		"[cloud.environments.production]\nmax = 21\n",
		"cloud.environments.production: max = 21 is more than the 20 replicas",
	);
	assert_message(
		"[cloud.environments.production]\nmax = 0\n",
		"cloud.environments.production: max = 0 would never run the process",
	);
	assert_message(
		"[services.search]\npackage = \
		 \"search\"\n\n[cloud.environments.production.services.search]\nmin = 2\nmax = 1\n",
		"cloud.environments.production.services.search: min = 2 is more than max = 1",
	);
}

#[test]
fn sleep_must_agree_with_min() {
	assert_message(
		"[cloud.environments.production]\nsleep = \"when-idle\"\nmin = 1\n",
		r#"cloud.environments.production: sleep = "when-idle" needs min = 0, but min = 1"#,
	);
	assert_message(
		"[cloud.environments.production]\nsleep = \"never\"\nmin = 0\n",
		r#"cloud.environments.production: sleep = "never" needs min = 1 or more"#,
	);
}

#[test]
fn sqlite_runs_one_replica() {
	let cloud = resolved("[db]\ndialect = \"sqlite\"\n\n[cloud]\nproject = \"notes\"\n");
	let production = environment(&cloud, "production");
	assert_eq!(
		production.database,
		ResolvedDatabase::Sqlite {
			backups: Backups::Daily
		}
	);
	assert_eq!(production.database.dialect(), Some(DbDialect::Sqlite));
	assert_eq!(production.app.scale.max(), 1);

	let (error, message) = problem(
		"[db]\ndialect = \"sqlite\"\n\n[cloud.environments.production]\nmin = 1\nmax = 2\n",
	);
	assert!(matches!(error, CloudError::SqliteReplicas { max: 2, .. }));
	assert!(
		message.contains("cloud.environments.production.max = 2"),
		"{message}"
	);
	assert!(message.contains("Postgres"), "{message}");
}

#[test]
fn a_service_with_a_sqlite_database_runs_one_replica() {
	let base = "[services.search]\npackage = \"search\"\ndatabase = true\n\n";
	let (error, message) = problem(&format!(
		"{base}[cloud.environments.production.services.search]\nmax = 2\n"
	));
	assert!(matches!(error, CloudError::SqliteReplicas { max: 2, .. }));
	assert!(
		message.contains("cloud.environments.production.services.search.max = 2"),
		"{message}"
	);

	let postgres = resolved(&format!(
		"[db]\n\n{base}[cloud.environments.production.services.search]\nmax = 2\n"
	));
	let search = environment(&postgres, "production").services["search"];
	assert!(search.database);
	assert_eq!(search.process.scale.max(), 2);
}

#[test]
fn sqlite_takes_neither_a_dedicated_instance_nor_continuous_backups() {
	assert_message(
		"[db]\ndialect = \"sqlite\"\n\n[cloud.database]\nsize = \"m\"\n",
		r#"cloud.database.size = "m" is a dedicated Postgres instance"#,
	);
	assert_message(
		"[db]\ndialect = \"sqlite\"\n\n[cloud.database]\nbackups = \"continuous\"\n",
		r#"cloud.database.backups = "continuous" archives Postgres's write-ahead log"#,
	);
	let shared = resolved("[db]\ndialect = \"sqlite\"\n\n[cloud.database]\nsize = \"shared\"\n");
	assert!(matches!(
		environment(&shared, "production").database,
		ResolvedDatabase::Sqlite { .. }
	));
}

#[test]
fn a_database_table_needs_db() {
	assert_message(
		"[cloud.database]\nsize = \"s\"\n",
		"[cloud.database] places the app's database, but the app has no [db] table",
	);
}

#[test]
fn services_must_exist_and_run_here() {
	let (error, message) =
		problem("[cloud.environments.production.services.search]\nsize = \"m\"\n");
	assert!(matches!(error, CloudError::UnknownService { .. }));
	assert!(
		message.contains(
			"cloud.environments.production.services.search places a service that has no \
			 [services.search] table"
		),
		"{message}"
	);

	let (error, message) = problem(
		"[services.billing]\nurl = \"https://billing.example.com\"\n\n[cloud.environments.\
		 production.services.billing]\nsize = \"m\"\n",
	);
	assert!(matches!(error, CloudError::RemoteService { .. }));
	assert!(
		message.contains("[services.billing] sets url, so it runs elsewhere"),
		"{message}"
	);
}

#[test]
fn services_without_a_table_follow_the_app_and_remote_ones_are_left_out() {
	let raw = r#"
[services.search]
package = "search"

[services.billing]
url = "https://billing.example.com"

[cloud.environments.production]
min = 2
max = 4

[cloud.environments.staging]
"#;
	let cloud = resolved(raw);
	let production = environment(&cloud, "production");
	assert_eq!(production.services.keys().collect::<Vec<_>>(), ["search"]);
	let search = production.services["search"].process;
	assert_eq!((search.scale.min(), search.scale.max()), (1, 1));
	assert_eq!(search.sleep, Sleep::Never);
	assert_eq!(search.size, Size::Xs);

	let staging = environment(&cloud, "staging").services["search"].process;
	assert_eq!((staging.scale.min(), staging.scale.max()), (0, 1));
	assert_eq!(staging.sleep, Sleep::WhenIdle);
}

#[test]
fn an_app_that_serves_live_queries_stays_awake_unless_allowed() {
	for (live, signal) in [
		("[rpc]\nlive_path = \"/socket\"\n", "rpc.live_path"),
		(
			"[rpc.live]\nrelay_url_env = \"NATS_URL\"\n",
			"rpc.live.relay_url_env",
		),
		("[db]\nlive_capture = \"replication\"\n", "db.live_capture"),
	] {
		let defaults = resolved(&format!("{live}\n[cloud]\nproject = \"notes\"\n"));
		let production = environment(&defaults, "production");
		assert_eq!(production.app.sleep, Sleep::Never, "{live}");
		assert_eq!(production.app.scale.min(), 1, "{live}");

		for table in ["sleep = \"when-idle\"\n", "min = 0\n"] {
			let (error, message) =
				problem(&format!("{live}\n[cloud.environments.production]\n{table}"));
			assert!(
				matches!(error, CloudError::SleepWithLive { signal: found, .. } if found == signal),
				"{live}{table}{message}"
			);
			assert!(
				message.contains(&format!(
					"cloud.environments.production sleeps when idle, and sleeping closes every \
					 live connection, but {signal} says the app serves live queries"
				)),
				"{message}"
			);
			assert!(
				message.contains("allow_sleep_with_live = true"),
				"{message}"
			);
		}

		let allowed = resolved(&format!(
			"{live}\n[cloud.environments.production]\nallow_sleep_with_live = true\n"
		));
		assert_eq!(
			environment(&allowed, "production").app.sleep,
			Sleep::WhenIdle
		);
	}
}

#[test]
fn cloud_and_a_control_url_both_claim_deploy() {
	let cloud = resolved(&format!("{MANAGED_INFRA}\n[cloud]\nproject = \"notes\"\n"));
	assert!(cloud.environments.contains_key("production"));

	let with_control = MANAGED_INFRA.replace(
		"secrets = []",
		"secrets = []\ncontrol_url = \"https://deploy.example.com\"",
	);
	let (error, message) = problem(&format!("{with_control}\n[cloud]\nproject = \"notes\"\n"));
	assert_eq!(error, CloudError::ControlUrl);
	assert!(message.contains("infra.control_url"), "{message}");
}

#[test]
fn the_cloud_url_is_an_https_origin() {
	for url in [
		"https://cloud.example.com",
		"https://cloud.example.com/",
		"https://cloud.example.com:8443",
		"http://localhost:8080",
		"http://127.0.0.1:3000",
		"http://[::1]:3000",
	] {
		let cloud = resolved(&format!("[cloud]\nurl = \"{url}\"\n"));
		assert_eq!(cloud.url, url);
	}

	for (url, reason) in [
		("cloud.example.com", "isn't an address"),
		("http://cloud.example.com", "must use https://"),
		("ftp://cloud.example.com", "must be an https:// address"),
		(
			"https://me:secret@cloud.example.com",
			"must not carry credentials",
		),
		(
			"https://cloud.example.com/api",
			"no path, query, or fragment",
		),
		(
			"https://cloud.example.com/?team=1",
			"no path, query, or fragment",
		),
		(
			"https://cloud.example.com/#top",
			"no path, query, or fragment",
		),
	] {
		let raw = format!("[cloud]\nurl = \"{url}\"\n");
		assert_message(&raw, &format!("cloud.url `{url}` "));
		assert_message(&raw, reason);
	}
}

#[test]
fn domains_are_lower_case_hostnames_outside_the_cloud_s_own() {
	for (domain, expected) in [
		(
			"Notes.example.com",
			"cloud.environments.production.domains: `Notes.example.com` must be lower case",
		),
		("*.example.com", "`*.example.com` is a wildcard"),
		("localhost", "`localhost` isn't a fully qualified hostname"),
		(
			"notes.leptoskit.app",
			"`notes.leptoskit.app` is under `leptoskit.app`",
		),
		("bad_label.example.com", "has the label `bad_label`"),
	] {
		assert_message(
			&format!("[cloud.environments.production]\ndomains = [\"{domain}\"]\n"),
			expected,
		);
	}

	assert_message(
		"[cloud.environments.production]\ndomains = [\"a.example.com\", \"a.example.com\"]\n",
		"cloud.environments.production.domains lists `a.example.com` twice",
	);
	assert_message(
		"[cloud.environments.production]\ndomains = \
		 [\"a.example.com\"]\n\n[cloud.environments.staging]\ndomains = [\"a.example.com\"]\n",
		"`a.example.com` is in both cloud.environments.production.domains and \
		 cloud.environments.staging.domains",
	);
}

#[test]
fn environment_names_are_slugs() {
	let (error, message) = problem("[cloud.environments.Prod]\n");
	assert!(matches!(error, CloudError::EnvironmentName { .. }));
	assert!(
		message.contains("cloud.environments.Prod: an environment's name is a slug"),
		"{message}"
	);
}

#[test]
fn project_and_organization_are_slugs() {
	for raw in [
		"[cloud]\nproject = \"My Notes\"\n",
		"[cloud]\norganization = \"api\"\n",
		"[cloud]\nregion = \"EU\"\n",
		"[cloud]\nunknown = 1\n",
		"[cloud.environments.production]\nsize = \"xxl\"\n",
		"[cloud.environments.production]\nsleep = \"sometimes\"\n",
	] {
		assert!(
			matches!(load(raw), Err(ConfigError::Parse { .. })),
			"{raw} should fail to parse"
		);
	}
}

#[test]
fn without_cloud_nothing_serializes_and_with_it_the_table_round_trips() {
	let plain = load("[app]\nname = \"notes\"\n").unwrap();
	assert_eq!(plain.cloud, None);
	assert!(!plain.to_toml_string().unwrap().contains("cloud"));

	let raw = r#"
[db]

[cloud]
project = "notes"
region = "eu-central"

[cloud.environments.production]
size = "m"
min = 1
max = 2
domains = ["notes.example.com"]
"#;
	let config = load(raw).unwrap();
	let serialized = config.to_toml_string().unwrap();
	assert!(serialized.contains("[cloud]"), "{serialized}");
	let reloaded = load(&serialized).unwrap();
	assert_eq!(reloaded.cloud, config.cloud);
}

#[cfg(feature = "schema")]
#[test]
fn the_editor_schema_accepts_the_contract_and_refuses_what_parsing_refuses() {
	let schema = serde_json::to_value(leptoskit_config::schema::configuration(
		&leptoskit_config::schema_url(),
	))
	.unwrap();
	let validator = jsonschema::validator_for(&schema).unwrap();
	let document = |raw: &str| {
		let value: toml::Value = toml::from_str(raw).unwrap();
		serde_json::to_value(value).unwrap()
	};
	let valid = r#"
[cloud]
project = "notes"
organization = "acme"
region = "eu-central"

[cloud.database]
size = "shared"
backups = "daily"

[cloud.environments.production]
size = "s"
min = 1
max = 3
sleep = "never"
domains = ["notes.example.com"]

[cloud.environments.production.services.search]
size = "m"
min = 1
max = 2
"#;
	validator.validate(&document(valid)).unwrap();

	for changed in [
		valid.replace("\"notes\"", "\"api\""),
		valid.replace("\"notes\"", "\"no--tes\""),
		valid.replace("\"notes\"", "\"Notes\""),
		valid.replace("\"eu-central\"", "\"EU\""),
		valid.replace("size = \"s\"", "size = \"xxl\""),
		valid.replace("max = 3", "max = 21"),
		valid.replace("max = 3", "max = 0"),
		valid.replace("sleep = \"never\"", "sleep = \"sometimes\""),
		valid.replace("backups = \"daily\"", "backups = \"hourly\""),
		valid.replace("[cloud.database]", "[cloud.database]\nextra = 1"),
	] {
		assert!(
			validator.validate(&document(&changed)).is_err(),
			"the schema accepted:\n{changed}"
		);
	}
}
