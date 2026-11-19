//! Prices, usage, and the monthly estimate for a `[cloud]` table.
//!
//! Prices are data: a [`Catalogue`] of plans and per-meter unit prices,
//! which the operator configures. No price is written in this crate.
//!
//! Money is an integer count of the currency's minor units (cents for
//! `usd`), never a float. A [`UnitPrice`] is a rational, `minor` minor units
//! for every `per` units of its meter, so a $0.005 build minute is
//! `{ minor = 1, per = 2 }`. Each estimate line is its billable quantity
//! times the unit price, rounded half up to a whole minor unit; the usage
//! subtotal is the sum of the rounded lines, so the lines always add up to
//! it. Amounts saturate at `u64::MAX` minor units, far beyond any real bill.
//!
//! An estimate is a forecast, not a bill: [`EstimateInput::from_cloud`]
//! states every assumption it makes, and the [`Estimate`] carries them as
//! text beside the numbers.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use leptoskit_config::cloud::ResolvedCloud;
use leptoskit_config::cloud::ResolvedDatabase;
use leptoskit_config::cloud::ResolvedProcess;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

use crate::size::DatabaseInstance;
use crate::size::Size;
use crate::slug::Slug;

/// Hours in the average month: 365 × 24 ÷ 12.
pub const HOURS_PER_MONTH: u64 = 730;

/// Seconds in the average month.
pub const SECONDS_PER_MONTH: u64 = HOURS_PER_MONTH * 3600;

/// Something the cloud measures and charges for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Meter {
	/// Seconds one replica of this size runs.
	Compute(Size),
	/// Hours a dedicated Postgres instance of this size runs.
	DatabaseHours(DatabaseInstance),
	/// Database storage, in megabyte-months (10⁶ bytes kept for a month).
	DatabaseStorage,
	/// Object storage, in megabyte-months.
	ObjectStorage,
	/// Data sent out of the cloud, in megabytes.
	Egress,
	/// Minutes of Linux build machine time.
	LinuxBuildMinutes,
	/// Minutes of macOS build machine time.
	MacosBuildMinutes,
	/// Custom hostnames, per hostname-month.
	CustomHostnames,
}

/// A string that isn't a [`Meter`] code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown meter `{0}`")]
pub struct UnknownMeter(String);

impl Meter {
	/// Every meter, in the order estimates list them.
	pub const ALL: [Self; 14] = [
		Self::Compute(Size::Xs),
		Self::Compute(Size::S),
		Self::Compute(Size::M),
		Self::Compute(Size::L),
		Self::Compute(Size::Xl),
		Self::DatabaseHours(DatabaseInstance::S),
		Self::DatabaseHours(DatabaseInstance::M),
		Self::DatabaseHours(DatabaseInstance::L),
		Self::DatabaseStorage,
		Self::ObjectStorage,
		Self::Egress,
		Self::LinuxBuildMinutes,
		Self::MacosBuildMinutes,
		Self::CustomHostnames,
	];

	/// The stable code, as catalogues and Stripe meters name it.
	#[must_use]
	pub const fn code(self) -> &'static str {
		match self {
			Self::Compute(Size::Xs) => "compute_seconds_xs",
			Self::Compute(Size::S) => "compute_seconds_s",
			Self::Compute(Size::M) => "compute_seconds_m",
			Self::Compute(Size::L) => "compute_seconds_l",
			Self::Compute(Size::Xl) => "compute_seconds_xl",
			Self::DatabaseHours(DatabaseInstance::S) => "database_hours_s",
			Self::DatabaseHours(DatabaseInstance::M) => "database_hours_m",
			Self::DatabaseHours(DatabaseInstance::L) => "database_hours_l",
			Self::DatabaseStorage => "database_storage_mb_months",
			Self::ObjectStorage => "object_storage_mb_months",
			Self::Egress => "egress_mb",
			Self::LinuxBuildMinutes => "build_minutes_linux",
			Self::MacosBuildMinutes => "build_minutes_macos",
			Self::CustomHostnames => "custom_hostname_months",
		}
	}

	/// The unit one quantity of this meter counts.
	#[must_use]
	pub const fn unit(self) -> &'static str {
		match self {
			Self::Compute(_) => "second",
			Self::DatabaseHours(_) => "hour",
			Self::DatabaseStorage | Self::ObjectStorage => "megabyte-month",
			Self::Egress => "megabyte",
			Self::LinuxBuildMinutes | Self::MacosBuildMinutes => "minute",
			Self::CustomHostnames => "hostname-month",
		}
	}
}

