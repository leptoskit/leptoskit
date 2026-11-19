//! Placement and scale: how big each process is, how many run, whether they
//! sleep, and the database and region they use.
//!
//! These are the one definition of each choice: `[cloud]` parses them, and
//! `leptoskit_cloud` re-exports them for the control plane, the estimate,
//! and the dashboard.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

/// How much CPU and memory one replica of a process gets.
///
/// | Size | CPU request / limit (millicores) | Memory (MiB) |
/// | ---- | -------------------------------- | ------------ |
/// | xs   | 100 / 500                        | 128          |
/// | s    | 250 / 1000                       | 256          |
/// | m    | 500 / 2000                       | 512          |
/// | l    | 1000 / 2000                      | 1024         |
/// | xl   | 2000 / 4000                      | 2048         |
///
/// The numbers are the defaults; an operator may override them later.
#[derive(
	Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Size {
	/// 0.1 CPU guaranteed, bursting to 0.5, and 128 MiB.
	#[default]
	Xs,
	/// 0.25 CPU guaranteed, bursting to 1, and 256 MiB.
	S,
	/// 0.5 CPU guaranteed, bursting to 2, and 512 MiB.
	M,
	/// 1 CPU guaranteed, bursting to 2, and 1 GiB.
	L,
	/// 2 CPUs guaranteed, bursting to 4, and 2 GiB.
	Xl,
}

impl Size {
	/// Every size, smallest first.
	pub const ALL: [Self; 5] = [Self::Xs, Self::S, Self::M, Self::L, Self::Xl];

	/// The size's name in `leptoskit.toml`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Xs => "xs",
			Self::S => "s",
			Self::M => "m",
			Self::L => "l",
			Self::Xl => "xl",
		}
	}

	/// The CPU a replica is guaranteed, in millicores.
	#[must_use]
	pub const fn cpu_request_millicores(self) -> u32 {
		match self {
			Self::Xs => 100,
			Self::S => 250,
			Self::M => 500,
			Self::L => 1000,
			Self::Xl => 2000,
		}
	}

	/// The most CPU a replica may use, in millicores.
	#[must_use]
	pub const fn cpu_limit_millicores(self) -> u32 {
		match self {
			Self::Xs => 500,
			Self::S => 1000,
			Self::M | Self::L => 2000,
			Self::Xl => 4000,
		}
	}

	/// The memory a replica gets, in MiB. Request and limit are the same, so
	/// a replica is never killed for using what it was promised.
	#[must_use]
	pub const fn memory_mib(self) -> u32 {
		match self {
			Self::Xs => 128,
			Self::S => 256,
			Self::M => 512,
			Self::L => 1024,
			Self::Xl => 2048,
		}
	}
}

impl fmt::Display for Size {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

impl FromStr for Size {
	type Err = UnknownChoice;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.into_iter()
			.find(|size| size.name() == value)
			.ok_or_else(|| UnknownChoice::new("size", value, "xs, s, m, l, or xl"))
	}
}

/// A name that isn't one of a choice's values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} `{value}`; expected {expected}")]
pub struct UnknownChoice {
	kind: &'static str,
	value: String,
	expected: &'static str,
}

impl UnknownChoice {
	fn new(kind: &'static str, value: &str, expected: &'static str) -> Self {
		Self {
			kind,
			value: value.to_owned(),
			expected,
		}
	}
}

/// The most replicas a process may run.
pub const MAX_REPLICAS: u32 = 20;

/// How many replicas of a process run: at least `min`, at most `max`.
/// `min = 0` means the process sleeps when idle and wakes on a request.
///
/// Construct it with [`Scale::new`], which holds the rules: `min <= max`,
/// `1 <= max <= 20`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Scale {
	min: u32,
	max: u32,
}

