//! The catalogue's validation and the estimate for a `[cloud]` table. Every
//! price here is made up for the test; the crate holds none.

use std::path::Path;

use leptoskit_cloud::pricing::Assumptions;
use leptoskit_cloud::pricing::Catalogue;
use leptoskit_cloud::pricing::CatalogueError;
use leptoskit_cloud::pricing::Estimate;
use leptoskit_cloud::pricing::EstimateInput;
use leptoskit_cloud::pricing::Meter;
use leptoskit_cloud::pricing::Percent;
use leptoskit_cloud::pricing::SECONDS_PER_MONTH;
use leptoskit_cloud::pricing::UnitPrice;
use leptoskit_cloud::pricing::Usage;
use leptoskit_cloud::pricing::estimate;
use leptoskit_cloud::size::DatabaseInstance;
use leptoskit_cloud::size::Size;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::cloud::ResolvedCloud;

const CATALOGUE: &str = r#"
currency = "usd"

[meters]
compute_seconds_xs = { minor = 300, per = 2628000 }
compute_seconds_s = { minor = 600, per = 2628000 }
database_hours_m = { minor = 2 }
database_storage_mb_months = { minor = 15, per = 1000 }
object_storage_mb_months = { minor = 2, per = 1000 }
egress_mb = { minor = 1, per = 1000 }
build_minutes_linux = { minor = 1, per = 2 }
custom_hostname_months = { minor = 10 }

[[plans]]
id = "hobby"
name = "Hobby"
monthly = 0
allowances = { build_minutes_linux = 100, database_storage_mb_months = 500 }

[[plans]]
id = "pro"
name = "Pro"
monthly = 2000
included_usage = 2000

[[plans]]
id = "scale"
name = "Scale"
"#;

fn catalogue() -> Catalogue {
	toml::from_str(CATALOGUE).expect("the test catalogue is valid")
}

fn cloud(raw: &str) -> ResolvedCloud {
	let config = LeptoskitConfig::from_toml(raw, Path::new("notes")).expect("valid");
	config
		.cloud
		.as_ref()
		.expect("[cloud]")
		.resolved(&config)
		.expect("resolves")
}

const APP: &str = r#"
[db]

[cloud.environments.production]
size = "s"
min = 1
max = 3
domains = ["notes.example.com"]

[cloud.environments.staging]
"#;

fn priced(plan: &str) -> Estimate {
	let catalogue = catalogue();
	let input = EstimateInput::from_cloud(&cloud(APP), &Assumptions::default());
	estimate(&catalogue, catalogue.plan(plan).expect("plan"), &input)
}

fn amounts(estimate: &Estimate) -> Vec<(&'static str, u64, u64)> {
	estimate
		.lines
		.iter()
		.map(|line| (line.meter.code(), line.billable, line.amount))
		.collect()
}

#[test]
fn the_input_counts_a_month_of_the_table() {
	let input = EstimateInput::from_cloud(&cloud(APP), &Assumptions::default());
	let usage: Vec<(Meter, u64)> = input.usage.iter().collect();
	assert_eq!(
		usage,
		[
			(Meter::Compute(Size::Xs), SECONDS_PER_MONTH / 10),
			(Meter::Compute(Size::S), SECONDS_PER_MONTH),
			(Meter::DatabaseStorage, 1000),
			(Meter::ObjectStorage, 1000),
			(Meter::Egress, 10_000),
			(Meter::LinuxBuildMinutes, 100),
			(Meter::CustomHostnames, 1),
		]
	);
	assert_eq!(
		input.assumptions,
		[
			"A month is 730 hours, and a process that sleeps when idle is awake 10% of it.",
			"production: the app's server runs 1 `s` replica all month; it can scale to 3, which \
			 costs more under load.",
			"production: one database holding 500 MB.",
			"production: 1 custom domain.",
			"staging: the app's server (`xs`) sleeps when idle, so one replica runs while it is \
			 awake.",
			"staging: one database holding 500 MB.",
			"The project keeps 1000 MB in object storage, sends 10000 MB out, and builds for 100 \
			 Linux and 0 macOS minutes a month.",
		]
	);
}