impl fmt::Display for Meter {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.code())
	}
}

impl FromStr for Meter {
	type Err = UnknownMeter;

	fn from_str(code: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.into_iter()
			.find(|meter| meter.code() == code)
			.ok_or_else(|| UnknownMeter(code.to_owned()))
	}
}

impl Serialize for Meter {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.code())
	}
}

impl<'de> Deserialize<'de> for Meter {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let code = String::deserialize(deserializer)?;
		code.parse().map_err(serde::de::Error::custom)
	}
}

/// A currency, as its lower-case ISO 4217 code, such as `usd` or `eur`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Currency(String);

/// Why a string isn't a [`Currency`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
	"`{0}` isn't a currency: write its three-letter ISO 4217 code in lower case, such as `usd`"
)]
pub struct CurrencyError(String);

impl Currency {
	/// Checks `code`.
	///
	/// # Errors
	///
	/// [`CurrencyError`] unless it is three lower-case ASCII letters.
	pub fn parse(code: &str) -> Result<Self, CurrencyError> {
		if code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_lowercase()) {
			Ok(Self(code.to_owned()))
		} else {
			Err(CurrencyError(code.to_owned()))
		}
	}

	/// The code.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for Currency {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for Currency {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Currency {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let code = String::deserialize(deserializer)?;
		Self::parse(&code).map_err(serde::de::Error::custom)
	}
}

/// A price: `minor` minor units for every `per` units of a meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct UnitPrice {
	minor: u64,
	per: u64,
}

/// Why a price can't be a [`UnitPrice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a price is for at least one unit: `per` can't be 0")]
pub struct ZeroPer;

impl UnitPrice {
	/// `minor` minor units for every `per` units.
	///
	/// # Errors
	///
	/// [`ZeroPer`] when `per` is 0.
	pub const fn new(minor: u64, per: u64) -> Result<Self, ZeroPer> {
		if per == 0 {
			Err(ZeroPer)
		} else {
			Ok(Self { minor, per })
		}
	}

	/// The minor units charged for every [`per`](Self::per) units.
	#[must_use]
	pub const fn minor(self) -> u64 {
		self.minor
	}

	/// How many units [`minor`](Self::minor) pays for.
	#[must_use]
	pub const fn per(self) -> u64 {
		self.per
	}

	/// What `quantity` units cost, rounded half up to a whole minor unit.
	#[must_use]
	pub fn amount(self, quantity: u64) -> u64 {
		let product = u128::from(quantity) * u128::from(self.minor);
		let per = u128::from(self.per);
		let (whole, remainder) = (product / per, product % per);
		// `remainder < per <= u64::MAX`, so doubling it can't overflow.
		let rounded = if remainder * 2 >= per {
			whole + 1
		} else {
			whole
		};
		saturate(rounded)
	}
}

impl<'de> Deserialize<'de> for UnitPrice {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(deny_unknown_fields)]
		struct Fields {
			minor: u64,
			#[serde(default = "one")]
			per: u64,
		}

		const fn one() -> u64 {
			1
		}

		let fields = Fields::deserialize(deserializer)?;
		Self::new(fields.minor, fields.per).map_err(serde::de::Error::custom)
	}
}

fn saturate(value: u128) -> u64 {
	u64::try_from(value).unwrap_or(u64::MAX)
}

/// A plan: a monthly price, usage it includes, and free quantities per
/// meter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
	/// The plan's slug, such as `hobby` or `pro`.
	pub id: Slug,
	/// The name people see.
	pub name: String,
	/// The monthly price in minor units, or `None` for a plan priced by
	/// agreement.
	pub monthly: Option<u64>,
	/// Usage, in minor units, the monthly price already pays for.
	#[serde(default)]
	pub included_usage: u64,
	/// Quantities each month that cost nothing, per meter.
	#[serde(default)]
	pub allowances: BTreeMap<Meter, u64>,
	/// What an organisation on the plan may have at once.
	#[serde(default)]
	pub limits: Limits,
	/// The most a month's usage may cost, in minor units, beyond what the
	/// plan includes, for an organisation that hasn't set a cap of its own.
	/// `None` is no cap unless one is set.
	#[serde(default)]
	pub spend_cap: Option<u64>,
}

impl Plan {
	/// Whether the plan costs nothing a month, so nothing pays for usage
	/// beyond what it includes.
	#[must_use]
	pub const fn is_free(&self) -> bool {
		matches!(self.monthly, Some(0))
	}
}