/// Why a `min` and `max` can't be a [`Scale`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScaleError {
	/// `max = 0`: the process would never run.
	#[error("max = 0 would never run the process; set max to at least 1")]
	MaxZero,
	/// `max` above [`MAX_REPLICAS`].
	#[error("max = {max} is more than the {MAX_REPLICAS} replicas a process may run")]
	MaxTooHigh {
		/// The `max` asked for.
		max: u32,
	},
	/// `min` above `max`.
	#[error("min = {min} is more than max = {max}")]
	MinAboveMax {
		/// The `min` asked for.
		min: u32,
		/// The `max` asked for.
		max: u32,
	},
}

impl Scale {
	/// One replica that sleeps when idle: what a process gets when nothing
	/// says otherwise.
	pub const SLEEPING: Self = Self { min: 0, max: 1 };

	/// Checks `min` and `max` against the rules.
	///
	/// # Errors
	///
	/// The first rule broken, as a [`ScaleError`].
	pub const fn new(min: u32, max: u32) -> Result<Self, ScaleError> {
		if max == 0 {
			return Err(ScaleError::MaxZero);
		}

		if max > MAX_REPLICAS {
			return Err(ScaleError::MaxTooHigh { max });
		}

		if min > max {
			return Err(ScaleError::MinAboveMax { min, max });
		}

		Ok(Self { min, max })
	}

	/// The fewest replicas that run, even with no traffic.
	#[must_use]
	pub const fn min(self) -> u32 {
		self.min
	}

	/// The most replicas that run under load.
	#[must_use]
	pub const fn max(self) -> u32 {
		self.max
	}

	/// Whether the process sleeps when idle: `min = 0`.
	#[must_use]
	pub const fn sleeps(self) -> bool {
		self.min == 0
	}
}

impl<'de> Deserialize<'de> for Scale {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(deny_unknown_fields)]
		struct Raw {
			min: u32,
			max: u32,
		}

		let raw = Raw::deserialize(deserializer)?;
		Self::new(raw.min, raw.max).map_err(serde::de::Error::custom)
	}
}

/// Whether a process sleeps when nothing is using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Sleep {
	/// Always running, at `min >= 1` replicas.
	Never,
	/// Stops after a quiet spell and starts again on the next request, which
	/// waits for it. Needs `min = 0`, and closes live connections when it
	/// sleeps.
	WhenIdle,
}

impl Sleep {
	/// What a process with this many minimum replicas does: `min = 0` sleeps
	/// when idle, anything more never sleeps.
	#[must_use]
	pub const fn for_min(min: u32) -> Self {
		if min == 0 {
			Self::WhenIdle
		} else {
			Self::Never
		}
	}

	/// The choice's name in `leptoskit.toml`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Never => "never",
			Self::WhenIdle => "when-idle",
		}
	}
}

impl fmt::Display for Sleep {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

/// Why a [`Sleep`] choice disagrees with a [`Scale`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SleepError {
	/// `sleep = "when-idle"` with replicas that must always run.
	#[error(
		r#"sleep = "when-idle" needs min = 0, but min = {min} keeps replicas running; set min = 0 or sleep = "never""#
	)]
	WhenIdleNeedsZeroMin {
		/// The `min` asked for.
		min: u32,
	},
	/// `sleep = "never"` with no replica that must run.
	#[error(r#"sleep = "never" needs min = 1 or more, but min = 0 sleeps when idle"#)]
	NeverNeedsReplica,
}

/// Checks that `sleep` and `scale` say the same thing: sleeping when idle
/// is `min = 0`, never sleeping is `min >= 1`.
///
/// # Errors
///
/// A [`SleepError`] naming the disagreement.
pub const fn check_sleep(sleep: Sleep, scale: Scale) -> Result<(), SleepError> {
	match sleep {
		Sleep::WhenIdle if scale.min > 0 => {
			Err(SleepError::WhenIdleNeedsZeroMin { min: scale.min })
		}
		Sleep::Never if scale.min == 0 => Err(SleepError::NeverNeedsReplica),
		Sleep::WhenIdle | Sleep::Never => Ok(()),
	}
}