#[test]
fn dedicated_databases_and_services_are_counted() {
	let raw = r#"
[db]

[services.search]
package = "search"
database = true

[cloud.database]
size = "m"

[cloud.environments.production]
min = 2

[cloud.environments.production.services.search]
size = "m"
min = 1
"#;
	let input = EstimateInput::from_cloud(
		&cloud(raw),
		&Assumptions {
			awake: Percent::new(50).unwrap(),
			database_storage_mb: 100,
			object_storage_mb: 0,
			egress_mb: 0,
			linux_build_minutes: 0,
			macos_build_minutes: 0,
		},
	);
	let usage: Vec<(Meter, u64)> = input.usage.iter().collect();
	assert_eq!(
		usage,
		[
			(Meter::Compute(Size::Xs), 2 * SECONDS_PER_MONTH),
			(Meter::Compute(Size::M), SECONDS_PER_MONTH),
			(Meter::DatabaseHours(DatabaseInstance::M), 730),
			(Meter::DatabaseStorage, 200),
		]
	);
	assert!(
		input
			.assumptions
			.contains(&"production: the service `search` runs 1 `m` replica all month.".to_owned()),
		"{:#?}",
		input.assumptions
	);
	assert!(
		input
			.assumptions
			.contains(&"production: a dedicated `m` Postgres instance runs all month.".to_owned())
	);
}

#[test]
fn a_plan_with_included_usage_pays_for_it() {
	let pro = priced("pro");
	assert_eq!(
		amounts(&pro),
		[
			("compute_seconds_xs", 262_800, 30),
			("compute_seconds_s", 2_628_000, 600),
			("database_storage_mb_months", 1000, 15),
			("object_storage_mb_months", 1000, 2),
			("egress_mb", 10_000, 10),
			("build_minutes_linux", 100, 50),
			("custom_hostname_months", 1, 10),
		]
	);
	assert_eq!(pro.currency.as_str(), "usd");
	assert_eq!(pro.plan.as_str(), "pro");
	assert_eq!(pro.plan_price, Some(2000));
	assert_eq!(pro.usage, 717);
	assert_eq!(pro.included, 717);
	assert_eq!(pro.total, 2000);
}

#[test]
fn allowances_come_off_before_pricing_and_lines_round_half_up() {
	let hobby = priced("hobby");
	let storage = hobby
		.lines
		.iter()
		.find(|line| line.meter == Meter::DatabaseStorage)
		.expect("a storage line");
	assert_eq!(
		(storage.quantity, storage.allowance, storage.billable),
		(1000, 500, 500)
	);
	assert_eq!(storage.amount, 8, "7.5 cents rounds half up");
	let builds = hobby
		.lines
		.iter()
		.find(|line| line.meter == Meter::LinuxBuildMinutes)
		.expect("a builds line");
	assert_eq!((builds.billable, builds.amount), (0, 0));
	assert_eq!(hobby.usage, 30 + 600 + 8 + 2 + 10 + 10);
	assert_eq!(hobby.included, 0);
	assert_eq!(hobby.total, hobby.usage);
	assert_eq!(
		hobby.lines.iter().map(|line| line.amount).sum::<u64>(),
		hobby.usage,
		"the lines add up to the subtotal"
	);
}