/// What an organisation on a plan may have at once. `None` is no limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
	/// Projects.
	#[serde(default)]
	pub projects: Option<u64>,
	/// Custom domains, over every environment.
	#[serde(default)]
	pub custom_domains: Option<u64>,
}

/// The plans and prices the cloud offers, validated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CatalogueFields")]
pub struct Catalogue {
	currency: Currency,
	plans: Vec<Plan>,
	meters: BTreeMap<Meter, UnitPrice>,
}

/// A catalogue as written, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogueFields {
	currency: Currency,
	plans: Vec<Plan>,
	#[serde(default)]
	meters: BTreeMap<Meter, UnitPrice>,
}

impl TryFrom<CatalogueFields> for Catalogue {
	type Error = CatalogueError;

	fn try_from(fields: CatalogueFields) -> Result<Self, Self::Error> {
		Self::new(fields.currency, fields.plans, fields.meters)
	}
}

/// Why a catalogue can't work.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueError {
	/// No plans.
	#[error("a catalogue offers at least one plan")]
	NoPlans,
	/// Two plans with one ID.
	#[error("two plans are called `{0}`")]
	DuplicatePlan(Slug),
	/// A plan with an empty name.
	#[error("the plan `{0}` has no name")]
	UnnamedPlan(Slug),
	/// An allowance for a meter the catalogue doesn't price.
	#[error("the plan `{plan}` includes `{meter}`, which the catalogue doesn't price")]
	UnpricedAllowance {
		/// The plan.
		plan: Slug,
		/// The meter.
		meter: Meter,
	},
}

impl Catalogue {
	/// A catalogue of `plans` and `meters`, priced in `currency`.
	///
	/// # Errors
	///
	/// A [`CatalogueError`] when there is no plan, two plans share an ID, a
	/// plan has no name, or a plan includes a meter that has no price.
	pub fn new(
		currency: Currency,
		plans: Vec<Plan>,
		meters: BTreeMap<Meter, UnitPrice>,
	) -> Result<Self, CatalogueError> {
		if plans.is_empty() {
			return Err(CatalogueError::NoPlans);
		}

		let mut ids = BTreeSet::new();

		for plan in &plans {
			if !ids.insert(&plan.id) {
				return Err(CatalogueError::DuplicatePlan(plan.id.clone()));
			}

			if plan.name.trim().is_empty() {
				return Err(CatalogueError::UnnamedPlan(plan.id.clone()));
			}

			if let Some(meter) = plan
				.allowances
				.keys()
				.find(|meter| !meters.contains_key(meter))
			{
				return Err(CatalogueError::UnpricedAllowance {
					plan: plan.id.clone(),
					meter: *meter,
				});
			}
		}

		Ok(Self {
			currency,
			plans,
			meters,
		})
	}

	/// The currency every price is in.
	#[must_use]
	pub fn currency(&self) -> &Currency {
		&self.currency
	}

	/// Every plan, in the catalogue's order.
	#[must_use]
	pub fn plans(&self) -> &[Plan] {
		&self.plans
	}

	/// The plan called `id`.
	#[must_use]
	pub fn plan(&self, id: &str) -> Option<&Plan> {
		self.plans.iter().find(|plan| plan.id.as_str() == id)
	}

	/// The price of `meter`, if the catalogue has one.
	#[must_use]
	pub fn price(&self, meter: Meter) -> Option<UnitPrice> {
		self.meters.get(&meter).copied()
	}

	/// Every meter with a price, in [`Meter`] order.
	pub fn prices(&self) -> impl Iterator<Item = (Meter, UnitPrice)> + '_ {
		self.meters.iter().map(|(meter, price)| (*meter, *price))
	}
}

/// Quantities per meter, such as a month's usage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Usage(BTreeMap<Meter, u64>);

impl Usage {
	/// Adds `quantity` to `meter`.
	pub fn add(&mut self, meter: Meter, quantity: u64) {
		if quantity == 0 {
			return;
		}

		let total = self.0.entry(meter).or_default();
		*total = total.saturating_add(quantity);
	}

	/// The quantity of `meter`; 0 when there is none.
	#[must_use]
	pub fn get(&self, meter: Meter) -> u64 {
		self.0.get(&meter).copied().unwrap_or_default()
	}

	/// Every meter with a quantity, in [`Meter`] order.
	pub fn iter(&self) -> impl Iterator<Item = (Meter, u64)> + '_ {
		self.0.iter().map(|(meter, quantity)| (*meter, *quantity))
	}
}