/// The database an environment gets when the app has a Postgres `[db]`.
#[derive(
	Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DatabaseSize {
	/// A database and login role of its own on a Postgres cluster shared with
	/// other environments.
	#[default]
	Shared,
	/// A dedicated Postgres instance with 1 GiB of memory.
	S,
	/// A dedicated Postgres instance with 2 GiB of memory.
	M,
	/// A dedicated Postgres instance with 4 GiB of memory.
	L,
}

impl DatabaseSize {
	/// The choice's name in `leptoskit.toml`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Shared => "shared",
			Self::S => "s",
			Self::M => "m",
			Self::L => "l",
		}
	}

	/// The dedicated instance this size runs, or `None` on the shared
	/// cluster.
	#[must_use]
	pub const fn instance(self) -> Option<DatabaseInstance> {
		match self {
			Self::Shared => None,
			Self::S => Some(DatabaseInstance::S),
			Self::M => Some(DatabaseInstance::M),
			Self::L => Some(DatabaseInstance::L),
		}
	}
}

impl fmt::Display for DatabaseSize {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

/// A dedicated Postgres instance: the sizes of [`DatabaseSize`] that aren't
/// shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseInstance {
	/// 1 GiB of memory.
	S,
	/// 2 GiB of memory.
	M,
	/// 4 GiB of memory.
	L,
}

impl DatabaseInstance {
	/// Every dedicated instance, smallest first.
	pub const ALL: [Self; 3] = [Self::S, Self::M, Self::L];

	/// The instance's name, as [`DatabaseSize`] spells it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::S => "s",
			Self::M => "m",
			Self::L => "l",
		}
	}

	/// The instance's memory, in MiB.
	#[must_use]
	pub const fn memory_mib(self) -> u32 {
		match self {
			Self::S => 1024,
			Self::M => 2048,
			Self::L => 4096,
		}
	}
}

impl fmt::Display for DatabaseInstance {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

/// How often the database is backed up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Backups {
	/// A snapshot every day.
	#[default]
	Daily,
	/// The write-ahead log is archived as it is written, so the database can
	/// be restored to any moment. Postgres only.
	Continuous,
}

impl Backups {
	/// The choice's name in `leptoskit.toml`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Daily => "daily",
			Self::Continuous => "continuous",
		}
	}
}

impl fmt::Display for Backups {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}

/// Where a project's environments run, such as `eu-central`. Regions are the
/// cloud operator's to define, so this checks only the shape: 2 to 32
/// lower-case letters, digits, and hyphens, starting with a letter and
/// ending with a letter or digit, with no `--`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Region(String);

/// The region grammar as a regular expression, without the ban on `--`.
pub const REGION_PATTERN: &str = "^[a-z][a-z0-9-]{0,30}[a-z0-9]$";

/// Why a string isn't a [`Region`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
	"`{0}` isn't a region name: use 2 to 32 lower-case letters, digits, and hyphens, such as \
	 `eu-central`"
)]
pub struct RegionError(String);

impl Region {
	/// Checks `value`'s shape.
	///
	/// # Errors
	///
	/// [`RegionError`] when it isn't a region name.
	pub fn parse(value: &str) -> Result<Self, RegionError> {
		let shaped = (2..=32).contains(&value.len())
			&& value.starts_with(|character: char| character.is_ascii_lowercase())
			&& !value.ends_with('-')
			&& !value.contains("--")
			&& value
				.chars()
				.all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'));

		if shaped {
			Ok(Self(value.to_owned()))
		} else {
			Err(RegionError(value.to_owned()))
		}
	}

	/// The region's name.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl FromStr for Region {
	type Err = RegionError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::parse(value)
	}
}