#[test]
fn a_plan_by_agreement_and_an_unpriced_meter_say_so() {
	let catalogue = catalogue();
	let mut usage = Usage::default();
	usage.add(Meter::MacosBuildMinutes, 30);
	usage.add(Meter::Egress, 1500);
	usage.add(Meter::Egress, 0);
	let input = EstimateInput {
		usage,
		assumptions: vec!["Thirty macOS minutes.".into()],
	};
	let scale = estimate(&catalogue, catalogue.plan("scale").unwrap(), &input);
	assert_eq!(
		amounts(&scale),
		[("egress_mb", 1500, 2), ("build_minutes_macos", 30, 0)]
	);
	assert_eq!(scale.lines[1].unit_price, None);
	assert_eq!(scale.plan_price, None);
	assert_eq!(scale.total, 2);
	assert_eq!(
		scale.assumptions,
		[
			"Thirty macOS minutes.",
			"`build_minutes_macos` has no price in this catalogue, so this estimate counts it as \
			 free.",
			"The `scale` plan is priced by agreement, so the total is usage alone.",
		]
	);
}

#[test]
fn unit_prices_are_rational_and_round_half_up() {
	let half = UnitPrice::new(1, 2).unwrap();
	assert_eq!(
		[0, 1, 2, 3, 4].map(|quantity| half.amount(quantity)),
		[0, 1, 1, 2, 2]
	);
	let third = UnitPrice::new(1, 3).unwrap();
	assert_eq!([1, 2, 3].map(|quantity| third.amount(quantity)), [0, 1, 1]);
	assert_eq!(
		UnitPrice::new(u64::MAX, 1).unwrap().amount(u64::MAX),
		u64::MAX,
		"saturates"
	);
	assert!(UnitPrice::new(1, 0).is_err());
	assert!(toml::from_str::<Catalogue>(&CATALOGUE.replace("per = 1000 }", "per = 0 }")).is_err());
}

#[test]
fn catalogues_are_validated() {
	let cases = [
		(
			CATALOGUE.replace("currency = \"usd\"", "currency = \"USD\""),
			"isn't a currency",
		),
		(
			CATALOGUE.replace("id = \"scale\"", "id = \"pro\""),
			"two plans are called `pro`",
		),
		(
			CATALOGUE.replace("name = \"Scale\"", "name = \" \""),
			"the plan `scale` has no name",
		),
		(
			CATALOGUE.replace("build_minutes_linux = 100", "build_minutes_macos = 100"),
			"the plan `hobby` includes `build_minutes_macos`, which the catalogue doesn't price",
		),
		(
			CATALOGUE.replace("egress_mb = ", "egress_gb = "),
			"unknown meter `egress_gb`",
		),
		(
			CATALOGUE.replace("id = \"hobby\"", "id = \"Hobby\""),
			"a slug is lower-case letters",
		),
	];

	for (raw, expected) in cases {
		let error = toml::from_str::<Catalogue>(&raw).unwrap_err().to_string();
		assert!(error.contains(expected), "expected `{expected}` in {error}");
	}

	let empty = Catalogue::new(
		leptoskit_cloud::pricing::Currency::parse("eur").unwrap(),
		Vec::new(),
		std::collections::BTreeMap::new(),
	);
	assert_eq!(empty, Err(CatalogueError::NoPlans));
}

#[test]
fn meters_have_stable_codes() {
	for meter in Meter::ALL {
		assert_eq!(meter.code().parse::<Meter>(), Ok(meter));
		assert_eq!(
			serde_json::to_string(&meter).unwrap(),
			format!("\"{}\"", meter.code())
		);
		assert_ne!(meter.unit(), "");
	}

	let catalogue = catalogue();
	let json = serde_json::to_string(&catalogue).unwrap();
	let back: Catalogue = serde_json::from_str(&json).unwrap();
	assert_eq!(back, catalogue);
}

#[test]
fn percentages_stop_at_one_hundred() {
	assert_eq!(Percent::new(100).map(|percent| percent.of(730)), Ok(730));
	assert_eq!(
		Percent::new(10).map(|percent| percent.of(SECONDS_PER_MONTH)),
		Ok(262_800)
	);
	assert!(Percent::new(101).is_err());
	assert!(serde_json::from_str::<Percent>("101").is_err());
}