/// A share of something, from 0 to 100 percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Percent(u8);

/// A percentage over 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0}% is more than 100%")]
pub struct PercentError(u8);

impl Percent {
	/// `value` percent.
	///
	/// # Errors
	///
	/// [`PercentError`] above 100.
	pub const fn new(value: u8) -> Result<Self, PercentError> {
		if value > 100 {
			Err(PercentError(value))
		} else {
			Ok(Self(value))
		}
	}

	/// The percentage.
	#[must_use]
	pub const fn value(self) -> u8 {
		self.0
	}

	/// This share of `whole`, rounded down.
	#[must_use]
	pub fn of(self, whole: u64) -> u64 {
		saturate(u128::from(whole) * u128::from(self.0) / 100)
	}
}

impl<'de> Deserialize<'de> for Percent {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Self::new(u8::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// What an estimate assumes beyond the `[cloud]` table: how long sleeping
/// processes are awake, and the storage, traffic, and builds a month brings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assumptions {
	/// The share of the month a process that sleeps when idle is awake.
	pub awake: Percent,
	/// Megabytes each database holds.
	pub database_storage_mb: u64,
	/// Megabytes kept in object storage.
	pub object_storage_mb: u64,
	/// Megabytes sent out each month.
	pub egress_mb: u64,
	/// Linux build minutes each month.
	pub linux_build_minutes: u64,
	/// macOS build minutes each month.
	pub macos_build_minutes: u64,
}

impl Default for Assumptions {
	/// Awake a tenth of the month, 500 MB per database, 1 GB of objects,
	/// 10 GB of egress, and 100 Linux build minutes.
	fn default() -> Self {
		Self {
			awake: Percent(10),
			database_storage_mb: 500,
			object_storage_mb: 1000,
			egress_mb: 10_000,
			linux_build_minutes: 100,
			macos_build_minutes: 0,
		}
	}
}

/// A month's expected usage, with what it assumes in words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstimateInput {
	/// Quantities per meter for one month.
	pub usage: Usage,
	/// Every assumption behind `usage`, as sentences.
	pub assumptions: Vec<String>,
}

impl EstimateInput {
	/// A month of the resolved `[cloud]` table: processes with `min >= 1`
	/// run `min` replicas all month (scaling up costs more and isn't
	/// counted); processes that sleep are awake for `assumptions.awake` of
	/// it, one replica at a time; dedicated databases run all month; each
	/// database holds `database_storage_mb`; each custom domain is one
	/// hostname-month; and storage, egress, and builds are as `assumptions`
	/// says, once for the project.
	#[must_use]
	pub fn from_cloud(cloud: &ResolvedCloud, assumptions: &Assumptions) -> Self {
		let mut input = Self::default();
		input.say(format!(
			"A month is {HOURS_PER_MONTH} hours, and a process that sleeps when idle is awake {}% \
			 of it.",
			assumptions.awake.value()
		));

		for (name, environment) in &cloud.environments {
			let name = name.as_str();
			input.process(name, "the app's server", environment.app, assumptions);

			for (service, resolved) in &environment.services {
				input.process(
					name,
					&format!("the service `{service}`"),
					resolved.process,
					assumptions,
				);
			}

			let databases = u64::from(environment.database != ResolvedDatabase::None)
				+ environment
					.services
					.values()
					.filter(|service| service.database)
					.count() as u64;

			if let Some(instance) = environment.database.instance() {
				input
					.usage
					.add(Meter::DatabaseHours(instance), HOURS_PER_MONTH);
				input.say(format!(
					"{name}: a dedicated `{instance}` Postgres instance runs all month."
				));
			}

			if databases > 0 {
				input.usage.add(
					Meter::DatabaseStorage,
					databases.saturating_mul(assumptions.database_storage_mb),
				);
				let megabytes = assumptions.database_storage_mb;
				input.say(if databases == 1 {
					format!("{name}: one database holding {megabytes} MB.")
				} else {
					format!("{name}: {databases} databases holding {megabytes} MB each.")
				});
			}

			let domains = environment.domains.len() as u64;

			if domains > 0 {
				input.usage.add(Meter::CustomHostnames, domains);
				input.say(format!(
					"{name}: {domains} custom domain{}.",
					plural(domains)
				));
			}
		}

		input
			.usage
			.add(Meter::ObjectStorage, assumptions.object_storage_mb);
		input.usage.add(Meter::Egress, assumptions.egress_mb);
		input
			.usage
			.add(Meter::LinuxBuildMinutes, assumptions.linux_build_minutes);
		input
			.usage
			.add(Meter::MacosBuildMinutes, assumptions.macos_build_minutes);
		input.say(format!(
			"The project keeps {} MB in object storage, sends {} MB out, and builds for {} Linux \
			 and {} macOS minutes a month.",
			assumptions.object_storage_mb,
			assumptions.egress_mb,
			assumptions.linux_build_minutes,
			assumptions.macos_build_minutes
		));

		input
	}