impl fmt::Display for Region {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for Region {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Region {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Self::parse(&value).map_err(serde::de::Error::custom)
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Region {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Region".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"description": "A region the cloud offers, such as `eu-central`.",
			"pattern": REGION_PATTERN,
			"not": { "pattern": "--" }
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sizes_follow_the_table() {
		let table: Vec<(Size, u32, u32, u32)> = Size::ALL
			.into_iter()
			.map(|size| {
				(
					size,
					size.cpu_request_millicores(),
					size.cpu_limit_millicores(),
					size.memory_mib(),
				)
			})
			.collect();
		assert_eq!(
			table,
			[
				(Size::Xs, 100, 500, 128),
				(Size::S, 250, 1000, 256),
				(Size::M, 500, 2000, 512),
				(Size::L, 1000, 2000, 1024),
				(Size::Xl, 2000, 4000, 2048),
			]
		);

		for size in Size::ALL {
			assert_eq!(size.name().parse::<Size>(), Ok(size));
			assert!(size.cpu_request_millicores() <= size.cpu_limit_millicores());
		}

		assert_eq!(
			"xxl".parse::<Size>().unwrap_err().to_string(),
			"unknown size `xxl`; expected xs, s, m, l, or xl"
		);
	}

	#[test]
	fn scales_hold_their_rules() {
		assert_eq!(Scale::new(0, 1).map(Scale::sleeps), Ok(true));
		assert_eq!(Scale::new(1, 3).map(Scale::sleeps), Ok(false));
		assert!(Scale::new(MAX_REPLICAS, MAX_REPLICAS).is_ok());
		assert_eq!(Scale::new(0, 0), Err(ScaleError::MaxZero));
		assert_eq!(
			Scale::new(1, MAX_REPLICAS + 1),
			Err(ScaleError::MaxTooHigh { max: 21 })
		);
		assert_eq!(
			Scale::new(3, 2),
			Err(ScaleError::MinAboveMax { min: 3, max: 2 })
		);
		assert_eq!(Scale::SLEEPING, Scale::new(0, 1).unwrap());
	}

	#[test]
	fn scales_validate_when_deserialized() {
		let scale: Scale = toml::from_str("min = 1\nmax = 2").unwrap();
		assert_eq!((scale.min(), scale.max()), (1, 2));
		let error = toml::from_str::<Scale>("min = 3\nmax = 2").unwrap_err();
		assert!(
			error.to_string().contains("min = 3 is more than max = 2"),
			"{error}"
		);
	}

	#[test]
	fn sleep_agrees_with_min() {
		let sleeping = Scale::new(0, 2).unwrap();
		let running = Scale::new(1, 2).unwrap();
		assert_eq!(check_sleep(Sleep::WhenIdle, sleeping), Ok(()));
		assert_eq!(check_sleep(Sleep::Never, running), Ok(()));
		assert_eq!(
			check_sleep(Sleep::WhenIdle, running),
			Err(SleepError::WhenIdleNeedsZeroMin { min: 1 })
		);
		assert_eq!(
			check_sleep(Sleep::Never, sleeping),
			Err(SleepError::NeverNeedsReplica)
		);
		assert_eq!(Sleep::for_min(0), Sleep::WhenIdle);
		assert_eq!(Sleep::for_min(2), Sleep::Never);
	}

	#[test]
	fn database_sizes_name_their_instances() {
		assert_eq!(DatabaseSize::Shared.instance(), None);
		assert_eq!(DatabaseSize::S.instance(), Some(DatabaseInstance::S));
		assert_eq!(
			DatabaseSize::L.instance().map(DatabaseInstance::memory_mib),
			Some(4096)
		);
		let memory: Vec<u32> = DatabaseInstance::ALL
			.into_iter()
			.map(DatabaseInstance::memory_mib)
			.collect();
		assert_eq!(memory, [1024, 2048, 4096]);
	}

	#[test]
	fn regions_are_shaped_names() {
		for region in ["eu-central", "us-east-1", "ap"] {
			assert_eq!(Region::parse(region).unwrap().as_str(), region);
		}

		for region in [
			"",
			"e",
			"EU",
			"eu--central",
			"eu-",
			"1eu",
			"eu_central",
			&"a".repeat(33),
		] {
			assert!(Region::parse(region).is_err(), "{region}");
		}
	}
}