	fn process(
		&mut self,
		environment: &str,
		what: &str,
		process: ResolvedProcess,
		assumptions: &Assumptions,
	) {
		let size = process.size;
		let (min, max) = (process.scale.min(), process.scale.max());

		if process.scale.sleeps() {
			self.usage.add(
				Meter::Compute(size),
				assumptions.awake.of(SECONDS_PER_MONTH),
			);
			self.say(format!(
				"{environment}: {what} (`{size}`) sleeps when idle, so one replica runs while it \
				 is awake."
			));
		} else {
			self.usage.add(
				Meter::Compute(size),
				u64::from(min).saturating_mul(SECONDS_PER_MONTH),
			);
			let scaling = if max > min {
				format!("; it can scale to {max}, which costs more under load")
			} else {
				String::new()
			};
			self.say(format!(
				"{environment}: {what} runs {min} `{size}` replica{} all month{scaling}.",
				plural(u64::from(min))
			));
		}
	}

	fn say(&mut self, assumption: String) {
		self.assumptions.push(assumption);
	}
}

fn plural(count: u64) -> &'static str {
	if count == 1 { "" } else { "s" }
}

/// One meter's part of an estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EstimateLine {
	/// The meter.
	pub meter: Meter,
	/// The quantity expected in a month.
	pub quantity: u64,
	/// How much of it the plan includes for nothing.
	pub allowance: u64,
	/// What is left to pay for.
	pub billable: u64,
	/// The meter's price, or `None` when the catalogue has none.
	pub unit_price: Option<UnitPrice>,
	/// What the billable quantity costs, in minor units.
	pub amount: u64,
}

/// A month's expected cost on one plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
	/// The currency every amount is in.
	pub currency: Currency,
	/// The plan.
	pub plan: Slug,
	/// The plan's monthly price, or `None` when it is priced by agreement.
	pub plan_price: Option<u64>,
	/// One line per meter with a quantity, in [`Meter`] order.
	pub lines: Vec<EstimateLine>,
	/// The lines' amounts added up.
	pub usage: u64,
	/// The part of `usage` the plan's price already pays for.
	pub included: u64,
	/// What the month costs: the plan's price, plus usage beyond what it
	/// includes.
	pub total: u64,
	/// Everything the estimate assumes, as sentences.
	pub assumptions: Vec<String>,
}

/// Prices `input` on `plan` from `catalogue`.
///
/// A meter the catalogue doesn't price costs nothing, and the estimate says
/// so in its assumptions; so does a plan priced by agreement.
#[must_use]
pub fn estimate(catalogue: &Catalogue, plan: &Plan, input: &EstimateInput) -> Estimate {
	let mut assumptions = input.assumptions.clone();
	let mut lines = Vec::new();

	for (meter, quantity) in input.usage.iter() {
		let allowance = plan
			.allowances
			.get(&meter)
			.copied()
			.unwrap_or_default()
			.min(quantity);
		let billable = quantity - allowance;
		let unit_price = catalogue.price(meter);
		let amount = unit_price.map_or(0, |price| price.amount(billable));

		if unit_price.is_none() && billable > 0 {
			assumptions.push(format!(
				"`{meter}` has no price in this catalogue, so this estimate counts it as free."
			));
		}

		lines.push(EstimateLine {
			meter,
			quantity,
			allowance,
			billable,
			unit_price,
			amount,
		});
	}

	let usage = lines
		.iter()
		.fold(0_u64, |sum, line| sum.saturating_add(line.amount));
	let included = usage.min(plan.included_usage);

	if plan.monthly.is_none() {
		assumptions.push(format!(
			"The `{}` plan is priced by agreement, so the total is usage alone.",
			plan.id
		));
	}

	Estimate {
		currency: catalogue.currency.clone(),
		plan: plan.id.clone(),
		plan_price: plan.monthly,
		total: plan
			.monthly
			.unwrap_or_default()
			.saturating_add(usage - included),
		lines,
		usage,
		included,
		assumptions,
	}
}
